//! Reading a PAM stack the way PAM reads it.
//!
//! Two questions are asked of `/etc/pam.d`: does the polkit stack try a
//! fingerprint before a password (`fingerprint.rs`), and does it record
//! failures in faillock (`attempts.rs`). Both are "does an `auth` line name
//! this module", and both have to follow `include` and `substack`, because
//! the module may live in `system-auth` rather than in the service's own
//! file. One reader, so the two answers cannot drift apart.

use std::fs;

/// The bound on `include` nesting. Real stacks are two or three deep; a loop
/// would otherwise never end.
const MAX_INCLUDE_DEPTH: u32 = 8;

/// True when an `auth` line of `service`'s effective PAM stack names `module`
/// (`"pam_fprintd"`, `"pam_faillock"`), following includes.
pub fn auth_stack_names(service: &str, module: &str) -> bool {
    auth_stack_names_with(&read_service, service, module, 0)
}

/// A service's PAM file as PAM would find it: `/etc/pam.d` first, then the
/// vendor copy in `/usr/lib/pam.d`. `None` when neither exists.
fn read_service(service: &str) -> Option<String> {
    ["/etc/pam.d", "/usr/lib/pam.d"]
        .iter()
        .find_map(|dir| fs::read_to_string(format!("{dir}/{service}")).ok())
}

/// The reader behind `auth_stack_names`, with the file lookup injected so it
/// can be tested against an in-memory `pam.d`.
///
/// Only the `auth` group counts: `pam_faillock` also appears in `account`
/// lines, where it does not record failures, and `pam_fprintd` in a `session`
/// line would not ask for a finger. Comments are skipped, and a bracketed
/// control such as `[success=1 default=ignore]` is stepped over as one field
/// even though it contains spaces. The module need not be first: a clamshell
/// `pam_exec` gate may precede it.
pub(crate) fn auth_stack_names_with(
    read: &dyn Fn(&str) -> Option<String>,
    service: &str,
    module: &str,
    depth: u32,
) -> bool {
    if depth > MAX_INCLUDE_DEPTH {
        return false;
    }
    let Some(text) = read(service) else {
        return false;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next() else {
            continue;
        };
        // Debian's `@include other` has no type and applies to every group.
        if kind == "@include" {
            if let Some(target) = fields.next()
                && auth_stack_names_with(read, target, module, depth + 1)
            {
                return true;
            }
            continue;
        }
        if kind != "auth" && kind != "-auth" {
            continue;
        }
        let Some(mut control) = fields.next() else {
            continue;
        };
        if control.starts_with('[') {
            while !control.ends_with(']') {
                match fields.next() {
                    Some(more) => control = more,
                    None => break,
                }
            }
        }
        let Some(target) = fields.next() else {
            continue;
        };
        if control == "include" || control == "substack" {
            if auth_stack_names_with(read, target, module, depth + 1) {
                return true;
            }
        } else if target.contains(module) {
            return true;
        }
    }
    false
}

/// A `pam.d` in memory for tests: service name to file text.
#[cfg(test)]
pub(crate) fn stack(files: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name: &str| {
        files
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
}

/// Arch's `/etc/pam.d/system-auth`, as the tests need it.
#[cfg(test)]
pub(crate) const SYSTEM_AUTH: &str = "\
auth       required                    pam_faillock.so      preauth silent deny=10
auth       [success=2 default=ignore]  pam_systemd_home.so
auth       [success=1 default=bad]     pam_unix.so          try_first_pass nullok
auth       [default=die]               pam_faillock.so      authfail deny=10
auth       optional                    pam_permit.so
auth       required                    pam_env.so
auth       required                    pam_faillock.so      authsucc
account    required                    pam_faillock.so
";

#[cfg(test)]
mod tests {
    use super::*;

    fn names(read: &dyn Fn(&str) -> Option<String>, service: &str, module: &str) -> bool {
        auth_stack_names_with(read, service, module, 0)
    }

    #[test]
    fn stock_arch_counts_through_system_auth() {
        let read = stack(&[
            (
                "polkit-1",
                "#%PAM-1.0\nauth include system-auth\naccount include system-auth\n",
            ),
            ("system-auth", SYSTEM_AUTH),
        ]);
        assert!(names(&read, "polkit-1", "pam_faillock"));
        assert!(
            !names(&read, "polkit-1", "pam_fprintd"),
            "a module that is not there is not found either"
        );
    }

    #[test]
    fn omarchys_fingerprint_polkit_stack_does_not_count() {
        let read = stack(&[
            (
                "polkit-1",
                "auth      [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth      sufficient pam_fprintd.so
auth      required pam_unix.so

account   required pam_unix.so
password  required pam_unix.so
session   required pam_unix.so
",
            ),
            ("system-auth", SYSTEM_AUTH),
        ]);
        assert!(
            !names(&read, "polkit-1", "pam_faillock"),
            "nothing includes system-auth, so nothing counts"
        );
        assert!(
            names(&read, "polkit-1", "pam_fprintd"),
            "the gate in front does not hide the fingerprint line"
        );
    }

    #[test]
    fn the_fingerprint_lines_over_an_include_still_count() {
        let read = stack(&[
            (
                "sudo",
                "auth      [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth      sufficient pam_fprintd.so
#%PAM-1.0
auth\t\tinclude\t\tsystem-auth
account\t\tinclude\t\tsystem-auth
",
            ),
            ("system-auth", SYSTEM_AUTH),
        ]);
        assert!(names(&read, "sudo", "pam_faillock"));
    }

    #[test]
    fn substack_and_debian_include_are_followed() {
        let read = stack(&[
            ("a", "auth substack b\n"),
            ("b", "@include c\n"),
            ("c", "auth required pam_faillock.so preauth\n"),
        ]);
        assert!(names(&read, "a", "pam_faillock"));
    }

    #[test]
    fn a_module_outside_the_auth_group_does_not_count() {
        let read = stack(&[(
            "svc",
            "account required pam_faillock.so\nsession required pam_fprintd.so\nauth required pam_unix.so\n",
        )]);
        assert!(!names(&read, "svc", "pam_faillock"));
        assert!(!names(&read, "svc", "pam_fprintd"));
    }

    #[test]
    fn a_commented_line_does_not_count() {
        let read = stack(&[(
            "svc",
            "# auth required pam_faillock.so\nauth required pam_unix.so\n",
        )]);
        assert!(!names(&read, "svc", "pam_faillock"));
        let read = stack(&[("svc", "# auth sufficient pam_fprintd.so\n")]);
        assert!(!names(&read, "svc", "pam_fprintd"));
    }

    #[test]
    fn a_missing_file_or_an_include_loop_is_not_counted() {
        let read = stack(&[("a", "auth include b\n"), ("b", "auth include a\n")]);
        assert!(!names(&read, "a", "pam_faillock"), "a loop ends");
        assert!(!names(&read, "nope", "pam_faillock"));
        let read = stack(&[("svc", "")]);
        assert!(
            !names(&read, "svc", "pam_fprintd"),
            "an empty file has nothing"
        );
    }

    #[test]
    fn module_arguments_do_not_change_the_answer() {
        // We do not parse `max-tries` or `deny`. Presence is the whole fact.
        let read = stack(&[(
            "svc",
            "auth sufficient pam_fprintd.so max-tries=1 timeout=10\n",
        )]);
        assert!(names(&read, "svc", "pam_fprintd"));
    }
}
