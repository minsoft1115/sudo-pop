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
    let uid = choose_uid(&unix_user_uids(identities), me)?;
    Some((uid, username(uid)?))
}

/// The uids of the `unix-user` identities, in the order polkit listed them.
/// Other identity kinds (groups, netgroups) are not something we authenticate.
fn unix_user_uids(identities: &[Identity]) -> Vec<u32> {
    identities
        .iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, attrs)| attrs.get("uid").and_then(|v| u32::try_from(v).ok()))
        .collect()
}

/// Which uid to authenticate: ours if it is offered, otherwise the first.
///
/// Picking the current user when possible is what §3-2 settled on; a multi-admin
/// chooser is deferred, so any other case just takes the first candidate.
fn choose_uid(uids: &[u32], me: u32) -> Option<u32> {
    if uids.contains(&me) {
        return Some(me);
    }
    uids.first().copied()
}

/// Tests that mutate process-global environment (XDG_RUNTIME_DIR, ...) take this
/// lock so the parallel test runner cannot interleave them.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{OwnedValue, Value};

    fn uid_attr(n: u32) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        m.insert("uid".to_owned(), OwnedValue::try_from(Value::U32(n)).unwrap());
        m
    }

    #[test]
    fn only_unix_user_uids_are_collected() {
        let ids = vec![
            ("unix-group".to_owned(), uid_attr(0)),    // wrong kind -> skipped
            ("unix-user".to_owned(), uid_attr(1000)),
            ("unix-user".to_owned(), HashMap::new()),  // no uid attr -> skipped
            ("unix-user".to_owned(), uid_attr(0)),
        ];
        assert_eq!(unix_user_uids(&ids), vec![1000, 0]);
    }

    #[test]
    fn the_current_user_is_preferred_when_offered() {
        assert_eq!(choose_uid(&[1000, 0], 1000), Some(1000));
        assert_eq!(choose_uid(&[0, 1000], 1000), Some(1000));
    }

    #[test]
    fn otherwise_the_first_identity_is_taken() {
        assert_eq!(choose_uid(&[0, 42], 1000), Some(0));
    }

    #[test]
    fn no_identities_means_none() {
        assert_eq!(choose_uid(&[], 1000), None);
    }
}
