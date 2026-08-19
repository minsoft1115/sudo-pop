#!/usr/bin/env bash
#
# The scenarios that cannot be a cargo test: they need polkitd, a session bus,
# a compositor, or all three. Everything here runs without a password.
#
#   ./tests/scenarios.sh                  run them
#   ./tests/scenarios.sh --keep           leave the agent registered at the end
#   ./tests/scenarios.sh --with-password  also open a foot window for the one
#                                         case that needs a human to type
#
# The session is put back the way it was found -- whichever agent held the seat
# gets it back -- even if a case fails or the run is interrupted.
set -u

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release/sudo-pop"
WORK="$(mktemp -d)"
KEEP=0
WITH_PASSWORD=0
for arg in "$@"; do
  case "$arg" in
    --keep) KEEP=1 ;;
    # foot 창을 띄워 사람이 비밀번호를 넣는 케이스까지 돌린다
    --with-password) WITH_PASSWORD=1 ;;
    *) echo "unknown option: $arg"; exit 2 ;;
  esac
done

PASS=0; FAIL=0
ok()   { printf '  \033[1;32mPASS\033[0m %s\n' "$1"; PASS=$((PASS+1)); }
bad()  { printf '  \033[1;31mFAIL\033[0m %s\n' "$1"; [ $# -gt 1 ] && printf '       %s\n' "$2"; FAIL=$((FAIL+1)); }
head_() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# --- state we must give back -------------------------------------------------
had_omarchy=$(omarchy-plugin-list --json 2>/dev/null | grep -q '"omarchy.polkit"' \
  && omarchy-plugin-list --json 2>/dev/null | tr -d ' \n' | grep -q '"id":"omarchy.polkit","[^}]*"enabled":true' && echo yes || echo no)
had_unit=$(systemctl --user is-active --quiet sudo-pop-agent.service && echo yes || echo no)
AGENT=""

agent_children() {
  # Our own prompt children, found by reading cmdlines rather than matching a
  # pattern that would also match this script.
  local p
  for p in $(pgrep -x sudo-pop 2>/dev/null); do
    tr '\0' ' ' <"/proc/$p/cmdline" 2>/dev/null | grep -q agent-prompt && echo "$p"
  done
}

cleanup() {
  [ -n "$AGENT" ] && kill "$AGENT" 2>/dev/null
  for p in $(agent_children); do kill "$p" 2>/dev/null; done
  if [ "$KEEP" = 0 ]; then
    # 되돌리기는 눈에 보여야 한다. 조용히 실패하면 세션이 인증 못 하는 채로 남는다.
    printf '\n\033[1m되돌리기\033[0m\n'
    if [ "$had_unit" = yes ]; then
      "$BIN" --init >/dev/null 2>&1
      printf '  sudo-pop-agent: %s\n' "$(systemctl --user is-active sudo-pop-agent.service 2>&1)"
    fi
    if [ "$had_omarchy" = yes ]; then
      omarchy-plugin-enable omarchy.polkit >/dev/null 2>&1
    else
      omarchy-plugin-disable omarchy.polkit >/dev/null 2>&1
    fi
    printf '  omarchy.polkit: %s\n' \
      "$(omarchy-plugin-list --json 2>/dev/null | jq -r '.[]|select(.id=="omarchy.polkit")|.enabled')"
    # 시나리오가 태운 실패는 시나리오가 치운다. 공유 카운터라 남겨 두면
    # 다음에 진짜로 필요할 때의 여유가 줄어든다.
    before=$(faillock 2>/dev/null | grep -c "^20" || echo 0)
    faillock --reset 2>/dev/null && printf '  faillock: %s건 정리\n' "$before"
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

windows() { hyprctl clients -j 2>/dev/null | jq '[.[]|select(.class=="sudo-askpass")]|length'; }
wait_window() { local i; for i in $(seq 1 40); do [ "$(windows)" -gt 0 ] && return 0; sleep 0.25; done; return 1; }

# 화면 캡처 제외는 창의 열림 애니메이션이 끝난 뒤에야 안정된다. 여러 번 찍어
# 최소 색 수를 본다 — 규칙이 걸리면 곧 4 색으로 떨어진다.
min_colors() {
  local geom="$1" c min=9999 i
  for i in 1 2 3 4 5 6; do
    grim -g "$geom" "$WORK/win.png" 2>/dev/null
    c=$(identify -format "%k" "$WORK/win.png" 2>/dev/null || echo 9999)
    [ "${c:-9999}" -lt "$min" ] && min=$c
    [ "$min" -le 16 ] && break
    sleep 0.4
  done
  echo "$min"
}

start_agent() {
  SUDO_POP_DEBUG=1 "$BIN" --agent >"$WORK/agent.log" 2>&1 &
  AGENT=$!
  local i
  for i in $(seq 1 40); do grep -q REGISTERED "$WORK/agent.log" && return 0; sleep 0.25; done
  return 1
}

[ -x "$BIN" ] || { echo "build first: cargo build --release"; exit 1; }
systemctl --user stop sudo-pop-agent.service 2>/dev/null

# =============================================================================
head_ "1. 한 세션에 에이전트는 하나"
omarchy-plugin-enable omarchy.polkit >/dev/null 2>&1; sleep 1
if timeout 15 "$BIN" --agent >"$WORK/conflict.log" 2>&1; then :; fi
grep -q "already exists for the given subject" "$WORK/conflict.log" \
  && ok "다른 에이전트가 있으면 등록이 거절된다" \
  || bad "등록 거절이 확인되지 않음" "$(tail -2 "$WORK/conflict.log")"

grep -q "REFUSED" "$WORK/conflict.log" && ok "거절을 사용자에게 알린다" || bad "거절 메시지가 없다"

# =============================================================================
head_ "2. 발신자 검증"
omarchy-plugin-disable omarchy.polkit >/dev/null 2>&1; sleep 1
if start_agent; then
  NAME=$(grep -o 'our bus name: .*' "$WORK/agent.log" | cut -d' ' -f4)
  out=$(timeout 10 busctl --system call "$NAME" /org/minsoft1115/sudo_pop/AuthenticationAgent \
        org.freedesktop.PolicyKit1.AuthenticationAgent BeginAuthentication \
        "sssa{ss}sa(sa{sv})" fake.action "Fake" "" 0 fake-cookie 0 2>&1)
  echo "$out" | grep -qi "access denied" \
    && ok "폴킷이 아닌 발신자는 거절된다" || bad "거절되지 않았다" "$out"
  sleep 1
  [ "$(windows)" = 0 ] && ok "거절된 요청은 창을 띄우지 않는다" || bad "창이 떴다"
else
  bad "에이전트가 등록되지 않아 2번을 건너뜀"
fi

# =============================================================================
head_ "3. 진짜 요청 — 취소·큐·비밀 노출"
if [ -n "$AGENT" ]; then
  ( timeout 12 run0 --background= true </dev/null >/dev/null 2>&1 ) & R1=$!
  if wait_window; then
    ok "폴킷 요청에 창이 뜬다"
    child=$(agent_children | head -1)
    if [ -n "$child" ]; then
      cookie=$(grep -o 'cookie[^ ]*' "$WORK/agent.log" | head -1)
      cmdline=$(tr '\0' ' ' <"/proc/$child/cmdline")
      echo "$cmdline" | grep -qi "cookie" && bad "쿠키가 argv 에 노출됨" || ok "쿠키가 argv 에 없다"
      # PR_SET_DUMPABLE=0 이면 남의 손이 닿는 /proc 항목이 root 소유로 바뀐다.
      # 우리 uid 로도 environ 을 못 읽는 것이 하드닝이 걸렸다는 증거다.
      if cat "/proc/$child/environ" >/dev/null 2>&1; then
        bad "자식의 environ 이 읽힌다 — PR_SET_DUMPABLE 이 안 걸렸다"
      else
        ok "자식의 environ 을 읽을 수 없다 (dumpable=0)"
      fi

      core=$(awk '/Max core file size/ {print $5}' "/proc/$child/limits")
      [ "$core" = "0" ] && ok "자식의 코어덤프 한도가 0" || bad "코어덤프 한도가 $core"
      lck=$(awk '/VmLck/ {print $2}' "/proc/$child/status")
      [ "${lck:-0}" -gt 0 ] && ok "비밀번호 버퍼가 잠겨 있다 (VmLck=${lck}kB)" || bad "VmLck 가 0 — mlock 실패"

      # 두 번째 요청은 줄을 서야 한다
      ( timeout 8 run0 --background= true </dev/null >/dev/null 2>&1 ) & R2=$!
      sleep 3
      [ "$(windows)" -le 1 ] && ok "요청이 겹쳐도 창은 하나" || bad "창이 $(windows) 개"

      # 화면 공유 제외: 창 영역을 찍으면 비어 있어야 한다
      geom=$(hyprctl clients -j | jq -r '.[]|select(.class=="sudo-askpass")|"\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' | head -1)
      if [ -n "$geom" ] && command -v grim >/dev/null && command -v identify >/dev/null; then
        # 실측: 규칙을 빼면 같은 자리에서 1101 색, 걸면 4 색.
        colors=$(min_colors "$geom")
        [ "${colors:-9999}" -le 16 ] \
          && ok "창이 화면 캡처에서 제외된다 (색 $colors 개)" \
          || bad "캡처에 내용이 찍힌다 (색 $colors 개)"
      fi
      kill $R2 2>/dev/null
    else
      bad "자식 프로세스를 찾지 못함"
    fi

    # 호출자가 포기하면 창이 곧 닫혀야 한다 (자체 백스톱은 30초)
    kill $R1 2>/dev/null
    closed=no
    for i in $(seq 1 24); do [ "$(windows)" = 0 ] && { closed=yes; break; }; sleep 0.5; done
    [ "$closed" = yes ] && ok "호출자가 포기하면 창이 닫힌다" || bad "창이 남아 있다"
    grep -q "CancelAuthentication" "$WORK/agent.log" && ok "취소가 처리된다" || bad "취소 로그가 없다"
  else
    bad "창이 뜨지 않았다" "$(tail -3 "$WORK/agent.log")"
  fi
fi
[ -n "$AGENT" ] && { kill "$AGENT" 2>/dev/null; AGENT=""; sleep 1; }

# =============================================================================
head_ "4. 라우팅"
out=$(SUDO_POP_DEBUG=1 "$BIN" -n true 2>&1)
echo "$out" | grep -q "leaving arguments untouched" \
  && ok "-n 은 손대지 않고 sudo 로" || bad "-n 처리가 다르다" "$out"

out=$(SUDO_POP_DEBUG=1 SUDO_POP_RUN0=0 timeout 3 "$BIN" -n true 2>&1)
echo "$out" | grep -q "routing to run0" \
  && bad "SUDO_POP_RUN0=0 인데 run0 로 보냈다" || ok "SUDO_POP_RUN0=0 이면 run0 로 안 보낸다"

# =============================================================================
head_ "5. 창 — 규칙과 Esc 취소"
"$BIN" --init >/dev/null 2>&1     # 창 규칙이 깔려 있어야 한다
sleep 60 & SUBJECT=$!
( echo test-cookie | SUDO_POP_USER="$USER" SUDO_POP_SUBJECT_PID=$SUBJECT SUDO_POP_MESSAGE=scenario \
    "$BIN" --agent-prompt >"$WORK/prompt.log" 2>&1; echo "exit=$?" >>"$WORK/prompt.log" ) &
if wait_window; then
  win=$(hyprctl clients -j | jq '.[]|select(.class=="sudo-askpass")')
  [ "$(echo "$win" | jq -r .floating)" = "true" ] && ok "창이 떠 있다 (floating)" || bad "floating 이 아니다"
  [ "$(echo "$win" | jq -r .pinned)" = "true" ]   && ok "창이 고정된다 (pin)"    || bad "pin 이 아니다"
  [ "$(echo "$win" | jq -r '.size|join("x")')" = "400x200" ] && ok "창 크기가 규칙대로" || bad "창 크기가 다르다"

  # 화면 공유 제외: 창 영역을 찍으면 내용이 없어야 한다
  geom=$(echo "$win" | jq -r '"\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"')
  if command -v grim >/dev/null && command -v identify >/dev/null; then
    colors=$(min_colors "$geom")
    [ "${colors:-9999}" -le 16 ] && ok "창이 화면 캡처에서 제외된다 (색 $colors 개)" \
                                 || bad "캡처에 내용이 찍힌다 (색 $colors 개)"
  fi

  # Esc 는 사용자가 창을 닫는 경로. 종료 코드 2 여야 요청이 취소로 끝난다.
  # 창이 포커스를 잡은 뒤에 보낸다. 규칙이 stay_focused 라 곧 잡는다.
  for i in $(seq 1 20); do
    [ "$(hyprctl activewindow -j | jq -r .class)" = "sudo-askpass" ] && break; sleep 0.25
  done
  closed=no
  for attempt in 1 2 3; do
    hyprctl dispatch 'hl.dsp.send_shortcut({ mods = 0, key = "escape", window = "class:sudo-askpass" })' >/dev/null 2>&1
    for i in $(seq 1 12); do [ "$(windows)" = 0 ] && { closed=yes; break; }; sleep 0.25; done
    [ "$closed" = yes ] && break
  done
  [ "$closed" = yes ] && ok "Esc 로 창이 닫힌다" || bad "Esc 후에도 창이 남아 있다"
  sleep 1
  grep -q "exit=2" "$WORK/prompt.log" && ok "취소는 종료 코드 2 (요청은 정상 종료)" \
                                      || bad "종료 코드가 2 가 아니다" "$(cat "$WORK/prompt.log")"
else
  bad "창이 뜨지 않아 5번을 건너뜀"
fi
kill $SUBJECT 2>/dev/null
for p in $(agent_children); do kill "$p" 2>/dev/null; done

# =============================================================================
head_ "6. 잠긴 계정 게이팅 (C1)"
# deny 는 /etc/security/faillock.conf(이 머신은 10) 에서 온다. 진짜 faillock 을
# 태우면 sudo·로그인까지 잠기므로, tally 만 10 건으로 흉내 내는 가짜 faillock 을
# PATH 앞에 두어 게이트만 격리한다. 실제 카운터는 건드리지 않는다.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/faillock" <<'FAKE'
#!/usr/bin/env bash
# --reset 은 무시(가짜라 지울 게 없다). --user <name> 이면 잠긴 것처럼 V 행 10 건.
for a in "$@"; do [ "$a" = "--reset" ] && exit 0; done
printf '%s:
' "${USER:-user}"
printf 'When                Type  Source   Valid
'
for i in $(seq 1 10); do printf '2026-08-19 12:00:%02d RHOST test V
' "$i"; done
FAKE
chmod +x "$WORK/bin/faillock"

sleep 60 & LSUBJECT=$!
( echo test-cookie | PATH="$WORK/bin:$PATH" SUDO_POP_USER="$USER"     SUDO_POP_SUBJECT_PID=$LSUBJECT SUDO_POP_MESSAGE=locked     "$BIN" --agent-prompt >"$WORK/locked.log" 2>&1; echo "exit=$?" >>"$WORK/locked.log" ) &
sleep 2
[ "$(windows)" = 0 ] && ok "잠긴 계정이면 창을 띄우지 않는다" || bad "창이 떴다"
# 종료 코드 2(취소)여야 polkitd 가 요청을 되던지지 않는다. 1 이면 빈 창이 반복된다.
grep -q "exit=2" "$WORK/locked.log" && ok "잠긴 계정은 종료 코드 2 (요청 정상 종료)"   || bad "종료 코드가 2 가 아니다" "$(cat "$WORK/locked.log")"
grep -qi "lock" "$WORK/locked.log" && ok "잠긴 계정 안내를 남긴다" || bad "안내 메시지가 없다"
kill $LSUBJECT 2>/dev/null
for p in $(agent_children); do kill "$p" 2>/dev/null; done

# =============================================================================
head_ "7. 설치 왕복"
cp ~/.config/hypr/hyprland.lua "$WORK/hl.before" 2>/dev/null
"$BIN" --init >"$WORK/init2.log" 2>&1
grep -q "already current" "$WORK/init2.log" && ok "--init 은 멱등하다" || bad "두 번째 --init 이 다시 쓴다"
"$BIN" --uninit >"$WORK/uninit.log" 2>&1
[ -e ~/.config/systemd/user/sudo-pop-agent.service ] && bad "유닛이 남았다" || ok "--uninit 이 유닛을 지운다"
[ -e ~/.config/minsoft1115/hypr/sudo-pop.lua ] && bad "창 규칙이 남았다" || ok "--uninit 이 창 규칙을 지운다"
[ -e ~/.config/minsoft1115/bash/sudo-pop.sh ] && bad "셸 스니펫이 남았다" || ok "--uninit 이 셸 스니펫을 지운다"
grep -q "minsoft1115-bash:begin" ~/.bashrc 2>/dev/null && ok "공유 로더 블록은 남긴다" || bad "남의 로더 블록을 지웠다"
if [ -f "$WORK/hl.before" ]; then
  strip() { grep -v 'sudo-pop' "$1" | grep -v '^[[:space:]]*$'; }
  diff -q <(strip "$WORK/hl.before") <(strip ~/.config/hypr/hyprland.lua) >/dev/null \
    && ok "hyprland.lua 의 다른 줄은 그대로" || bad "hyprland.lua 의 다른 줄이 바뀌었다"
fi

# =============================================================================
# 비밀번호가 필요한 케이스. 사람이 있어야 하므로 foot 창을 띄워 맡긴다.
if [ "$WITH_PASSWORD" = 1 ]; then
  head_ "8. 성공 경로 (직접 입력)"
  "$BIN" --init >/dev/null 2>&1
  omarchy-plugin-disable omarchy.polkit >/dev/null 2>&1; sleep 1
  systemctl --user restart sudo-pop-agent.service 2>/dev/null || start_agent
  sleep 1
  foot -a sudo-pop-scenario bash -lc "echo '창이 뜨면 비밀번호를 입력하세요 (25초 안).'; \
      run0 --background= true; echo \"run0 exit=\$?\" | tee $WORK/run0.txt; sleep 3" >/dev/null 2>&1
  grep -q "run0 exit=0" "$WORK/run0.txt" 2>/dev/null \
    && ok "인증에 성공하면 명령이 실행된다" || bad "성공 경로가 확인되지 않았다" "$(cat "$WORK/run0.txt" 2>/dev/null)"
fi

# =============================================================================
printf '\n\033[1m결과: %d 통과, %d 실패\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
