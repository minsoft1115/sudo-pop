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

const POLKIT_SERVICE: &str = "org.freedesktop.PolicyKit1";
const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const POLKIT_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_IFACE: &str = "org.freedesktop.login1.Manager";

const AGENT_PATH: &str = "/org/minsoft1115/sudo_pop/AuthenticationAgent";

/// An identity polkit will accept, as it comes off the wire.
type Identity = (String, HashMap<String, OwnedValue>);

struct Agent;

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    /// Log everything and refuse. A spike must not be able to authenticate
    /// anything, and refusing is also the honest answer while there is no UI.
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
        Err(zbus::fdo::Error::Failed("spike: no UI yet".into()))
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
        .serve_at(AGENT_PATH, Agent)?
        .build()?;
    println!("agent object: {AGENT_PATH}");

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
