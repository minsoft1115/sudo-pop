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

/// Longest purpose line shown before it is cut short. Shorter than the command
/// because it is the second line and a whole sentence, not an invocation.
const MAX_PURPOSE_CHARS: usize = 64;

/// polkit writes every message as "Authentication is required to <do a thing>."
/// The window has already said it is asking for a password, so the wrapper is
/// dead weight. Stripped only when it matches: we register with a locale and
/// the sentence comes back translated, and a translation we do not recognise is
/// shown whole rather than mangled.
const MESSAGE_PREFIX: &str = "Authentication is required to ";

/// Actions whose message says less than the command line already does.
///
/// `manage-units` is how `run0` runs everything, and its message names a
/// transient unit -- `run-p1592228-i1586931.service`, a number generated for
/// that one invocation. Measured: the window's first line already reads
/// `run0 pacman -Syu`, so the sentence adds a random name and nothing else.
const UNINFORMATIVE_ACTIONS: [&str; 1] = ["org.freedesktop.systemd1.manage-units"];

/// The command sudo is about to run, read from our parent.
///
/// The askpass path has no polkit details to consult: sudo forks us and its own
/// command line is `sudo -A <command>`. `/proc/<pid>/cmdline` stays world
/// readable even for a setuid process, so no extra channel is needed.
pub fn command_from_sudo() -> Option<String> {
    // SAFETY: getppid cannot fail.
    let parent = unsafe { libc::getppid() } as u32;
    let raw = std::fs::read(format!("/proc/{parent}/cmdline")).ok()?;
    let argv: Vec<OsString> = raw
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| OsString::from_vec(part.to_vec()))
        .collect();

    // Only trust a parent that really is sudo; anything else means we are not
    // in the flow this is meant to describe.
    let program = argv.first()?;
    if std::path::Path::new(program).file_name()? != std::ffi::OsStr::new("sudo") {
        return None;
    }

    let start = crate::sudo_args::command_start(&argv[1..])? + 1;
    let command = argv[start..]
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    (!command.is_empty()).then(|| shorten(&command))
}

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

/// What the request will do, from polkit's own wording, or `None` when that
/// wording would add nothing.
///
/// The two paths need opposite things. On `run0` the command line is the whole
/// story and polkit's sentence is noise (see `UNINFORMATIVE_ACTIONS`). A
/// request from a desktop app is the other way round: the command line is
/// whatever binary happens to be running -- `quickshell -n -p ...` -- which
/// says who is asking but not what for, and the sentence is the only place
/// "mount the filesystem" appears.
///
/// `have_command` decides which of those we are in: with no command to lead
/// with, the sentence is all there is and is never suppressed.
pub fn purpose(message: &str, action_id: &str, have_command: bool) -> Option<String> {
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    if have_command && UNINFORMATIVE_ACTIONS.contains(&action_id) {
        return None;
    }
    let text = message.strip_prefix(MESSAGE_PREFIX).unwrap_or(message);
    let text = text.trim().trim_end_matches('.').trim();
    (!text.is_empty()).then(|| cut(text, MAX_PURPOSE_CHARS))
}

fn shorten(command: &str) -> String {
    cut(command, MAX_DISPLAY_CHARS)
}

fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit - 1).collect();
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

    const RUN0: &str = "org.freedesktop.systemd1.manage-units";
    const MOUNT: &str = "org.freedesktop.udisks2.filesystem-mount-system";

    #[test]
    fn a_desktop_requests_purpose_is_what_the_command_line_cannot_say() {
        // Measured: this is exactly what polkitd sends for a udisks mount, and
        // the command line beside it reads `quickshell -n -p ...`.
        assert_eq!(
            purpose(
                "Authentication is required to mount the filesystem",
                MOUNT,
                true
            )
            .as_deref(),
            Some("mount the filesystem")
        );
    }

    #[test]
    fn run0s_transient_unit_name_is_not_worth_a_line() {
        // Measured wording. The window's first line already says `run0 ...`.
        let message =
            "Authentication is required to start transient unit 'run-p1592228-i1586931.service'.";
        assert_eq!(purpose(message, RUN0, true), None);
        // ... unless there is no command line, when it is all we have.
        assert_eq!(
            purpose(message, RUN0, false).as_deref(),
            Some("start transient unit 'run-p1592228-i1586931.service'")
        );
    }

    #[test]
    fn the_boilerplate_and_the_full_stop_go() {
        assert_eq!(
            purpose("Authentication is required to reboot the system.", MOUNT, true)
                .as_deref(),
            Some("reboot the system")
        );
    }

    #[test]
    fn a_sentence_we_do_not_recognise_is_shown_whole() {
        // We register with a locale, so the sentence can come back translated.
        // Mangling it would be worse than leaving the wrapper on.
        assert_eq!(
            purpose("파일 시스템을 마운트하려면 인증이 필요합니다", MOUNT, true).as_deref(),
            Some("파일 시스템을 마운트하려면 인증이 필요합니다")
        );
    }

    #[test]
    fn nothing_to_say_says_nothing() {
        assert_eq!(purpose("", MOUNT, true), None);
        assert_eq!(purpose("   ", MOUNT, true), None);
        // A message that is only the boilerplate leaves an empty line, not a
        // line containing nothing.
        assert_eq!(purpose("Authentication is required to .", MOUNT, true), None);
    }

    #[test]
    fn a_long_sentence_is_cut_short() {
        let shown = purpose(&format!("Authentication is required to {}", "x".repeat(200)), MOUNT, true).unwrap();
        assert!(shown.chars().count() <= MAX_PURPOSE_CHARS, "{shown}");
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn our_own_process_is_described() {
        let me = std::process::id();
        assert!(owned_by_us(me));
        assert!(command_of(me).is_some());
    }
}
