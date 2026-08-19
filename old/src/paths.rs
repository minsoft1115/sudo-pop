//! Filesystem locations sudo-pop owns.
//!
//! The askpass helper must be reachable through a path that `sudo` can exec
//! directly, because SUDO_ASKPASS carries no arguments. A symlink inside
//! $XDG_RUNTIME_DIR gives us that without dropping an executable into a
//! world-writable directory: /run/user/<uid> is mode 0700 and owned by us, and
//! exec permission is evaluated on the symlink target, so a noexec mount on the
//! runtime dir is irrelevant.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, symlink};
use std::path::PathBuf;

/// Directory name created under $XDG_RUNTIME_DIR.
const RUNTIME_SUBDIR: &str = "sudo-pop";

/// Link name. Its basename is what selects askpass mode in `main`, so changing
/// it means changing `crate::ASKPASS_ARGV0` as well.
const ASKPASS_LINK: &str = "askpass";

fn err(msg: &str) -> io::Error {
    io::Error::other(msg)
}

/// $XDG_RUNTIME_DIR, rejecting an unset or empty value.
pub fn runtime_dir() -> io::Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(v) if !v.is_empty() => Ok(PathBuf::from(v)),
        _ => Err(err("XDG_RUNTIME_DIR is unset or empty")),
    }
}

/// Create (or validate) $XDG_RUNTIME_DIR/sudo-pop with mode 0700.
///
/// An existing entry is accepted only if it is a real directory we own with
/// exactly mode 0700 — not a symlink, not group/world reachable. Anything else
/// is refused so the caller falls back to plain sudo rather than trusting a
/// path someone else may have prepared.
fn ensure_private_dir() -> io::Result<PathBuf> {
    let dir = runtime_dir()?.join(RUNTIME_SUBDIR);

    match fs::symlink_metadata(&dir) {
        Ok(md) => {
            if !md.is_dir() {
                return Err(err("runtime dir path exists but is not a directory"));
            }
            if md.uid() != unsafe { libc::geteuid() } {
                return Err(err("runtime dir is owned by another user"));
            }
            if md.mode() & 0o777 != 0o700 {
                return Err(err("runtime dir is not mode 0700"));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(&dir)?;
        }
        Err(e) => return Err(e),
    }

    Ok(dir)
}

/// Absolute path of the running binary, used as the symlink target.
///
/// `current_exe` resolves through /proc/self/exe, which is exactly what we want
/// here: the link must point at the real file, not at another symlink.
fn binary_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Ensure $XDG_RUNTIME_DIR/sudo-pop/askpass points at this binary.
///
/// An existing link with the right target is reused as-is; only a wrong or
/// non-symlink entry is replaced. Returns the link path to put in SUDO_ASKPASS.
pub fn ensure_askpass_symlink() -> io::Result<PathBuf> {
    let dir = ensure_private_dir()?;
    let link = dir.join(ASKPASS_LINK);
    let target = binary_path()?;

    match fs::symlink_metadata(&link) {
        Ok(md) if md.file_type().is_symlink() => {
            if fs::read_link(&link)? == target {
                return Ok(link);
            }
            fs::remove_file(&link)?;
        }
        Ok(_) => fs::remove_file(&link)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    symlink(&target, &link)?;
    Ok(link)
}

/// Basename of `path`, for argv[0] mode detection.
pub fn basename(path: &OsString) -> &std::ffi::OsStr {
    std::path::Path::new(path)
        .file_name()
        .unwrap_or(path.as_os_str())
}
