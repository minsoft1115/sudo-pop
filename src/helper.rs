//! The conversation with polkit-agent-helper-1.
//!
//! The helper is what runs PAM and what tells polkitd the answer; we never call
//! AuthenticationAgentResponse2 ourselves. Two ways in, and the first can fail
//! in a way that looks like a refusal, so both are needed:
//!
//!   socket  connect /run/polkit/agent-helper.socket, send "username\ncookie\n"
//!   fork    exec the setuid binary with the username, cookie on stdin
//!
//! On a kernel without SO_PEERPIDFD the socket helper closes without ever
//! prompting. "Did we see a prompt" is therefore the signal that matters: it
//! decides whether to fall back to fork, and it decides how the request has to
//! end -- a refusal before any prompt must not be reported as an error, or
//! polkitd re-issues the request and the window reopens forever.

use std::io::{BufRead, BufReader, Write};

use crate::secret::Secret;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};

const SOCKET: &str = "/run/polkit/agent-helper.socket";
const HELPERS: [&str; 2] = [
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-1/polkit-agent-helper-1",
];

/// How one attempt ended.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    Success,
    /// PAM asked and the answer was wrong. Another attempt may help.
    Failed,
    /// The helper gave up before asking anything: a locked account, a broken
    /// PAM stack, or a socket helper the kernel cannot vouch for.
    RefusedWithoutPrompt,
    /// The user closed the prompt.
    Cancelled,
}

/// What the caller shows, and how it asks. `echo` is true when the input is not
/// a password (PAM_PROMPT_ECHO_ON) and may be shown on screen.
pub trait Conversation {
    /// `None` means the user closed the prompt.
    fn ask(&mut self, prompt: &str, echo: bool) -> Option<Secret>;
    fn info(&mut self, text: &str);
    fn error(&mut self, text: &str);
}

/// Reading and writing are separate ends on purpose: the socket is cloned and
/// the forked helper has two pipes, so answering never borrows the reader.
struct Channel {
    reader: Box<dyn BufRead>,
    writer: Box<dyn Write>,
    child: Option<Child>,
}

impl Channel {
    fn socket(username: &str, cookie: &str) -> std::io::Result<Self> {
        let stream = UnixStream::connect(SOCKET)?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        write!(writer, "{username}\n{cookie}\n")?;
        writer.flush()?;
        Ok(Channel {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
        })
    }

    fn fork(username: &str, cookie: &str) -> std::io::Result<Self> {
        let path = HELPERS
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .ok_or_else(|| std::io::Error::other("no polkit-agent-helper-1 on this system"))?;

        let mut child = Command::new(path)
            .arg(username)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let mut stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        // The cookie goes on stdin, not in argv, so it stays out of ps. stdin
        // has to be a pipe anyway: the helper refuses a tty outright.
        writeln!(stdin, "{cookie}")?;
        stdin.flush()?;

        Ok(Channel {
            reader: Box::new(BufReader::new(stdout)),
            writer: Box::new(stdin),
            child: Some(child),
        })
    }

    /// Write the password and its newline as two raw writes.
    ///
    /// Formatting it into one line would allocate a second copy that nothing
    /// wipes -- the same reason the sudo path never used `println!` here.
    fn answer(&mut self, secret: &Secret) -> std::io::Result<()> {
        self.writer.write_all(secret.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn split_tag(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((tag, rest)) => (tag, rest),
        None => (line, ""),
    }
}

/// One pass through the helper, from connect to SUCCESS/FAILURE.
fn attempt(channel: std::io::Result<Channel>, conv: &mut dyn Conversation) -> Outcome {
    let mut channel = match channel {
        Ok(c) => c,
        Err(e) => {
            conv.error(&format!("cannot reach the polkit helper: {e}"));
            return Outcome::RefusedWithoutPrompt;
        }
    };

    let mut saw_prompt = false;
    let mut line = String::new();
    loop {
        line.clear();
        match channel.reader.read_line(&mut line) {
            Ok(0) => break,                      // EOF
            Ok(_) => {}
            Err(e) => {
                conv.error(&format!("helper went away: {e}"));
                break;
            }
        }
        let (tag, rest) = split_tag(line.trim_end_matches('\n'));
        match tag {
            "PAM_PROMPT_ECHO_OFF" | "PAM_PROMPT_ECHO_ON" => {
                saw_prompt = true;
                let Some(mut answer) = conv.ask(rest, tag.ends_with("ON")) else {
                    return Outcome::Cancelled;
                };
                let sent = channel.answer(&answer);
                answer.wipe();
                if let Err(e) = sent {
                    conv.error(&format!("cannot answer the helper: {e}"));
                    return Outcome::Failed;
                }
            }
            "PAM_ERROR_MSG" => conv.error(rest),
            "PAM_TEXT_INFO" => conv.info(rest),
            "SUCCESS" => return Outcome::Success,
            "FAILURE" => break,
            _ => {}
        }
    }

    if saw_prompt {
        Outcome::Failed
    } else {
        Outcome::RefusedWithoutPrompt
    }
}

/// Authenticate `username` for `cookie`, socket first and fork as the fallback.
///
/// A refusal that arrives before any prompt is the one case worth retrying by
/// another door: that is exactly how the socket helper fails on a kernel that
/// cannot pass a pidfd.
pub fn authenticate(username: &str, cookie: &str, conv: &mut dyn Conversation) -> Outcome {
    let socket_reachable = std::path::Path::new(SOCKET).exists();

    if socket_reachable {
        match attempt(Channel::socket(username, cookie), conv) {
            Outcome::RefusedWithoutPrompt => {
                conv.info("socket helper closed without asking; trying the setuid helper");
            }
            other => return other,
        }
    }
    attempt(Channel::fork(username, cookie), conv)
}
