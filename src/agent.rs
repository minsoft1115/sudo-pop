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
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    pub running: Arc<Mutex<HashMap<String, i32>>>,
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
            running: Arc::new(Mutex::new(HashMap::new())),
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
/// request really ends when polkitd cancels it. Where the caller's own clock
/// can be read it shortens this (`caller_timeout`); nothing lengthens it.
const CALLER_TIMEOUT: Duration = Duration::from_secs(25);

/// The caller's clock as far as we can know it.
///
/// An sd-bus client takes its method-call timeout from `SYSTEMD_BUS_TIMEOUT`
/// in its own environment, and polkitd names that client as the subject, so
/// its `/proc/<pid>/environ` says how long it will wait. The kernel lets only
/// the same uid read that file, and not for a setuid process, so a subject we
/// cannot read (gone, `pkexec`, somebody else's) keeps the default.
///
/// The request also passes through PID 1, whose own 25 s clock we cannot read,
/// and the deadline is the shorter of the two. So this only ever shortens the
/// countdown: a caller with `=5` is shown 5, one with `=120` or `infinity` is
/// still shown 25 (rationale §23).
fn caller_timeout(subject_pid: u32) -> Duration {
    if subject_pid == 0 {
        return CALLER_TIMEOUT;
    }
    std::fs::read(format!("/proc/{subject_pid}/environ"))
        .ok()
        .and_then(|environ| bus_timeout_in(&environ))
        .map_or(CALLER_TIMEOUT, |theirs| theirs.min(CALLER_TIMEOUT))
}

/// `SYSTEMD_BUS_TIMEOUT` out of a NUL-separated environment block, as sd-bus
/// would read it: the first entry wins, and a value it would reject (empty,
/// zero, unparsable) means the default, which is `None` here.
fn bus_timeout_in(environ: &[u8]) -> Option<Duration> {
    let value = environ
        .split(|b| *b == 0)
        .find_map(|entry| entry.strip_prefix(b"SYSTEMD_BUS_TIMEOUT="))?;
    parse_sec(std::str::from_utf8(value).ok()?)
}

/// systemd's time syntax, the part of it a timeout can be written in: a bare
/// number is seconds, else each part is a number (with an optional fraction)
/// and a unit, with whitespace allowed between and around them, and the parts
/// add up. `0` is `None` because sd-bus falls back to its default for it;
/// `infinity` is `None` because a caller with no clock cannot shorten ours.
/// Anything systemd would reject is `None` too, so a caller we cannot follow
/// is shown the default rather than a guess.
fn parse_sec(text: &str) -> Option<Duration> {
    let text = text.trim();
    if text.is_empty() || text == "infinity" {
        return None;
    }
    let mut total = Duration::ZERO;
    let mut rest = text;
    let mut first = true;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        let (number, after) = rest.split_at(digits);
        let number: f64 = number.parse().ok()?;
        let after = after.trim_start();
        let unit_len = after
            .find(|c: char| !c.is_ascii_alphabetic() && c != 'µ')
            .unwrap_or(after.len());
        let (unit, after) = after.split_at(unit_len);
        // A bare number is seconds only when it is the whole value, as in
        // systemd: after a unit has been given, every part needs one.
        if unit.is_empty() && !first {
            return None;
        }
        first = false;
        let usec_per_unit: f64 = match unit {
            "" | "s" | "sec" | "second" | "seconds" => 1_000_000.0,
            "ms" | "msec" => 1_000.0,
            "us" | "usec" | "µs" => 1.0,
            "ns" | "nsec" => 0.001,
            "m" | "min" | "minute" | "minutes" => 60_000_000.0,
            "h" | "hr" | "hour" | "hours" => 3_600_000_000.0,
            "d" | "day" | "days" => 86_400_000_000.0,
            _ => return None,
        };
        total += Duration::from_micros((number * usec_per_unit) as u64);
        rest = after.trim_start();
    }
    (total > Duration::ZERO).then_some(total)
}

