//! Which command is asking for the password.
//!
//! sudo tells askpass nothing but the prompt string, so the command has to come
//! from somewhere else. It is right there in the parent process: sudo forks us,
//! and its own command line is `sudo -A <command>`. `/proc/<pid>/cmdline` stays
//! world readable even for a setuid process, so no extra channel is needed.
//!
//! Showing it is worth a line of the window. A password box that says only
//! "password for you" gives no way to notice that something unexpected is
//! asking; naming the command does.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use crate::sudo_args;

/// Longest command shown before it is cut short.
const MAX_DISPLAY_CHARS: usize = 120;

/// The command sudo is about to run, or `None` if it cannot be established.
///
/// Returns `None` whenever the parent is not a sudo invocation with a command —
/// a manually set SUDO_ASKPASS, `sudo -v`, a test harness. Showing nothing is
/// better than showing a guess.
pub fn command() -> Option<String> {
    // SAFETY: getppid cannot fail.
    let parent = unsafe { libc::getppid() };
    let raw = std::fs::read(format!("/proc/{parent}/cmdline")).ok()?;
    describe(&raw)
}

/// Pull the command out of a NUL-separated `/proc/<pid>/cmdline`.
fn describe(raw: &[u8]) -> Option<String> {
    let mut argv: Vec<OsString> = raw
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| OsString::from_vec(part.to_vec()))
        .collect();

    if argv.is_empty() {
        return None;
    }

    // Only trust a parent that really is sudo; anything else means we are not
    // in the flow this is meant to describe.
    let program = argv.remove(0);
    let name = std::path::Path::new(&program).file_name()?;
    if name != std::ffi::OsStr::new("sudo") {
        return None;
    }

    let start = sudo_args::command_start(&argv)?;
    let command = argv[start..]
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
    fn finds_the_command_past_sudos_options() {
        assert_eq!(
            describe(&cmdline(&["sudo", "-A", "pacman", "-Syu"])),
            Some("pacman -Syu".into())
        );
        assert_eq!(
            describe(&cmdline(&["/usr/bin/sudo", "-A", "-u", "root", "id"])),
            Some("id".into())
        );
    }

    #[test]
    fn respects_the_double_dash() {
        assert_eq!(
            describe(&cmdline(&["sudo", "-A", "--", "ls", "-l"])),
            Some("ls -l".into())
        );
    }

    #[test]
    fn options_only_invocations_have_no_command() {
        assert_eq!(describe(&cmdline(&["sudo", "-A", "-v"])), None);
        assert_eq!(describe(&cmdline(&["sudo"])), None);
    }

    #[test]
    fn a_parent_that_is_not_sudo_is_ignored() {
        assert_eq!(describe(&cmdline(&["bash", "-c", "something"])), None);
        assert_eq!(describe(&[]), None);
    }

    #[test]
    fn long_commands_are_cut_short() {
        let shown = describe(&cmdline(&["sudo", "-A", "sh", "-c", &"x".repeat(300)])).unwrap();
        assert!(shown.chars().count() <= MAX_DISPLAY_CHARS, "{shown}");
        assert!(shown.ends_with('…'));
        assert!(shown.starts_with("sh -c xxx"));
    }
}
