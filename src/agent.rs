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

    /// Remember a cookie cancelled before its request started, so the queued
    /// begin_authentication can end without drawing. Bounded so a stream of
    /// cancels for cookies that never begin cannot grow it without limit.
    fn remember_cancelled(&self, cookie: String) {
        if let Ok(mut set) = self.cancelled.lock() {
            if set.len() > 256 {
                set.clear();
            }
            set.insert(cookie);
        }
    }

    /// Consume a pending cancel for `cookie`, returning whether there was one.
    fn take_cancelled(&self, cookie: &str) -> bool {
        self.cancelled
            .lock()
            .map(|mut set| set.remove(cookie))
            .unwrap_or(false)
    }
}

/// How long the caller waits for the whole authentication before giving up.
///
/// **Not ours.** It is the default method-call timeout of the bus library the
/// caller uses -- 25 seconds for sd-bus (`run0`, `systemctl`) and for GDBus
/// (udisks, NetworkManager), measured at 25.03 s end to end. A caller that
/// passes its own timeout is not covered, so what the window draws from this is
/// a countdown, not a promise: our own backstop stays a little longer and the
/// request really ends when polkitd cancels it.
const CALLER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

/// Per-request tracing goes to the journal, so it is off unless asked for.
/// Only the security-relevant lines (a refused sender, an error) log always.
fn tracing() -> bool {
    std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty())
}

/// A child exit code that ends the request without a D-Bus error. Success and
/// cancellation (and a refusal before any prompt, which the child also reports
/// as cancelled) return normally; anything else becomes an error, and an error
/// makes polkitd re-issue the request -- the reopen-forever trap of §3-3.
fn is_ok_exit(code: i32) -> bool {
    matches!(code, prompt::EXIT_SUCCESS | prompt::EXIT_CANCELLED)
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
        if self.take_cancelled(&cookie) {
            if tracing() {
                println!("  cancelled before it started");
            }
            return Ok(());
        }

        // Measured from the top of this method, not from the child's start:
        // a request that waited its turn in the queue has already spent some
        // of the caller's patience, and the window must not offer it again.
        let left = CALLER_TIMEOUT.saturating_sub(started.elapsed());
        if tracing() {
            println!("  left       : {} ms", left.as_millis());
        }
        let code = self.ask(&name, &cookie, subject_pid, &message, left).await;

        // Drop any late cancel marker so the set cannot grow without bound.
        let _ = self.take_cancelled(&cookie);

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

        if is_ok_exit(code) {
            // Success, cancel, or a refusal before any prompt all end the
            // request normally. An error would have polkitd hand it straight
            // back and the window would reopen forever.
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed("authentication failed".into()))
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
        self.remember_cancelled(cookie);
        if tracing() {
            println!("  nothing running yet; marked cancelled");
        }
    }
}

impl Agent {
    /// Run one request in a child and wait for its exit code.
    async fn ask(
        &self,
        username: &str,
        cookie: &str,
        subject_pid: u32,
        message: &str,
        left: std::time::Duration,
    ) -> i32 {
        let Ok(exe) = std::env::current_exe() else {
            return prompt::EXIT_FAILED;
        };

        let child = Command::new(exe)
            .arg("--agent-prompt")
            .env("SUDO_POP_USER", username)
            .env("SUDO_POP_SUBJECT_PID", subject_pid.to_string())
            .env("SUDO_POP_MESSAGE", message)
            .env("SUDO_POP_LEFT_MS", left.as_millis().to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_polkitd_owner_passes_the_sender_check() {
        let agent = Agent::new(":1.12".to_owned(), false);
        assert!(agent.is_polkitd(Some(":1.12")));
        assert!(!agent.is_polkitd(Some(":1.99")), "a different name is not polkit");
        assert!(!agent.is_polkitd(None), "no sender is not polkit");
    }

    #[test]
    fn success_and_cancel_end_without_an_error() {
        assert!(is_ok_exit(prompt::EXIT_SUCCESS));
        assert!(is_ok_exit(prompt::EXIT_CANCELLED));
        assert!(!is_ok_exit(prompt::EXIT_FAILED), "a real failure is a D-Bus error");
        assert!(!is_ok_exit(42), "an unexpected code is a D-Bus error");
    }

    #[test]
    fn a_cancel_before_start_is_remembered_then_consumed_once() {
        let agent = Agent::new(":1.12".to_owned(), false);
        agent.remember_cancelled("c1".to_owned());
        assert!(agent.take_cancelled("c1"), "the queued request sees the cancel");
        assert!(!agent.take_cancelled("c1"), "and it is consumed, not sticky");
        assert!(!agent.take_cancelled("never"), "an unknown cookie was not cancelled");
    }

    #[test]
    fn the_cancelled_set_stays_bounded() {
        let agent = Agent::new(":1.12".to_owned(), false);
        for i in 0..300 {
            agent.remember_cancelled(format!("c{i}"));
        }
        let len = agent.cancelled.lock().unwrap().len();
        assert!(len <= 257, "the set must not grow without bound, was {len}");
    }
}
