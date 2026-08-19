//! Install mode: wiring the agent into the session.
//!
//! Three things get written, all under $HOME and all inside markers so they can
//! be found and taken back out exactly:
//!
//!   ~/.config/minsoft1115/hypr/sudo-pop.lua        window rules for the prompt
//!   ~/.config/systemd/user/sudo-pop-agent.service  what starts the agent
//!   ~/.config/minsoft1115/bash/sudo-pop.sh         alias sudo='sudo-pop'
//!
//! The snippet directory is shared with other tools, and so is the loader block
//! that sources it. --uninit removes our file and leaves the loader alone.
//!
//! Everything is idempotent. Running --init twice must not append a second copy
//! of anything, because the usual way people find a problem is by running the
//! installer again.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Window rules, compiled in so one binary is the whole installer.
const HYPR_SNIPPET: &str = include_str!("../assets/sudo-pop.lua");

const HYPR_BLOCK_BEGIN: &str = "-- sudo-pop:begin";
const HYPR_BLOCK_END: &str = "-- sudo-pop:end";
const HYPR_BLOCK_BODY: &str = r#"require("minsoft1115.hypr.sudo-pop")"#;

const UNIT_NAME: &str = "sudo-pop-agent.service";

/// Shell snippet, compiled in so one binary is the whole installer.
const SHELL_SNIPPET: &str = include_str!("../assets/sudo-pop.sh");

/// Marker for the loader that sources every snippet in the bash directory.
/// Shared with the other tools in this config, so --uninit leaves it alone.
const BASH_LOADER_BEGIN: &str = "# minsoft1115-bash:begin";
const BASH_LOADER_END: &str = "# minsoft1115-bash:end";
const BASH_LOADER_BODY: &str = r#"for __minsoft1115_rc in "$HOME/.config/minsoft1115/bash"/*.sh; do
  [ -r "$__minsoft1115_rc" ] && . "$__minsoft1115_rc"
done
unset __minsoft1115_rc"#;

/// Agents that would already hold the seat. polkit allows exactly one per
/// session, so ours cannot start while any of these runs.
const KNOWN_AGENTS: [&str; 4] = [
    "hyprpolkitagent",
    "polkit-gnome-authentication-agent-1",
    "polkit-kde-authentication-agent-1",
    "lxpolkit",
];

fn unit_body(exe: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=sudo-pop polkit authentication agent\n\
         Documentation=https://github.com/minsoft1115/sudo-pop\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         Type=exec\n\
         ExecStart={} --agent\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        exe.display()
    )
}

struct Layout {
    config: PathBuf,
    home: PathBuf,
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
        Ok(Layout { config, home })
    }

    fn hypr_snippet(&self) -> PathBuf {
        self.config.join("minsoft1115/hypr/sudo-pop.lua")
    }
    fn hypr_config(&self) -> PathBuf {
        self.config.join("hypr/hyprland.lua")
    }
    fn unit(&self) -> PathBuf {
        self.config.join("systemd/user").join(UNIT_NAME)
    }
    fn shell_snippet(&self) -> PathBuf {
        self.config.join("minsoft1115/bash/sudo-pop.sh")
    }
    fn rc_files(&self) -> [PathBuf; 2] {
        [self.home.join(".bashrc"), self.home.join(".zshrc")]
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
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("sudo-pop: {e}");
            std::process::exit(1)
        }
    }
}

fn install_all(layout: &Layout) -> io::Result<()> {
    // The alias, so `sudo` reaches the router (plain commands go to run0, the
    // rest keep sudo's meaning and get their password from our own window).
    write_snippet(&layout.shell_snippet(), SHELL_SNIPPET)?;
    install_loader(layout)?;

    // Window rules: the agent may be asked to draw the moment it starts.
    write_snippet(&layout.hypr_snippet(), HYPR_SNIPPET)?;
    let hypr = layout.hypr_config();
    if hypr.exists() {
        if add_block(&hypr, HYPR_BLOCK_BEGIN, HYPR_BLOCK_END, HYPR_BLOCK_BODY)? {
            println!("added the window rules to {}", hypr.display());
            reload_hyprland();
        } else {
            println!("window rules already present in {}", hypr.display());
        }
    } else {
        println!(
            "note: {} not found — add this yourself:\n  {HYPR_BLOCK_BODY}",
            hypr.display()
        );
    }

    let exe = std::env::current_exe()?;
    let unit = layout.unit();
    if let Some(parent) = unit.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = unit_body(&exe);
    if fs::read_to_string(&unit).ok().as_deref() == Some(body.as_str()) {
        println!("unit already current: {}", unit.display());
    } else {
        fs::write(&unit, &body)?;
        println!("wrote {}", unit.display());
    }
    systemctl(&["daemon-reload"]);

    // polkit allows one agent per session. Starting ours while another holds
    // the seat would fail to register and then be restarted forever.
    if let Some(other) = other_agent() {
        println!("\n{other} already holds this session's polkit seat.");
        println!("The unit is installed but not enabled. To switch:");
        println!("  {}", switch_hint(&other));
        println!("  sudo-pop --init");
        return Ok(());
    }

    if !target_active("graphical-session.target") {
        println!(
            "note: graphical-session.target is not active here, so the agent may not\n  \
             start automatically at login. It is enabled either way."
        );
    }

    if systemctl(&["enable", "--now", UNIT_NAME]) {
        println!("enabled and started {UNIT_NAME}");
    }
    // enable --now leaves an already-running agent on the old binary.
    if unit_active() {
        systemctl(&["restart", UNIT_NAME]);
    }
    println!("\nDone. Privileged prompts in this session now come from sudo-pop.");
    println!("Open a new shell, or: source ~/.bashrc");
    Ok(())
}

