//! Which command is asking for the password.
//!
//! polkit's own `message` does not say. For run0 it reads "Authentication is
//! required to start transient unit 'run-p1630183-i1624710.service'." -- a
//! random unit name and nothing about what will run. A password box that says
//! only that gives no way to notice that something unexpected is asking.
//!
//! `details` carries `polkit.subject-pid`, the process that wanted the
//! privilege, and `/proc/<pid>/cmdline` is right there. This is the same job
//! the sudo wrapper did by reading its parent; only the pid comes from
//! somewhere else now.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

/// Longest command shown before it is cut short.
const MAX_DISPLAY_CHARS: usize = 120;

/// The command behind an authentication request, or `None` when it cannot be
/// established. Showing nothing is better than showing a guess.
pub fn command_of(pid: u32) -> Option<String> {
    if !owned_by_us(pid) {
        return None;
    }
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    describe(&raw)
}

/// Only describe a process running as us.
///
/// The pid arrives over D-Bus and the process may have exited and been
/// replaced by the time we look, so this is a check against describing
/// somebody else's command, not a security boundary of its own.
fn owned_by_us(pid: u32) -> bool {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    // SAFETY: getuid cannot fail.
    let me = unsafe { libc::getuid() };
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|rest| rest.split_whitespace().next()?.parse::<u32>().ok())
        .is_some_and(|uid| uid == me)
}

/// Render a NUL-separated `/proc/<pid>/cmdline` as one line.
fn describe(raw: &[u8]) -> Option<String> {
    let argv: Vec<OsString> = raw
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| OsString::from_vec(part.to_vec()))
        .collect();

    if argv.is_empty() {
        return None;
    }

    let command = argv
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    if command.is_empty() {
        return None;
    }
    Some(shorten(&command))
}

fn shorten(command: &str) -> String {
    if command.chars().count() <= MAX_DISPLAY_CHARS {
        return command.to_owned();
    }
    let kept: String = command.chars().take(MAX_DISPLAY_CHARS - 1).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmdline(parts: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for part in parts {
            out.extend_from_slice(part.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn shows_the_whole_invocation() {
        assert_eq!(
            describe(&cmdline(&["run0", "pacman", "-Syu"])),
            Some("run0 pacman -Syu".into())
        );
    }

    #[test]
    fn an_empty_cmdline_says_nothing() {
        assert_eq!(describe(&[]), None);
        assert_eq!(describe(&cmdline(&[])), None);
    }

    #[test]
    fn long_commands_are_cut_short() {
        let shown = describe(&cmdline(&["run0", "sh", "-c", &"x".repeat(300)])).unwrap();
        assert!(shown.chars().count() <= MAX_DISPLAY_CHARS, "{shown}");
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn our_own_process_is_described() {
        let me = std::process::id();
        assert!(owned_by_us(me));
        assert!(command_of(me).is_some());
    }
}
