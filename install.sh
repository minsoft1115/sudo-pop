#!/usr/bin/env bash
#
# sudo-pop installer.
#
#   curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
#
# Fetches the source, builds it with cargo, puts the binary on PATH and runs
# `sudo-pop --init`. Everything it writes lives under $HOME, and it never calls
# sudo itself — installing a sudo replacement as root is exactly the shape of
# mistake this tool should not make.
#
# --uninstall reverses it, in the order that matters: --uninit first, binary
# second. The other way round leaves the shell alias pointing at nothing.

set -euo pipefail

REPO="minsoft1115/sudo-pop"
REF="${SUDO_POP_REF:-main}"
PREFIX="${SUDO_POP_PREFIX:-$HOME/.local/bin}"
RUN_INIT=1
[ -n "${SUDO_POP_NO_INIT:-}" ] && RUN_INIT=0
UNINSTALL=0

if [ -t 1 ]; then
  B=$'\033[1m'; Y=$'\033[1;33m'; R=$'\033[1;31m'; N=$'\033[0m'
else
  B=""; Y=""; R=""; N=""
fi

say()  { printf '%s==>%s %s\n' "$B" "$N" "$*"; }
warn() { printf '%swarning:%s %s\n' "$Y" "$N" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$R" "$N" "$*" >&2; exit 1; }

usage() {
  cat <<EOF
sudo-pop installer

  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash -s -- --no-init

Options                       Environment           Default
  --prefix DIR                SUDO_POP_PREFIX       \$HOME/.local/bin
  --ref REF                   SUDO_POP_REF          main
  --no-init                   SUDO_POP_NO_INIT=1    runs sudo-pop --init
  --uninstall                                       removes it again
  -h, --help
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) [ $# -ge 2 ] || die "--prefix needs a directory"; PREFIX="$2"; shift 2 ;;
    --prefix=*) PREFIX="${1#*=}"; shift ;;
    --ref) [ $# -ge 2 ] || die "--ref needs a value"; REF="$2"; shift 2 ;;
    --ref=*) REF="${1#*=}"; shift ;;
    --no-init) RUN_INIT=0; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
done

# --- preflight ---------------------------------------------------------------

# --init writes a shell alias and Hyprland rules into $HOME. As root those land
# in root's home, where they are useless at best.
[ "$(id -u)" -eq 0 ] && die "do not run this as root — it installs into \$HOME and needs no privileges"
[ -n "${HOME:-}" ] || die "HOME is unset"

# --- uninstall ---------------------------------------------------------------

CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"

# What --init writes, removed without the binary's help. Needed when the binary
# is already gone, which is exactly when the leftover alias hurts most.
remove_config_by_hand() {
  local f hypr="$CONFIG/hypr/hyprland.lua"

  for f in "$CONFIG/minsoft1115/bash/sudo-pop.sh" "$CONFIG/minsoft1115/hypr/sudo-pop.lua"; do
    if [ -e "$f" ]; then
      rm -f "$f"
      say "removed $f"
    fi
  done

  if [ -f "$hypr" ] && grep -q -- '-- sudo-pop:begin' "$hypr"; then
    if grep -q -- '-- sudo-pop:end' "$hypr"; then
      sed -i '/^-- sudo-pop:begin$/,/^-- sudo-pop:end$/d' "$hypr"
      say "removed the window rule from $hypr"
    else
      # Same rule the binary follows: a stray marker line beats eating a config.
      warn "$hypr has -- sudo-pop:begin without -- sudo-pop:end — left alone, remove it by hand"
    fi
  fi
}

do_uninstall() {
  local bin="" uninit_ok=0

  if [ -x "$PREFIX/sudo-pop" ]; then
    bin="$PREFIX/sudo-pop"
  else
    bin="$(command -v sudo-pop 2>/dev/null || true)"
  fi

  if [ -n "$bin" ] && [ -x "$bin" ]; then
    # --uninit before rm: the binary is what knows where its files went.
    say "running $bin --uninit"
    if "$bin" --uninit; then
      uninit_ok=1
    else
      warn "--uninit failed — removing its files directly instead"
    fi
    rm -f "$bin" && say "removed $bin"
  else
    warn "no sudo-pop binary in $PREFIX or on PATH — removing its files directly"
  fi

  [ "$uninit_ok" -eq 1 ] || remove_config_by_hand

  # The symlink sudo execs. Lives on tmpfs, but a stale one would point at a
  # binary that no longer exists.
  if [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -e "$XDG_RUNTIME_DIR/sudo-pop" ]; then
    rm -rf "$XDG_RUNTIME_DIR/sudo-pop"
    say "removed $XDG_RUNTIME_DIR/sudo-pop"
  fi

  if [ "$uninit_ok" -eq 0 ] && command -v hyprctl >/dev/null 2>&1; then
    hyprctl reload >/dev/null 2>&1 && say "reloaded Hyprland"
  fi

  cat <<EOF

${B}Uninstalled.${N} The shared snippet loader in ~/.bashrc was left in place —
other tools use it.

This shell still has the alias until you drop it:

  unalias sudo       or just open a new shell
EOF
}

