//! Bounded, escaped request metadata for display. A process command line is
//! caller-controlled reference information, not proof of what will execute.

use std::ffi::OsString;
use std::io::Read;
use std::os::unix::ffi::OsStringExt;

const MAX_DISPLAY_CHARS: usize = 120;
const MAX_PURPOSE_CHARS: usize = 64;
const MAX_COMMAND_BYTES: usize = 16 * 1024;
const MAX_ARGUMENTS: usize = 256;
const MAX_METADATA_BYTES: usize = 8192;
const MESSAGE_PREFIX: &str = "Authentication is required to ";

#[derive(Clone, Debug)]
pub struct CommandInfo {
    /// Escaped arguments, preserving empty arguments and their boundaries.
    pub arguments: Vec<String>,
    pub truncated: bool,
}

impl CommandInfo {
    pub fn full(&self) -> String {
        let mut text = self.arguments.join(" ");
        if self.truncated {
            text.push_str(" [command data truncated]");
        }
        text
    }

    pub fn summary(&self) -> String {
        cut(&self.full(), MAX_DISPLAY_CHARS)
    }
}

/// Byte-preserving display, not a shell command to copy and execute.
fn argument(bytes: &[u8]) -> String {
    if !bytes.is_empty()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:@%+=,-".contains(b))
    {
        return String::from_utf8(bytes.to_vec()).unwrap();
    }
    let mut result = String::from("\"");
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            result.extend(c.escape_debug());
        }
        for b in chunk.invalid() {
            use std::fmt::Write;
            write!(result, "\\x{b:02x}").unwrap();
        }
    }
    result.push('"');
    result
}

