//! sudo-pop — the password prompt for polkit, on Omarchy.
//!
//! Two modes, one binary:
//!
//! ```text
//! --agent-prompt   the child that draws one window and talks to the helper
//! (anything else)  the agent: register with polkit, hand requests to children
//! ```
//!
//! The split is not decoration. winit allows one event loop per process, so a
//! long-lived daemon cannot draw; and a long-lived process is exactly where a
//! password should never be. Both problems have the same answer: a child that
//! lives for one authentication and dies.
//!
//! See docs/polkit-agent.md.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_lite::StreamExt;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy, connection};

mod agent;
mod font;
mod init;
mod gui;
mod harden;
mod helper;
mod invocation;
mod prompt;
mod secret;
mod theme;

use agent::{Agent, Identity};

const POLKIT_SERVICE: &str = "org.freedesktop.PolicyKit1";
const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const POLKIT_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_IFACE: &str = "org.freedesktop.login1.Manager";

const AGENT_PATH: &str = "/org/minsoft1115/sudo_pop/AuthenticationAgent";

/// Prompts allowed for one cookie, out of the ten faillock would give.
///
/// A child is spawned per cookie and owns the retry loop, so this is per
/// request without the daemon keeping any state.
pub const MAX_ATTEMPTS: u32 = 3;

/// Set when one request has been handled end to end, so a spike run can stop
/// holding the seat by itself.
pub static HANDLED: AtomicBool = AtomicBool::new(false);

