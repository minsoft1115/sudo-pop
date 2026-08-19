//! The helper conversation, driven against a stand-in helper.
//!
//! These are the cases that decide whether a request ends as success, a retry,
//! or a cancel -- and getting the last one wrong makes polkitd re-issue the
//! request until the window stops closing. None of them need a password, a
//! session, or root: `SUDO_POP_HELPER_BIN` points the fork door at
//! `tests/fake-helper.sh`.

use std::sync::Mutex;

use sudo_pop::helper::{Conversation, Outcome, authenticate};
use sudo_pop::secret::Secret;

/// Answers from a script, and records what it was shown.
struct Scripted {
    answers: Vec<String>,
    pub prompts: Vec<(String, bool)>,
    pub infos: Vec<String>,
    pub errors: Vec<String>,
}

impl Scripted {
    fn new(answers: &[&str]) -> Self {
        Self {
            answers: answers.iter().rev().map(|s| (*s).to_owned()).collect(),
            prompts: Vec::new(),
            infos: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl Conversation for Scripted {
    fn ask(&mut self, prompt: &str, echo: bool) -> Option<Secret> {
        self.prompts.push((prompt.to_owned(), echo));
        let answer = self.answers.pop()?;
        let mut secret = Secret::new();
        secret.buffer_mut().push_str(&answer);
        Some(secret)
    }
    fn info(&mut self, text: &str) {
        self.infos.push(text.to_owned());
    }
    fn error(&mut self, text: &str) {
        self.errors.push(text.to_owned());
    }
}

/// The environment is process-wide, so the cases take turns.
static ENV: Mutex<()> = Mutex::new(());

fn fake_helper() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-helper.sh").to_owned()
}

/// Run one case with the fork door pointed at the stand-in and the socket door
/// pointed at nothing, so the socket is skipped.
fn run(mode: &str, answers: &[&str]) -> (Outcome, Scripted) {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("SUDO_POP_HELPER_BIN", fake_helper());
        std::env::set_var("SUDO_POP_HELPER_SOCKET", "/nonexistent/sudo-pop-test.socket");
        std::env::set_var("FAKE_HELPER_MODE", mode);
    }
    let mut conv = Scripted::new(answers);
    let outcome = authenticate("tester", "cookie-1234", &mut conv);
    (outcome, conv)
}

#[test]
fn a_right_answer_succeeds() {
    let (outcome, conv) = run("success", &["hunter2"]);
    assert_eq!(outcome, Outcome::Success);
    assert_eq!(conv.prompts, vec![("Password:".to_owned(), false)]);
}

#[test]
fn a_wrong_answer_is_a_failure_worth_retrying() {
    let (outcome, _) = run("wrong", &["nope"]);
    assert_eq!(outcome, Outcome::Failed);
}

/// A locked account, or a broken PAM stack. Reporting this as a failure would
/// have polkitd hand the request straight back and the window would reopen for
/// ever, so it has to be distinguishable.
#[test]
fn a_refusal_before_any_prompt_is_not_a_failure() {
    let (outcome, conv) = run("no-prompt", &[]);
    assert_eq!(outcome, Outcome::RefusedWithoutPrompt);
    assert!(conv.prompts.is_empty(), "nothing should have been asked");
}

/// Closing the window mid-conversation must not be reported as a wrong answer.
#[test]
fn closing_the_prompt_cancels() {
    let (outcome, _) = run("success", &[]); // no answers: the window "closed"
    assert_eq!(outcome, Outcome::Cancelled);
}

#[test]
fn an_echo_on_prompt_is_marked_visible() {
    let (outcome, conv) = run("echo-on", &["tester"]);
    assert_eq!(outcome, Outcome::Success);
    assert_eq!(conv.prompts, vec![("Username:".to_owned(), true)]);
}

#[test]
fn messages_reach_the_window() {
    let (_, conv) = run("info", &["hunter2"]);
    assert_eq!(conv.infos, vec!["Place your finger".to_owned()]);

    let (_, conv) = run("error-then-ok", &["hunter2"]);
    assert_eq!(conv.errors, vec!["Try again".to_owned()]);
}

/// The username and the cookie have to arrive, and the cookie must travel on
/// stdin rather than in argv where anything on the machine could read it.
#[test]
fn the_helper_is_told_who_and_which_request() {
    let log = std::env::temp_dir().join("sudo-pop-fake-helper.log");
    let _ = std::fs::remove_file(&log);
    {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("FAKE_HELPER_LOG", &log) };
    }
    let (outcome, _) = run("success", &["hunter2"]);
    assert_eq!(outcome, Outcome::Success);

    let recorded = std::fs::read_to_string(&log).expect("the helper logged nothing");
    assert!(recorded.contains("user=tester"), "{recorded}");
    assert!(recorded.contains("cookie=cookie-1234"), "{recorded}");
    {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var("FAKE_HELPER_LOG") };
    }
}

/// The socket helper on a kernel that cannot pass a pidfd: it accepts the
/// connection, says nothing, and closes. Nothing else distinguishes that from a
/// refusal, so "did we see a prompt" is what decides whether to try the other
/// door -- and this is the only way to exercise that path on a machine whose
/// real socket helper always works.
#[test]
fn a_silent_socket_falls_back_to_the_fork_helper() {
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    let socket = std::env::temp_dir().join(format!("sudo-pop-test-{}.socket", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("cannot bind the test socket");

    let served = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return String::new();
        };
        // The preamble is two lines and may arrive in more than one write, so
        // read until both are here rather than trusting one read.
        let mut seen = Vec::new();
        let mut chunk = [0u8; 64];
        while seen.iter().filter(|&&b| b == b'\n').count() < 2 {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => seen.extend_from_slice(&chunk[..n]),
            }
        }
        // Then drop the connection without ever prompting.
        String::from_utf8_lossy(&seen).into_owned()
    });

    let outcome = {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("SUDO_POP_HELPER_BIN", fake_helper());
            std::env::set_var("SUDO_POP_HELPER_SOCKET", &socket);
            std::env::set_var("FAKE_HELPER_MODE", "success");
        }
        let mut conv = Scripted::new(&["hunter2"]);
        let outcome = authenticate("tester", "cookie-1234", &mut conv);
        // The fallback asked once, through the fork helper.
        assert_eq!(conv.prompts.len(), 1, "the fallback should have asked once");
        outcome
    };

    let preamble = served.join().unwrap_or_default();
    assert!(
        preamble.starts_with("tester\ncookie-1234\n"),
        "the socket door got the wrong preamble: {preamble:?}"
    );
    assert_eq!(outcome, Outcome::Success, "the fork fallback should succeed");
    let _ = std::fs::remove_file(&socket);
}
