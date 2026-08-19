//! Process hardening for askpass mode.
//!
//! Everything here must run before a password can reach memory and before any
//! other thread exists. The measured reason: with none of it applied, aborting
//! with a secret on the heap leaves that secret in a systemd-coredump file that
//! `strings | grep` recovers verbatim.
//!
//! These calls are what make `panic = "abort"` safe to keep. Unwinding
//! would not help — a SIGSEGV from the graphics stack is not a Rust panic, and
//! only the kernel-level limits below stop that from dumping core either.

/// Apply all hardening. Failures are reported but never fatal: refusing to
/// prompt would be worse for the user than prompting with one mitigation
/// missing, and the caller has no better fallback at this point.
pub fn apply() {
    // SAFETY: called at process start, before any other thread exists.
    unsafe {
        // Blocks the coredump handler outright and stops same-uid processes
        // from attaching. Strongest of the three, so it goes first.
        if libc::prctl(libc::PR_SET_DUMPABLE, 0) != 0 {
            warn("PR_SET_DUMPABLE");
        }

        // Belt and braces: even where the dump handler still runs, it stores
        // nothing.
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
            warn("RLIMIT_CORE");
        }
    }

    // Swap protection is not done here. mlockall(MCL_CURRENT | MCL_FUTURE)
    // covers the whole address space, which on this machine is far larger than
    // RLIMIT_MEMLOCK (8 MB) once the GUI stack is mapped — it simply fails with
    // ENOMEM and protects nothing. The password buffer locks itself instead,
    // which costs one page and always succeeds. See `secret::Secret`.
}

fn warn(what: &str) {
    // stderr only. stdout belongs to the password channel.
    eprintln!(
        "sudo-pop: {what} failed ({})",
        std::io::Error::last_os_error()
    );
}

/// Report what the kernel actually applied, for `SUDO_POP_DEBUG` runs.
///
/// Reads the state back instead of trusting the return codes above. The
/// dumpable flag comes from prctl rather than /proc/self/status, which has no
/// field for it. Locked memory is not reported here: the password buffer locks
/// itself later, and says so on stderr if it cannot.
pub fn report() {
    // SAFETY: PR_GET_DUMPABLE takes no output pointer and cannot fail here.
    let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE) };

    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit is a valid, writable rlimit.
    unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) };

    eprintln!(
        "sudo-pop: hardening — dumpable={dumpable} core_limit={}",
        limit.rlim_cur
    );
}
