# sudo-pop

**English** · [한국어](README.ko.md)

The password prompt for privileged actions on Omarchy, in a window that keeps
the password out of core dumps, swap, and screen shares.

It is a **polkit authentication agent** with a **sudo router** in front of it:

```
sudo pacman -Syu   →  run0 pacman -Syu   ─┐
sudo -E make       →  sudo -A make       ─┤→  the same window
disk mounts · NetworkManager · systemctl ─┘
```

Everything that asks for a password ends up at one prompt, and that prompt
tells you **which command is asking** — polkit's own wording does not.

---

## What this promises, and what it does not

**This is a convenience tool, not a security boundary.** Malware running as your
user can replace the alias, the binary and the unit alike.

What it does defend against is **careless leakage**:

- a crash cannot write the password to disk in a core dump
- the password buffer is locked into RAM, so it cannot reach swap or a
  hibernation image
- the window is excluded from screen sharing and recording
- the password appears in no log, no command line, and no environment variable
- only polkit may ask it to draw; anything else on the bus is refused before a
  window appears

---

## Requirements

| | |
|---|---|
| Omarchy | 4.0+ — the shell's own polkit agent has to step aside (below) |
| Hyprland | 0.56+, Lua config. Window rules assume it |
| systemd | 256+ for `run0`. Verified on 261 |
| Rust | to build. With `mise`, `mise.toml` pins the toolchain |

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
```

That builds it, puts the binary in `~/.local/bin` and runs `sudo-pop --init`.
Everything it writes lives under `$HOME`, so **do not run it as root** — it
refuses if you try.

`--init` installs four things, all inside markers so they can be taken back out
exactly: the `sudo` alias, the Hyprland window rules, the require line in
`hyprland.lua`, and a systemd user unit for the agent.

### Omarchy already has an agent

polkit allows one agent per session, and the Omarchy shell ships its own. While
it holds the seat, `--init` installs the unit but **does not enable it** and
tells you so. To switch:

```bash
omarchy plugin disable omarchy.polkit
sudo-pop --init
```

You give up that dialog's fingerprint path and theme integration, and you get
the hardening and the command line of whatever is asking.

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
omarchy plugin enable omarchy.polkit
```

---

## Things worth knowing

**`/usr/bin/sudo` always reaches the real sudo.** A backslash does not: `\sudo`
suppresses alias expansion but not shell functions, and other tools in this
config make `sudo` one.

**Plain commands go through run0, and that is not sudo.** `sudoers` rules do not
apply — no `NOPASSWD`, no `env_keep` (so no `SSH_AUTH_SOCK` or `DISPLAY`) — the
command runs as a systemd unit rather than a child of your shell, and the
authentication is cached by polkit rather than by sudo. Anything with an option
or an environment assignment keeps sudo's meaning instead. `SUDO_POP_RUN0=0`
turns the routing off.

**Answer within 25 seconds.** That is the caller's D-Bus timeout, not ours; past
it the action fails with "Connection timed out" no matter what you type. The
sudo side has no such limit.

**A wrong password costs the same budget everywhere.** polkit and sudo share one
faillock counter, so failures at this prompt can lock your account for both.
The window says how many attempts are left before that happens.

---

## Documentation

| | |
|---|---|
| [docs/plan.md](docs/plan.md) | what it is and what the implementation must hold to |
| [docs/rationale.md](docs/rationale.md) | why, what was measured, what was rejected |
| `old/` | the previous implementation — a sudo askpass wrapper — kept whole, with its own docs |

Both are written in Korean.

## Development

```bash
cargo test                          # unit and protocol tests, no environment needed
./tests/scenarios.sh                # needs polkitd, the bus and a compositor
./tests/scenarios.sh --with-password  # opens a foot window for the one case that needs typing
```

The scenario suite puts the session back the way it found it, prints what it
restored, and clears the faillock entries it burned.

## License

MIT
