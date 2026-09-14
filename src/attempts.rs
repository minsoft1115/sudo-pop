//! Retry guard and failure budget.
//!
//! Two separate protections against the same hazard: sudoers here sets
//! `passwd_tries=10` and PAM sets faillock `deny=10`, the same number. One sudo
//! command that keeps answering wrongly can therefore consume the entire
//! lockout budget on its own.
//!
//! The guard caps a single command at three prompts. The budget query is what
//! lets the window warn before the last few attempts are spent, and refuse
//! outright once the account is already locked.
//!
//! The budget only means something where the PAM stack being answered runs
//! `pam_faillock`. Stock Arch does for both `sudo` and `polkit-1` (through
//! `system-auth`), but Omarchy's fingerprint setup writes a `polkit-1` without
//! it, and then a wrong polkit password neither counts nor locks. The stack is
//! followed through its `include`s before any number is shown (rationale §24).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Prompts allowed per sudo command, out of the ten sudo would otherwise give.
pub const MAX_ATTEMPTS: u32 = 3;

/// What the window says after a rejected password. ASCII, like `Checking...`.
pub const WRONG_PASSWORD: &str = "Wrong password";

/// At or below this many remaining failures the standing line turns to the
/// error colour. Above it the same line is drawn quietly.
pub const WARN_AT_OR_BELOW: u32 = 3;

/// A stale counter is ignored. This only matters when sudo was invoked without
/// going through our wrapper, which is the one path that never resets it.
const STALE_AFTER_SECS: u64 = 60;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn counter_path() -> Option<PathBuf> {
    crate::paths::runtime_dir()
        .ok()
        .map(|dir| dir.join("sudo-pop/attempts"))
}

/// Prompts already spent on the current sudo command.
///
/// Counts submissions, not appearances: cancelling costs nothing here, because
/// a cancel makes sudo give up immediately and cannot loop.
pub fn used() -> u32 {
    let Some(path) = counter_path() else {
        return 0;
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return 0;
    };

    let mut parts = text.split_whitespace();
    let stamp: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let count: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);

    if now_secs().saturating_sub(stamp) > STALE_AFTER_SECS {
        return 0;
    }
    count
}

/// Record that a password is about to be handed to sudo.
///
/// Never stores the password itself — only a timestamp and a count.
pub fn record() {
    let Some(path) = counter_path() else {
        return;
    };
    let next = used() + 1;
    let _ = write_private(&path, &format!("{} {next}\n", now_secs()));
}

/// Start a fresh count. The wrapper calls this per sudo command, so the three
/// prompts are per command rather than per minute.
pub fn reset() {
    if let Some(path) = counter_path() {
        let _ = fs::remove_file(path);
    }
}

fn write_private(path: &PathBuf, contents: &str) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())
}

/// How much failure budget PAM has left for this account.
pub struct Budget {
    /// Failures that can still be recorded before the account locks.
    pub remaining: u32,
    /// Seconds until the lockout lifts, when already locked and computable.
    pub unlock_in: Option<u64>,
}

impl Budget {
    pub fn is_locked(&self) -> bool {
        self.remaining == 0
    }

    /// The line that stands under the field for the whole life of the window:
    /// what is left of the shared faillock budget, and whether that is few
    /// enough to say so in the error colour.
    ///
    /// Shown at all times rather than only when it runs low. The number is
    /// what the window is for -- polkit failures lock sudo and login too, so
    /// "how much room is left" is worth knowing before it is nearly gone.
    ///
    /// `None` only when the account is already locked, which `refusal` speaks
    /// to instead and which never reaches a window anyway.
    pub fn status(&self) -> Option<(String, bool)> {
        (self.remaining > 0).then(|| {
            (
                format!(
                    "{} attempt(s) left before the account locks",
                    self.remaining
                ),
                self.remaining <= WARN_AT_OR_BELOW,
            )
        })
    }

    /// The message to show instead of a prompt when the account is locked, or
    /// `None` when it is not. Both prompt paths refuse rather than ask, so this
    /// is the one place that decides the wording.
    pub fn refusal(&self) -> Option<String> {
        if !self.is_locked() {
            return None;
        }
        Some(match self.unlock_in {
            Some(secs) => format!("account locked, {secs}s to go"),
            None => "account is locked out".to_owned(),
        })
    }
}

/// The PAM service the polkit helper authenticates against.
pub const POLKIT_SERVICE: &str = "polkit-1";
/// The PAM service sudo authenticates against.
pub const SUDO_SERVICE: &str = "sudo";

