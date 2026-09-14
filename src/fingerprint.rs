//! Whether the polkit PAM stack will try a fingerprint before a password.
//!
//! Fingerprint authentication is PAM's job (`pam_fprintd`), not ours. We only
//! look at whether the stack mentions that module, and whether the laptop lid
//! is closed, so the window can hide the password field while the helper waits
//! on the reader. We do not parse `max-tries`, do not count remaining swipes,
//! and do not write PAM files. pam_fprintd keeps its own tally and sends the
//! same `PAM_ERROR_MSG` for a bad swipe it charges a try for and one it does
//! not, so any count we kept would drift from the module's.

use std::process::{Command, ExitStatus, Stdio};

const LID: &str = "/usr/bin/omarchy-hw-laptop-closed";

/// Whether this machine's polkit PAM stack will try a fingerprint: an `auth`
/// line naming `pam_fprintd.so`, in `/etc/pam.d/polkit-1` (or the vendor copy
/// in `/usr/lib/pam.d` when `/etc` has none) or in anything it includes.
///
/// Omarchy puts the line in `polkit-1` itself; a hand-made stack may put it in
/// `system-auth` and include that. Both are the same stack to PAM, so both
/// are the same answer here (`pam::auth_stack_names`, shared with the faillock
/// check so the two cannot disagree about what the stack contains).
pub fn configured() -> bool {
    crate::pam::auth_stack_names(crate::attempts::POLKIT_SERVICE, "pam_fprintd")
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

    use crate::pam::{auth_stack_names_with, stack};

    fn configured_in(files: &[(&str, &str)]) -> bool {
        auth_stack_names_with(&stack(files), "polkit-1", "pam_fprintd", 0)
    }

    #[test]
    fn an_auth_line_naming_pam_fprintd_is_configured() {
        assert!(configured_in(&[(
            "polkit-1",
            "auth sufficient pam_fprintd.so\n"
        )]));
    }

    #[test]
    fn a_clamshell_gate_in_front_still_counts() {
        assert!(configured_in(&[(
            "polkit-1",
            "auth  [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth  sufficient pam_fprintd.so
auth  required pam_unix.so
",
        )]));
    }

    #[test]
    fn a_line_reached_through_an_include_counts() {
        // Stock polkit-1 includes system-auth; a hand-made stack may put the
        // fingerprint there rather than in polkit-1 itself.
        assert!(configured_in(&[
            ("polkit-1", "#%PAM-1.0\nauth include system-auth\n"),
            (
                "system-auth",
                "auth sufficient pam_fprintd.so\nauth required pam_unix.so\n"
            ),
        ]));
        assert!(
            !configured_in(&[
                ("polkit-1", "#%PAM-1.0\nauth include system-auth\n"),
                ("system-auth", crate::pam::SYSTEM_AUTH),
            ]),
            "stock Arch without a fingerprint line anywhere"
        );
    }

    #[test]
    fn account_and_session_lines_and_comments_do_not_count() {
        assert!(!configured_in(&[(
            "polkit-1",
            "# auth sufficient pam_fprintd.so
account  required pam_fprintd.so
session  required pam_fprintd.so
auth     required pam_unix.so
",
        )]));
    }

    #[test]
    fn a_missing_file_is_not_configured() {
        assert!(!configured_in(&[]));
        assert!(!configured_in(&[("polkit-1", "   \n# nothing\n")]));
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
