//! Wrapper mode: what runs when the user types `sudo <args>` through the alias.
//!
//! The job is small on purpose. Decide whether a GUI prompt is even possible,
//! point SUDO_ASKPASS at our own binary, and hand the process over to sudo with
//! `exec` so exit codes, signals, job control and TTY ownership need no
//! forwarding logic of our own.
//!
//! Every failure path falls back to plain sudo. Not being able to show a popup
//! is a far smaller problem than not being able to run sudo at all.

use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::attempts;
use crate::paths;
use crate::sudo_args::has_conflicting_flag;

fn debug(msg: &str) {
    if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
        eprintln!("sudo-pop: {msg}");
    }
}

/// True if some display server is reachable.
///
/// Without this check an SSH session or a bare TTY would hand sudo an askpass
/// helper that can never draw a window, and the prompt would hang with no way
/// to type a password.
fn has_display() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
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

    // 3. No display: a popup cannot appear, so keep the terminal prompt.
    if !has_display() {
        debug("no WAYLAND_DISPLAY or DISPLAY, using terminal prompt");
        exec_sudo(None, args);
    }

    // 4. No runtime dir: nowhere private to put the askpass link.
    if let Err(e) = paths::runtime_dir() {
        debug(&format!("{e}, using terminal prompt"));
        exec_sudo(None, args);
    }

    // 5. Link could not be prepared.
    let link = match paths::ensure_askpass_symlink() {
        Ok(link) => link,
        Err(e) => {
            debug(&format!(
                "askpass link unavailable ({e}), using terminal prompt"
            ));
            exec_sudo(None, args);
        }
    };

    // 6 & 7. Point sudo at ourselves and hand over the process.
    // The prompt allowance is per sudo command, so it starts fresh here rather
    // than decaying on a timer.
    attempts::reset();
    debug(&format!("askpass at {}", link.display()));
    exec_sudo(Some(link.as_os_str()), args)
}
