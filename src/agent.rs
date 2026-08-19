//! The D-Bus side: register, answer polkit, and nothing else.
//!
//! Two things have to happen at once here. A request blocks until the user
//! answers -- up to the caller's 25 seconds -- and `CancelAuthentication` for
//! that same request can arrive in the middle of it. So the methods are async
//! and the work between them is shared through locks rather than a call stack.
//!
//! The password is not in this file, and not in this process. Every request is
//! handed to a child (`--agent-prompt`) and the only thing that comes back is
//! an exit code.

use std::collections::HashMap;
use std::sync::Mutex;

use async_process::{Command, Stdio};
use futures_lite::io::AsyncWriteExt;
use zbus::interface;
use zbus::zvariant::OwnedValue;

use crate::prompt;

/// An identity polkit will accept, as it comes off the wire.
pub type Identity = (String, HashMap<String, OwnedValue>);

pub struct Agent {
    /// Unique bus name polkitd owns. Anything else calling us is not polkit,
    /// and is refused before a window can be drawn.
    pub polkitd: Mutex<String>,
    /// Quit after one handled request (spike runs).
    pub once: bool,
    /// One request at a time. A second one waits here rather than putting a
    /// second window on screen.
    pub turn: async_lock::Mutex<()>,
    /// Cookie -> pid of the child asking about it, so a cancel can reach it.
    pub running: Mutex<HashMap<String, u32>>,
}

impl Agent {
    pub fn new(polkitd: String, once: bool) -> Self {
        Self {
            polkitd: Mutex::new(polkitd),
            once,
            turn: async_lock::Mutex::new(()),
            running: Mutex::new(HashMap::new()),
        }
    }

    fn is_polkitd(&self, sender: Option<&str>) -> bool {
        let expected = self.polkitd.lock().ok();
        matches!((sender, expected), (Some(s), Some(e)) if s == e.as_str())
    }
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[allow(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<Identity>,
    ) -> zbus::fdo::Result<()> {
        let started = std::time::Instant::now();
        println!("\n== BeginAuthentication ==  {}", crate::stamp());
        println!("  action_id  : {action_id}");
        println!("  message    : {message}");
        println!("  icon_name  : {icon_name}");
        println!("  details    : {details:?}");

        // Only polkit may ask us to prompt. Without this any process on the bus
        // can put an attacker-worded dialog on screen, learn whether the
        // password was right, and burn the shared faillock budget.
        let sender = header.sender().map(|s| s.as_str().to_owned());
        if !self.is_polkitd(sender.as_deref()) {
            println!("  REJECTED: sender {sender:?} is not polkitd");
            return Err(zbus::fdo::Error::AccessDenied("not polkit".into()));
        }

        let Some((uid, name)) = crate::choose_identity(&identities) else {
            return Err(zbus::fdo::Error::Failed("no usable identity".into()));
        };
        let subject_pid: u32 = details
            .get("polkit.subject-pid")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        println!("  chosen     : {name} (uid {uid}), subject pid {subject_pid}");

        // Queue: one window at a time. Held for the whole request.
        let _turn = self.turn.lock().await;

        let code = self.ask(&name, &cookie, subject_pid, &message).await;

        println!(
            "  exit {code}  ({} 초 경과, {})",
            started.elapsed().as_secs_f32().round(),
            crate::stamp()
        );
        if self.once {
            crate::HANDLED.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        match code {
            prompt::EXIT_SUCCESS => Ok(()),
            // Cancelled, or refused before any prompt. Both end the request
            // normally: an error would have polkitd hand it straight back and
            // the window would reopen forever.
            prompt::EXIT_CANCELLED => Ok(()),
            _ => Err(zbus::fdo::Error::Failed("authentication failed".into())),
        }
    }

    /// polkit gives up on a request -- the caller stopped waiting, or the
    /// action was withdrawn. Close the window that belongs to that cookie.
    async fn cancel_authentication(&self, cookie: String) {
        println!("== CancelAuthentication ==  {}", crate::stamp());
        let pid = self
            .running
            .lock()
            .ok()
            .and_then(|map| map.get(&cookie).copied());
        match pid {
            Some(pid) => {
                // SAFETY: signalling a child we started.
                unsafe { libc::kill(pid as i32, libc::SIGTERM) };
                println!("  closed the prompt (pid {pid})");
            }
            None => println!("  nothing running for that cookie"),
        }
    }
}

impl Agent {
    /// Run one request in a child and wait for its exit code.
    async fn ask(&self, username: &str, cookie: &str, subject_pid: u32, message: &str) -> i32 {
        let Ok(exe) = std::env::current_exe() else {
            return prompt::EXIT_FAILED;
        };

        let child = Command::new(exe)
            .arg("--agent-prompt")
            .env("SUDO_POP_USER", username)
            .env("SUDO_POP_SUBJECT_PID", subject_pid.to_string())
            .env("SUDO_POP_MESSAGE", message)
            .stdin(Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                eprintln!("sudo-pop: cannot start the prompt: {e}");
                return prompt::EXIT_FAILED;
            }
        };

        let pid = child.id();
        if let Ok(mut map) = self.running.lock() {
            map.insert(cookie.to_owned(), pid);
        }

        // The cookie goes down a pipe, not through argv or the environment:
        // both are readable by anything that can see the process.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(format!("{cookie}\n").as_bytes()).await;
            let _ = stdin.flush().await;
        }

        let code = match child.status().await {
            Ok(status) => status.code().unwrap_or(prompt::EXIT_CANCELLED),
            Err(e) => {
                eprintln!("sudo-pop: prompt did not finish: {e}");
                prompt::EXIT_FAILED
            }
        };

        if let Ok(mut map) = self.running.lock() {
            map.remove(cookie);
        }
        code
    }
}
