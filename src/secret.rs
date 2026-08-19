//! The password buffer.
//!
//! Held for as short a time as possible, pinned so it cannot reach swap, and
//! wiped by hand rather than by `Drop` -- under `panic = "abort"` destructors
//! never run.
//!
//! Where it goes afterwards is the caller's business: in agent mode it is
//! written straight to the polkit helper, never formatted into a `String` or a
//! `println!` buffer that nothing zeroizes.

use std::io;

use zeroize::Zeroize;

/// Room for a generous password without reallocating. A `String` that outgrows
/// its capacity leaves the old buffer freed but not cleared — and, worse, the
/// freed pages would no longer be the ones we locked. The input widget caps
/// entry at `MAX_CHARS`, so at four bytes per character the buffer can never
/// reach this size.
const CAPACITY: usize = 2048;

/// Characters the password field accepts. Paired with `CAPACITY` to rule out
/// reallocation.
pub const MAX_CHARS: usize = 256;

/// A password held in memory for as short a time as possible.
///
/// `Drop` is not relied on: under `panic = "abort"` destructors never run, so
/// the caller wipes this explicitly on the normal path.
pub struct Secret(String);

impl Secret {
    /// Allocate the buffer and pin it in RAM.
    ///
    /// This machine has a 15 GB swapfile, so an unlocked password can reach the
    /// disk — and stays there verbatim in a hibernation image. Locking just
    /// this allocation keeps well inside RLIMIT_MEMLOCK, unlike locking the
    /// whole address space.
    pub fn new() -> Self {
        let buffer = String::with_capacity(CAPACITY);
        // SAFETY: the allocation is live and CAPACITY bytes long. mlock rounds
        // to page boundaries itself.
        let locked = unsafe { libc::mlock(buffer.as_ptr().cast(), CAPACITY) } == 0;
        if !locked {
            eprintln!(
                "sudo-pop: cannot lock the password buffer into RAM ({})",
                io::Error::last_os_error()
            );
        }
        Secret(buffer)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Mutable access for the input widget to write into.
    pub fn buffer_mut(&mut self) -> &mut String {
        &mut self.0
    }

    /// The bytes, for writing straight to a descriptor.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Overwrite the contents. Call as soon as the password has been handed on.
    pub fn wipe(&mut self) {
        self.0.zeroize();
    }
}

impl Default for Secret {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.wipe();
        // SAFETY: mirrors the mlock in `new`, on the same allocation.
        unsafe { libc::munlock(self.0.as_ptr().cast(), CAPACITY) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wipe_clears_the_buffer() {
        let mut secret = Secret::new();
        secret.buffer_mut().push_str("hunter2");
        assert!(!secret.is_empty());

        secret.wipe();
        assert!(secret.is_empty());
    }

    #[test]
    fn the_buffer_never_reallocates_within_the_input_limit() {
        let mut secret = Secret::new();
        let start = secret.0.as_ptr();

        // Worst case the widget allows: MAX_CHARS four-byte characters.
        let longest: String = std::iter::repeat_n('🔐', MAX_CHARS).collect();
        assert!(
            longest.len() <= CAPACITY,
            "capacity too small for the limit"
        );
        secret.buffer_mut().push_str(&longest);

        assert_eq!(
            start,
            secret.0.as_ptr(),
            "buffer moved; the mlocked pages no longer hold the password"
        );
    }

    #[test]
    fn a_wiped_buffer_keeps_its_locked_allocation() {
        let mut secret = Secret::new();
        let start = secret.0.as_ptr();
        secret.buffer_mut().push_str("hunter2");
        secret.wipe();
        assert_eq!(start, secret.0.as_ptr());
    }
}
