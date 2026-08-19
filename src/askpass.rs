//! Askpass mode — the process sudo forks to collect a password.
//!
//! Reached through the symlink whose basename is `askpass`; sudo hands us the
//! prompt as argv[1] and reads the first line of our stdout. It knows nothing
//! about the terminal the command will run in, and neither do we; that
//! separation is the whole point.
//!
//! This is one turn of the same conversation the agent has many of, so it uses
//! the same window. Only the destination differs: here the answer goes to the
//! descriptor sudo is reading rather than to the polkit helper.

use std::ffi::OsString;
use std::sync::mpsc::channel;

use crate::gui::{self, FromUi, Subject, ToUi};
use crate::secret::PasswordChannel;
use crate::{attempts, harden, invocation};

/// Shown when sudo gives us no prompt of its own.
const DEFAULT_PROMPT: &str = "Password:";

/// Entry point for askpass mode. Never returns.
///
/// Exit 0 means a password was written. Every other outcome exits non-zero
/// having written nothing at all -- an empty line reads as a wrong password and
/// costs an attempt out of the shared faillock budget.
pub fn run(prompt: Option<OsString>) -> ! {
    harden::apply();
    if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
        harden::report();
    }

    let channel_out = match PasswordChannel::take() {
        Ok(channel) => channel,
        Err(e) => {
            eprintln!("sudo-pop: cannot isolate stdout: {e}");
            std::process::exit(1);
        }
    };

    // Refuse before drawing anything when the per-command allowance is spent:
    // prompting there can only waste an attempt.
    if attempts::used() >= attempts::MAX_ATTEMPTS {
        eprintln!("sudo-pop: too many attempts for one command");
        std::process::exit(1);
    }

    let prompt = prompt
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT.to_owned());

    // Asking while the account is locked can only waste the attempt, and the
    // terminal message saying so is hidden behind the dim-around rule.
    let budget = attempts::budget();
    if let Some(budget) = &budget
        && budget.is_locked()
    {
        match budget.unlock_in {
            Some(secs) => eprintln!("sudo-pop: account locked, {secs}s to go"),
            None => eprintln!("sudo-pop: account is locked out"),
        }
        std::process::exit(1);
    }
    let warning = budget.and_then(|budget| {
        (budget.remaining < attempts::WARN_BELOW).then(|| {
            format!(
                "{} attempt(s) left before the account locks",
                budget.remaining
            )
        })
    });

    let (to_ui_tx, to_ui_rx) = channel::<ToUi>();
    let (from_ui_tx, from_ui_rx) = channel::<FromUi>();

    let worker = std::thread::spawn(move || {
        if let Some(text) = warning {
            let _ = to_ui_tx.send(ToUi::Error(text));
        }
        let _ = to_ui_tx.send(ToUi::Prompt {
            text: prompt,
            echo: false,
        });
        let written = match from_ui_rx.recv() {
            Ok(FromUi::Answer(mut secret)) => {
                attempts::record();
                let sent = channel_out.send(&secret).is_ok();
                secret.wipe();
                sent
            }
            _ => false,
        };
        let _ = to_ui_tx.send(ToUi::Done);
        written
    });

    let subject = Subject {
        command: invocation::command_from_sudo(),
        message: "sudo".to_owned(),
    };

    if let Err(e) = gui::run(subject, to_ui_rx, from_ui_tx) {
        eprintln!("sudo-pop: {e}");
        std::process::exit(1);
    }

    match worker.join() {
        Ok(true) => std::process::exit(0),
        _ => std::process::exit(1),
    }
}
