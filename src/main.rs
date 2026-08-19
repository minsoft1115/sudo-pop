//! Spike 1 — register as a polkit authentication agent, and nothing else.
//!
//! What this answers, and only this:
//!   1. which of the three session-id paths actually works here (plan §3-1)
//!   2. what polkitd says when another agent already holds the seat (plan §6)
//!   3. what BeginAuthentication carries, if we ever get one (plan §3-2)
//!   4. what zbus costs to build and link (plan §5)
//!
//! It never asks for a password. Every request is refused, so running it while
//! it holds the seat means privileged actions fail -- that is the whole risk,
//! and it is undone by starting the other agent again.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy, connection};
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

mod helper;
use helper::{Conversation, Outcome};

const POLKIT_SERVICE: &str = "org.freedesktop.PolicyKit1";
const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const POLKIT_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_IFACE: &str = "org.freedesktop.login1.Manager";

const AGENT_PATH: &str = "/org/minsoft1115/sudo_pop/AuthenticationAgent";

/// An identity polkit will accept, as it comes off the wire.
type Identity = (String, HashMap<String, OwnedValue>);

/// Prompts allowed for one cookie, out of the ten faillock would give. Carried
/// over from the sudo path: one authentication request is one command.
const MAX_ATTEMPTS: u32 = 3;

struct Agent {
    /// Unique bus name polkitd owns. Anything else calling us is not polkit.
    polkitd: String,
}

/// Spike-only conversation: the terminal. The real one is a window.
struct Terminal;

impl Conversation for Terminal {
    fn ask(&mut self, prompt: &str, echo: bool) -> Option<String> {
        use std::io::Write;
        print!("  [{}] {prompt} ", if echo { "echo" } else { "hidden" });
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches('\n').to_string()),
        }
    }
    fn info(&mut self, text: &str) {
        println!("  info : {text}");
    }
    fn error(&mut self, text: &str) {
        println!("  error: {text}");
    }
}

/// Account name for a uid, for the helper preamble.
fn username(uid: u32) -> Option<String> {
    // SAFETY: getpwuid returns a pointer into a static buffer, read at once.
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_name)
            .to_str()
            .ok()
            .map(str::to_owned)
    }
}

/// The identity to authenticate: ours if polkit offers it, otherwise the first.
fn choose_identity(identities: &[Identity]) -> Option<(u32, String)> {
    // SAFETY: getuid cannot fail.
    let me = unsafe { libc_getuid() };
    let mut first = None;
    for (kind, attrs) in identities {
        if kind != "unix-user" {
            continue;
        }
        let Some(uid) = attrs.get("uid").and_then(|v| u32::try_from(v).ok()) else {
            continue;
        };
        let name = username(uid)?;
        if uid == me {
            return Some((uid, name));
        }
        first.get_or_insert((uid, name));
    }
    first
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    /// Log what came in, check who sent it, then run the helper conversation.
    ///
    /// Spike 2 asks on the terminal rather than in a window. Everything else --
    /// sender check, identity choice, retry budget, how the request ends -- is
    /// what the real agent does.
    #[allow(clippy::too_many_arguments)]
    fn begin_authentication(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<Identity>,
    ) -> zbus::fdo::Result<()> {
        println!("\n== BeginAuthentication ==");
        println!("  sender     : {:?}", header.sender());
        println!("  action_id  : {action_id}");
        println!("  message    : {message}");
        println!("  icon_name  : {icon_name}");
        println!("  cookie     : {} chars", cookie.len());
        println!("  details    : {details:?}");
        for (kind, attrs) in &identities {
            println!("  identity   : {kind} {:?}", attrs.keys().collect::<Vec<_>>());
        }

        // Only polkit may ask us to prompt. Without this check any process on
        // the bus can put an attacker-worded dialog on screen, learn whether
        // the password was right, and burn the shared faillock budget.
        match header.sender() {
            Some(sender) if sender.as_str() == self.polkitd => {}
            other => {
                println!("  REJECTED: sender {other:?} is not polkitd ({})", self.polkitd);
                return Err(zbus::fdo::Error::AccessDenied("not polkit".into()));
            }
        }

        let Some((uid, name)) = choose_identity(&identities) else {
            println!("  no usable identity");
            return Err(zbus::fdo::Error::Failed("no usable identity".into()));
        };
        println!("  chosen     : {name} (uid {uid})");

        let mut conv = Terminal;
        for attempt in 1..=MAX_ATTEMPTS {
            println!("  -- attempt {attempt}/{MAX_ATTEMPTS} --");
            match helper::authenticate(&name, &cookie, &mut conv) {
                Outcome::Success => {
                    println!("  SUCCESS");
                    return Ok(());
                }
                Outcome::Cancelled => {
                    println!("  cancelled by the user");
                    // Cancelling ends the request normally: an error would have
                    // polkitd hand it straight back to us.
                    return Ok(());
                }
                Outcome::RefusedWithoutPrompt => {
                    println!("  refused before any prompt (locked account or broken stack)");
                    return Ok(());
                }
                Outcome::Failed => continue,
            }
        }
        println!("  out of attempts");
        Err(zbus::fdo::Error::Failed("authentication failed".into()))
    }

    fn cancel_authentication(&self, cookie: String) {
        println!("== CancelAuthentication == ({} chars)", cookie.len());
    }
}

