//! Install mode: wiring sudo-pop into the shell and into Hyprland.
//!
//! Both halves follow the convention this machine already uses for personal
//! config: drop a snippet into ~/.config/minsoft1115/, and reference it from
//! the main config inside a marker block that can be found again later.
//!
//! Everything here is idempotent. Running --init twice must not append a second
//! copy of anything, because the usual way people discover a problem is by
//! running the installer again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Snippet contents, compiled in so a single binary is the whole installer.
const SHELL_SNIPPET: &str = include_str!("../assets/sudo-pop.sh");
const HYPR_SNIPPET: &str = include_str!("../assets/sudo-pop.lua");

/// Marker for the loader that sources every snippet in the bash directory.
/// Shared with the other tools in this config, so --uninit leaves it alone.
const BASH_LOADER_BEGIN: &str = "# minsoft1115-bash:begin";
const BASH_LOADER_END: &str = "# minsoft1115-bash:end";

/// Marker for our own `require` line in hyprland.lua.
const HYPR_BLOCK_BEGIN: &str = "-- sudo-pop:begin";
const HYPR_BLOCK_END: &str = "-- sudo-pop:end";

const BASH_LOADER_BODY: &str = r#"for __minsoft1115_rc in "$HOME/.config/minsoft1115/bash"/*.sh; do
  [ -r "$__minsoft1115_rc" ] && . "$__minsoft1115_rc"
done
unset __minsoft1115_rc"#;

const HYPR_BLOCK_BODY: &str = r#"require("minsoft1115.hypr.sudo-pop")"#;

struct Layout {
    home: PathBuf,
    config: PathBuf,
}

impl Layout {
    fn detect() -> io::Result<Self> {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("HOME is unset"))?;
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Ok(Layout { home, config })
    }

    fn shell_snippet(&self) -> PathBuf {
        self.config.join("minsoft1115/bash/sudo-pop.sh")
    }

    fn hypr_snippet(&self) -> PathBuf {
        self.config.join("minsoft1115/hypr/sudo-pop.lua")
    }

    fn hypr_config(&self) -> PathBuf {
        self.config.join("hypr/hyprland.lua")
    }

    fn bashrc(&self) -> PathBuf {
        self.home.join(".bashrc")
    }

    fn zshrc(&self) -> PathBuf {
        self.home.join(".zshrc")
    }
}

/// Entry point for `--init` / `--uninit`. Never returns.
pub fn run(uninstall: bool) -> ! {
    let layout = match Layout::detect() {
        Ok(layout) => layout,
        Err(e) => {
            eprintln!("sudo-pop: {e}");
            std::process::exit(1);
        }
    };

    let result = if uninstall {
        uninstall_all(&layout)
    } else {
        install_all(&layout)
    };

    match result {
        Ok(()) => {
            reload_hyprland();
            std::process::exit(0)
        }
        Err(e) => {
            eprintln!("sudo-pop: {e}");
            std::process::exit(1)
        }
    }
}

fn install_all(layout: &Layout) -> io::Result<()> {
    // Shell alias.
    write_snippet(&layout.shell_snippet(), SHELL_SNIPPET)?;
    install_loader(layout)?;

    // Hyprland window rule.
    write_snippet(&layout.hypr_snippet(), HYPR_SNIPPET)?;
    let hypr = layout.hypr_config();
    if hypr.exists() {
        if add_block(&hypr, HYPR_BLOCK_BEGIN, HYPR_BLOCK_END, HYPR_BLOCK_BODY)? {
            println!("added the window rule to {}", hypr.display());
        } else {
            println!("window rule already present in {}", hypr.display());
        }
    } else {
        println!(
            "note: {} not found — add this yourself:\n  {HYPR_BLOCK_BODY}",
            hypr.display()
        );
    }

    report_binary_location();
    println!(
        "\nopen a new shell, or run: source {}",
        layout.bashrc().display()
    );
    Ok(())
}

/// Make sure some shell sources the snippet directory.
///
/// The loader block is usually already there from another tool, in which case
/// dropping the snippet file in is all it takes and no rc file is touched.
fn install_loader(layout: &Layout) -> io::Result<()> {
    let mut wired = false;

    for rc in [layout.bashrc(), layout.zshrc()] {
        if !rc.exists() {
            continue;
        }
        if add_block(&rc, BASH_LOADER_BEGIN, BASH_LOADER_END, BASH_LOADER_BODY)? {
            println!("added the snippet loader to {}", rc.display());
        } else {
            println!("snippet loader already present in {}", rc.display());
        }
        wired = true;
    }

    if !wired {
        println!("note: no ~/.bashrc or ~/.zshrc found — add this to your shell config:");
        println!("  alias sudo='sudo-pop'");
    }
    Ok(())
}