/// Read the current failure budget, or `None` if it cannot be determined.
///
/// Guessing is worse than staying quiet: a wrong number shown next to a
/// password box is actively misleading, so every parse failure gives up. So
/// does a `service` whose PAM stack never runs `pam_faillock`: there the tally
/// is real but this password does not move it, and a locked account is not
/// refused either, because that stack would accept the password anyway.
pub fn budget(service: &str) -> Option<Budget> {
    if !stack_counts_failures(service) {
        return None;
    }
    let deny = read_setting("deny")?;
    let user = current_user()?;
    let (valid, newest) = failure_tally(&user)?;

    let remaining = deny.saturating_sub(valid);
    let unlock_in = if remaining == 0 {
        read_setting("unlock_time").zip(newest).map(|(window, at)| {
            let elapsed = now_secs().saturating_sub(at);
            u64::from(window).saturating_sub(elapsed)
        })
    } else {
        None
    };

    Some(Budget {
        remaining,
        unlock_in,
    })
}

/// Whether an `auth` line of the service's effective PAM stack runs
/// `pam_faillock`, following `include` and `substack` the way PAM does.
pub fn stack_counts_failures(service: &str) -> bool {
    crate::pam::auth_stack_names(service, "pam_faillock")
}

/// Look up a pam_faillock setting, preferring faillock.conf over the pam stack.
fn read_setting(key: &str) -> Option<u32> {
    if let Ok(text) = fs::read_to_string("/etc/security/faillock.conf")
        && let Some(value) = parse_conf_setting(&text, key)
    {
        return Some(value);
    }
    for file in ["/etc/pam.d/system-auth", "/etc/pam.d/sudo"] {
        if let Ok(text) = fs::read_to_string(file)
            && let Some(value) = parse_pam_setting(&text, key)
        {
            return Some(value);
        }
    }
    None
}

/// `key = value`, faillock.conf style. Comments and other keys are ignored.
fn parse_conf_setting(text: &str, key: &str) -> Option<u32> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(value) = line
            .strip_prefix(key)
            .and_then(|v| v.trim().strip_prefix('='))
            .and_then(|v| v.trim().parse().ok())
        {
            return Some(value);
        }
    }
    None
}

/// `key=value` glued onto a pam_faillock module line. Only that module's lines
/// count, so a `deny=` on some other module is not mistaken for ours.
fn parse_pam_setting(text: &str, key: &str) -> Option<u32> {
    let needle = format!("{key}=");
    for line in text.lines() {
        if line.trim_start().starts_with('#') || !line.contains("pam_faillock") {
            continue;
        }
        if let Some(rest) = line.split(&needle).nth(1) {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse() {
                return Some(n);
            }
        }
    }
    None
}

/// Count the failures PAM still considers valid, plus the newest one's time.
///
/// Entries past `fail_interval` stay listed but flip the Valid column from `V`
/// to `I`. Counting every row would understate the remaining budget, so only
/// `V` rows count.
/// Where `faillock` lives. Arch ships it in both places; a name alone would go
/// through the prompt child's PATH, which on Omarchy's user manager puts
/// `~/.local/bin` and the mise shims ahead of `/usr/bin` -- a place any process
/// of the same uid can write to. It would only get to lie about the tally
/// (no password passes through here), but the window is hardened against
/// exactly that neighbour, so it does not get to pick the binary either.
const FAILLOCK: [&str; 2] = ["/usr/bin/faillock", "/usr/sbin/faillock"];

/// The tally reader, or `None` when the system has none (then there is no
/// line and no refusal, as for any other parse failure).
///
/// The override is compiled in only for debug builds, like the helper doors
/// in `helper.rs`: scenarios stand in a fake tally without burning the real
/// one. A release binary never reads it -- the string is not even in it.
fn faillock_binary() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("SUDO_POP_FAILLOCK_BIN") {
        return Some(path);
    }
    FAILLOCK
        .iter()
        .find(|p| Path::new(p).is_file())
        .map(|p| (*p).to_owned())
}

fn failure_tally(user: &str) -> Option<(u32, Option<u64>)> {
    // stdin is nulled the way the lid probe's is (rationale §22-4): this can
    // run after a wrong answer, while a retyped password sits in `pending`,
    // and a child should inherit nothing from a process in that state.
    let out = Command::new(faillock_binary()?)
        .arg("--user")
        .arg(user)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (valid, newest_row) = parse_tally(&text);
    let newest = newest_row.and_then(|(date, time)| parse_stamp(&date, &time));
    Some((valid, newest))
}

