//! Per-request helper ownership, passed over the private agent/prompt socket.
//!
//! The prompt connects (so polkit sees its pidfd), but transfers a duplicate to
//! the agent BEFORE sending the PAM preamble. The agent never reads a live
//! conversation. After release or prompt death it shuts down input and drains
//! to EOF, holding the gate until then. A timeout is not proof of completion.
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use crate::helper::Conversation;

const TICK: Duration = Duration::from_millis(100);
const WAIT_LIMIT: Duration = Duration::from_secs(30);
/// A true value means cleanup failed without EOF. Fail closed until restart.
pub(crate) type Gate = Arc<Mutex<bool>>;

pub struct Monitor(UnixStream);
pub struct Ticket(UnixStream);

impl Monitor {
    pub fn new(stream: UnixStream) -> Self {
        Self(stream)
    }

    pub fn begin(&mut self, conv: &mut dyn Conversation) -> io::Result<Option<Ticket>> {
        self.0.write_all(b"B")?;
        let started = Instant::now();
        let mut waiting = false;
        loop {
            if conv.cancelled() {
                return Ok(None);
            }
            if ready(self.0.as_raw_fd(), TICK)? {
                let mut answer = [0];
                self.0.read_exact(&mut answer)?;
                if answer != *b"R" {
                    return Err(io::Error::other(
                        "previous authentication cleanup is incomplete",
                    ));
                }
                if waiting {
                    conv.cleanup_wait(false);
                }
                return Ok(Some(Ticket(self.0.try_clone()?)));
            }
            if !waiting && started.elapsed() >= Duration::from_millis(200) {
                conv.cleanup_wait(true);
                waiting = true;
            }
            if started.elapsed() >= WAIT_LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "previous authentication is still finishing; try again later",
                ));
            }
        }
    }
}

impl Ticket {
    pub fn track(&mut self, socket: &UnixStream) -> io::Result<()> {
        send_fd(&self.0, socket.as_raw_fd())?;
        self.0.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut answer = [0];
        let result = self.0.read_exact(&mut answer);
        self.0.set_read_timeout(None)?;
        result?;
        if answer == *b"R" {
            Ok(())
        } else {
            Err(io::Error::other("helper tracking refused"))
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let _ = self.0.write_all(b"D");
    }
}

/// Runs on a background thread. The private socket's EOF also covers crashes
/// and SIGKILL; tracking does not depend on the prompt's destructors running.
pub(crate) fn serve(mut control: UnixStream, gate: Gate) {
    while let Ok(Some((b'B', None))) = receive(&control) {
        let mut guard = loop {
            match gate.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(_)) => {
                    let _ = control.write_all(b"E");
                    return;
                }
                Err(TryLockError::WouldBlock) => {
                    // There can be no further command until the grant. Any
                    // input/EOF now means the waiting prompt has gone away.
                    if ready(control.as_raw_fd(), TICK).unwrap_or(true) {
                        return;
                    }
                }
            }
        };
        if *guard {
            let _ = control.write_all(b"E");
            return;
        }
        if control.write_all(b"R").is_err() {
            return;
        }
        let mut helper = None;
        let released = loop {
            match receive(&control) {
                Ok(Some((b'T', Some(fd)))) if helper.is_none() => {
                    helper = Some(UnixStream::from(fd));
                    if control.write_all(b"R").is_err() {
                        break false;
                    }
                }
                Ok(Some((b'D', None))) => break true,
                _ => break false,
            }
        };
        if let Some(socket) = helper
            && let Err(e) = drain(socket)
        {
            *guard = true;
            eprintln!("sudo-pop: helper cleanup could not be confirmed: {e}");
        }
        drop(guard);
        if !released {
            return;
        }
    }
}

