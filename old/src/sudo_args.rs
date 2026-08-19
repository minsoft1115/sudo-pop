//! Reading a sudo command line the way sudo itself would.
//!
//! Two places need this: the wrapper, to notice that the caller already chose a
//! password source, and askpass, to show which command is asking. Both hinge on
//! knowing where sudo's own options stop and the command begins, which means
//! knowing which options swallow the next argument.

use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;

/// Short sudo options that consume a value, either glued to the bundle
/// (`-uroot`) or as the following argument (`-u root`).
const SHORT_TAKES_VALUE: &[u8] = b"CDghpRTUu";

/// Long sudo options that consume a value when written without `=`.
const LONG_TAKES_VALUE: &[&str] = &[
    "close-from",
    "chdir",
    "group",
    "host",
    "prompt",
    "chroot",
    "command-timeout",
    "other-user",
    "user",
];

/// Options that mean the caller already decided how the password is supplied.
/// If any of them is present we must not add `-A` on top.
const CONFLICTING_SHORT: &[u8] = b"AnS";
const CONFLICTING_LONG: &[&str] = &["askpass", "non-interactive", "stdin"];

/// True if the caller already passed -A, -n or -S in any spelling.
///
/// Scans only the option section: parsing stops at `--` or at the first
/// argument that is not an option, which is where the command begins. Options
/// that take a value are skipped over so a value like `-u -n-ish-user` cannot
/// be mistaken for a flag.
pub fn has_conflicting_flag(args: &[OsString]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_bytes();

        if arg == b"--" {
            return false;
        }

        if let Some(long) = arg.strip_prefix(b"--") {
            let (name, has_value) = match long.iter().position(|&c| c == b'=') {
                Some(eq) => (&long[..eq], true),
                None => (long, false),
            };
            let name = String::from_utf8_lossy(name).into_owned();
            if CONFLICTING_LONG.contains(&name.as_str()) {
                return true;
            }
            if !has_value && LONG_TAKES_VALUE.contains(&name.as_str()) {
                i += 1; // value is the next argument
            }
        } else if arg.len() > 1 && arg[0] == b'-' {
            let bundle = &arg[1..];
            for (pos, &c) in bundle.iter().enumerate() {
                if CONFLICTING_SHORT.contains(&c) {
                    return true;
                }
                if SHORT_TAKES_VALUE.contains(&c) {
                    // The rest of the bundle is this option's value; if the
                    // bundle ends here the value is the next argument.
                    if pos + 1 == bundle.len() {
                        i += 1;
                    }
                    break;
                }
            }
        } else {
            return false; // first non-option argument: the command itself
        }

        i += 1;
    }
    false
}

/// Index of the first argument that is not a sudo option — where the command
/// starts. `None` when the arguments are all options.
pub fn command_start(args: &[OsString]) -> Option<usize> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_bytes();

        if arg == b"--" {
            return (i + 1 < args.len()).then_some(i + 1);
        }

        if let Some(long) = arg.strip_prefix(b"--") {
            let (name, has_value) = match long.iter().position(|&c| c == b'=') {
                Some(eq) => (&long[..eq], true),
                None => (long, false),
            };
            let name = String::from_utf8_lossy(name).into_owned();
            if !has_value && LONG_TAKES_VALUE.contains(&name.as_str()) {
                i += 1;
            }
        } else if arg.len() > 1 && arg[0] == b'-' {
            let bundle = &arg[1..];
            for (pos, &c) in bundle.iter().enumerate() {
                if SHORT_TAKES_VALUE.contains(&c) {
                    if pos + 1 == bundle.len() {
                        i += 1;
                    }
                    break;
                }
            }
        } else {
            return Some(i);
        }

        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn plain_command_has_no_conflict() {
        assert!(!has_conflicting_flag(&args(["pacman", "-Syu"].as_ref())));
    }

    #[test]
    fn detects_short_flags() {
        assert!(has_conflicting_flag(&args(["-n", "true"].as_ref())));
        assert!(has_conflicting_flag(&args(["-S", "true"].as_ref())));
        assert!(has_conflicting_flag(&args(["-A", "true"].as_ref())));
    }

    #[test]
    fn detects_bundled_flags() {
        assert!(has_conflicting_flag(&args(["-kn", "true"].as_ref())));
        assert!(has_conflicting_flag(&args(["-nk", "true"].as_ref())));
    }

    #[test]
    fn detects_long_flags() {
        assert!(has_conflicting_flag(&args(["--non-interactive"].as_ref())));
        assert!(has_conflicting_flag(&args(["--stdin", "true"].as_ref())));
        assert!(has_conflicting_flag(&args(["--askpass", "true"].as_ref())));
    }

    #[test]
    fn skips_option_values() {
        // "-n" here is the user name, not the non-interactive flag.
        assert!(!has_conflicting_flag(&args(["-u", "-n", "true"].as_ref())));
        assert!(!has_conflicting_flag(&args(
            ["--user", "-S", "true"].as_ref()
        )));
        assert!(!has_conflicting_flag(&args(["--user=-A", "true"].as_ref())));
        // Value glued to the bundle.
        assert!(!has_conflicting_flag(&args(["-un", "true"].as_ref())));
    }

    #[test]
    fn finds_flag_after_a_valued_option() {
        assert!(has_conflicting_flag(&args(
            ["-u", "root", "-n", "true"].as_ref()
        )));
    }

    #[test]
    fn stops_at_command_and_double_dash() {
        // Flags belonging to the command must not be read as sudo options.
        assert!(!has_conflicting_flag(&args(
            ["grep", "-n", "pattern"].as_ref()
        )));
        assert!(!has_conflicting_flag(&args(["--", "-n"].as_ref())));
    }

    #[test]
    fn passthrough_options_are_not_conflicts() {
        for a in [
            ["-k"].as_ref(),
            ["-v"].as_ref(),
            ["-l"].as_ref(),
            ["-e", "/etc/hosts"].as_ref(),
        ] {
            assert!(!has_conflicting_flag(&args(a)), "{a:?}");
        }
    }
}