/// Count the still-valid failures and pick out the newest one's raw date/time.
///
/// `faillock` lists entries oldest-first and flips the Valid column from `V` to
/// `I` once past `fail_interval`. Only `V` rows count toward the budget, and the
/// newest of them -- the last one listed -- carries the time for the countdown.
fn parse_tally(text: &str) -> (u32, Option<(String, String)>) {
    let mut valid = 0;
    let mut newest = None;
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // "<date> <time> <type> <source> V"
        if fields.len() < 5 || fields.last() != Some(&"V") {
            continue;
        }
        valid += 1;
        newest = Some((fields[0].to_owned(), fields[1].to_owned()));
    }
    (valid, newest)
}

/// Turn faillock's local "YYYY-MM-DD HH:MM:SS" into a unix timestamp.
///
/// The C library does the calendar: `strptime` splits the fields and `mktime`
/// applies this process's time zone, with `tm_isdst = -1` so it also decides
/// daylight saving for that local time. That is what `date -d` did when this
/// shelled out to it; doing it in-process spares the hardened child a fork
/// and a PATH lookup. faillock's format is fixed in its source, so one
/// pattern is enough; anything else is `None`, and the lock message then
/// just omits the countdown.
fn parse_stamp(date: &str, time: &str) -> Option<u64> {
    let text = std::ffi::CString::new(format!("{date} {time}")).ok()?;
    // SAFETY: an all-zero `tm` is a valid value for strptime to fill, and both
    // strings are NUL-terminated for as long as the calls run.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let end = libc::strptime(text.as_ptr(), c"%Y-%m-%d %H:%M:%S".as_ptr(), &mut tm);
        if end.is_null() || *end != 0 {
            return None;
        }
        tm.tm_isdst = -1;
        let secs = libc::mktime(&mut tm);
        (secs >= 0).then_some(secs as u64)
    }
}