fn drain(mut socket: UnixStream) -> io::Result<()> {
    socket.shutdown(std::net::Shutdown::Write)?;
    let mut bytes = [0u8; 4096];
    let start = Instant::now();
    let mut warned = false;
    loop {
        if !warned && start.elapsed() >= WAIT_LIMIT {
            eprintln!(
                "sudo-pop: previous helper is still running; new authentication remains blocked"
            );
            warned = true;
        }
        if !ready(socket.as_raw_fd(), TICK)? {
            continue;
        }
        match socket.read(&mut bytes) {
            Ok(0) => return Ok(()),
            Ok(_) => {} // Never forward late PAM messages or SUCCESS.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

fn ready(fd: RawFd, timeout: Duration) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll borrows one valid pollfd for this call only.
    let result = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    Ok(result > 0)
}

// Fixed-size, aligned ancillary storage. Each packet is exactly one byte;
// recvmsg never consumes the following command along with an fd transfer.
fn send_fd(stream: &UnixStream, fd: RawFd) -> io::Result<()> {
    let mut byte = b'T';
    let mut iov = libc::iovec {
        iov_base: (&mut byte as *mut u8).cast(),
        iov_len: 1,
    };
    let mut ancillary = [0usize; 8];
    // SAFETY: buffers are aligned, live for the syscall, and sized for one fd.
    let sent = unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = ancillary.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as _) as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as _) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        libc::sendmsg(stream.as_raw_fd(), &msg, libc::MSG_NOSIGNAL)
    };
    if sent == 1 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn receive(stream: &UnixStream) -> io::Result<Option<(u8, Option<OwnedFd>)>> {
    loop {
        let mut byte = 0u8;
        let mut iov = libc::iovec {
            iov_base: (&mut byte as *mut u8).cast(),
            iov_len: 1,
        };
        let mut ancillary = [0usize; 8];
        // SAFETY: recvmsg fills our valid buffers. Received descriptors become
        // OwnedFd immediately, including on malformed messages, avoiding leaks.
        unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = ancillary.as_mut_ptr().cast();
            msg.msg_controllen = std::mem::size_of_val(&ancillary);
            let n = libc::recvmsg(stream.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                return Ok(None);
            }
            let mut fds = Vec::new();
            let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
            while !cmsg.is_null() {
                if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                    let size = (*cmsg).cmsg_len.saturating_sub(libc::CMSG_LEN(0) as usize);
                    for i in 0..size / std::mem::size_of::<RawFd>() {
                        let fd =
                            std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>().add(i));
                        fds.push(OwnedFd::from_raw_fd(fd));
                    }
                }
                cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
            }
            if msg.msg_flags & libc::MSG_CTRUNC != 0 || fds.len() > 1 {
                return Err(io::Error::other("invalid helper tracking message"));
            }
            return Ok(Some((byte, fds.pop())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Conv {
        cancel: Arc<AtomicBool>,
        notices: Vec<bool>,
    }
    impl Conversation for Conv {
        fn ask(&mut self, _: &str, _: bool) -> Option<Secret> {
            None
        }
        fn info(&mut self, _: &str) {}
        fn error(&mut self, _: &str) {}
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::Relaxed)
        }
        fn cleanup_wait(&mut self, waiting: bool) {
            self.notices.push(waiting);
        }
    }
    fn conv() -> Conv {
        Conv {
            cancel: Arc::new(AtomicBool::new(false)),
            notices: vec![],
        }
    }
    fn start(gate: Gate) -> (Monitor, std::thread::JoinHandle<()>) {
        let (client, server) = UnixStream::pair().unwrap();
        (
            Monitor::new(client),
            std::thread::spawn(move || serve(server, gate)),
        )
    }
    fn grant(control: &mut UnixStream) {
        control.write_all(b"B").unwrap();
        let mut byte = [0];
        control.read_exact(&mut byte).unwrap();
        assert_eq!(byte, *b"R");
    }

    #[test]
    fn cancelled_helper_blocks_next_request_until_eof_even_after_late_success() {
        let gate = Arc::new(Mutex::new(false));
        let (mut first, first_thread) = start(gate.clone());
        let mut ticket = first.begin(&mut conv()).unwrap().unwrap();
        let (socket, mut fake_helper) = UnixStream::pair().unwrap();
        ticket.track(&socket).unwrap();
        drop(socket);
        // Prompt returns immediately. It neither waits for PAM nor keeps its
        // cookie-bearing process alive while cleanup holds the gate.
        drop(ticket);
        drop(first);
        fake_helper
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(
            fake_helper.read(&mut [0]).unwrap(),
            0,
            "helper input was half-closed"
        );
        let (mut next, next_thread) = start(gate);
        next.0.write_all(b"B").unwrap();
        assert!(!ready(next.0.as_raw_fd(), Duration::from_millis(150)).unwrap());
        fake_helper.write_all(b"SUCCESS\n").unwrap();
        assert!(
            !ready(next.0.as_raw_fd(), Duration::from_millis(150)).unwrap(),
            "SUCCESS is not EOF"
        );
        drop(fake_helper);
        next.0
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut ack = [0];
        next.0.read_exact(&mut ack).unwrap();
        assert_eq!(ack, *b"R");
        drop(next);
        first_thread.join().unwrap();
        next_thread.join().unwrap();
    }

    #[test]
    fn prompt_death_without_release_still_drains_its_tracked_helper() {
        let gate = Arc::new(Mutex::new(false));
        let (mut control, server) = UnixStream::pair().unwrap();
        let thread = std::thread::spawn(move || serve(server, gate));
        grant(&mut control);
        let (socket, mut helper) = UnixStream::pair().unwrap();
        send_fd(&control, socket.as_raw_fd()).unwrap();
        let mut ack = [0];
        control.read_exact(&mut ack).unwrap();
        assert_eq!(ack, *b"R");
        drop(socket);
        drop(control); // SIGKILL: no Ticket destructor / D.
        helper
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(helper.read(&mut [0]).unwrap(), 0);
        assert!(!thread.is_finished());
        drop(helper);
        thread.join().unwrap();
    }

    #[test]
    fn waiting_can_be_cancelled_without_starting_another_helper() {
        let gate = Arc::new(Mutex::new(false));
        let held = gate.lock().unwrap();
        let (mut monitor, thread) = start(gate.clone());
        let mut conv = conv();
        let cancel = conv.cancel.clone();
        let cancel_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(350));
            cancel.store(true, Ordering::Relaxed);
        });
        assert!(monitor.begin(&mut conv).unwrap().is_none());
        assert_eq!(conv.notices, [true]);
        drop(monitor);
        thread.join().unwrap(); // No gate release needed to notice cancellation.
        drop(held);
        cancel_thread.join().unwrap();
    }

    #[test]
    fn normal_release_allows_a_retry_on_the_same_control_connection() {
        let (mut monitor, thread) = start(Arc::new(Mutex::new(false)));
        let first = monitor.begin(&mut conv()).unwrap().unwrap();
        drop(first); // Fork helper, already reaped by Channel before this.
        let second = monitor.begin(&mut conv()).unwrap().unwrap();
        drop(second);
        drop(monitor);
        thread.join().unwrap();
    }

    #[test]
    fn unconfirmed_cleanup_refuses_new_authentication() {
        let (mut monitor, thread) = start(Arc::new(Mutex::new(true)));
        assert!(monitor.begin(&mut conv()).is_err());
        drop(monitor);
        thread.join().unwrap();
    }

    #[test]
    fn transferred_fds_are_cloexec_and_keep_the_socket_alive() {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let (socket, mut peer) = UnixStream::pair().unwrap();
        send_fd(&sender, socket.as_raw_fd()).unwrap();
        let (tag, fd) = receive(&receiver).unwrap().unwrap();
        assert_eq!(tag, b'T');
        let fd = fd.unwrap();
        assert_ne!(
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        drop(socket);
        peer.write_all(b"x").unwrap();
        let mut owned_socket = UnixStream::from(fd);
        let mut byte = [0];
        owned_socket.read_exact(&mut byte).unwrap();
        assert_eq!(byte, *b"x");
    }
}
