//! Prompt mode (`--agent-prompt`): the short-lived child that owns one request.
//!
//! The daemon never sees the password. It forks this, hands over what is needed
//! to ask, and reads the exit code. Everything that touches the secret happens
//! here, in a process that lives for one authentication and dies:
//!
//!   hardening -> window -> helper conversation -> exit code
//!
//! The helper conversation runs on a second thread because the window owns the
//! main one (winit allows a single event loop per process).

use std::sync::mpsc::{Receiver, Sender, channel};

use crate::gui::{self, FromUi, Subject, ToUi};
use crate::helper::{self, Conversation, Outcome};
use crate::secret::Secret;
use crate::attempts::{self, MAX_ATTEMPTS};
use crate::{harden, invocation};

/// Exit codes. The daemon turns these back into a D-Bus answer, so the
/// distinction between "failed" and "cancelled" matters: reporting a refusal
/// as an error makes polkitd re-issue the request forever.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_CANCELLED: i32 = 2;

/// Bridge between the helper thread and the window.
struct WindowConversation {
    to_ui: Sender<ToUi>,
    from_ui: Receiver<FromUi>,
}

impl Conversation for WindowConversation {
    fn ask(&mut self, prompt: &str, echo: bool) -> Option<Secret> {
        self.to_ui
            .send(ToUi::Prompt {
                text: prompt.to_owned(),
                echo,
            })
            .ok()?;
        match self.from_ui.recv() {
            Ok(FromUi::Answer(secret)) => Some(secret),
            _ => None,
        }
    }

    fn info(&mut self, text: &str) {
        let _ = self.to_ui.send(ToUi::Info(text.to_owned()));
    }

    fn error(&mut self, text: &str) {
        let _ = self.to_ui.send(ToUi::Error(text.to_owned()));
    }
}

/// Drive up to `MAX_ATTEMPTS` authentications, re-prompting after a wrong
/// password and stopping on anything else. The cap is per cookie, so this is
/// where "three tries then give up" for one request lives.
///
/// `authenticate` is a parameter so the loop can be tested without a helper, a
/// window, or a password.
fn run_attempts(
    conv: &mut dyn Conversation,
    mut authenticate: impl FnMut(&mut dyn Conversation) -> Outcome,
) -> Outcome {
    let mut last = Outcome::Failed;
    for attempt in 1..=MAX_ATTEMPTS {
        last = authenticate(conv);
        match last {
            // PAM already said its piece; this is the one word the window needs
            // before asking again.
            Outcome::Failed if attempt < MAX_ATTEMPTS => conv.error("Wrong"),
            _ => break,
        }
    }
    last
}

