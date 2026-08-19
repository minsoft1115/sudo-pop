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

The Wayland and OpenGL libraries (`wayland`, `libxkbcommon`, `mesa`,
`libglvnd`) are already present on most desktop installs.

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

What `--init` writes:

| Path | Contents |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | window rules (float, center, dim_around, excluded from screen sharing) |
| `~/.config/hypr/hyprland.lua` | a marker block that `require`s the file above |

It is idempotent — a second run adds nothing twice, and `.bashrc` is not touched
at all when the snippet loader block is already there.

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

Removed: the installed files, the marker block in `hyprland.lua`, the runtime
symlink. Left alone: the shared snippet loader, and the alias in shells that are
already open (`unalias sudo`, or open a new one).

---

## Things worth knowing

### The escape hatch — read this first

If the alias survives but the binary does not, `sudo` becomes "command not
found".

```bash
/usr/bin/sudo whoami
```

An absolute path goes around aliases, shell functions and PATH alike. `\sudo`
and `command sudo` are weaker — a backslash does not suppress functions, and
`command` still searches PATH. Run `--uninit` before deleting the binary.

### The alias only expands in interactive shells

**The real sudo runs** in shell scripts, `sh -c`, Makefile recipes,
`xargs sudo`, systemd units and cron. sudo-pop only changes what a person types.

### It steps aside when it cannot draw

Any one of these falls back to the ordinary terminal prompt:

- neither `WAYLAND_DISPLAY` nor `DISPLAY` is set (SSH, a console TTY)
- the arguments already contain `-n`, `-S` or `-A`
- `XDG_RUNTIME_DIR` is missing, or preparing it failed

**Logging in over SSH does not lock you out of sudo.**

### faillock — a cancel also counts as one failure

PAM's behaviour, not sudo-pop's: by the time askpass runs the authentication
conversation has already started, so Esc costs exactly what a wrong password
costs — one failure.

| | PAM default | this machine (Arch + Omarchy) |
|---|---|---|
| `deny` — failures before lockout | 3 | 10 |
| `fail_interval` — window they must land in | 900s | 900s |
| `unlock_time` — how long the lockout lasts | 600s | 120s |
| `passwd_tries` — sudo's own retries | 3 | 10 |

```bash
grep faillock /etc/pam.d/system-auth /etc/security/faillock.conf
sudo -l | grep passwd_tries
faillock --user "$USER"     # only rows with V in the Valid column count
```

`deny` and `unlock_time` are read at run time, so sudo-pop follows yours. On top
of them:

- **at most 3 popups per `sudo` command** — sudo retries askpass `passwd_tries`
  times on its own, enough for one command to spend the whole budget. No effect
  where `passwd_tries` is already 3 or less.
- **the window warns** once 3 or fewer failures remain.
- **no popup at all when the account is locked** — a doomed attempt cannot cost
  more.

**One successful authentication clears the record.** While locked, waiting out
`unlock_time` is the only cure.

### Using the window

| Key | |
|---|---|
| Enter | submit. An empty box is ignored and the window stays |
| Esc | cancel |
| (left alone) | cancels itself after 90 seconds |

The top line is the command about to run, so an unexpected one stands out. When
it cannot be determined (`sudo -v` and friends) that line is omitted.

The field takes 256 characters. The window's wording is English — CJK glyphs
would mean loading a CJK font, and instant startup is the point of this tool.

### Colors and font follow the desktop

| | |
|---|---|
| colors | the current Omarchy theme, `~/.local/state/omarchy/current/theme/colors.toml` |
| font | whatever `fc-match monospace` reports, so `omarchy-font-set` applies as-is |
| font size | fixed here. Omarchy has no global size, and the terminal's is wrong for this window |

Each popup is a fresh process, so a theme or font change shows up on the **very
next popup**. Anything unreadable falls back to defaults without complaining.

### You cannot screenshot the window

`no_screen_share` is on, and screenshot tools such as `grim` use the same
protocol, so the window comes out as **a black rectangle**. That is not a bug —
it is the proof that the password window stays out of recordings and shared
screens.

The image at the top of this README was taken with that rule turned off in
`~/.config/minsoft1115/hypr/sudo-pop.lua` (`hyprctl reload`, capture, put it
back).

### What it protects, and what it does not

| | |
|---|---|
| **Unchanged** | Malware running as your user can replace the alias, the binary and `SUDO_ASKPASS` — but it could already alias `sudo`, shadow it on PATH, or fake the prompt from a shell function. No new path opens here. |
| **One thing worse** | Phishing. A terminal prompt at least appears where you just typed; a popup gives that up, and a look-alike window needs no privileges. The command on the top line is the answer to it — a cue, not a guarantee. |

What it does protect is careless leakage, measured rather than assumed:

- no core dump can hold the password — `PR_SET_DUMPABLE=0` and `RLIMIT_CORE=0`;
  aborting the release binary leaves no `coredumpctl` entry
- the buffer is locked into RAM, out of swap and hibernation images — `VmLck` is
  non-zero while the window is open
- the window is excluded from screen sharing and recording (above)
- no log, no command line, no environment variable — the password goes straight
  into the pipe sudo reads

---

## Troubleshooting

**The popup does not appear and the terminal asks instead**
One of the conditions for stepping aside was met. `SUDO_POP_DEBUG=1 sudo true`
prints which one, on stderr. It never touches stdout, so leaving it on is safe.

**The window opens in a corner, or the background is not dimmed**
The Hyprland rules did not apply. Check `~/.config/hypr/hyprland.lua` for the
`-- sudo-pop:begin` block, then `hyprctl reload`.

**`sudo: command not found`**
The binary is not on PATH. Use `/usr/bin/sudo` meanwhile.

**The password is right but keeps failing**
The account may be locked. `faillock --user "$USER"`, then wait out
`unlock_time`.

---

## Documentation

| File | |
|---|---|
| `docs/architecture.html` | structure and flow diagrams |
| `docs/plan.md` | implementation spec |
| `docs/rationale.md` | design decisions, each with the measurement that settled it |

**These three are in Korean.** They are the author's working record, not usage
instructions — this README covers everything needed to use the tool.

---

## Development

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`SUDO_POP_DEBUG=1` reports fallback decisions, hardening results and the retry
counter on stderr.

---

## License

MIT. See [LICENSE](LICENSE).
