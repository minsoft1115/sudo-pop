#!/usr/bin/env bash
# Isolated GUI -> fake helper check. No polkit registration, real PAM, service
# changes, clipboard changes, installation, or faillock reset. Test strings only.
# Exit 77 means prerequisites are unavailable (SKIP), never PASS.
set -u
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
for tool in cargo hyprctl jq wtype; do
  command -v "$tool" >/dev/null || { echo "SKIP: missing $tool"; exit 77; }
done
hyprctl clients -j >/dev/null 2>&1 || { echo 'SKIP: no Hyprland session'; exit 77; }
cargo build --manifest-path "$ROOT/Cargo.toml" --target-dir "$ROOT/target" --locked --offline || exit 1
WORK="$(mktemp -d)" || exit 1
PROMPT_PID=""
OLD_WINDOW=$(hyprctl activewindow -j | jq -r '.address // empty')
cleanup() {
  if [ -n "$PROMPT_PID" ]; then
    kill "$PROMPT_PID" 2>/dev/null || true
    wait "$PROMPT_PID" 2>/dev/null || true
  fi
  if [ -n "$OLD_WINDOW" ]; then
    hyprctl dispatch "hl.dsp.focus({ window = \"address:$OLD_WINDOW\" })" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
fail() {
  echo "FAIL: $*"
  # The fake helper never logs answers. Only show this test process's diagnostics.
  tail -n 15 "$WORK/prompt.log" 2>/dev/null || true
  hyprctl clients -j | jq --argjson pid "${PROMPT_PID:-0}" '[.[] | select(.pid == $pid or .class == "sudo-askpass") | {pid, class, address}]'
  cat "$WORK/focus.log" 2>/dev/null || true
  exit 1
}
printf 'test-cookie\n' >"$WORK/cookie"
# A debug-only empty tally prevents this test from depending on account state.
printf '#!/bin/sh\nexit 0\n' >"$WORK/faillock"
chmod 700 "$WORK/faillock"
PASS=0
focus_prompt() {
  local address i
  for i in $(seq 1 80); do
    address=$(hyprctl clients -j | jq -r --argjson pid "$PROMPT_PID" '.[] | select(.pid == $pid and .class == "sudo-askpass") | .address')
    if [ -n "$address" ]; then
      hyprctl dispatch "hl.dsp.focus({ window = \"address:$address\" })" >"$WORK/focus.log" 2>&1 || return 1
      sleep 0.15
      [ "$(hyprctl activewindow -j | jq -r '.address')" = "$address" ] && return 0
    fi
    kill -0 "$PROMPT_PID" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}
wait_result() {
  local expected="$1" i
  for i in $(seq 1 80); do
    [ "$(tail -n 1 "$WORK/result" 2>/dev/null)" = "$expected" ] && return 0
    kill -0 "$PROMPT_PID" 2>/dev/null || break
    sleep 0.1
  done
  [ "$(tail -n 1 "$WORK/result" 2>/dev/null)" = "$expected" ]
}
for case_name in edited_unicode retry; do
  : >"$WORK/result"
  expected='  sudo-pop 한🔐 test  '
  SUDO_POP_USER="$(id -un)" SUDO_POP_MESSAGE='sudo-pop input test (fake password only)' \
    SUDO_POP_HELPER_BIN="$ROOT/tests/fake-helper.sh" \
    SUDO_POP_HELPER_SOCKET="$WORK/no-helper.socket" \
    SUDO_POP_FAILLOCK_BIN="$WORK/faillock" \
    FAKE_HELPER_MODE=check FAKE_HELPER_PASSWORD="$expected" FAKE_HELPER_RESULT="$WORK/result" \
    "$ROOT/target/debug/sudo-pop" --agent-prompt <"$WORK/cookie" >"$WORK/prompt.log" 2>&1 &
  PROMPT_PID=$!
  focus_prompt || fail "$case_name: test window did not acquire focus"
  sleep 0.3
  if [ "$case_name" = retry ]; then
    wtype -s 20 'deliberately-wrong-test-input' || fail 'typing failed'
    wtype -k Return || fail 'Enter failed'
    wait_result MISMATCH || fail 'wrong test string was not rejected'
    sleep 0.3
    focus_prompt || fail 'retry window lost'
  fi
  wtype -s 20 "$expected" || fail 'Unicode typing failed'
  wtype x || fail 'typing suffix failed'
  wtype -k BackSpace || fail 'editing failed'
  wtype -k Return || fail 'Enter failed'
  wait_result MATCH || fail "$case_name: helper did not receive the exact test string"
  for _ in $(seq 1 60); do
    kill -0 "$PROMPT_PID" 2>/dev/null || break
    sleep 0.1
  done
  kill -0 "$PROMPT_PID" 2>/dev/null && fail 'prompt did not exit after success'
  wait "$PROMPT_PID" || fail 'prompt reported failure despite matching input'
  PROMPT_PID=""
  if [ "$case_name" = retry ]; then
    [ "$(cat "$WORK/result")" = $'MISMATCH\nMATCH' ] || fail 'unexpected retry sequence'
  else
    [ "$(cat "$WORK/result")" = MATCH ] || fail 'unexpected helper attempts'
  fi
  echo "PASS: $case_name (exact bytes, including Unicode and surrounding spaces)"
  PASS=$((PASS+1))
done
printf 'Result: %d PASS, 0 FAIL, 0 SKIP\n' "$PASS"