/// How long a cancelled child gets to leave on its own before SIGKILL.
///
/// SIGTERM is a request: the child turns it into a cancel so the helper
/// channel is dropped cleanly (`prompt::TERMINATED`). A child whose window
/// loop is stuck would ignore that, so the cancel is made certain by a second,
/// unconditional signal a little later. The child normally exits within one
/// frame, well inside this.
const CANCEL_GRACE: Duration = Duration::from_secs(2);

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
        let timeout = caller_timeout(subject_pid);
        let left = timeout.saturating_sub(started.elapsed());
        if tracing() {
            println!(
                "  left       : {} ms (caller waits {} ms)",
                left.as_millis(),
                timeout.as_millis()
            );
        }
        let code = self
            .ask(&name, &cookie, subject_pid, &message, &action_id, left)
            .await;

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
            self.escalate_later(cookie);
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
    /// SIGKILL the child for `cookie` if it is still registered after the
    /// grace period. Looked up by cookie under the lock, never by the fd
    /// value: `ask` removes the entry before closing the fd, so a hit here is
    /// this request's own live pidfd and nothing else.
    fn escalate_later(&self, cookie: String) {
        let running = Arc::clone(&self.running);
        std::thread::spawn(move || {
            std::thread::sleep(CANCEL_GRACE);
            if let Ok(map) = running.lock()
                && let Some(&pidfd) = map.get(&cookie)
            {
                pidfd_signal(pidfd, libc::SIGKILL);
                if tracing() {
                    println!("  prompt ignored SIGTERM; killed");
                }
            }
        });
    }

    /// Run one request in a child and wait for its exit code.
    async fn ask(
        &self,
        username: &str,
        cookie: &str,
        subject_pid: u32,
        message: &str,
        action_id: &str,
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
            .env("SUDO_POP_ACTION", action_id)
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
        // path falls back to the window's 30s backstop rather than a stale
        // kill -- except during a fingerprint wait, which has no backstop
        // before PAM asks; there only the caller's own cancel ends it.
        let pid = child.id();
        // SAFETY: plain syscall; the child is alive here, freshly spawned.
        let pidfd =
            unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::c_long, 0 as libc::c_long) }
                as i32;
        if pidfd >= 0 {
            if let Ok(mut map) = self.running.lock() {
                map.insert(cookie.to_owned(), pidfd);
            }
            // A cancel that landed between begin_authentication's check and
            // this registration found nothing to signal and left a marker
            // instead. Consume it now, or the child would run to its backstop
            // for a request polkitd has already dropped.
            if self.take_cancelled(cookie) {
                if tracing() {
                    println!("  cancelled while starting; closing it");
                }
                pidfd_signal(pidfd, libc::SIGTERM);
                self.escalate_later(cookie.to_owned());
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
        assert!(
            !agent.is_polkitd(Some(":1.99")),
            "a different name is not polkit"
        );
        assert!(!agent.is_polkitd(None), "no sender is not polkit");
    }

    #[test]
    fn success_and_cancel_end_without_an_error() {
        assert!(is_ok_exit(prompt::EXIT_SUCCESS));
        assert!(is_ok_exit(prompt::EXIT_CANCELLED));
        assert!(
            !is_ok_exit(prompt::EXIT_FAILED),
            "a real failure is a D-Bus error"
        );
        assert!(!is_ok_exit(42), "an unexpected code is a D-Bus error");
    }

    #[test]
    fn a_cancel_before_start_is_remembered_then_consumed_once() {
        let agent = Agent::new(":1.12".to_owned(), false);
        agent.remember_cancelled("c1".to_owned());
        assert!(
            agent.take_cancelled("c1"),
            "the queued request sees the cancel"
        );
        assert!(
            !agent.take_cancelled("c1"),
            "and it is consumed, not sticky"
        );
        assert!(
            !agent.take_cancelled("never"),
            "an unknown cookie was not cancelled"
        );
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

    #[test]
    fn systemd_time_syntax_is_read_the_way_sd_bus_reads_it() {
        assert_eq!(
            parse_sec("5"),
            Some(Duration::from_secs(5)),
            "bare = seconds"
        );
        assert_eq!(parse_sec("5s"), Some(Duration::from_secs(5)));
        assert_eq!(parse_sec("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_sec("1.5s"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_sec("1min"), Some(Duration::from_secs(60)));
        assert_eq!(parse_sec("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_sec(" 1min 30s "), Some(Duration::from_secs(90)));
        assert_eq!(
            parse_sec("5 s"),
            Some(Duration::from_secs(5)),
            "systemd allows a space before the unit"
        );
        assert_eq!(parse_sec("1 min 30 s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_sec("5000000000ns"), Some(Duration::from_secs(5)));
        assert_eq!(parse_sec("0"), None, "sd-bus treats 0 as unset");
        assert_eq!(parse_sec("0.0004ms"), None, "rounds to nothing, so unset");
        assert_eq!(parse_sec("infinity"), None, "no clock cannot shorten ours");
        assert_eq!(parse_sec(""), None);
        assert_eq!(parse_sec("abc"), None);
        assert_eq!(parse_sec("-5"), None, "systemd rejects a sign");
        assert_eq!(parse_sec("5S"), None, "units are case-sensitive");
        assert_eq!(
            parse_sec("5 apples"),
            None,
            "an unknown unit is not a guess"
        );
        assert_eq!(parse_sec("5s5"), None, "a number needs a unit after it");
    }

    #[test]
    fn the_first_entry_in_the_block_wins_and_others_are_ignored() {
        let block = b"HOME=/home/x\0SYSTEMD_BUS_TIMEOUT=7\0SYSTEMD_BUS_TIMEOUT=9\0";
        assert_eq!(bus_timeout_in(block), Some(Duration::from_secs(7)));
        assert_eq!(bus_timeout_in(b"HOME=/home/x\0PATH=/bin\0"), None);
        assert_eq!(
            bus_timeout_in(b"SYSTEMD_BUS_TIMEOUT_X=3\0"),
            None,
            "a longer name is a different variable"
        );
        assert_eq!(bus_timeout_in(b""), None);
    }

    /// A child of ours with the variable set, read the way a real subject is.
    /// Killed on drop, so a failed assertion does not leave it around.
    struct Sleeper(std::process::Child);

    impl Sleeper {
        fn with(timeout: Option<&str>) -> Self {
            let mut cmd = std::process::Command::new("sleep");
            cmd.arg("30").env_remove("SYSTEMD_BUS_TIMEOUT");
            if let Some(t) = timeout {
                cmd.env("SYSTEMD_BUS_TIMEOUT", t);
            }
            Sleeper(cmd.spawn().expect("sleep is available"))
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn a_callers_own_clock_shortens_the_countdown_but_never_lengthens_it() {
        let short = Sleeper::with(Some("5"));
        let long = Sleeper::with(Some("120"));
        let none = Sleeper::with(None);

        assert_eq!(caller_timeout(short.pid()), Duration::from_secs(5));
        assert_eq!(
            caller_timeout(long.pid()),
            CALLER_TIMEOUT,
            "PID 1's clock is still 25 s, so more is not on offer"
        );
        assert_eq!(caller_timeout(none.pid()), CALLER_TIMEOUT);
        assert_eq!(caller_timeout(0), CALLER_TIMEOUT, "no subject, no reading");
    }
}
