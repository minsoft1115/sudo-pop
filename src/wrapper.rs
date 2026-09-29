//! Wrapper policy. Default: run0 only, with no implicit sudo execution.
//! `SUDO_POP_MODE=compat` explicitly opts into the historical sudo routing.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::attempts;
use crate::paths;
use crate::sudo_args::{command_start, has_conflicting_flag};

fn debug(msg: &str) {
    if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
        eprintln!("sudo-pop: {msg}");
    }
}

/// True if some display server is reachable.
fn has_display() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

/// `NAME=value` in the command position is an environment assignment, which
/// sudo applies and run0 would silently drop.
fn is_assignment(arg: &OsStr) -> bool {
    let bytes = arg.as_bytes();
    match bytes.iter().position(|&b| b == b'=') {
        Some(0) | None => false,
        Some(eq) => bytes[..eq]
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'_'),
    }
}

/// Can this invocation go to run0 unchanged?
///
/// Only when there is nothing but a command: no sudo options, no environment
/// assignments. Anything else keeps sudo's meaning, which run0 does not share.
fn plain_command(args: &[OsString]) -> bool {
    matches!(command_start(args), Some(0)) && !args.first().is_some_and(|a| is_assignment(a))
}

/// Absolute path to a system binary, chosen so that a `sudo` (or `run0`) shim
/// earlier on PATH cannot capture our own call to the real thing.
///
/// The wrapper runs *as* `sudo` — through the alias, and increasingly through a
/// PATH shim another tool (minsh) installs to catch non-interactive
/// `bash -c "sudo …"`, which the alias never sees. If we then spawned a bare
/// `sudo`/`run0`, PATH would resolve it straight back to that shim:
/// sudo-pop → shim → sudo-pop, forever, and every branch here except the run0
/// one goes through `exec_sudo`. The shim is expected to strip itself from PATH
/// before exec-ing us, but leaning on that alone is one mistake away from an
/// unkillable loop. An absolute path shuts the door on our side regardless.
///
/// The candidates are the standard locations, first existing wins. If none do —
/// an unusual layout — we fall back to the bare name rather than refuse, keeping
/// the "always reach the real tool" guarantee; there the shim's own PATH
/// cleaning is the remaining defence.
fn absolute(name: &str, candidates: &[&str]) -> OsString {
    for c in candidates {
        if std::path::Path::new(c).exists() {
            return OsString::from(c);
        }
    }
    OsString::from(name)
}

fn real_sudo() -> OsString {
    absolute("sudo", &["/usr/bin/sudo", "/bin/sudo"])
}

fn real_run0() -> OsString {
    absolute("run0", &["/usr/bin/run0", "/bin/run0"])
}

fn exec_run0(args: &[OsString]) -> ! {
    let mut cmd = Command::new(real_run0());
    cmd.args(args);
    let e = cmd.exec();
    // run0 missing or unrunnable: sudo is still there.
    debug(&format!("run0 did not start ({e}), falling back to sudo"));
    exec_sudo(None, args)
}

/// Replace this process with sudo. Never returns on success.
fn exec_sudo(askpass: Option<&OsStr>, args: &[OsString]) -> ! {
    let mut cmd = Command::new(real_sudo());
    if let Some(link) = askpass {
        cmd.arg("-A");
        cmd.env("SUDO_ASKPASS", link);
    }
    cmd.args(args);

    let e = cmd.exec(); // only returns on failure
    eprintln!("sudo-pop: cannot execute sudo: {e}");
    std::process::exit(1);
}

#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Run0,
    Compat,
}

fn mode(value: Option<&OsStr>) -> Result<Mode, &'static str> {
    match value {
        None => Ok(Mode::Run0),
        Some(v) if v == "run0" => Ok(Mode::Run0),
        Some(v) if v == "compat" => Ok(Mode::Compat),
        _ => Err("SUDO_POP_MODE must be run0 or compat"),
    }
}

fn run0_arguments<'a>(
    args: &'a [OsString],
    legacy: Option<&OsStr>,
) -> Result<&'a [OsString], &'static str> {
    if legacy.is_some_and(|v| v != "1") {
        return Err(
            "SUDO_POP_RUN0 conflicts with run0-only mode; unset it (or use SUDO_POP_MODE=compat explicitly)",
        );
    }
    let command = if args.first().is_some_and(|a| a == "--") {
        &args[1..]
    } else {
        if args.first().is_some_and(|a| a.as_bytes().starts_with(b"-")) {
            return Err(
                "sudo options are unsupported in run0-only mode; use /usr/bin/sudo explicitly when needed",
            );
        }
        args
    };
    if command.first().is_none_or(|a| a.is_empty()) {
        return Err("usage: sudo-pop [--] COMMAND [ARG ...] (run0-only mode)");
    }
    if command.first().is_some_and(|a| is_assignment(a)) {
        return Err(
            "leading environment assignments are unsupported in run0-only mode; use /usr/bin/sudo explicitly when needed",
        );
    }
    Ok(command)
}