if [ "$UNINSTALL" -eq 1 ]; then
  do_uninstall
  exit 0
fi

# --- build tools -------------------------------------------------------------

command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1 \
  || die "no C linker found (cc). Install base-devel / build-essential first"

# Cargo directly if it is on PATH, otherwise through mise, which is what
# mise.toml in the repository is for.
CARGO=""
if command -v cargo >/dev/null 2>&1; then
  CARGO="cargo"
elif command -v mise >/dev/null 2>&1; then
  CARGO="mise-cargo"
else
  die "cargo not found. Install Rust (https://rustup.rs, or your package manager), or install mise"
fi

# --- source ------------------------------------------------------------------

# Running ./install.sh from a checkout builds that checkout; piped from curl
# there is no checkout, so fetch one.
script_dir=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

tmp=""
cleanup() { [ -n "$tmp" ] && rm -rf "$tmp"; return 0; }
trap cleanup EXIT

if [ -n "$script_dir" ] && grep -q '^name = "sudo-pop"' "$script_dir/Cargo.toml" 2>/dev/null; then
  src="$script_dir"
  say "building the checkout at $src"
else
  tmp="$(mktemp -d)"
  src="$tmp/sudo-pop"
  mkdir -p "$src"
  if command -v git >/dev/null 2>&1; then
    say "cloning $REPO ($REF)"
    git clone --quiet --depth 1 --branch "$REF" "https://github.com/$REPO.git" "$src" \
      || die "clone failed — is '$REF' a branch or tag of $REPO?"
  elif command -v curl >/dev/null 2>&1 && command -v tar >/dev/null 2>&1; then
    say "downloading $REPO ($REF)"
    curl -fsSL "https://github.com/$REPO/archive/$REF.tar.gz" \
      | tar xz --strip-components=1 -C "$src" \
      || die "download failed — is '$REF' a branch or tag of $REPO?"
  else
    die "need git, or curl and tar, to fetch the source"
  fi
fi

# --- build -------------------------------------------------------------------

say "building (this takes a few minutes the first time)"
if [ "$CARGO" = "mise-cargo" ]; then
  # mise refuses to use a config file it has not been told to trust.
  mise trust --quiet "$src" >/dev/null 2>&1 || true
  ( cd "$src" && mise install >/dev/null && mise exec -- cargo build --release --locked ) \
    || die "build failed"
else
  ( cd "$src" && cargo build --release --locked ) || die "build failed"
fi

binary="$src/target/release/sudo-pop"
[ -x "$binary" ] || die "build produced no binary at $binary"

# --- install -----------------------------------------------------------------

# Install beside the target and rename, so replacing a binary that is currently
# running cannot fail with ETXTBSY.
staged="$PREFIX/.sudo-pop.new.$$"
install -Dm755 "$binary" "$staged" || die "cannot write to $PREFIX"
mv -f "$staged" "$PREFIX/sudo-pop" || { rm -f "$staged"; die "cannot install into $PREFIX"; }
say "installed $PREFIX/sudo-pop"

case ":$PATH:" in
  *":$PREFIX:"*) ;;
  *) warn "$PREFIX is not on PATH — the sudo alias will not resolve.
  add it, for example:  echo 'export PATH=\"$PREFIX:\$PATH\"' >> ~/.bashrc" ;;
esac

command -v hyprctl >/dev/null 2>&1 \
  || warn "hyprctl not found — the window rules are Hyprland-specific and will not apply"

# --- init --------------------------------------------------------------------

if [ "$RUN_INIT" -eq 1 ]; then
  say "running sudo-pop --init"
  "$PREFIX/sudo-pop" --init
else
  say "skipping --init. Run it yourself: $PREFIX/sudo-pop --init"
fi

cat <<EOF

${B}Done.${N} Open a new shell, or: source ~/.bashrc

  sudo whoami        goes through the popup
  /usr/bin/sudo ...  always runs the real sudo, alias or not

To remove it, run --uninit ${B}before${N} deleting the binary, or the sudo alias
is left pointing at nothing:

  sudo-pop --uninit && rm $PREFIX/sudo-pop
EOF
