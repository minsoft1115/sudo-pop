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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use crate::attempts::{self, MAX_ATTEMPTS};
use crate::gui::{self, FromUi, Subject, ToUi};
use crate::helper::{self, Conversation, Outcome};
use crate::secret::Secret;
use crate::{harden, invocation};

/// Exit codes. The daemon turns these back into a D-Bus answer, so the
/// distinction between "failed" and "cancelled" matters: reporting a refusal
/// as an error makes polkitd re-issue the request forever.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_CANCELLED: i32 = 2;

/// Set by the SIGTERM handler. polkitd's cancel reaches this process as a
/// SIGTERM from the agent (`agent.rs`), and dying on the spot would skip
/// `Channel`'s drop: the socket helper would notice only at its next write,
/// and a forked setuid helper would not be killed at all, leaving PAM at the
/// sensor for the rest of its timeout. So the signal only raises this flag;
/// the window and the helper loop both poll it and take the same road as Esc.
pub static TERMINATED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_terminate(_: libc::c_int) {
    // Async-signal-safe: a relaxed store and nothing else.
    TERMINATED.store(true, Ordering::Relaxed);
}

/// Turn SIGTERM into a cancel rather than an immediate death. A second
/// SIGTERM is the same flag again; the agent escalates to SIGKILL on its own
/// if this process does not leave in time.
fn cancel_on_sigterm() {
    // SAFETY: installing a handler that only touches an atomic.
    unsafe {
        let handler: extern "C" fn(libc::c_int) = on_terminate;
        libc::signal(libc::SIGTERM, handler as usize as libc::sighandler_t);
    }
}

/// True once SIGTERM has arrived. Read by the window each frame and by the
/// helper loop between reads.
pub fn terminated() -> bool {
    TERMINATED.load(Ordering::Relaxed)
}

/// Bridge between the helper thread and the window.
struct WindowConversation {
    to_ui: Sender<ToUi>,
    from_ui: Receiver<FromUi>,
    /// Typed while PAM was still in `pam_fprintd` after a wrong password.
    /// `cancelled` must not wipe it — the next `ask` is that password.
    pending: std::sync::Mutex<Option<Secret>>,
}