fn current_user() -> Option<String> {
    // SAFETY: getpwuid returns a pointer into static storage, or null.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_str()
            .ok()
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_runtime_dir<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("sudo-pop-attempts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sudo-pop")).unwrap();
        let prev = std::env::var_os("XDG_RUNTIME_DIR");
        // SAFETY: the whole block is serialized by TEST_ENV_LOCK.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };
        let out = f();
        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) },
            None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
        }
        let _ = fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn the_counter_counts_submissions_and_resets() {
        with_runtime_dir(|| {
            reset();
            assert_eq!(used(), 0);
            record();
            assert_eq!(used(), 1);
            record();
            assert_eq!(used(), 2);
            reset();
            assert_eq!(used(), 0);
        });
    }

    #[test]
    fn a_stale_counter_is_ignored() {
        with_runtime_dir(|| {
            let path = counter_path().unwrap();
            let old = now_secs().saturating_sub(STALE_AFTER_SECS + 5);
            write_private(&path, &format!("{old} 3\n")).unwrap();
            assert_eq!(used(), 0, "a stale count must not gate a fresh command");
        });
    }

    #[test]
    fn deny_is_read_from_faillock_conf() {
        let conf = "# faillock\ndeny = 10\nunlock_time = 120\n";
        assert_eq!(parse_conf_setting(conf, "deny"), Some(10));
        assert_eq!(parse_conf_setting(conf, "unlock_time"), Some(120));
        assert_eq!(parse_conf_setting(conf, "audit"), None);
        // a commented-out setting does not count
        assert_eq!(parse_conf_setting("# deny = 3\n", "deny"), None);
        // a longer key that merely starts with ours is not a match
        assert_eq!(parse_conf_setting("denyfoo = 3\n", "deny"), None);
    }

    #[test]
    fn deny_is_read_from_a_pam_faillock_line() {
        let pam = "auth  required  pam_faillock.so  preauth deny=10 unlock_time=120\n";
        assert_eq!(parse_pam_setting(pam, "deny"), Some(10));
        assert_eq!(parse_pam_setting(pam, "unlock_time"), Some(120));
        // only pam_faillock lines are consulted
        assert_eq!(
            parse_pam_setting("auth required pam_unix.so deny=3\n", "deny"),
            None
        );
        assert_eq!(
            parse_pam_setting("# pam_faillock.so deny=3\n", "deny"),
            None
        );
    }

    #[test]
    fn the_tally_counts_only_valid_rows() {
        let out = "\
lmh:
When                Type  Source   Valid
2026-08-19 12:00:01 RHOST host     V
2026-08-19 12:00:02 RHOST host     I
2026-08-19 12:00:03 RHOST host     V
";
        let (valid, newest) = parse_tally(out);
        assert_eq!(valid, 2, "the I row must not count toward the budget");
        assert_eq!(
            newest,
            Some(("2026-08-19".to_owned(), "12:00:03".to_owned())),
            "newest is the last valid row"
        );
    }

    #[test]
    fn a_tally_with_no_valid_rows_is_zero() {
        assert_eq!(parse_tally("lmh:\nWhen Type Source Valid\n"), (0, None));
        assert_eq!(parse_tally(""), (0, None));
    }

    #[test]
    fn the_budget_line_is_always_there() {
        let b = |remaining| Budget {
            remaining,
            unlock_in: None,
        };
        // Ten left is not a warning, but the window says so all the same.
        assert_eq!(
            b(10).status(),
            Some((
                "10 attempt(s) left before the account locks".to_owned(),
                false
            ))
        );
        assert_eq!(
            b(0).status(),
            None,
            "a locked account speaks through refusal"
        );
    }

    #[test]
    fn only_three_or_fewer_turn_the_line_red() {
        let warned = |remaining| {
            Budget {
                remaining,
                unlock_in: None,
            }
            .status()
            .expect("not locked")
            .1
        };
        assert!(!warned(5));
        assert!(!warned(4));
        assert!(warned(WARN_AT_OR_BELOW));
        assert!(warned(3));
        assert!(warned(1));
    }

    #[test]
    fn a_service_that_does_not_count_has_no_budget() {
        // The polkit-1 stack on an Omarchy machine with a fingerprint set up
        // has no faillock, so there is no line and no refusal, whatever the
        // tally says. Looked up through the real /etc, so the assertion is
        // only about the "not counted" branch.
        assert!(budget("sudo-pop-no-such-pam-service").is_none());
    }

    /// The local time of `secs`, in faillock's own format, from the same C
    /// library `parse_stamp` uses -- so the test holds in any time zone.
    fn local_stamp(secs: u64) -> (String, String) {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        let mut buf = [0u8; 32];
        // SAFETY: localtime_r and strftime write only into the buffers given.
        let n = unsafe {
            libc::localtime_r(&t, &mut tm);
            libc::strftime(
                buf.as_mut_ptr().cast(),
                buf.len(),
                c"%Y-%m-%d %H:%M:%S".as_ptr(),
                &tm,
            )
        };
        let text = std::str::from_utf8(&buf[..n]).unwrap().to_owned();
        let (d, t) = text.split_once(' ').unwrap();
        (d.to_owned(), t.to_owned())
    }

    #[test]
    fn a_faillock_stamp_round_trips_through_the_c_library() {
        // Whole seconds now, and a fixed instant, both in this zone.
        for secs in [now_secs(), 1_755_576_003] {
            let (d, t) = local_stamp(secs);
            assert_eq!(parse_stamp(&d, &t), Some(secs), "{d} {t}");
        }
    }

    #[test]
    fn a_malformed_stamp_is_none_not_a_guess() {
        assert_eq!(parse_stamp("2026-08-19", "12:00"), None, "seconds missing");
        assert_eq!(parse_stamp("yesterday", "noon"), None);
        assert_eq!(
            parse_stamp("2026-08-19", "12:00:03 extra"),
            None,
            "trailing text"
        );
        assert_eq!(parse_stamp("", ""), None);
    }

    #[test]
    fn the_tally_reader_is_a_fixed_path_unless_a_debug_build_is_told_otherwise() {
        let _guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var("SUDO_POP_FAILLOCK_BIN") };
        let found = faillock_binary();
        assert!(
            found.as_deref().is_none_or(|p| FAILLOCK.contains(&p)),
            "without the override only the fixed candidates qualify: {found:?}"
        );
        unsafe { std::env::set_var("SUDO_POP_FAILLOCK_BIN", "/tmp/fake-faillock") };
        assert_eq!(
            faillock_binary().as_deref(),
            Some("/tmp/fake-faillock"),
            "a debug build takes the stand-in"
        );
        unsafe { std::env::remove_var("SUDO_POP_FAILLOCK_BIN") };
    }

    #[test]
    fn refusal_speaks_only_when_locked() {
        assert_eq!(
            Budget {
                remaining: 1,
                unlock_in: None
            }
            .refusal(),
            None
        );
        assert_eq!(
            Budget {
                remaining: 0,
                unlock_in: None
            }
            .refusal()
            .as_deref(),
            Some("account is locked out")
        );
        assert_eq!(
            Budget {
                remaining: 0,
                unlock_in: Some(90)
            }
            .refusal()
            .as_deref(),
            Some("account locked, 90s to go")
        );
    }
}