fn system_run0(candidates: &[&str]) -> Result<OsString, &'static str> {
    candidates
        .iter()
        .find(|p| std::path::Path::new(p).is_absolute() && std::path::Path::new(p).is_file())
        .map(OsString::from)
        .ok_or("system run0 was not found; refusing PATH lookup or sudo fallback")
}

/// Success replaces this process, preserving the command's exit/signal status.
/// A startup error is returned to the caller, never retried through sudo.
fn start_run0(program: &OsStr, args: &[OsString]) -> std::io::Error {
    Command::new(program).arg("--").args(args).exec()
}

pub fn run(args: &[OsString]) -> ! {
    let chosen = mode(std::env::var_os("SUDO_POP_MODE").as_deref());
    let error = match chosen {
        Ok(Mode::Compat) => run_compat(args),
        Err(error) => error.to_owned(),
        Ok(Mode::Run0) => {
            if args == [OsString::from("--help")] {
                println!(
                    "Usage: sudo-pop [--] COMMAND [ARG ...]\nDefault: run0 only; sudo options and automatic sudo fallback are disabled.\nUse /usr/bin/sudo explicitly for sudo semantics, or SUDO_POP_MODE=compat for legacy routing."
                );
                std::process::exit(0);
            }
            match run0_arguments(args, std::env::var_os("SUDO_POP_RUN0").as_deref()) {
                Err(error) => error.to_owned(),
                Ok(command) => match system_run0(&["/usr/bin/run0", "/bin/run0"]) {
                    Err(error) => error.to_owned(),
                    Ok(program) => {
                        let error = start_run0(&program, command);
                        eprintln!(
                            "sudo-pop: cannot execute system run0: {error}; no sudo fallback"
                        );
                        std::process::exit(if error.kind() == std::io::ErrorKind::NotFound {
                            127
                        } else {
                            126
                        });
                    }
                },
            }
        }
    };
    eprintln!("sudo-pop: {error}");
    std::process::exit(2);
}

