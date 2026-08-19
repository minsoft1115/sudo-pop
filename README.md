# sudo-pop

**English** · [한국어](README.ko.md)

A single Rust binary that asks for your `sudo` password in a GUI popup on
Hyprland/Wayland.

Only the password prompt is moved out of the terminal, into a separate process,
so the terminal's stdin, stdout and stderr reach the real command untouched.
`pacman`'s `[Y/n]` still works. So does a full-screen `vim`.

```
$ sudo pacman -Syu
   → a password window opens in the middle of the screen, and once you type it
     pacman carries on exactly as it always did
```

---

## What this promises, and what it does not

**This is a convenience tool, not a security tool.**

Malware running as your user can replace the alias, the binary and the
`SUDO_ASKPASS` variable alike. There is no line here for it to cross. If
anything, making "a GUI window that asks for your password" an everyday sight
makes phishing easier: an identical-looking fake window needs no privileges at
all.

What sudo-pop does defend against is **careless leakage**:

- a crash cannot write the password to disk in a core dump
- the password buffer is locked into RAM, so it cannot reach swap or a
  hibernation image
- the window is excluded from screen sharing and recording
- the password appears in no log, no command line, and no environment variable

---

## Requirements

| | |
|---|---|
| Hyprland | verified on 0.56+. The window rules assume the Lua config |
| sudo | with askpass (`-A`) support. Verified on 1.9.17 |
| Rust | to build. With `mise`, `mise.toml` pins the toolchain |

The Wayland and OpenGL libraries (`wayland`, `libxkbcommon`, `mesa`,
`libglvnd`) are already present on most desktop installs.

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
```

That fetches the source, builds it, puts the binary in `~/.local/bin` and runs
`sudo-pop --init`. Everything it writes lives under `$HOME`, so **do not run it
as root** — it refuses if you try. Re-running it upgrades in place.

It needs `cargo` (or `mise`, which picks up the pinned toolchain from
`mise.toml`) and a C linker. There are no prebuilt binaries.

Options, either as flags after `-s --` or as environment variables:

| Flag | Variable | Default |
|---|---|---|
| `--prefix DIR` | `SUDO_POP_PREFIX` | `~/.local/bin` |
| `--ref REF` | `SUDO_POP_REF` | `main` |
| `--no-init` | `SUDO_POP_NO_INIT=1` | runs `--init` |

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh \
  | bash -s -- --prefix ~/bin --no-init
```

Running `./install.sh` from inside a checkout builds that checkout instead of
downloading one — `--ref` is then ignored.

<details>
<summary>By hand</summary>

```bash
# 1. build
cargo build --release

# 2. put it on PATH (the alias needs to resolve)
install -Dm755 target/release/sudo-pop ~/.local/bin/sudo-pop

# 3. register the shell alias and the Hyprland window rules
sudo-pop --init

# 4. open a new shell, or
source ~/.bashrc
```

</details>

What `--init` writes:

| Path | Contents |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | popup window rules (float, center, dim_around, excluded from screen sharing) |
| `~/.config/hypr/hyprland.lua` | a marker block that `require`s the file above |

Running it again adds nothing twice. If the snippet loader block is already
there, `.bashrc` is not touched at all.

## Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
```

Or by hand — **in this order**:

```bash
sudo-pop --uninit
rm ~/.local/bin/sudo-pop
```

Either way this removes the files it installed, the marker block in
`hyprland.lua` and the runtime symlink. The snippet loader block is left alone —
it is shared with other tools. The alias stays in the shell you are already in
until you `unalias sudo` or open a new one.

**The order matters.** Deleting the binary first leaves the alias pointing at
nothing, and `sudo` becomes "command not found". If that already happened,
`--uninstall` handles it: with no binary to ask, it removes those files itself.

---

## Things worth knowing

### Escape hatches — read this first

If the alias survives but the binary does not, `sudo` becomes "command not
found". Any of these runs the real sudo:

```bash
command sudo whoami   # ignores both aliases and functions
/usr/bin/sudo whoami
\sudo whoami          # ignores aliases only — see below
```

Do not delete the binary before running `--uninit`.

**`\sudo` is not a reliable escape hatch.** A backslash suppresses alias
expansion, but **not shell functions**. Another tool sharing the same snippet
folder may define `sudo` as a function — omarchy-setup's package guard
(`zz-pkg-guards.sh`) does exactly that, taking over this alias when it loads and
calling it from inside itself. In such a shell, `\sudo` lands in that function.
Use `command sudo` or an absolute path to be sure.

### Aliases only expand in interactive shells

That is how shell aliases work, and it cannot be changed. In all of these, **the
real sudo runs**:

- shell scripts, `sh -c "..."`
- Makefile recipes
- `xargs sudo ...`
- systemd units, cron

So sudo-pop only changes what happens when a person types a command. It leaves
every automated path alone.

### It steps aside when it cannot draw

Any one of these makes it give up on the popup and fall back to the ordinary
terminal prompt:

- neither `WAYLAND_DISPLAY` nor `DISPLAY` is set (SSH, a console TTY)
- the arguments already contain `-n`, `-S` or `-A`
- `XDG_RUNTIME_DIR` is missing, or preparing it failed

**Logging in over SSH does not lock you out of sudo.**

### faillock — a cancel also counts as one failure

This is PAM's behaviour, not sudo-pop's. By the time askpass runs, the
authentication conversation has already started, so cancelling inside it is
recorded as a failed attempt.

```
Esc to cancel      → failures +1
wrong password     → failures +1
```

With the stock configuration, **10 failures within 15 minutes lock the account
for 120 seconds** (`deny` in `/etc/security/faillock.conf`, `unlock_time` in
`/etc/pam.d/system-auth`).

sudo-pop softens that in three ways:

- one `sudo` command opens the popup **at most 3 times**. sudo itself allows
  ten, which lets a single command spend the whole budget.
- when the remaining budget drops to **3 or fewer, the window says so**.
- if the account is already locked, it explains that instead of asking for a
  password, so a doomed attempt cannot spend more of the budget.

**One successful authentication clears the record.** If failures have piled up,
typing the password correctly once puts it back to zero. While locked, waiting
out the 120 seconds is the only cure.

You can always look:

```bash
faillock --user "$USER"    # only rows with V in the Valid column count
```

### Using the window

| Key | |
|---|---|
| Enter | submit. An empty box is ignored and the window stays |
| Esc | cancel |
| (left alone) | cancels itself after 90 seconds |

Top to bottom, the window is:

```
      pacman -Syu               ← the command about to run (theme accent)
  [sudo] password for you:      ← the prompt sudo handed over (dimmed)
  ┌────────────────────────┐
  └────────────────────────┘
   Enter to confirm  Esc to cancel
```

**The top line tells you what is about to run.** An unexpected command asking
for your password stands out there. When the command cannot be determined
(`sudo -v` and friends), that line is simply omitted.

The password field takes up to 256 characters. The window's own wording is in
English: Korean glyphs would mean loading a CJK font, and a popup that appears
instantly is the whole point of this tool, so that startup cost was not worth
paying. The most important text on screen is the prompt sudo passes through
anyway.

### Colors and font follow the desktop

**Colors** come from the current Omarchy theme
(`~/.local/state/omarchy/current/theme/colors.toml`) — background, input field,
accent and warning colors all come from that palette.

**The font** is whatever `fc-match monospace` reports, so a font set with
`omarchy-font-set` applies as-is, and outside Omarchy it follows the system
default monospace.

Each popup is a fresh process, so **changing the theme or the font shows up on
the very next popup** — nothing to restart or reload. If either cannot be read,
it falls back to defaults without complaining.

Only the font size is fixed here. Omarchy has no global size setting, and the
terminal's size is not right for this window.

### You cannot screenshot the window

The screen-share exclusion (`no_screen_share`) is on, and screenshot tools such
as `grim` use the same protocol, so the window comes out as **a black
rectangle**. That is not a bug — it is the proof that the password window does
not leak into recordings or shared screens.

To capture it anyway, turn `no_screen_share` off in
`~/.config/minsoft1115/hypr/sudo-pop.lua`, run `hyprctl reload`, and put it
back afterwards.

---

## Troubleshooting

**The popup does not appear and the terminal asks instead**
One of the conditions for stepping aside was met. To see which:

```bash
SUDO_POP_DEBUG=1 sudo true
```

It prints which gate it stepped aside at, on stderr. The variable never touches
stdout, so leaving it on is safe.

**The window opens in a corner, or the background is not dimmed**
The Hyprland rules did not apply. Check that `~/.config/hypr/hyprland.lua`
contains the `-- sudo-pop:begin` block, then run `hyprctl reload`.

**`sudo: command not found`**
The binary is not on PATH. Use `command sudo` from the escape hatches above, and
check that `~/.local/bin` is on your PATH.

**The password is right but keeps failing**
The account may be locked. Check `faillock --user "$USER"` and wait 120 seconds.

---

## Documentation

| File | |
|---|---|
| `docs/architecture.html` | structure and flow diagrams |
| `docs/plan.md` | implementation spec |
| `docs/rationale.md` | design decisions and measurements |

**These three are written in Korean.** They are the author's working record, not
usage instructions — this README covers everything needed to use the tool.

They are where the reasoning lives: why `sudo -A` is used at all, why the
askpass path is a symlink under `$XDG_RUNTIME_DIR` rather than a script in
`/tmp`, and how core dumps are suppressed while keeping `panic = "abort"` — each
with the measurement that settled it.

---

## Development

```bash
cargo test           # unit tests
cargo clippy --all-targets
cargo fmt
```

`SUDO_POP_DEBUG=1` reports fallback decisions, the hardening results and the
retry counter on stderr.

---

## License

MIT. See [LICENSE](LICENSE).