fn uninstall_all(layout: &Layout) -> io::Result<()> {
    for path in [layout.shell_snippet(), layout.hypr_snippet()] {
        match fs::remove_file(&path) {
            Ok(()) => println!("removed {}", path.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }

    let hypr = layout.hypr_config();
    if hypr.exists() && remove_block(&hypr, HYPR_BLOCK_BEGIN, HYPR_BLOCK_END)? {
        println!("removed the window rule from {}", hypr.display());
    }

    // The loader block stays: other tools in this config rely on it, and an
    // empty snippet directory costs nothing.
    println!("\nthe shared snippet loader was left in place");
    Ok(())
}

fn write_snippet(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Append a marker-delimited block unless it is already there.
///
/// Returns whether anything was written.
fn add_block(path: &Path, begin: &str, end: &str, body: &str) -> io::Result<bool> {
    let existing = read_or_empty(path)?;
    if existing.contains(begin) {
        return Ok(false);
    }

    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(begin);
    out.push('\n');
    out.push_str(body);
    out.push('\n');
    out.push_str(end);
    out.push('\n');

    fs::write(path, out)?;
    Ok(true)
}

/// Delete a marker-delimited block, markers included.
///
/// Returns whether anything was removed. A begin without a matching end is left
/// untouched rather than guessed at — better to leave a stray line than to eat
/// the rest of someone's config.
fn remove_block(path: &Path, begin: &str, end: &str) -> io::Result<bool> {
    let existing = read_or_empty(path)?;
    let Some(start) = existing.find(begin) else {
        return Ok(false);
    };
    let Some(stop) = existing[start..].find(end) else {
        return Err(io::Error::other(format!(
            "{} has {begin} without {end}; not touching it",
            path.display()
        )));
    };

    let mut out = String::with_capacity(existing.len());
    out.push_str(existing[..start].trim_end_matches('\n'));
    let tail = &existing[start + stop + end.len()..];
    out.push('\n');
    out.push_str(tail.trim_start_matches('\n'));

    fs::write(path, out)?;
    Ok(true)
}

fn read_or_empty(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Warn when the alias would not resolve, which is easy to miss: the shell
/// reports "command not found" for `sudo` and the way out is `\sudo`.
fn report_binary_location() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let on_path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join("sudo-pop").exists()))
        .unwrap_or(false);

    if on_path {
        println!("sudo-pop is on PATH");
    } else {
        println!(
            "warning: sudo-pop is not on PATH — the alias will not resolve.\n  \
             install it, for example: cp {} ~/.local/bin/",
            exe.display()
        );
    }
}

/// Ask Hyprland to re-read its config so the rule takes effect now.
fn reload_hyprland() {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return;
    }
    match Command::new("hyprctl").arg("reload").output() {
        Ok(out) if out.status.success() => println!("reloaded Hyprland"),
        Ok(out) => eprintln!(
            "sudo-pop: hyprctl reload failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) => eprintln!("sudo-pop: cannot run hyprctl: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sudo-pop-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn add_block_is_idempotent() {
        let file = tmp("idem").join("rc");
        fs::write(&file, "existing line\n").unwrap();

        assert!(add_block(&file, "#b", "#e", "body").unwrap());
        let once = fs::read_to_string(&file).unwrap();

        assert!(!add_block(&file, "#b", "#e", "body").unwrap());
        assert_eq!(once, fs::read_to_string(&file).unwrap());
        assert_eq!(once.matches("#b").count(), 1);
        assert!(once.starts_with("existing line\n"));
    }

    #[test]
    fn add_block_handles_missing_trailing_newline() {
        let file = tmp("nonl").join("rc");
        fs::write(&file, "no newline").unwrap();
        add_block(&file, "#b", "#e", "body").unwrap();
        let out = fs::read_to_string(&file).unwrap();
        assert!(out.starts_with("no newline\n"), "{out:?}");
    }

    #[test]
    fn remove_block_restores_surroundings() {
        let file = tmp("remove").join("rc");
        fs::write(&file, "before\n").unwrap();
        add_block(&file, "#b", "#e", "body").unwrap();
        fs::write(
            &file,
            format!("{}after\n", fs::read_to_string(&file).unwrap()),
        )
        .unwrap();

        assert!(remove_block(&file, "#b", "#e").unwrap());
        let out = fs::read_to_string(&file).unwrap();
        assert!(!out.contains("#b"), "{out:?}");
        assert!(!out.contains("body"), "{out:?}");
        assert!(out.contains("before"), "{out:?}");
        assert!(out.contains("after"), "{out:?}");
    }

    #[test]
    fn remove_block_on_absent_marker_is_noop() {
        let file = tmp("absent").join("rc");
        fs::write(&file, "untouched\n").unwrap();
        assert!(!remove_block(&file, "#b", "#e").unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "untouched\n");
    }

    #[test]
    fn unterminated_block_is_refused() {
        let file = tmp("unterminated").join("rc");
        fs::write(&file, "#b\nbody\nrest of config\n").unwrap();
        assert!(remove_block(&file, "#b", "#e").is_err());
        assert!(
            fs::read_to_string(&file)
                .unwrap()
                .contains("rest of config")
        );
    }
}
