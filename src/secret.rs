//! The password buffer.
//!
//! Held for as short a time as possible, pinned so it cannot reach swap, and
//! wiped by hand rather than by `Drop` -- under `panic = "abort"` destructors
//! never run.
//!
//! Where it goes afterwards depends on the path: the agent writes it straight
//! to the polkit helper, and askpass writes it to the descriptor sudo is
//! reading. Neither ever formats it into a `String` or a `println!` buffer that
//! nothing zeroizes.

use std::collections::BTreeMap;
use std::io;
use std::os::unix::io::RawFd;
use std::sync::Mutex;

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
pub struct Secret(String, Vec<usize>);

// Heap allocations may share a page. munlock is not reference counted by the
// kernel, so dropping an input payload must not unlock the live password too.
static LOCKED_PAGES: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

fn page_size() -> usize {
    // SAFETY: sysconf has no pointer arguments. Fail rather than guess alignment.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    assert!(size > 0, "cannot determine memory page size");
    size as usize
}

fn lock_buffer(ptr: *const u8) -> Vec<usize> {
    let size = page_size();
    let start = ptr as usize / size * size;
    let last = (ptr as usize + CAPACITY - 1) / size * size;
    let mut pages = LOCKED_PAGES.lock().unwrap();
    let mut locked = Vec::new();
    for page in (start..=last).step_by(size) {
        let count = pages.entry(page).or_default();
        // SAFETY: these are pages covering the live String allocation.
        if *count > 0 || unsafe { libc::mlock(page as *const libc::c_void, size) } == 0 {
            *count += 1;
            locked.push(page);
        } else {
            eprintln!(
                "sudo-pop: cannot lock password memory ({})",
                io::Error::last_os_error()
            );
            pages.remove(&page);
        }
    }
    locked
}

fn unlock_pages(locked: &[usize]) {
    let mut pages = LOCKED_PAGES.lock().unwrap();
    for page in locked {
        let count = pages.get_mut(page).expect("registered locked page");
        *count -= 1;
        if *count == 0 {
            // SAFETY: no other live Secret relies on this page's lock.
            unsafe { libc::munlock(*page as *const libc::c_void, page_size()) };
            pages.remove(page);
        }
    }
}

impl Secret {
    /// Allocate the buffer and pin it in RAM.
    ///
    /// This machine has a 15 GB swapfile, so an unlocked password can reach the
    /// disk. This does not protect against hibernation images. Locking just
    /// this allocation keeps well inside RLIMIT_MEMLOCK, unlike locking the
    /// whole address space.
    pub fn new() -> Self {
        let buffer = String::with_capacity(CAPACITY);
        let locked = lock_buffer(buffer.as_ptr());
        Secret(buffer, locked)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Mutable access for the input widget to write into.
    pub fn buffer_mut(&mut self) -> &mut String {
        &mut self.0
    }

    pub(crate) fn text(&self) -> &str {
        &self.0
    }

    /// Edit within the locked allocation. Never reallocate or leave deleted bytes
    /// beyond String::len, where ordinary String editing would retain them.
    pub(crate) fn insert(&mut self, index: usize, text: &str, limit: usize) -> usize {
        let count = self.0.chars().count();
        let allowed = MAX_CHARS.saturating_sub(count).min(limit);
        let end = text
            .char_indices()
            .nth(allowed)
            .map_or(text.len(), |(i, _)| i);
        let text = &text[..end];
        let at = self
            .0
            .char_indices()
            .nth(index)
            .map_or(self.0.len(), |(i, _)| i);
        let inserted = text.chars().count();
        assert!(self.0.len() + text.len() <= CAPACITY);
        self.0.insert_str(at, text);
        inserted
    }

    pub(crate) fn delete(&mut self, range: std::ops::Range<usize>) {
        let start = self
            .0
            .char_indices()
            .nth(range.start)
            .map_or(self.0.len(), |(i, _)| i);
        let end = self
            .0
            .char_indices()
            .nth(range.end)
            .map_or(self.0.len(), |(i, _)| i);
        if start >= end {
            return;
        }
        // SAFETY: both boundaries are UTF-8 character boundaries. The retained
        // prefix/suffix form valid UTF-8; zero the vacated tail before truncating.
        unsafe {
            let bytes = self.0.as_mut_vec();
            let len = bytes.len();
            bytes.copy_within(end..len, start);
            let new_len = len - (end - start);
            bytes[new_len..].zeroize();
            bytes.truncate(new_len);
        }
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
        unlock_pages(&self.1);
    }
}

/// The saved write end of the pipe sudo is reading.
///
/// Constructing this is what makes stdout safe: the original descriptor is
/// duplicated out of the way and fd 1 is pointed at /dev/null.
pub struct PasswordChannel {
    fd: RawFd,
}

impl PasswordChannel {
    /// Move the real stdout aside and blank fd 1.
    pub fn take() -> io::Result<Self> {
        // SAFETY: plain descriptor manipulation, single-threaded at this point.
        let saved = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved < 0 {
            return Err(io::Error::last_os_error());
        }

        let devnull = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY) };
        if devnull < 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(saved) };
            return Err(e);
        }

        let rc = unsafe { libc::dup2(devnull, libc::STDOUT_FILENO) };
        unsafe { libc::close(devnull) };
        if rc < 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(saved) };
            return Err(e);
        }

        Ok(PasswordChannel { fd: saved })
    }

    /// Send the password to sudo, then let the caller wipe it.
    ///
    /// Written as two raw writes rather than one formatted line: joining the
    /// password and the newline would allocate a second copy that nothing
    /// zeroizes. `println!` is avoided for the same reason — its buffer would
    /// keep a copy too.
    pub fn send(&self, secret: &Secret) -> io::Result<()> {
        write_all(self.fd, secret.0.as_bytes())?;
        write_all(self.fd, b"\n")
    }
}

