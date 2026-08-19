//! The D-Bus side: register, answer polkit, and nothing else.
//!
//! Two things have to happen at once here. A request blocks until the user
//! answers -- up to the caller's 25 seconds -- and `CancelAuthentication` for
//! that same request can arrive in the middle of it. So the methods are async
//! and the work between them is shared through locks rather than a call stack.
//!
//! The password is not in this file, and not in this process. Every request is
//! handed to a child (`--agent-prompt`) and the only thing that comes back is
//! an exit code.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_process::{Command, Stdio};
use futures_lite::io::AsyncWriteExt;
use zbus::interface;
use zbus::zvariant::OwnedValue;

use crate::prompt;

/// An identity polkit will accept, as it comes off the wire.
pub type Identity = (String, HashMap<String, OwnedValue>);

pub struct Agent {
    /// Unique bus name polkitd owns. Anything else calling us is not polkit,
    /// and is refused before a window can be drawn.
    pub polkitd: Mutex<String>,
    /// Quit after one handled request (spike runs).
    pub once: bool,
    /// One request at a time. A second one waits here rather than putting a
    /// second window on screen.
    pub turn: async_lock::Mutex<()>,
    /// Cookie -> pidfd of the child asking about it, so a cancel can signal the
    /// exact process. A pidfd, not a bare pid: once the child exits the pid can
    /// be recycled, and a stale kill would land on a stranger.
    pub running: Mutex<HashMap<String, i32>>,
    /// Cookies cancelled before their turn came up. The queued request checks
    /// this after taking the lock and ends without drawing a window.
    pub cancelled: Mutex<HashSet<String>>,
}

impl Agent {
    pub fn new(polkitd: String, once: bool) -> Self {
        Self {
            polkitd: Mutex::new(polkitd),
            once,
            turn: async_lock::Mutex::new(()),
            running: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashSet::new()),
        }
    }

    fn is_polkitd(&self, sender: Option<&str>) -> bool {
        let expected = self.polkitd.lock().ok();
        matches!((sender, expected), (Some(s), Some(e)) if s == e.as_str())
    }
}

/// Per-request tracing goes to the journal, so it is off unless asked for.
/// Only the security-relevant lines (a refused sender, an error) log always.
fn tracing() -> bool {
    std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty())
}