/// Wall clock, for lining our log up against what the caller saw.
pub fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let secs = now % 86400;
    format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600 + 9) % 24,
        (secs / 60) % 60,
        secs % 60
    )
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
pub fn choose_identity(identities: &[Identity]) -> Option<(u32, String)> {
    // SAFETY: getuid cannot fail.
    let me = unsafe { libc::getuid() };
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

/// Ask a property of one object, as a string.
async fn get_string(
    conn: &Connection,
    service: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> Option<String> {
    let path = ObjectPath::try_from(path).ok()?;
    let proxy = Proxy::new(conn, service, path, "org.freedesktop.DBus.Properties")
        .await
        .ok()?;
    let value: OwnedValue = proxy.call("Get", &(iface, name)).await.ok()?;
    String::try_from(value).ok()
}

/// The logind session we belong to, and which lookup found it.
///
/// Three steps because one alone comes up empty depending on how we were
/// started: a systemd user unit sits outside the session scope, so step 2 fails
/// there and step 3 is the one that answers.
async fn session_id(conn: &Connection) -> Option<(String, &'static str)> {
    if let Some(id) = std::env::var_os("XDG_SESSION_ID").and_then(|v| v.into_string().ok())
        && !id.is_empty()
    {
        return Some((id, "XDG_SESSION_ID"));
    }

    let manager = Proxy::new(conn, LOGIND_SERVICE, LOGIND_PATH, LOGIND_IFACE)
        .await
        .ok()?;

    if let Ok(path) = manager
        .call::<_, _, OwnedObjectPath>("GetSessionByPID", &(std::process::id()))
        .await
        && let Some(id) = get_string(
            conn,
            LOGIND_SERVICE,
            path.as_str(),
            "org.freedesktop.login1.Session",
            "Id",
        )
        .await
    {
        return Some((id, "GetSessionByPID"));
    }

    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    if let Ok(path) = manager.call::<_, _, OwnedObjectPath>("GetUser", &(uid,)).await {
        let props = Proxy::new(
            conn,
            LOGIND_SERVICE,
            ObjectPath::try_from(path.as_str()).ok()?,
            "org.freedesktop.DBus.Properties",
        )
        .await
        .ok()?;
        if let Ok(value) = props
            .call::<_, _, OwnedValue>("Get", &("org.freedesktop.login1.User", "Display"))
            .await
            && let Ok((id, _)) = <(String, OwnedObjectPath)>::try_from(value)
            && !id.is_empty()
        {
            return Some((id, "User.Display"));
        }
    }
    None
}

/// The subject we register for: this logind session.
fn subject(session: &str) -> (&'static str, HashMap<&'static str, Value<'_>>) {
    let mut details: HashMap<&str, Value> = HashMap::new();
    details.insert("session-id", Value::from(session));
    ("unix-session", details)
}

async fn register(conn: &Connection, session: &str) -> zbus::Result<()> {
    let authority = Proxy::new(
        conn,
        POLKIT_SERVICE,
        ObjectPath::try_from(POLKIT_PATH)?,
        POLKIT_IFACE,
    )
    .await?;
    let locale = std::env::var("LANG").unwrap_or_default();
    authority
        .call::<_, _, ()>(
            "RegisterAuthenticationAgent",
            &(&subject(session), locale.as_str(), AGENT_PATH),
        )
        .await
}

async fn unregister(conn: &Connection, session: &str) -> zbus::Result<()> {
    let authority = Proxy::new(
        conn,
        POLKIT_SERVICE,
        ObjectPath::try_from(POLKIT_PATH)?,
        POLKIT_IFACE,
    )
    .await?;
    authority
        .call::<_, _, ()>(
            "UnregisterAuthenticationAgent",
            &(&subject(session), AGENT_PATH),
        )
        .await
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The child that draws the window and talks to the helper. Checked before
    // anything else so it never touches the bus.
    match std::env::args().nth(1).as_deref() {
        Some("--agent-prompt") => prompt::run(),
        Some("--init") => init::run(false),
        Some("--uninit") => init::run(true),
        _ => {}
    }
    async_io::block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let once = std::env::var_os("SUDO_POP_SPIKE_ONCE").is_some();

    let probe = Connection::system().await?;
    let dbus = zbus::fdo::DBusProxy::new(&probe).await?;
    let owner = dbus.get_name_owner(POLKIT_SERVICE.try_into()?).await?;
    println!("polkitd owns {POLKIT_SERVICE} as {owner}");

    let Some((session, how)) = session_id(&probe).await else {
        eprintln!("no logind session id: none of the three lookups answered");
        std::process::exit(1);
    };
    println!("session id  : {session}  (found by {how})");

    let conn = connection::Builder::system()?
        .serve_at(AGENT_PATH, Agent::new(owner.to_string(), once))?
        .build()
        .await?;
    println!("agent object: {AGENT_PATH}");
    if let Some(me) = conn.unique_name() {
        println!("our bus name: {me}");
    }

    if let Err(e) = register(&conn, &session).await {
        println!("\nREFUSED: {e}");
        println!("(expected while another agent holds this session)");
        return Ok(());
    }
    println!("\nREGISTERED. this session's prompts come here now.");
    if once {
        println!("(stopping after the first request)");
    }

    // polkitd restarting takes our registration with it, and its unique name
    // changes -- which is also what the sender check compares against.
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut owner_changes = dbus.receive_name_owner_changed().await?;

    loop {
        if HANDLED.load(Ordering::SeqCst) {
            break;
        }

        let next = async_io::Timer::after(std::time::Duration::from_millis(200));
        futures_lite::future::or(
            async {
                if let Some(signal) = owner_changes.next().await
                    && let Ok(args) = signal.args()
                    && args.name() == POLKIT_SERVICE
                    && let Some(new_owner) = args.new_owner().as_ref()
                {
                    println!("polkitd came back as {new_owner}; registering again");
                    if let Ok(iface) = conn
                        .object_server()
                        .interface::<_, Agent>(AGENT_PATH)
                        .await
                        && let Ok(mut polkitd) = iface.get().await.polkitd.lock()
                    {
                        *polkitd = new_owner.to_string();
                    }
                    if let Err(e) = register(&conn, &session).await {
                        eprintln!("sudo-pop: could not register again: {e}");
                    }
                }
            },
            async {
                next.await;
            },
        )
        .await;
    }

    // Give the seat back rather than leaving polkitd to notice we left.
    match unregister(&conn, &session).await {
        Ok(()) => println!("unregistered"),
        Err(e) => println!("unregister failed: {e}"),
    }
    Ok(())
}
