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

/// Both doors can be pointed elsewhere, which is how the protocol is tested
/// without a real PAM stack -- and the only way to exercise the fork fallback
/// on a kernel whose socket helper always works.
///
/// The overrides are compiled in only for debug builds (`cargo test`). A
/// release binary -- which is all install.sh ever builds -- never reads them,
/// so an environment variable can never redirect the password to another path.
fn socket_path() -> String {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("SUDO_POP_HELPER_SOCKET") {
        return path;
    }
    SOCKET.to_owned()
}

fn helper_binary() -> Option<String> {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("SUDO_POP_HELPER_BIN") {
        return std::path::Path::new(&path).exists().then_some(path);
    }
    // In production only a setuid-root helper is acceptable: the whole point of
    // the fork door is to reach a binary that can run PAM as root, and exec'ing
    // anything else here would hand it the password for nothing.
    HELPERS
        .iter()
        .find(|p| is_setuid_root(p))
        .map(|p| (*p).to_owned())
}

/// True if `path` is owned by root and carries the setuid bit.
#[cfg(not(debug_assertions))]
fn is_setuid_root(path: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).is_ok_and(|md| md.uid() == 0 && md.mode() & 0o4000 != 0)
}

/// In debug builds the fork door is only ever pointed at the test helper, which
/// is deliberately not setuid; the check would reject it.
#[cfg(debug_assertions)]
fn is_setuid_root(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

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
        let stream = UnixStream::connect(socket_path())?;
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
        let path = helper_binary()
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
    let socket_reachable = std::path::Path::new(&socket_path()).exists();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_splits_off_its_body_on_the_first_space() {
        assert_eq!(
            split_tag("PAM_PROMPT_ECHO_OFF Password:"),
            ("PAM_PROMPT_ECHO_OFF", "Password:")
        );
        assert_eq!(
            split_tag("PAM_TEXT_INFO Place your finger"),
            ("PAM_TEXT_INFO", "Place your finger")
        );
    }

    #[test]
    fn a_bare_tag_has_an_empty_body() {
        assert_eq!(split_tag("SUCCESS"), ("SUCCESS", ""));
        assert_eq!(split_tag("FAILURE"), ("FAILURE", ""));
    }

    #[test]
    fn only_the_first_space_is_the_separator() {
        // The protocol drops exactly one space; any extra belongs to the body.
        assert_eq!(split_tag("PAM_ERROR_MSG  two spaces"), ("PAM_ERROR_MSG", " two spaces"));
    }
}