/// Ask a property of one object, as a string.
fn get_string(conn: &Connection, service: &str, path: &str, iface: &str, name: &str) -> Option<String> {
    let path = ObjectPath::try_from(path).ok()?;
    let proxy = Proxy::new(conn, service, path, "org.freedesktop.DBus.Properties").ok()?;
    let value: OwnedValue = proxy.call("Get", &(iface, name)).ok()?;
    String::try_from(value).ok()
}

/// The logind session this process belongs to, and which lookup found it.
///
/// Three steps because one alone comes up empty depending on how we were
/// started; a systemd user unit is outside the session scope, so step 2 fails
/// there and step 3 is the one that answers.
fn session_id(conn: &Connection) -> Option<(String, &'static str)> {
    if let Some(id) = std::env::var_os("XDG_SESSION_ID").and_then(|v| v.into_string().ok())
        && !id.is_empty()
    {
        return Some((id, "XDG_SESSION_ID"));
    }

    let manager = Proxy::new(conn, LOGIND_SERVICE, LOGIND_PATH, LOGIND_IFACE).ok()?;

    if let Ok(path) = manager.call::<_, _, OwnedObjectPath>("GetSessionByPID", &(std::process::id()))
        && let Some(id) = get_string(conn, LOGIND_SERVICE, path.as_str(), "org.freedesktop.login1.Session", "Id")
    {
        return Some((id, "GetSessionByPID"));
    }

    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc_getuid() };
    if let Ok(path) = manager.call::<_, _, OwnedObjectPath>("GetUser", &(uid,)) {
        let props = Proxy::new(
            conn,
            LOGIND_SERVICE,
            ObjectPath::try_from(path.as_str()).ok()?,
            "org.freedesktop.DBus.Properties",
        )
        .ok()?;
        if let Ok(value) = props.call::<_, _, OwnedValue>("Get", &("org.freedesktop.login1.User", "Display")) {
            if let Ok((id, _)) = <(String, OwnedObjectPath)>::try_from(value) {
                if !id.is_empty() {
                    return Some((id, "User.Display"));
                }
            }
        }
    }
    None
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::system()?;

    // Who owns the polkit name right now. This is what a real agent compares
    // the sender against before drawing anything (plan §3-4).
    let dbus = Proxy::new(
        &conn,
        "org.freedesktop.DBus",
        ObjectPath::try_from("/org/freedesktop/DBus")?,
        "org.freedesktop.DBus",
    )?;
    let owner: String = dbus.call("GetNameOwner", &(POLKIT_SERVICE))?;
    println!("polkitd owns {POLKIT_SERVICE} as {owner}");

    let Some((session, how)) = session_id(&conn) else {
        eprintln!("no logind session id: none of the three lookups answered");
        std::process::exit(1);
    };
    println!("session id  : {session}  (found by {how})");

    let conn = connection::Builder::system()?
        .serve_at(AGENT_PATH, Agent { polkitd: owner.clone() })?
        .build()?;
    println!("agent object: {AGENT_PATH}");
    if let Some(me) = conn.unique_name() {
        println!("our bus name: {me}");
    }

    let authority = Proxy::new(&conn, POLKIT_SERVICE, ObjectPath::try_from(POLKIT_PATH)?, POLKIT_IFACE)?;
    let mut subject_details: HashMap<&str, Value> = HashMap::new();
    subject_details.insert("session-id", Value::from(session.as_str()));
    let subject = ("unix-session", subject_details);
    let locale = std::env::var("LANG").unwrap_or_default();

    match authority.call::<_, _, ()>(
        "RegisterAuthenticationAgent",
        &(&subject, locale.as_str(), AGENT_PATH),
    ) {
        Ok(()) => {
            println!("\nREGISTERED. holding the seat; every request will be refused.");
            println!("Ctrl-C to stop.");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        Err(e) => {
            println!("\nREFUSED: {e}");
            println!("(expected while another agent holds this session)");
            Ok(())
        }
    }
}
