//! sudo-pop — a GUI askpass front end for sudo that leaves the terminal alone.
//!
//! One binary, three modes. Which one runs is decided by how it was invoked:
//!
//! ```text
//! basename(argv[0]) == "askpass"   -> askpass mode (sudo calls us here)
//! argv[1] == --init / --uninit     -> install mode
//! anything else                    -> wrapper mode
//! ```
//!
//! The askpass check reads argv[0] rather than `current_exe`, because sudo
//! reaches us through a symlink and `current_exe` resolves it away.

mod askpass;
mod attempts;
mod init;
mod paths;
mod sudo_args;
mod wrapper;

/// Link basename that selects askpass mode. Must match `paths::ASKPASS_LINK`.
const ASKPASS_ARGV0: &str = "askpass";

fn main() {
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
        Some("--init") => init::run(false),
        Some("--uninit") => init::run(true),
        _ => wrapper::run(&args),
    }
}
