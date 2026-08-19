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

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Prompts allowed per sudo command, out of the ten sudo would otherwise give.
pub const MAX_ATTEMPTS: u32 = 3;

/// Below this many remaining failures the window starts warning.
pub const WARN_BELOW: u32 = 4;

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
}

/// Read the current failure budget, or `None` if it cannot be determined.
///
/// Guessing is worse than staying quiet: a wrong number shown next to a
/// password box is actively misleading, so every parse failure gives up.
pub fn budget() -> Option<Budget> {
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

/// Look up a pam_faillock setting, preferring faillock.conf over the pam stack.
fn read_setting(key: &str) -> Option<u32> {
    // faillock.conf spells it `deny = 10`.
    if let Ok(text) = fs::read_to_string("/etc/security/faillock.conf") {
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
    }

    // The pam stack spells it `deny=10` on the module line.
    let needle = format!("{key}=");
    for file in ["/etc/pam.d/system-auth", "/etc/pam.d/sudo"] {
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
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
    }
    None
}

/// Count the failures PAM still considers valid, plus the newest one's time.
///
/// Entries past `fail_interval` stay listed but flip the Valid column from `V`
/// to `I`. Counting every row would understate the remaining budget, so only
/// `V` rows count.
fn failure_tally(user: &str) -> Option<(u32, Option<u64>)> {
    let out = Command::new("faillock")
        .arg("--user")
        .arg(user)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);

    let mut valid = 0;
    let mut newest = None;
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // "<date> <time> <type> <source> V"
        if fields.len() < 5 || fields.last() != Some(&"V") {
            continue;
        }
        valid += 1;
        if let Some(at) = parse_stamp(fields[0], fields[1]) {
            newest = Some(newest.map_or(at, |prev: u64| prev.max(at)));
        }
    }
    Some((valid, newest))
}

/// Turn faillock's local "YYYY-MM-DD HH:MM:SS" into a unix timestamp.
///
/// Shelling out to `date` keeps the local-time and DST handling with the system
/// rather than reimplementing a calendar here for one cosmetic countdown.
fn parse_stamp(date: &str, time: &str) -> Option<u64> {
    let out = Command::new("date")
        .arg("-d")
        .arg(format!("{date} {time}"))
        .arg("+%s")
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
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