/// Make sure some shell sources the snippet directory.
///
/// The loader block is usually already there from another tool, in which case
/// dropping the snippet in is all it takes and no rc file is touched.
fn install_loader(layout: &Layout) -> io::Result<()> {
    let mut wired = false;
    for rc in layout.rc_files() {
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

fn unit_active() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", UNIT_NAME])
        .status()
        .is_ok_and(|s| s.success())
}

fn uninstall_all(layout: &Layout) -> io::Result<()> {
    let snippet = layout.shell_snippet();
    match fs::remove_file(&snippet) {
        Ok(()) => println!("removed {}", snippet.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    systemctl(&["disable", "--now", UNIT_NAME]);
    let unit = layout.unit();
    match fs::remove_file(&unit) {
        Ok(()) => println!("removed {}", unit.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    systemctl(&["daemon-reload"]);

    let snippet = layout.hypr_snippet();
    match fs::remove_file(&snippet) {
        Ok(()) => println!("removed {}", snippet.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    let hypr = layout.hypr_config();
    if hypr.exists() && remove_block(&hypr, HYPR_BLOCK_BEGIN, HYPR_BLOCK_END)? {
        println!("removed the window rules from {}", hypr.display());
        reload_hyprland();
    }

    println!("\nUninstalled. Whatever agent was there before takes over again;");
    println!("with none, polkit falls back to asking in the terminal.");
    println!("The shared snippet loader was left in place — other tools use it.");
    Ok(())
}

/// Which other polkit agent is holding the seat, if any.
///
/// Omarchy's lives inside the shell process rather than one of its own, so it
/// cannot be found by looking at process names -- ask the plugin list instead.
fn other_agent() -> Option<String> {
    if omarchy_polkit_enabled() {
        return Some("omarchy.polkit (the Omarchy shell's own agent)".into());
    }
    for name in KNOWN_AGENTS {
        if Command::new("pgrep")
            .args(["-x", name])
            .output()
            .is_ok_and(|out| out.status.success())
        {
            return Some(name.into());
        }
    }
    None
}

fn switch_hint(other: &str) -> String {
    if other.starts_with("omarchy.polkit") {
        "omarchy plugin disable omarchy.polkit".into()
    } else {
        format!("systemctl --user disable --now {other}.service")
    }
}

fn omarchy_polkit_enabled() -> bool {
    let Ok(out) = Command::new("omarchy-plugin-list").arg("--json").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(at) = text.find("omarchy.polkit") else {
        return false;
    };
    // Look only as far as the end of that plugin's object.
    let rest = &text[at..];
    let scope = rest.find('}').map(|end| &rest[..end]).unwrap_or(rest);
    scope.contains("\"enabled\": true") || scope.contains("\"enabled\":true")
}

fn target_active(target: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", target])
        .status()
        .is_ok_and(|s| s.success())
}

fn systemctl(args: &[&str]) -> bool {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    match Command::new("systemctl").args(&full).output() {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!("sudo-pop: systemctl {} failed: {}", args.join(" "), stderr.trim());
            false
        }
        Err(e) => {
            eprintln!("sudo-pop: cannot run systemctl: {e}");
            false
        }
    }
}

fn write_snippet(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::read_to_string(path).ok().as_deref() == Some(contents) {
        println!("already current: {}", path.display());
        return Ok(());
    }
    fs::write(path, contents)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Append a marker-delimited block unless it is already there.
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
/// A begin without a matching end is left untouched rather than guessed at:
/// better a stray line than eating the rest of someone's config.
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

/// Ask Hyprland to re-read its config so the rules take effect now.
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