/// Escape untrusted metadata too; never render control/bidi bytes as UI syntax.
pub fn metadata(text: &str) -> String {
    let mut end = text.len().min(MAX_METADATA_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut shown: String = text[..end]
        .chars()
        .map(|c| {
            if c == '\'' || c == '"' {
                c.to_string()
            } else {
                c.escape_debug().to_string()
            }
        })
        .collect();
    if end < text.len() {
        shown.push_str(" [metadata truncated]");
    }
    shown
}

fn read_command(pid: u32) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    std::fs::File::open(format!("/proc/{pid}/cmdline"))
        .ok()?
        .take((MAX_COMMAND_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .ok()?;
    Some(raw)
}

fn parse(raw: &[u8]) -> (Vec<OsString>, bool) {
    let truncated_bytes = raw.len() > MAX_COMMAND_BYTES;
    let raw = &raw[..raw.len().min(MAX_COMMAND_BYTES)];
    if raw.is_empty() {
        return (vec![], truncated_bytes);
    }
    // Remove the terminator only. Interior and final empty arguments matter.
    let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
    let mut parts = raw.split(|&b| b == 0);
    let argv = parts
        .by_ref()
        .take(MAX_ARGUMENTS)
        .map(|part| OsString::from_vec(part.to_vec()))
        .collect();
    (argv, truncated_bytes || parts.next().is_some())
}

fn from_args(argv: &[OsString], truncated: bool) -> Option<CommandInfo> {
    (!argv.is_empty()).then(|| CommandInfo {
        arguments: argv
            .iter()
            .map(|a| argument(a.as_encoded_bytes()))
            .collect(),
        truncated,
    })
}

/// Askpass metadata has no polkit action. Even the parent's argv is only a hint.
pub fn command_from_sudo() -> Option<CommandInfo> {
    // SAFETY: getppid cannot fail.
    let raw = read_command(unsafe { libc::getppid() } as u32)?;
    let (argv, truncated) = parse(&raw);
    let program = argv.first()?;
    if std::path::Path::new(program).file_name()? != std::ffi::OsStr::new("sudo") {
        return None;
    }
    let start = crate::sudo_args::command_start(&argv[1..])? + 1;
    from_args(&argv[start..], truncated)
}

pub fn command_of(pid: u32) -> Option<CommandInfo> {
    if !owned_by_us(pid) {
        return None;
    }
    describe(&read_command(pid)?)
}

// This UID check limits accidental disclosure; it is not an execution guarantee.
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

fn describe(raw: &[u8]) -> Option<CommandInfo> {
    let (argv, truncated) = parse(raw);
    from_args(&argv, truncated)
}

/// Compact purpose line. The full polkit message and action remain in details.
pub fn purpose(message: &str, _action_id: &str, _have_command: bool) -> Option<String> {
    let message = message.trim();
    let text = message.strip_prefix(MESSAGE_PREFIX).unwrap_or(message);
    let text = text.trim().trim_end_matches('.').trim();
    (!text.is_empty()).then(|| cut(&metadata(text), MAX_PURPOSE_CHARS))
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
            describe(&cmdline(&["run0", "pacman", "-Syu"])).map(|c| c.full()),
            Some("run0 pacman -Syu".into())
        );
    }

    #[test]
    fn an_empty_cmdline_says_nothing() {
        assert!(describe(&[]).is_none());
        assert!(describe(&cmdline(&[])).is_none());
    }

    #[test]
    fn long_commands_are_cut_short() {
        let command = describe(&cmdline(&["run0", "sh", "-c", &"x".repeat(300)])).unwrap();
        assert!(command.full().ends_with(&"x".repeat(300)));
        let shown = command.summary();
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
    fn run0_purpose_is_retained_even_with_a_command() {
        // Measured wording. The window's first line already says `run0 ...`.
        let message =
            "Authentication is required to start transient unit 'run-p1592228-i1586931.service'.";
        assert_eq!(purpose(message, RUN0, true), purpose(message, RUN0, false));
        // The purpose is retained whether or not a command line is available.
        assert_eq!(
            purpose(message, RUN0, false).as_deref(),
            Some("start transient unit 'run-p1592228-i1586931.service'")
        );
    }

    #[test]
    fn the_boilerplate_and_the_full_stop_go() {
        assert_eq!(
            purpose(
                "Authentication is required to reboot the system.",
                MOUNT,
                true
            )
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
        assert_eq!(
            purpose("Authentication is required to .", MOUNT, true),
            None
        );
    }

    #[test]
    fn a_long_sentence_is_cut_short() {
        let shown = purpose(
            &format!("Authentication is required to {}", "x".repeat(200)),
            MOUNT,
            true,
        )
        .unwrap();
        assert!(shown.chars().count() <= MAX_PURPOSE_CHARS, "{shown}");
        assert!(shown.ends_with('…'));
    }

    #[test]
    fn argument_boundaries_empty_values_and_invalid_bytes_are_preserved() {
        let info = describe(b"echo\0a b\0\0bad\xff\0").unwrap();
        assert_eq!(info.arguments, ["echo", "\"a b\"", "\"\"", "\"bad\\xff\""]);
        assert_ne!(info.full(), describe(b"echo\0a\0b\0").unwrap().full());
        assert_ne!(argument(b"bad\xff"), argument(b"bad\\xff"));
    }

    #[test]
    fn controls_and_direction_overrides_cannot_spoof_the_display() {
        let text = "a\n\t\r\u{1b}\u{202e}\u{2066}\u{200b}b";
        for shown in [argument(text.as_bytes()), metadata(text)] {
            for c in [
                '\n', '\t', '\r', '\u{1b}', '\u{202e}', '\u{2066}', '\u{200b}',
            ] {
                assert!(!shown.contains(c), "{shown:?}");
            }
        }
    }

    #[test]
    fn collection_limits_are_explicit() {
        let raw = vec![b'x'; MAX_COMMAND_BYTES + 1];
        let info = describe(&raw).unwrap();
        assert!(info.truncated);
        assert!(info.full().ends_with("[command data truncated]"));
        let info = describe(&cmdline(&vec![""; MAX_ARGUMENTS + 1])).unwrap();
        assert_eq!(info.arguments.len(), MAX_ARGUMENTS);
        assert!(info.truncated);
        assert!(metadata(&"가".repeat(MAX_METADATA_BYTES)).ends_with("[metadata truncated]"));
    }

    #[test]
    fn vanished_process_is_unavailable() {
        assert!(command_of(u32::MAX).is_none());
    }

    #[test]
    fn our_own_process_is_described() {
        let me = std::process::id();
        assert!(owned_by_us(me));
        assert!(command_of(me).is_some());
    }
}
