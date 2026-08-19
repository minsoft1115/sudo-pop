# sudo-pop

**English** · [한국어](README.ko.md)

A single Rust binary that asks for your `sudo` password in a GUI popup on
Hyprland/Wayland. Only the prompt leaves the terminal — stdin, stdout and stderr
reach the real command untouched, so `pacman`'s `[Y/n]` and a full-screen `vim`
work as they always did.

![The sudo-pop window: pacman -Syu on the top line, sudo's own prompt below it,
and a masked input field](screenshots/sudo-pop.png)

> **A convenience tool, not a security tool.** It changes where the prompt
> appears, not how safe your password is — see [what it protects, and what it
> does not](#what-it-protects-and-what-it-does-not).

---

## Requirements

| | |
|---|---|
| Hyprland | 0.56+. The window rules assume the Lua config |
| sudo | with askpass (`-A`). Verified on 1.9.17 |
| Rust | to build. `mise.toml` pins the toolchain |

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
```

Fetches the source, builds it, installs into `~/.local/bin`, runs `--init`.
Re-running upgrades in place. Needs `cargo` (or `mise`) and a C linker; there are
no prebuilt binaries.

Everything it writes lives under `$HOME`, so **do not run it as root** — it
refuses if you try.

| Flag | Variable | Default |
|---|---|---|
| `--prefix DIR` | `SUDO_POP_PREFIX` | `~/.local/bin` |
| `--ref REF` | `SUDO_POP_REF` | `main` |
| `--no-init` | `SUDO_POP_NO_INIT=1` | runs `--init` |

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh \
  | bash -s -- --prefix ~/bin --no-init
```

Running `./install.sh` from a checkout builds that checkout; `--ref` is ignored.

<details>
<summary>By hand</summary>

```bash
cargo build --release
install -Dm755 target/release/sudo-pop ~/.local/bin/sudo-pop   # the alias needs it on PATH
sudo-pop --init                                                # alias + Hyprland rules
source ~/.bashrc                                               # or open a new shell
```

</details>

`--init` is idempotent, and writes only these:

| Path | Contents |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | the popup's window rules |
| `~/.config/hypr/hyprland.lua` | a marker block that `require`s the file above |

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
```

Or by hand — **in this order**:

```bash
sudo-pop --uninit
rm ~/.local/bin/sudo-pop
```

**The order matters.** Deleting the binary first leaves the alias pointing at
nothing; `--uninstall` handles that case too, removing the files itself.

The shared snippet loader is left alone, and so is the alias in shells that are
already open (`unalias sudo`, or open a new one).

---

## Things worth knowing

### Using the window

| Key | |
|---|---|
| Enter | submit. An empty box is ignored and the window stays |
| Esc | cancel |
| (left alone) | cancels itself after 90 seconds |

The top line is the command about to run, so an unexpected one stands out. When
it cannot be determined (`sudo -v` and friends) that line is omitted. The
window's own wording is English.

### It follows your Omarchy theme

Colors come from the current theme, the font from `fc-match monospace`. Each
popup is a fresh process, so switching either shows up on the **very next
popup** — nothing to reload. Outside Omarchy, or when a palette cannot be read,
it falls back to sensible defaults.

### The alias only expands in interactive shells

**The real sudo runs** in shell scripts, `sh -c`, Makefile recipes,
`xargs sudo`, systemd units and cron. sudo-pop only changes what a person types.

### It falls back to the terminal when it cannot draw

Any one of these skips the popup and uses the ordinary terminal prompt:

- neither `WAYLAND_DISPLAY` nor `DISPLAY` is set (SSH, a console TTY)
- the arguments already contain `-n`, `-S` or `-A`
- `XDG_RUNTIME_DIR` is missing, or preparing it failed

**Logging in over SSH does not lock you out of sudo.**

### The window stays out of screen shares

`no_screen_share` keeps it out of screen sharing and recording. Screenshot tools
use the same protocol, so the window captures as a black rectangle — the image
at the top of this README was taken with that rule turned off in
`~/.config/minsoft1115/hypr/sudo-pop.lua`.

### faillock — a cancel costs one failure

PAM's behaviour, not sudo-pop's: by the time the popup appears the
authentication conversation has already started, so Esc costs exactly what a
wrong password costs. Enough failures in a row and PAM locks the account for a
while.

The thresholds are your PAM configuration's — sudo-pop reads `deny` and
`unlock_time` at run time and follows them. On top of them:

- **at most 3 popups per `sudo` command**, where sudo on its own retries up to
  `passwd_tries` times and can spend the whole budget
- **the window warns** when few failures remain
- **no popup at all when the account is already locked**

**One successful authentication clears the record.**

```bash
faillock --user "$USER"     # only rows with V in the Valid column count
```

### If sudo ever stops working

Deleting the binary while the alias is still installed leaves `sudo` as "command
not found". An absolute path always works:

```bash
/usr/bin/sudo whoami
```

It goes around aliases, shell functions and PATH alike, where `\sudo` and
`command sudo` do not. Running `--uninit` before deleting the binary avoids the
situation entirely.

### What it protects, and what it does not

| | |
|---|---|
| **Unchanged** | Malware running as your user can replace the alias, the binary and `SUDO_ASKPASS` — but it could already alias `sudo`, shadow it on PATH, or fake the prompt from a shell function. No new path opens here. |
| **One thing worse** | Phishing. A terminal prompt at least appears where you just typed; a popup gives that up, and a look-alike window needs no privileges. The command on the top line is the answer to it — a cue, not a guarantee. |

What it does protect is careless leakage:

- a crash cannot leave the password in a core dump
- the buffer is locked into RAM, out of swap and hibernation images
- the window is excluded from screen sharing and recording
- no log, no command line and no environment variable ever holds it

Each of those was measured rather than assumed — `docs/rationale.md` §6.

---

## Troubleshooting

**The popup does not appear and the terminal asks instead**
One of the conditions for falling back was met. `SUDO_POP_DEBUG=1 sudo true`
prints which one, on stderr.

**The window opens in a corner, or the background is not dimmed**
The Hyprland rules did not apply. Check `~/.config/hypr/hyprland.lua` for the
`-- sudo-pop:begin` block, then `hyprctl reload`.

**`sudo: command not found`**
The binary is not on PATH. Use `/usr/bin/sudo` meanwhile.

**The password is right but keeps failing**
The account may be locked. Check with `faillock --user "$USER"` and wait out
`unlock_time`.

---

## Documentation

| File | |
|---|---|
| `docs/architecture.html` | structure and flow diagrams |
| `docs/plan.md` | implementation spec |
| `docs/rationale.md` | design decisions, each with the measurement that settled it |

**These three are in Korean**, and they are the author's working record — this
README covers everything needed to use the tool.

---

## Development

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`SUDO_POP_DEBUG=1` reports fallback decisions, hardening results and the retry
counter on stderr. It never touches stdout, so leaving it on is safe.

---

## License

MIT. See [LICENSE](LICENSE).