/// Entry point for prompt mode. Never returns.
pub fn run() -> ! {
    harden::apply();
    if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
        harden::report();
    }

    let username = std::env::var("SUDO_POP_USER").unwrap_or_default();
    let message = std::env::var("SUDO_POP_MESSAGE").unwrap_or_default();
    let subject_pid: u32 = std::env::var("SUDO_POP_SUBJECT_PID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // The cookie arrives on stdin rather than in argv or the environment, both
    // of which any process on the machine can read.
    let mut cookie = String::new();
    if std::io::stdin().read_line(&mut cookie).is_err() || cookie.trim().is_empty() {
        eprintln!("sudo-pop: no cookie on stdin");
        std::process::exit(EXIT_FAILED);
    }
    let cookie = cookie.trim_end_matches('\n').to_owned();

    if username.is_empty() {
        eprintln!("sudo-pop: no user to authenticate");
        std::process::exit(EXIT_FAILED);
    }

    // faillock is shared with sudo and login (deny=10), so a prompt spent on a
    // locked account only burns everyone's budget. The live tally is also the
    // cross-cookie cap: each request re-reads it, so repeated requests cannot
    // hand out three fresh attempts each once the account is close to locking.
    let budget = attempts::budget();
    if let Some(reason) = budget.as_ref().and_then(attempts::Budget::refusal) {
        // Report as cancelled, not failed: a failure has polkitd re-issue the
        // request and the window would reopen forever (see helper.rs, §3-3).
        eprintln!("sudo-pop: {reason}");
        std::process::exit(EXIT_CANCELLED);
    }
    let warning = budget.and_then(|budget| budget.warning());

    // Kept for the window before `username` is moved into the worker.
    let user_display = username.clone();

    let (to_ui_tx, to_ui_rx) = channel::<ToUi>();
    let (from_ui_tx, from_ui_rx) = channel::<FromUi>();

    let worker = std::thread::spawn(move || {
        let mut conv = WindowConversation {
            to_ui: to_ui_tx.clone(),
            from_ui: from_ui_rx,
        };
        if let Some(text) = warning {
            let _ = to_ui_tx.send(ToUi::Error(text));
        }
        let last = run_attempts(&mut conv, |conv| {
            helper::authenticate(&username, &cookie, conv)
        });
        let _ = to_ui_tx.send(ToUi::Done);
        last
    });

    let subject = Subject {
        command: (subject_pid != 0)
            .then(|| invocation::command_of(subject_pid))
            .flatten(),
        message,
        user: (!user_display.is_empty()).then_some(user_display),
    };

    if let Err(e) = gui::run(subject, to_ui_rx, from_ui_tx) {
        eprintln!("sudo-pop: {e}");
        // Without a window there is nothing to type into; end the request
        // rather than leaving the caller waiting for the full 25 seconds.
        std::process::exit(EXIT_CANCELLED);
    }

    let outcome = worker.join().unwrap_or(Outcome::Failed);
    std::process::exit(match outcome {
        Outcome::Success => EXIT_SUCCESS,
        Outcome::Failed => EXIT_FAILED,
        Outcome::Cancelled | Outcome::RefusedWithoutPrompt => EXIT_CANCELLED,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records what the window was told; answers nothing (the scripted
    /// `authenticate` never asks it to).
    struct Rec {
        errors: Vec<String>,
    }
    impl Conversation for Rec {
        fn ask(&mut self, _prompt: &str, _echo: bool) -> Option<Secret> {
            None
        }
        fn info(&mut self, _text: &str) {}
        fn error(&mut self, text: &str) {
            self.errors.push(text.to_owned());
        }
    }

    /// Run the loop against a fixed list of outcomes, counting the attempts.
    fn drive(outcomes: Vec<Outcome>) -> (Outcome, usize, Rec) {
        let mut rec = Rec { errors: Vec::new() };
        let mut it = outcomes.into_iter();
        let mut calls = 0;
        let last = run_attempts(&mut rec, |_conv| {
            calls += 1;
            it.next().expect("run_attempts asked more times than scripted")
        });
        (last, calls, rec)
    }

    #[test]
    fn a_right_answer_stops_after_one_attempt() {
        let (last, calls, rec) = drive(vec![Outcome::Success]);
        assert_eq!(last, Outcome::Success);
        assert_eq!(calls, 1);
        assert!(rec.errors.is_empty(), "no retry, so no 'Wrong'");
    }

    #[test]
    fn a_cancel_stops_immediately() {
        let (last, calls, _) = drive(vec![Outcome::Cancelled]);
        assert_eq!(last, Outcome::Cancelled);
        assert_eq!(calls, 1);
    }

    #[test]
    fn wrong_then_right_retries_once() {
        let (last, calls, rec) = drive(vec![Outcome::Failed, Outcome::Success]);
        assert_eq!(last, Outcome::Success);
        assert_eq!(calls, 2);
        assert_eq!(rec.errors, vec!["Wrong".to_owned()]);
    }

    #[test]
    fn three_wrong_answers_stop_at_the_cap() {
        let (last, calls, rec) = drive(vec![Outcome::Failed; MAX_ATTEMPTS as usize]);
        assert_eq!(last, Outcome::Failed);
        assert_eq!(calls, MAX_ATTEMPTS as usize, "no fourth prompt");
        // "Wrong" between attempts, but not after the final one.
        assert_eq!(rec.errors.len(), (MAX_ATTEMPTS - 1) as usize);
    }
}
