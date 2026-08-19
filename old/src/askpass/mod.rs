//! Askpass mode — the short-lived process sudo forks to collect a password.
//!
//! sudo hands us the prompt as argv[1] and reads the first line of our stdout.
//! It knows nothing about the terminal the command will run in, and we know
//! nothing about it either; that separation is the whole point of the design.
//!
//! Order matters here. Hardening runs before a password can exist, and stdout
//! is put out of reach before anything can print to it by accident.

mod font;
mod gui;
mod harden;
mod invocation;
mod secret;
mod theme;

use std::ffi::OsString;

use secret::{PasswordChannel, Secret};

/// Shown when sudo gives us no prompt of its own.
const DEFAULT_PROMPT: &str = "Password:";

fn debug_enabled() -> bool {
    std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty())
}

/// Entry point for askpass mode. Never returns.
///
/// Exit 0 means a password was written. Every other outcome exits non-zero
/// having written nothing at all — see `secret` for why that distinction
/// decides between one failed attempt and a locked account.
pub fn run(prompt: Option<OsString>) -> ! {
    harden::apply();
    if debug_enabled() {
        harden::report();
    }

    let channel = match PasswordChannel::take() {
        Ok(channel) => channel,
        Err(e) => {
            eprintln!("sudo-pop: cannot isolate stdout: {e}");
            std::process::exit(1);
        }
    };

    // Refuse before drawing anything when the account is already locked or the
    // per-command allowance is spent: prompting there can only waste an
    // attempt, and both paths must stay silent on stdout.
    let used = crate::attempts::used();
    if debug_enabled() {
        eprintln!(
            "sudo-pop: attempts used={used}/{}",
            crate::attempts::MAX_ATTEMPTS
        );
    }
    if used >= crate::attempts::MAX_ATTEMPTS {
        eprintln!("sudo-pop: too many attempts for one command");
        std::process::exit(1);
    }

    let budget = crate::attempts::budget();
    if let Some(b) = budget.as_ref().filter(|b| b.is_locked()) {
        gui::notice(&locked_message(b.unlock_in));
        std::process::exit(1);
    }
    let warning = budget.as_ref().and_then(warning_for);

    let prompt = prompt
        .as_deref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());

    let Some(mut password) = prompt_for_password(&prompt, warning.as_deref()) else {
        std::process::exit(1);
    };

    // An empty buffer must never be sent: sudo would read it as a wrong
    // password and retry, spending the failure budget the caller still needs.
    if password.is_empty() {
        std::process::exit(1);
    }

    // Counted here rather than when the window opened: a cancelled prompt
    // cannot loop, so only a real submission spends the allowance.
    crate::attempts::record();

    let sent = channel.send(&password);
    password.wipe();

    match sent {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("sudo-pop: cannot send password: {e}");
            std::process::exit(1);
        }
    }
}

/// Collect the password from the user.
fn prompt_for_password(prompt: &str, warning: Option<&str>) -> Option<Secret> {
    gui::prompt(prompt, warning)
}

/// Warn only when the account is close to locking out.
fn warning_for(budget: &crate::attempts::Budget) -> Option<String> {
    (budget.remaining < crate::attempts::WARN_BELOW).then(|| {
        let plural = if budget.remaining == 1 { "" } else { "s" };
        format!(
            "{} failed attempt{plural} left before lockout",
            budget.remaining
        )
    })
}

fn locked_message(unlock_in: Option<u64>) -> String {
    match unlock_in {
        Some(secs) => format!("Account locked by failed attempts.\nTry again in about {secs}s."),
        None => "Account locked by failed attempts.\nTry again shortly.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attempts::Budget;

    fn budget(remaining: u32) -> Budget {
        Budget {
            remaining,
            unlock_in: None,
        }
    }

    #[test]
    fn warns_only_when_the_budget_runs_low() {
        assert_eq!(warning_for(&budget(9)), None);
        assert_eq!(warning_for(&budget(4)), None);
        assert!(warning_for(&budget(3)).is_some());
        assert!(warning_for(&budget(1)).is_some());
    }

    #[test]
    fn warning_counts_read_naturally() {
        assert_eq!(
            warning_for(&budget(1)).unwrap(),
            "1 failed attempt left before lockout"
        );
        assert_eq!(
            warning_for(&budget(2)).unwrap(),
            "2 failed attempts left before lockout"
        );
    }

    #[test]
    fn locked_message_mentions_the_wait_when_known() {
        assert!(locked_message(Some(90)).contains("90s"));
        assert!(locked_message(None).contains("shortly"));
        assert!(!locked_message(None).contains("about"));
    }
}