/// Write every byte, retrying short writes and EINTR.
fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        // SAFETY: buf is a valid slice for the length passed.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_deletion_wipes_the_vacated_bytes_without_moving_the_buffer() {
        let mut secret = Secret::new();
        secret.insert(0, "a한🔐z", MAX_CHARS);
        let ptr = secret.as_bytes().as_ptr();
        let old_len = secret.as_bytes().len();
        secret.delete(1..3);
        assert_eq!(secret.text(), "az");
        assert_eq!(ptr, secret.as_bytes().as_ptr());
        // SAFETY: all old_len bytes were initialized and the allocation is live.
        let tail = unsafe { std::slice::from_raw_parts(ptr.add(2), old_len - 2) };
        assert!(tail.iter().all(|b| *b == 0));
        secret.delete(0..2);
        assert!(secret.is_empty());
        let old = unsafe { std::slice::from_raw_parts(ptr, old_len) };
        assert!(old.iter().all(|b| *b == 0));
    }

    #[test]
    fn repeated_locks_keep_the_original_secret_pages_registered() {
        let secret = Secret::new();
        assert!(
            !secret.1.is_empty(),
            "test requires an available memlock allowance"
        );
        let extra = lock_buffer(secret.as_bytes().as_ptr());
        unlock_pages(&extra);
        let pages = LOCKED_PAGES.lock().unwrap();
        for page in &secret.1 {
            assert!(pages.get(page).is_some_and(|count| *count >= 1));
        }
    }

    #[test]
    fn kernel_keeps_password_pages_locked_after_other_buffers_drop() {
        let secret = Secret::new();
        let address = secret.as_bytes().as_ptr() as usize;
        for _ in 0..100 {
            drop(Secret::new());
        }
        let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
        let mut contains_buffer = false;
        for line in smaps.lines() {
            if let Some((start, end)) = line.split_whitespace().next().unwrap_or("").split_once('-')
            {
                if let (Ok(start), Ok(end)) = (
                    usize::from_str_radix(start, 16),
                    usize::from_str_radix(end, 16),
                ) {
                    contains_buffer = start <= address && address < end;
                }
            }
            if contains_buffer && line.starts_with("VmFlags:") {
                assert!(
                    line.split_whitespace().any(|flag| flag == "lo"),
                    "kernel mapping is not locked"
                );
                return;
            }
        }
        panic!("password mapping missing from smaps");
    }

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