/// Historical routing, reached only through explicit compatibility selection.
fn run_compat(args: &[OsString]) -> ! {
    // 1. No arguments: let sudo print its own usage.
    if args.is_empty() {
        debug("no arguments, deferring to sudo");
        exec_sudo(None, args);
    }

    // 2. Caller already chose a password source.
    if has_conflicting_flag(args) {
        debug("caller passed -A/-n/-S, leaving arguments untouched");
        exec_sudo(None, args);
    }

    // 3. The fork in the road.
    let routed = std::env::var_os("SUDO_POP_RUN0").is_none_or(|v| v != "0");
    if routed && plain_command(args) {
        debug("plain command, routing to run0");
        exec_run0(args);
    }

    // 4. No display: a popup cannot appear, so keep the terminal prompt.
    if !has_display() {
        debug("no WAYLAND_DISPLAY or DISPLAY, using terminal prompt");
        exec_sudo(None, args);
    }

    // 5. No runtime dir, or no link: nowhere private to put the askpass hook.
    if let Err(e) = paths::runtime_dir() {
        debug(&format!("{e}, using terminal prompt"));
        exec_sudo(None, args);
    }
    let link = match paths::ensure_askpass_symlink() {
        Ok(link) => link,
        Err(e) => {
            debug(&format!("askpass link unavailable ({e}), terminal prompt"));
            exec_sudo(None, args);
        }
    };

    // The prompt allowance is per sudo command, so it starts fresh here.
    attempts::reset();
    exec_sudo(Some(link.as_os_str()), args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn run0_mode_is_default_and_legacy_override_is_not_silent() {
        assert_eq!(mode(None), Ok(Mode::Run0));
        assert_eq!(mode(Some(OsStr::new("run0"))), Ok(Mode::Run0));
        assert_eq!(mode(Some(OsStr::new("compat"))), Ok(Mode::Compat));
        assert!(mode(Some(OsStr::new("typo"))).is_err());
        assert!(run0_arguments(&args(&["true"]), Some(OsStr::new("0"))).is_err());
        assert!(run0_arguments(&args(&["true"]), Some(OsStr::new("garbage"))).is_err());
    }

    #[test]
    fn run0_mode_rejects_sudo_semantics() {
        for input in [
            vec![],
            vec!["--"],
            vec![""],
            vec!["-v"],
            vec!["-k"],
            vec!["-l"],
            vec!["-n", "true"],
            vec!["-A", "true"],
            vec!["-S", "true"],
            vec!["-u", "root", "id"],
            vec!["-E", "make"],
            vec!["FOO=1", "make"],
            vec!["--", "FOO=1", "make"],
        ] {
            assert!(run0_arguments(&args(&input), None).is_err(), "{input:?}");
        }
    }

    #[test]
    fn run0_mode_preserves_command_arguments_and_separator() {
        use std::os::unix::ffi::OsStringExt;
        let input = vec![
            "--".into(),
            "command".into(),
            "".into(),
            "a b".into(),
            "--help".into(),
            OsString::from_vec(vec![b'x', 0xff]),
        ];
        assert_eq!(run0_arguments(&input, None).unwrap(), &input[1..]);
        assert_eq!(run0_arguments(&input[1..], None).unwrap(), &input[1..]);
        assert_eq!(
            run0_arguments(&args(&["--", "-name"]), None).unwrap(),
            args(&["-name"])
        );
    }

    #[test]
    fn run0_resolution_never_uses_path_lookup() {
        assert!(system_run0(&["run0", "/nonexistent/sudo-pop/run0"]).is_err());
        assert!(system_run0(&["/tmp"]).is_err());
        assert_eq!(
            system_run0(&["/bin/sh"]).unwrap(),
            OsString::from("/bin/sh")
        );
    }

    // Only the test harness can inject a fake executable; production has fixed paths.
    #[test]
    fn exec_probe() {
        let Some(program) = std::env::var_os("SUDO_POP_TEST_EXEC") else {
            return;
        };
        use std::os::unix::ffi::OsStringExt;
        let error = start_run0(
            &program,
            &[
                "a b".into(),
                "".into(),
                "--help".into(),
                OsString::from_vec(vec![b'x', 0xff]),
            ],
        );
        eprintln!("probe startup failure: {error}");
        std::process::exit(126);
    }

    #[test]
    fn exec_preserves_bytes_exit_status_and_signal_and_never_retries() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::ExitStatusExt;
        let dir = std::env::temp_dir().join(format!(
            "sudo-pop-exec-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let fake = dir.join("run0");
        let sudo = dir.join("sudo");
        std::fs::write(&sudo, "#!/bin/sh\necho UNEXPECTED_SUDO\nexit 99\n").unwrap();
        std::fs::set_permissions(&sudo, std::fs::Permissions::from_mode(0o700)).unwrap();
        let probe = |program: &std::path::Path| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "wrapper::tests::exec_probe", "--nocapture"])
                .env("SUDO_POP_TEST_EXEC", program)
                .env("PATH", &dir)
                .output()
                .unwrap()
        };
        std::fs::write(&fake, "#!/bin/sh\nprintf '%s\\0' \"$@\"\nexit 37\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = probe(&fake);
        assert_eq!(output.status.code(), Some(37));
        assert!(output.stdout.ends_with(b"--\0a b\0\0--help\0x\xff\0"));
        std::fs::write(&fake, "#!/bin/sh\nkill -TERM $$\n").unwrap();
        assert_eq!(probe(&fake).status.signal(), Some(libc::SIGTERM));
        // Missing executable while both PATH candidates exist must still fail.
        let output = probe(&dir.join("missing"));
        assert_eq!(output.status.code(), Some(126));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("UNEXPECTED_SUDO"));
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(probe(&fake).status.code(), Some(126));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_bare_command_goes_to_run0() {
        assert!(plain_command(&args(&["pacman", "-Syu"])));
        assert!(plain_command(&args(&["ls"])));
    }

    #[test]
    fn sudo_options_keep_it_on_sudo() {
        assert!(!plain_command(&args(&["-E", "make"])));
        assert!(!plain_command(&args(&["-u", "root", "id"])));
        assert!(!plain_command(&args(&["--", "ls"])));
        assert!(!plain_command(&args(&["-v"])));
    }

    #[test]
    fn environment_assignments_keep_it_on_sudo() {
        assert!(!plain_command(&args(&["FOO=1", "make"])));
        assert!(!plain_command(&args(&["PATH=/x:/y", "sh", "-c", "true"])));
    }

    #[test]
    fn an_argument_that_merely_contains_equals_is_not_an_assignment() {
        assert!(plain_command(&args(&["find", "-name=x"])));
        assert!(plain_command(&args(&["=weird"])));
    }

    #[test]
    fn absolute_prefers_an_existing_candidate_over_the_bare_name() {
        // The test binary itself is a path guaranteed to exist right now, so this
        // stays independent of what is installed on the machine.
        let me = std::env::current_exe().unwrap();
        let me_str = me.to_str().unwrap();
        assert_eq!(
            absolute("sudo", &["/no/such/path", me_str]),
            OsString::from(me_str),
        );
    }

    #[test]
    fn absolute_falls_back_to_the_bare_name_when_nothing_exists() {
        // No candidate on disk: keep the "always reach the real tool" guarantee
        // by returning the bare name for a PATH lookup, as before.
        assert_eq!(
            absolute("run0", &["/no/such/path", "/also/missing"]),
            OsString::from("run0"),
        );
    }
}
