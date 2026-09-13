//! Whether the polkit PAM stack will try a fingerprint before a password.
//!
//! Fingerprint authentication is PAM's job (`pam_fprintd`), not ours. We only
//! look at whether the stack mentions that module, and whether the laptop lid
//! is closed, so the window can hide the password field while the helper waits
//! on the reader. We do not parse `max-tries`, do not count remaining swipes,
//! and do not write PAM files. pam_fprintd keeps its own tally and sends the
//! same `PAM_ERROR_MSG` for a bad swipe it charges a try for and one it does
//! not, so any count we kept would drift from the module's.

use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

const ETC_PAM: &str = "/etc/pam.d/polkit-1";
const USR_PAM: &str = "/usr/lib/pam.d/polkit-1";
const LID: &str = "/usr/bin/omarchy-hw-laptop-closed";

/// True when an `auth` line in the polkit PAM stack names `pam_fprintd.so`.
///
/// Commented lines and non-auth groups (`account` / `session`) do not count.
/// The module need not be first: a clamshell `pam_exec` gate may precede it.
pub fn configured_from_pam(raw: &str) -> bool {
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(kind) = line.split_whitespace().next() else {
            continue;
        };
        if kind != "auth" {
            continue;
        }
        if line.contains("pam_fprintd.so") {
            return true;
        }
    }
    false
}

/// `/etc` wins when it exists, matching PAM's own search order.
pub fn choose_pam_path(etc_exists: bool, usr_exists: bool) -> Option<&'static str> {
    if etc_exists {
        Some(ETC_PAM)
    } else if usr_exists {
        Some(USR_PAM)
    } else {
        None
    }
}

fn pam_path() -> Option<&'static str> {
    choose_pam_path(Path::new(ETC_PAM).is_file(), Path::new(USR_PAM).is_file())
}

/// Whether this machine's polkit PAM stack will try a fingerprint.
pub fn configured() -> bool {
    pam_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|raw| configured_from_pam(&raw))
}

/// `omarchy-hw-laptop-closed` exits 0 when the lid is shut.
///
/// Missing binary, spawn failure, or a non-zero exit all mean "open": hiding
/// the password field to wait on a reader you cannot reach is worse than
/// showing it.
pub fn lid_closed_from(status: std::io::Result<ExitStatus>) -> bool {
    status.is_ok_and(|s| s.success())
}

/// The probe runs from the hardened prompt child, so it inherits nothing it
/// could read or write: not the cookie pipe on stdin, not our stderr.
pub fn lid_closed() -> bool {
    lid_closed_from(
        Command::new(LID)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
}

/// Hide the password field and wait on the reader.
///
/// Checked once when the child starts. The PAM clamshell gate already ran by
/// the time a later lid close could matter, so we do not poll.
pub fn should_wait() -> bool {
    configured() && !lid_closed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_auth_line_naming_pam_fprintd_is_configured() {
        assert!(configured_from_pam("auth sufficient pam_fprintd.so\n"));
    }

    #[test]
    fn a_clamshell_gate_in_front_still_counts() {
        let raw = "\
auth  [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth  sufficient pam_fprintd.so
auth  required pam_unix.so
";
        assert!(configured_from_pam(raw));
    }

    #[test]
    fn a_commented_module_does_not_count() {
        assert!(!configured_from_pam("# auth sufficient pam_fprintd.so\n"));
        assert!(!configured_from_pam("auth required pam_unix.so\n"));
    }

    #[test]
    fn account_and_session_lines_do_not_count() {
        let raw = "\
account  required pam_fprintd.so
session  required pam_fprintd.so
auth     required pam_unix.so
";
        assert!(!configured_from_pam(raw));
    }

    #[test]
    fn empty_or_missing_text_is_not_configured() {
        assert!(!configured_from_pam(""));
        assert!(!configured_from_pam("   \n# nothing\n"));
    }

    #[test]
    fn max_tries_on_the_line_does_not_change_the_answer() {
        // We do not parse the number. Presence of the module is the whole fact.
        assert!(configured_from_pam(
            "auth sufficient pam_fprintd.so max-tries=1 timeout=10\n"
        ));
    }

    #[test]
    fn etc_wins_over_usr_when_both_exist() {
        assert_eq!(choose_pam_path(true, true), Some(ETC_PAM));
        assert_eq!(choose_pam_path(true, false), Some(ETC_PAM));
    }

    #[test]
    fn usr_is_used_only_when_etc_is_absent() {
        assert_eq!(choose_pam_path(false, true), Some(USR_PAM));
        assert_eq!(choose_pam_path(false, false), None);
    }

    #[test]
    fn a_successful_lid_command_means_closed() {
        assert!(lid_closed_from(Command::new("true").status()));
        assert!(!lid_closed_from(Command::new("false").status()));
        assert!(!lid_closed_from(
            Command::new("/nonexistent-sudo-pop-lid").status()
        ));
    }
}
