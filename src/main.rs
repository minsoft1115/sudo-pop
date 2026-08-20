//! sudo-pop — the password prompt for polkit, on Omarchy.
//!
//! One binary, four modes, chosen by how it was invoked:
//!
//! ```text
//! basename(argv[0]) == "askpass"   askpass mode (sudo calls us through a symlink)
//! --agent                          the agent: register with polkit, hand requests on
//! --agent-prompt                   the child that draws one window for the agent
//! --init / --uninit                install mode
//! anything else                    wrapper mode: sudo <args> arrives here
//! ```
//!
//! The askpass check reads argv[0] rather than `current_exe`, because sudo
//! reaches us through a symlink and `current_exe` resolves it away.
//!
//! The split is not decoration. winit allows one event loop per process, so a
//! long-lived daemon cannot draw; and a long-lived process is exactly where a
//! password should never be. Both problems have the same answer: a child that
//! lives for one authentication and dies.
//!
//! See docs/polkit-agent.md.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use futures_lite::StreamExt;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, Proxy, connection};

use sudo_pop::{HANDLED, agent, init, prompt, wrapper, paths, askpass};

use agent::Agent;

const POLKIT_SERVICE: &str = "org.freedesktop.PolicyKit1";
const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const POLKIT_IFACE: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND_SERVICE: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_IFACE: &str = "org.freedesktop.login1.Manager";

const AGENT_PATH: &str = "/org/minsoft1115/sudo_pop/AuthenticationAgent";

/// Startup tracing goes to the journal, so it is off unless asked for.
fn tracing() -> bool {
    std::env::var_os("SUDO_POP_DEBUG").is_some_and(|v| !v.is_empty())
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

/// Does this registration error mean another agent already holds the seat?
///
/// polkitd answers a second registration for the same subject with this, and
/// only this. Everything else -- polkitd not on the bus yet, a bus error --
/// is something a restart might get past, and the two must not be confused:
/// treating a transient error as "seat taken" leaves the session with no agent
/// and no complaint.
fn seat_is_taken(message: &str) -> bool {
    message.contains("already exists for the given subject")
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

/// Link basename that selects askpass mode. Must match `paths::ASKPASS_LINK`.
const ASKPASS_ARGV0: &str = "askpass";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // args_os throughout: command arguments may be non-UTF-8 paths and must
    // reach sudo byte for byte.
    let mut argv = std::env::args_os();
    let argv0 = argv.next().unwrap_or_default();
    let args: Vec<std::ffi::OsString> = argv.collect();

    if paths::basename(&argv0) == std::ffi::OsStr::new(ASKPASS_ARGV0) {
        // sudo passes the prompt it would have printed as the only argument.
        askpass::run(args.into_iter().next());
    }

    match args.first().and_then(|a| a.to_str()) {
        Some("--agent") => async_io::block_on(run()),
        Some("--agent-prompt") => prompt::run(),
        Some("--init") => init::run(false),
        Some("--uninit") => init::run(true),
        // Everything else is a sudo command line.
        _ => wrapper::run(&args),
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let once = std::env::var_os("SUDO_POP_SPIKE_ONCE").is_some();

    let probe = Connection::system().await?;
    let dbus = zbus::fdo::DBusProxy::new(&probe).await?;

    // Subscribe BEFORE reading the owner, not after registering.
    //
    // The sender check compares each request against polkitd's unique name,
    // and that name changes when polkitd restarts. Reading it first and
    // subscribing later leaves a window: a restart inside it emits the signal
    // before there is anything listening, so the check keeps comparing against
    // a dead name and refuses the real polkit -- for good, since the process
    // never fails and so is never restarted. Subscribing first closes it.
    let mut owner_changes = dbus.receive_name_owner_changed().await?;
    if tracing() {
        println!("watching {POLKIT_SERVICE} for owner changes");
    }

    let owner = dbus.get_name_owner(POLKIT_SERVICE.try_into()?).await?;
    if tracing() {
        println!("polkitd owns {POLKIT_SERVICE} as {owner}");
    }

    let Some((session, how)) = session_id(&probe).await else {
        // Exit cleanly rather than non-zero: with Restart=on-failure a restart
        // would just fail the same way, two seconds forever. A missing session
        // is a permanent condition here, not a transient one.
        eprintln!(
            "sudo-pop: no logind session id (none of the three lookups answered); \
             not starting the agent"
        );
        std::process::exit(0);
    };
    if tracing() {
        println!("session id  : {session}  (found by {how})");
    }

    let conn = connection::Builder::system()?
        .serve_at(AGENT_PATH, Agent::new(owner.to_string(), once))?
        .build()
        .await?;
    if tracing() {
        println!("agent object: {AGENT_PATH}");
        if let Some(me) = conn.unique_name() {
            println!("our bus name: {me}");
        }
    }

    if let Err(e) = register(&conn, &session).await {
        println!("\nREFUSED: {e}");
        if seat_is_taken(&e.to_string()) {
            // Someone else holds the seat. Restarting cannot win it, and
            // `Restart=on-failure` would keep trying, so end successfully.
            println!("(expected while another agent holds this session)");
            return Ok(());
        }
        // Anything else -- polkitd away, the bus refusing -- may well be gone
        // by the next try, and exiting 0 here would leave the session with no
        // agent and nothing saying so.
        eprintln!("sudo-pop: registration failed for a reason a restart may fix");
        std::process::exit(1);
    }
    if tracing() {
        println!("\nREGISTERED. this session's prompts come here now.");
        if once {
            println!("(stopping after the first request)");
        }
    }

    // polkitd restarting takes our registration with it, and its unique name
    // changes -- which is also what the sender check compares against. The
    // stream this reads was subscribed before any of the above, so a restart
    // during startup is waiting in it rather than lost.
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
                    // Subscribing early means a signal can arrive for the
                    // owner we already have. Re-registering on that one would
                    // draw an "already exists" error out of polkitd for no
                    // reason, so only a name we do not hold counts.
                    let known = conn
                        .object_server()
                        .interface::<_, Agent>(AGENT_PATH)
                        .await
                        .ok();
                    let changed = match &known {
                        Some(iface) => iface
                            .get()
                            .await
                            .polkitd
                            .lock()
                            .is_ok_and(|held| *held != new_owner.to_string()),
                        None => false,
                    };
                    if changed {
                        if tracing() {
                            println!("polkitd came back as {new_owner}; registering again");
                        }
                        if let Some(iface) = &known
                            && let Ok(mut polkitd) = iface.get().await.polkitd.lock()
                        {
                            *polkitd = new_owner.to_string();
                        }
                        if let Err(e) = register(&conn, &session).await {
                            eprintln!("sudo-pop: could not register again: {e}");
                        }
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
        Ok(()) => {
            if tracing() {
                println!("unregistered");
            }
        }
        Err(e) => eprintln!("sudo-pop: unregister failed: {e}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two registration failures must not be confused. polkitd's wording
    /// for a taken seat is the only thing that ends the process successfully;
    /// read it too loosely and a session that merely raced polkitd's start is
    /// left with no agent and nothing said about it.
    #[test]
    fn only_a_taken_seat_ends_the_agent_successfully() {
        assert!(seat_is_taken(
            "An authentication agent already exists for the given subject"
        ));
        for other in [
            "The name org.freedesktop.PolicyKit1 was not provided by any .service files",
            "Connection timed out",
            "Message recipient disconnected from message bus without replying",
            "",
        ] {
            assert!(!seat_is_taken(other), "{other:?} may be worth a restart");
        }
    }
}