/// Send a signal through a pidfd. Immune to pid reuse: after the child exits
/// this fails with ESRCH rather than reaching a recycled pid.
fn pidfd_signal(pidfd: i32, sig: i32) {
    // SAFETY: a plain syscall with integer arguments; a null siginfo and no
    // flags. The fd is owned by us and valid while held in `running`.
    unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd as libc::c_long,
            sig as libc::c_long,
            0 as libc::c_long,
            0 as libc::c_long,
        );
    }
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<Identity>,
    ) -> zbus::fdo::Result<()> {
        let started = std::time::Instant::now();
        if tracing() {
            println!("\n== BeginAuthentication ==  {}", crate::stamp());
            println!("  action_id  : {action_id}");
            println!("  message    : {message}");
            println!("  icon_name  : {icon_name}");
            println!("  details    : {details:?}");
        }

        // Only polkit may ask us to prompt. Without this any process on the bus
        // can put an attacker-worded dialog on screen, learn whether the
        // password was right, and burn the shared faillock budget.
        let sender = header.sender().map(|s| s.as_str().to_owned());
        if !self.is_polkitd(sender.as_deref()) {
            eprintln!("sudo-pop: REJECTED begin from {sender:?}: not polkitd");
            return Err(zbus::fdo::Error::AccessDenied("not polkit".into()));
        }

        let Some((uid, name)) = crate::choose_identity(&identities) else {
            return Err(zbus::fdo::Error::Failed("no usable identity".into()));
        };
        let subject_pid: u32 = details
            .get("polkit.subject-pid")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if tracing() {
            println!("  chosen     : {name} (uid {uid}), subject pid {subject_pid}");
        }

        // Queue: one window at a time. Held for the whole request.
        let _turn = self.turn.lock().await;

        // A cancel may have arrived while this waited its turn. If so, end it
        // now rather than opening a window for a request polkitd has dropped.
        if self
            .cancelled
            .lock()
            .map(|mut set| set.remove(&cookie))
            .unwrap_or(false)
        {
            if tracing() {
                println!("  cancelled before it started");
            }
            return Ok(());
        }

        let code = self.ask(&name, &cookie, subject_pid, &message).await;

        // Drop any late cancel marker so the set cannot grow without bound.
        if let Ok(mut set) = self.cancelled.lock() {
            set.remove(&cookie);
        }

        if tracing() {
            println!(
                "  exit {code}  ({} 초 경과, {})",
                started.elapsed().as_secs_f32().round(),
                crate::stamp()
            );
        }
        if self.once {
            crate::HANDLED.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        match code {
            prompt::EXIT_SUCCESS => Ok(()),
            // Cancelled, or refused before any prompt. Both end the request
            // normally: an error would have polkitd hand it straight back and
            // the window would reopen forever.
            prompt::EXIT_CANCELLED => Ok(()),
            _ => Err(zbus::fdo::Error::Failed("authentication failed".into())),
        }
    }

    /// polkit gives up on a request -- the caller stopped waiting, or the
    /// action was withdrawn. Close the window that belongs to that cookie.
    async fn cancel_authentication(&self, cookie: String) {
        if tracing() {
            println!("== CancelAuthentication ==  {}", crate::stamp());
        }
        // Signal under the lock so `ask` cannot close the pidfd underneath us.
        let mut signalled = false;
        if let Ok(map) = self.running.lock()
            && let Some(&pidfd) = map.get(&cookie)
        {
            pidfd_signal(pidfd, libc::SIGTERM);
            signalled = true;
        }
        if signalled {
            if tracing() {
                println!("  closed the prompt for that cookie");
            }
            return;
        }
        // Not started yet (still queued) or already gone. Remember it so the
        // queued begin_authentication ends without a window when its turn comes.
        if let Ok(mut set) = self.cancelled.lock() {
            if set.len() > 256 {
                set.clear();
            }
            set.insert(cookie);
        }
        if tracing() {
            println!("  nothing running yet; marked cancelled");
        }
    }
}

impl Agent {
    /// Run one request in a child and wait for its exit code.
    async fn ask(&self, username: &str, cookie: &str, subject_pid: u32, message: &str) -> i32 {
        let Ok(exe) = std::env::current_exe() else {
            return prompt::EXIT_FAILED;
        };

        let child = Command::new(exe)
            .arg("--agent-prompt")
            .env("SUDO_POP_USER", username)
            .env("SUDO_POP_SUBJECT_PID", subject_pid.to_string())
            .env("SUDO_POP_MESSAGE", message)
            .stdin(Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                eprintln!("sudo-pop: cannot start the prompt: {e}");
                return prompt::EXIT_FAILED;
            }
        };

        // A pidfd for the child, so a cancel signals this exact process even
        // after its pid could be recycled. If it cannot be opened the cancel
        // path falls back to the 30s window backstop rather than a stale kill.
        let pid = child.id();
        // SAFETY: plain syscall; the child is alive here, freshly spawned.
        let pidfd = unsafe {
            libc::syscall(libc::SYS_pidfd_open, pid as libc::c_long, 0 as libc::c_long)
        } as i32;
        if pidfd >= 0 {
            if let Ok(mut map) = self.running.lock() {
                map.insert(cookie.to_owned(), pidfd);
            }
        } else {
            eprintln!("sudo-pop: cannot open pidfd for the prompt child");
        }

        // The cookie goes down a pipe, not through argv or the environment:
        // both are readable by anything that can see the process.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(format!("{cookie}\n").as_bytes()).await;
            let _ = stdin.flush().await;
        }

        let code = match child.status().await {
            Ok(status) => status.code().unwrap_or(prompt::EXIT_CANCELLED),
            Err(e) => {
                eprintln!("sudo-pop: prompt did not finish: {e}");
                prompt::EXIT_FAILED
            }
        };

        // Remove and close under the lock: a concurrent cancel either
        // signalled before this, or finds nothing -- never an fd we are closing.
        if let Ok(mut map) = self.running.lock()
            && let Some(fd) = map.remove(cookie)
        {
            // SAFETY: our own pidfd, opened above and not closed elsewhere.
            unsafe { libc::close(fd) };
        }
        code
    }
}
