//! Wrapper mode: what runs when the user types `sudo <args>` through the alias.
//!
//! A fork in the road, not a translation. Anything polkit can carry goes to
//! `run0`, where our agent draws the prompt and the command runs as a systemd
//! unit rather than through setuid. Everything else stays on sudo, with our own
//! window supplying the password:
//!
//!   sudo pacman -Syu     -> run0 pacman -Syu
//!   sudo -E make install -> sudo -A make install   (run0 has no -E)
//!   sudo VAR=1 make      -> sudo -A VAR=1 make     (run0 would drop VAR)
//!
//! Deciding is left to `sudo_args`, which already knows which options swallow
//! the next word. A shell alias or function cannot make the same call, and two
//! places deciding the same thing will disagree eventually.
//!
//! Every failure path falls back to plain sudo. Not being able to show a popup
//! is a far smaller problem than not being able to run sudo at all.

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

fn exec_run0(args: &[OsString]) -> ! {
    let mut cmd = Command::new("run0");
    cmd.args(args);
    let e = cmd.exec();
    // run0 missing or unrunnable: sudo is still there.
    debug(&format!("run0 did not start ({e}), falling back to sudo"));
    exec_sudo(None, args)
}

/// Replace this process with sudo. Never returns on success.
fn exec_sudo(askpass: Option<&OsStr>, args: &[OsString]) -> ! {
    let mut cmd = Command::new("sudo");
    if let Some(link) = askpass {
        cmd.arg("-A");
        cmd.env("SUDO_ASKPASS", link);
    }
    cmd.args(args);

    let e = cmd.exec(); // only returns on failure
    eprintln!("sudo-pop: cannot execute sudo: {e}");
    std::process::exit(1);
}

/// Entry point for wrapper mode.
pub fn run(args: &[OsString]) -> ! {
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
}
