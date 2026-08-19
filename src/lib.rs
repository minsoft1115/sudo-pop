//! sudo-pop as a library, so the parts that can be tested without a session,
//! a bus, or a password are reachable from `tests/`.
//!
//! The binary is a thin shell over this: mode dispatch and startup live in
//! `main.rs`, everything it calls lives here.

pub mod agent;
pub mod askpass;
pub mod attempts;
pub mod font;
pub mod gui;
pub mod harden;
pub mod helper;
pub mod init;
pub mod invocation;
pub mod paths;
pub mod prompt;
pub mod secret;
pub mod sudo_args;
pub mod theme;
pub mod wrapper;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

/// Set when one request has been handled end to end, so a spike run can stop
/// holding the seat by itself.
pub static HANDLED: AtomicBool = AtomicBool::new(false);

/// An identity polkit will accept, as it comes off the wire.
pub type Identity = (String, HashMap<String, zbus::zvariant::OwnedValue>);

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
pub fn username(uid: u32) -> Option<String> {
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