impl Conversation for WindowConversation {
    fn ask(&mut self, prompt: &str, echo: bool) -> Option<Secret> {
        self.to_ui
            .send(ToUi::Prompt {
                text: prompt.to_owned(),
                echo,
            })
            .ok()?;
        if let Some(mut secret) = self.pending.lock().ok().and_then(|mut g| g.take()) {
            // The stash was typed into a password field, so it answers a
            // password prompt and nothing else. An echoed prompt (a username,
            // a one-time code) may be logged by the module that asked; a
            // password must never travel that way.
            if !echo {
                return Some(secret);
            }
            secret.wipe();
        }
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

    fn update_attempts(&mut self, attempts: Option<(String, bool)>) {
        let _ = self.to_ui.send(ToUi::Attempts(attempts));
    }

    fn cancelled(&self) -> bool {
        if terminated() {
            return true;
        }
        match self.from_ui.try_recv() {
            Ok(FromUi::Cancel) | Err(TryRecvError::Disconnected) => true,
            Ok(FromUi::Answer(secret)) => {
                if let Ok(mut g) = self.pending.lock()
                    && let Some(mut old) = g.replace(secret)
                {
                    old.wipe();
                }
                false
            }
            Err(TryRecvError::Empty) => false,
        }
    }
}

/// Drive up to `MAX_ATTEMPTS` authentications, re-prompting after a wrong
/// password and stopping on anything else. The cap is per cookie, so this is
/// where "three tries then give up" for one request lives.
///
/// A retry opens a new helper, so PAM starts at `pam_fprintd` again. The
/// window stays on the password field; it does not go back to the sensor.
///
/// `authenticate` is a parameter so the loop can be tested without a helper, a
/// window, or a password.
fn run_attempts(
    conv: &mut dyn Conversation,
    authenticate: impl FnMut(&mut dyn Conversation) -> Outcome,
) -> Outcome {
    run_attempts_with(conv, authenticate, || {
        attempts::budget(attempts::POLKIT_SERVICE)
    })
}

fn run_attempts_with(
    conv: &mut dyn Conversation,
    mut authenticate: impl FnMut(&mut dyn Conversation) -> Outcome,
    mut get_budget: impl FnMut() -> Option<attempts::Budget>,
) -> Outcome {
    let mut last = Outcome::Failed;
    for attempt in 1..=MAX_ATTEMPTS {
        last = authenticate(conv);
        match last {
            // PAM already said its piece; this is the one word the window needs
            // before asking again.
            Outcome::Failed if attempt < MAX_ATTEMPTS => {
                let budget = get_budget();
                if let Some(ref b) = budget {
                    if b.is_locked() {
                        return Outcome::Cancelled;
                    }
                    conv.update_attempts(b.status());
                }
                conv.error(attempts::WRONG_PASSWORD);
            }
            _ => break,
        }
    }
    last
}

/// Entry point for prompt mode. Never returns.
pub fn run() -> ! {
    harden::apply();
    cancel_on_sigterm();
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
    // None of that holds where the polkit-1 stack has no pam_faillock (Omarchy
    // after a fingerprint setup); then there is no budget to show or enforce.
    let budget = attempts::budget(attempts::POLKIT_SERVICE);
    if let Some(reason) = budget.as_ref().and_then(attempts::Budget::refusal) {
        // Report as cancelled, not failed: a failure has polkitd re-issue the
        // request and the window would reopen forever (see helper.rs, §3-3).
        eprintln!("sudo-pop: {reason}");
        std::process::exit(EXIT_CANCELLED);
    }
    let attempts = budget.and_then(|budget| budget.status());

    // The agent measured this from the moment polkitd called it, so a request
    // that queued behind another one does not get the full span offered back.
    let deadline = std::env::var("SUDO_POP_LEFT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));

    // Kept for the window before `username` is moved into the worker.
    let user_display = username.clone();

    // Decided before the helper is connected: this spawns the lid probe, and
    // a child should not be forked out of a process that is mid-conversation.
    let fingerprint_wait = crate::fingerprint::should_wait();
    if std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty()) {
        eprintln!("sudo-pop: fingerprint_wait={fingerprint_wait}");
    }

    let (to_ui_tx, to_ui_rx) = channel::<ToUi>();
    let (from_ui_tx, from_ui_rx) = channel::<FromUi>();

    let worker = std::thread::spawn(move || {
        let mut conv = WindowConversation {
            to_ui: to_ui_tx.clone(),
            from_ui: from_ui_rx,
            pending: std::sync::Mutex::new(None),
        };
        let last = run_attempts(&mut conv, |conv| {
            helper::authenticate(&username, &cookie, conv)
        });
        let _ = to_ui_tx.send(ToUi::Done);
        last
    });

    let command = (subject_pid != 0)
        .then(|| invocation::command_of(subject_pid))
        .flatten();
    let action = std::env::var("SUDO_POP_ACTION").unwrap_or_default();
    let subject = Subject {
        purpose: invocation::purpose(&message, &action, command.is_some()),
        command,
        message,
        user: (!user_display.is_empty()).then_some(user_display),
        attempts,
        deadline,
        fingerprint_wait,
    };

    if let Err(e) = gui::run(subject, to_ui_rx, from_ui_tx) {
        eprintln!("sudo-pop: {e}");
        // Without a window there is nothing to type into; end the request
        // rather than leaving the caller waiting for the full 25 seconds.
        std::process::exit(EXIT_CANCELLED);
    }

    let outcome = worker.join().unwrap_or(Outcome::Failed);
    std::process::exit(exit_for(outcome))
}

/// D-Bus-facing code for one finished prompt. Wrong passwords must not be a
/// D-Bus error: polkitd would re-issue and the next child would start at the
/// fingerprint again.
fn exit_for(outcome: Outcome) -> i32 {
    match outcome {
        Outcome::Success => EXIT_SUCCESS,
        Outcome::Failed | Outcome::Cancelled | Outcome::RefusedWithoutPrompt => EXIT_CANCELLED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sigterm_reads_as_a_cancel_to_the_helper_loop() {
        let (to_ui, _rx) = channel::<ToUi>();
        let (_tx, from_ui) = channel::<FromUi>();
        let conv = WindowConversation {
            to_ui,
            from_ui,
            pending: std::sync::Mutex::new(None),
        };
        assert!(!conv.cancelled(), "nothing has happened yet");
        TERMINATED.store(true, Ordering::Relaxed);
        let seen = conv.cancelled();
        TERMINATED.store(false, Ordering::Relaxed);
        assert!(
            seen,
            "the flag alone ends the wait, without a window message"
        );
    }

    /// Records what the window was told; answers nothing (the scripted
    /// `authenticate` never asks it to).
    struct Rec {
        errors: Vec<String>,
        attempts: Vec<Option<(String, bool)>>,
    }
    impl Conversation for Rec {
        fn ask(&mut self, _prompt: &str, _echo: bool) -> Option<Secret> {
            None
        }
        fn info(&mut self, _text: &str) {}
        fn error(&mut self, text: &str) {
            self.errors.push(text.to_owned());
        }
        fn update_attempts(&mut self, attempts: Option<(String, bool)>) {
            self.attempts.push(attempts);
        }
    }

    /// Run the loop against a fixed list of outcomes, counting the attempts.
    fn drive(outcomes: Vec<Outcome>) -> (Outcome, usize, Rec) {
        drive_with_budget(outcomes, || None)
    }

    fn drive_with_budget(
        outcomes: Vec<Outcome>,
        mut get_budget: impl FnMut() -> Option<attempts::Budget>,
    ) -> (Outcome, usize, Rec) {
        let mut rec = Rec {
            errors: Vec::new(),
            attempts: Vec::new(),
        };
        let mut it = outcomes.into_iter();
        let mut calls = 0;
        let last = run_attempts_with(
            &mut rec,
            |_conv| {
                calls += 1;
                it.next()
                    .expect("run_attempts asked more times than scripted")
            },
            &mut get_budget,
        );
        (last, calls, rec)
    }

    #[test]
    fn spent_password_tries_end_the_request_without_a_dbus_error() {
        assert_eq!(exit_for(Outcome::Success), EXIT_SUCCESS);
        assert_eq!(exit_for(Outcome::Failed), EXIT_CANCELLED);
        assert_eq!(exit_for(Outcome::Cancelled), EXIT_CANCELLED);
        assert_eq!(exit_for(Outcome::RefusedWithoutPrompt), EXIT_CANCELLED);
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
        assert_eq!(rec.errors, vec![attempts::WRONG_PASSWORD.to_owned()]);
    }

    #[test]
    fn three_wrong_answers_stop_at_the_cap() {
        let (last, calls, rec) = drive(vec![Outcome::Failed; MAX_ATTEMPTS as usize]);
        assert_eq!(last, Outcome::Failed);
        assert_eq!(calls, MAX_ATTEMPTS as usize, "no fourth prompt");
        // "Wrong" between attempts, but not after the final one.
        assert_eq!(rec.errors.len(), (MAX_ATTEMPTS - 1) as usize);
    }

    #[test]
    fn wrong_updates_attempts_on_the_window() {
        let mut budgets = vec![
            Some(attempts::Budget {
                remaining: 9,
                unlock_in: None,
            }),
            Some(attempts::Budget {
                remaining: 8,
                unlock_in: None,
            }),
        ]
        .into_iter();

        let (last, calls, rec) = drive_with_budget(vec![Outcome::Failed, Outcome::Success], || {
            budgets.next().flatten()
        });
        assert_eq!(last, Outcome::Success);
        assert_eq!(calls, 2);
        assert_eq!(
            rec.attempts,
            vec![Some((
                "9 attempt(s) left before the account locks".to_owned(),
                false
            ))]
        );
    }

    #[test]
    fn lockout_during_retries_stops_immediately() {
        let mut budgets = vec![Some(attempts::Budget {
            remaining: 0,
            unlock_in: Some(60),
        })]
        .into_iter();

        let (last, calls, rec) = drive_with_budget(vec![Outcome::Failed, Outcome::Failed], || {
            budgets.next().flatten()
        });
        // When the account locks mid-conversation, stop prompting immediately.
        assert_eq!(last, Outcome::Cancelled);
        assert_eq!(calls, 1, "must not attempt a second time when locked");
        assert!(rec.errors.is_empty(), "no 'Wrong' when locked");
    }

    #[test]
    fn a_password_typed_during_a_fingerprint_retry_is_kept() {
        let (to_ui_tx, to_ui_rx) = channel();
        let (from_ui_tx, from_ui_rx) = channel();
        let mut conv = WindowConversation {
            to_ui: to_ui_tx,
            from_ui: from_ui_rx,
            pending: std::sync::Mutex::new(None),
        };
        let mut secret = Secret::new();
        secret.buffer_mut().push_str("hunter2");
        from_ui_tx.send(FromUi::Answer(secret)).unwrap();
        assert!(!conv.cancelled(), "a typed password is not a cancel");
        let got = conv.ask("Password:", false).expect("stashed answer");
        assert_eq!(got.as_bytes(), b"hunter2");
        drop(to_ui_rx);
    }

    #[test]
    fn a_stashed_password_never_answers_an_echoed_prompt() {
        let (to_ui_tx, to_ui_rx) = channel();
        let (from_ui_tx, from_ui_rx) = channel();
        let mut conv = WindowConversation {
            to_ui: to_ui_tx,
            from_ui: from_ui_rx,
            pending: std::sync::Mutex::new(None),
        };
        let mut secret = Secret::new();
        secret.buffer_mut().push_str("hunter2");
        from_ui_tx.send(FromUi::Answer(secret)).unwrap();
        assert!(!conv.cancelled());
        // The window closes before anything else is typed, so `ask` for an
        // echoed prompt gets neither the stash nor a fresh answer.
        drop(from_ui_tx);
        assert!(
            conv.ask("Username:", true).is_none(),
            "a password typed for a hidden field must not be handed to an echoed prompt"
        );
        assert!(
            conv.pending.lock().unwrap().is_none(),
            "and it is wiped, not kept for later"
        );
        drop(to_ui_rx);
    }

    #[test]
    fn a_spent_budget_alarms_with_error_color() {
        let mut budgets = vec![Some(attempts::Budget {
            remaining: 3,
            unlock_in: None,
        })]
        .into_iter();

        let (last, calls, rec) = drive_with_budget(vec![Outcome::Failed, Outcome::Success], || {
            budgets.next().flatten()
        });
        assert_eq!(last, Outcome::Success);
        assert_eq!(calls, 2);
        assert_eq!(
            rec.attempts,
            vec![Some((
                "3 attempt(s) left before the account locks".to_owned(),
                true
            ))]
        );
    }
}
