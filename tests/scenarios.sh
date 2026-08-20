#!/usr/bin/env bash
#
# The scenarios that cannot be a cargo test: they need polkitd, a session bus,
# a compositor, or all three. Everything here runs without a password.
#
#   ./tests/scenarios.sh                  run them
#   ./tests/scenarios.sh --keep           leave the agent registered at the end
#   ./tests/scenarios.sh --with-password  also open a foot window for the one
#                                         case that needs a human to type
#   ./tests/scenarios.sh --restart-polkitd  also restart polkitd and check the
#                                         agent follows it (needs one password)
#
# The session is put back the way it was found -- whichever agent held the seat
# gets it back -- even if a case fails or the run is interrupted.
set -u

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release/sudo-pop"
WORK="$(mktemp -d)"
KEEP=0
WITH_PASSWORD=0
RESTART_POLKITD=0
for arg in "$@"; do
  case "$arg" in
    --keep) KEEP=1 ;;
    # foot 창을 띄워 사람이 비밀번호를 넣는 케이스까지 돌린다
    --with-password) WITH_PASSWORD=1 ;;
    # polkitd 를 실제로 재시작한다. 비밀번호가 한 번 필요하고, 세션 전체에
    # 영향이 있으므로 따로 켜야 한다.
    --restart-polkitd) RESTART_POLKITD=1 ;;
    *) echo "unknown option: $arg"; exit 2 ;;
  esac
done

POLKIT_NAME="org.freedesktop.PolicyKit1"

# 되돌릴 때는 설치된 바이너리로 --init 한다. $BIN 으로 되돌리면 유닛의 ExecStart 가
# 개발 트리를 가리킨 채 남아, 다음부터 cargo build 가 도는 동안 에이전트가 죽는다
# (rationale §6-1). 설치본이 없으면 어쩔 수 없이 $BIN 이다.
INSTALLED="$(command -v sudo-pop 2>/dev/null || true)"
[ -z "$INSTALLED" ] && INSTALLED="$BIN"

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
      "$INSTALLED" --init >/dev/null 2>&1
      printf '  sudo-pop-agent: %s (%s)\n' \
        "$(systemctl --user is-active sudo-pop-agent.service 2>&1)" \
        "$(awk -F= '/^ExecStart=/{print $2}' ~/.config/systemd/user/sudo-pop-agent.service 2>/dev/null)"
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
      # 창에 뜨는 카운트다운의 기준. 요청이 도착한 시각부터 재므로 첫 요청은
      # 25초에 거의 붙어 있어야 하고, 어떤 요청도 그것을 넘겨선 안 된다 —
      # 줄을 서느라 흘린 시간을 창이 다시 내주면 없는 여유를 약속하는 셈이다.
      lefts=$(grep -o 'left       : [0-9]* ms' "$WORK/agent.log" | grep -o '[0-9]*')
      first=$(echo "$lefts" | head -1)
      if [ -n "$first" ] && [ "$first" -le 25000 ] && [ "$first" -ge 24000 ]; then
        ok "첫 요청의 남은 시간이 25초에 붙어 있다 (${first}ms)"
      else
        bad "남은 시간이 이상하다" "first=${first:-없음}"
      fi
      over=$(echo "$lefts" | awk '$1 > 25000' | wc -l)
      [ "$over" = 0 ] && ok "어떤 요청도 25초를 넘겨 받지 않는다" \
                      || bad "25초를 넘겨 받은 요청이 $over 건"

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
  # 폭은 보여 줄 줄에 맞춰 400~800 사이에서 정해진다. 높이는 고정이다.
  ww=$(echo "$win" | jq -r '.size[0]'); wh=$(echo "$win" | jq -r '.size[1]')
  { [ "$ww" -ge 400 ] && [ "$ww" -le 800 ] && [ "$wh" = 200 ]; } \
    && ok "창 크기가 규칙대로 (${ww}x${wh})" || bad "창 크기가 다르다" "${ww}x${wh}"

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
head_ "8. 다른 에이전트 감지 (L1)"
# 예전 코드는 `pgrep -x <이름>` 으로 봤는데, comm 은 커널이 15자에서 자르므로
# `polkit-gnome-authentication-agent-1`(35자) 같은 이름은 영영 안 걸렸다.
# 여기서는 진짜로 그 이름을 단 프로세스를 만들어 그 구멍을 재현한다.
omarchy-plugin-disable omarchy.polkit >/dev/null 2>&1; sleep 1
"$BIN" --uninit >/dev/null 2>&1      # enable 여부를 깨끗한 자리에서 본다

mkdir -p "$WORK/bin"
FAKE_AGENT="$WORK/bin/polkit-kde-authentication-agent-1"
cp "$(command -v sleep)" "$FAKE_AGENT"
"$FAKE_AGENT" 120 & FAKE_PID=$!
sleep 0.5
fake_comm=$(cat "/proc/$FAKE_PID/comm" 2>/dev/null)
[ "$fake_comm" = "polkit-kde-auth" ] \
  && ok "커널이 이름을 15자로 자른다 (comm=$fake_comm)" \
  || bad "comm 이 예상과 다르다" "comm=$fake_comm"
# 옛 방식이 왜 못 잡았는지를 같은 자리에서 보여 준다.
pgrep -x polkit-kde-authentication-agent-1 >/dev/null 2>&1 \
  && bad "pgrep -x 가 잡았다 — 전제가 바뀌었다" \
  || ok "pgrep -x 로는 못 잡는다 (옛 방식의 구멍)"

"$BIN" --init >"$WORK/init-proc.log" 2>&1
grep -q "already holds this session's polkit seat" "$WORK/init-proc.log" \
  && ok "프로세스로 도는 다른 에이전트를 잡는다" \
  || bad "감지하지 못했다" "$(tail -3 "$WORK/init-proc.log")"
grep -q "polkit-kde-auth" "$WORK/init-proc.log" \
  && ok "무엇을 찾았는지 이름으로 말한다" || bad "찾은 이름이 안 나온다"
[ "$(systemctl --user is-enabled sudo-pop-agent.service 2>&1)" = "enabled" ] \
  && bad "다른 에이전트가 있는데 유닛을 enable 했다" \
  || ok "유닛은 깔되 enable 하지 않는다"
[ -e ~/.config/systemd/user/sudo-pop-agent.service ] \
  && ok "유닛 파일 자체는 깔린다" || bad "유닛 파일이 없다"

kill $FAKE_PID 2>/dev/null; wait $FAKE_PID 2>/dev/null; sleep 0.5

# 유닛으로만 도는 경우 — 프로세스 이름(sleep)에는 단서가 없으므로 3순위만 본다.
systemd-run --user --unit=scenario-polkit-agent.service --quiet sleep 120 2>/dev/null
sleep 1
if systemctl --user is-active --quiet scenario-polkit-agent.service; then
  "$BIN" --init >"$WORK/init-unit.log" 2>&1
  grep -q "already holds this session's polkit seat" "$WORK/init-unit.log" \
    && ok "활성 user 유닛으로만 있는 에이전트를 잡는다 (3순위)" \
    || bad "유닛 감지가 안 된다" "$(tail -3 "$WORK/init-unit.log")"
  grep -q "systemctl --user disable --now scenario-polkit-agent.service" "$WORK/init-unit.log" \
    && ok "유닛일 때는 끄는 명령을 정확히 알려 준다" \
    || bad "안내가 유닛에 맞지 않다" "$(grep -A2 'To switch' "$WORK/init-unit.log")"
  systemctl --user stop scenario-polkit-agent.service 2>/dev/null
else
  bad "가짜 유닛을 띄우지 못해 3순위를 건너뜀"
fi
sleep 1

# 아무도 없으면 이제 켜져야 한다 — 감지가 과하게 걸리지 않는지 보는 반대편.
"$BIN" --init >"$WORK/init-clear.log" 2>&1
grep -q "already holds this session's polkit seat" "$WORK/init-clear.log" \
  && bad "아무도 없는데 자리가 찼다고 한다" "$(tail -3 "$WORK/init-clear.log")" \
  || ok "자리가 비면 평소대로 enable 한다"
systemctl --user stop sudo-pop-agent.service 2>/dev/null

# =============================================================================
head_ "9. polkitd 소유자 추적 (L5)"
# 결함은 "소유자를 먼저 읽고 구독을 나중에" 였다. 그 사이에 polkitd 가 재시작하면
# 신호를 못 받고, 죽은 고유 이름으로 진짜 폴킷을 영영 거절한다. 순서가 곧 수정이라
# 순서를 로그에서 확인한다.
if start_agent; then
  sub=$(grep -n "watching $POLKIT_NAME for owner changes" "$WORK/agent.log" | head -1 | cut -d: -f1)
  reg=$(grep -n "REGISTERED" "$WORK/agent.log" | head -1 | cut -d: -f1)
  own=$(grep -n "polkitd owns" "$WORK/agent.log" | head -1 | cut -d: -f1)
  if [ -n "$sub" ] && [ -n "$own" ] && [ "$sub" -lt "$own" ]; then
    ok "소유자를 읽기 전에 구독한다 (구독 $sub 줄 < 읽기 $own 줄)"
  else
    bad "구독이 소유자 읽기보다 늦다" "구독=$sub 읽기=$own"
  fi
  if [ -n "$sub" ] && [ -n "$reg" ] && [ "$sub" -lt "$reg" ]; then
    ok "등록보다도 먼저 구독한다 (등록 $reg 줄)"
  else
    bad "구독이 등록보다 늦다" "구독=$sub 등록=$reg"
  fi
  # 구독을 일찍 하면 이미 들고 있는 소유자에 대한 신호가 올 수 있다. 그것으로
  # 재등록하면 polkitd 가 "already exists" 를 돌려주므로, 걸러야 한다.
  grep -q "could not register again" "$WORK/agent.log" \
    && bad "같은 소유자에 대해 재등록을 시도했다" || ok "이미 아는 소유자로는 재등록하지 않는다"

  if [ "$RESTART_POLKITD" = 1 ]; then
    printf '  polkitd 를 재시작합니다 — foot 창에서 비밀번호를 한 번 넣어 주세요.\n'
    foot -a sudo-pop-scenario bash -lc \
      "echo 'polkitd 재시작 — 비밀번호를 넣으세요.'; run0 systemctl restart polkit.service; \
       echo \"exit=\$?\" > $WORK/restart.txt; sleep 2" >/dev/null 2>&1
    if grep -q "exit=0" "$WORK/restart.txt" 2>/dev/null; then
      ok "polkitd 를 재시작했다"
      for i in $(seq 1 40); do grep -q "came back as" "$WORK/agent.log" && break; sleep 0.25; done
      grep -q "came back as" "$WORK/agent.log" \
        && ok "새 소유자를 보고 다시 등록한다" || bad "재등록 로그가 없다" "$(tail -3 "$WORK/agent.log")"
      # 진짜 시험은 여기다. 검증 기준이 낡았으면 polkitd 의 요청이 거절되어
      # 창이 안 뜬다. polkitd 재시작이 인증 캐시도 지우므로 반드시 물어본다.
      ( timeout 12 run0 --background= true </dev/null >/dev/null 2>&1 ) & RP=$!
      if wait_window; then
        ok "재시작 뒤에도 진짜 폴킷 요청에 창이 뜬다 (검증 기준이 따라갔다)"
      else
        bad "창이 안 뜬다 — 낡은 고유 이름으로 거절하고 있다" "$(tail -5 "$WORK/agent.log")"
      fi
      kill $RP 2>/dev/null
      for p in $(agent_children); do kill "$p" 2>/dev/null; done
    else
      bad "polkitd 재시작이 확인되지 않아 건너뜀" "$(cat "$WORK/restart.txt" 2>/dev/null)"
    fi
  fi
  kill "$AGENT" 2>/dev/null; AGENT=""
else
  bad "에이전트가 등록되지 않아 9번을 건너뜀"
fi

# =============================================================================
head_ "10. GUI 유래 요청과 자체 백스톱"
# 여기까지의 시나리오는 전부 run0 을 통했다. 데스크톱 앱이 거는 요청은 폴킷이
# 주는 것이 다르다 — message 가 사람 말이고, subject 의 cmdline 은 "무엇을 할지"
# 가 아니라 "누가 묻는지"다. pkcheck 로 그 경로를 그대로 만든다.
if start_agent; then
  MOUNT=org.freedesktop.udisks2.filesystem-mount-system
  pkcheck --action-id "$MOUNT" --process $$ --allow-user-interaction >/dev/null 2>&1 &
  PKC=$!
  if wait_window; then
    ok "데스크톱 액션에도 창이 뜬다 (run0 이 아닌 경로)"
    grep -q "action_id  : $MOUNT" "$WORK/agent.log" \
      && ok "폴킷이 보낸 액션이 그대로 온다" || bad "action_id 가 로그에 없다"
    # 이 문장이 창의 둘째 줄이 된다. run0 의 message 와 달리 쓸모가 있다.
    grep -q "message    : Authentication is required to mount" "$WORK/agent.log" \
      && ok "message 가 무엇을 할지 말해 준다" \
      || bad "message 가 예상과 다르다" "$(grep 'message    :' "$WORK/agent.log" | tail -1)"

    for i in $(seq 1 20); do
      [ "$(hyprctl activewindow -j | jq -r .class)" = "sudo-askpass" ] && break; sleep 0.25
    done
    closed=no
    for attempt in 1 2 3; do
      hyprctl dispatch 'hl.dsp.send_shortcut({ mods = 0, key = "escape", window = "class:sudo-askpass" })' >/dev/null 2>&1
      for i in $(seq 1 12); do [ "$(windows)" = 0 ] && { closed=yes; break; }; sleep 0.25; done
      [ "$closed" = yes ] && break
    done
    [ "$closed" = yes ] && ok "Esc 로 닫힌다" || bad "Esc 후에도 창이 남아 있다"
  else
    bad "데스크톱 액션에 창이 뜨지 않았다" "$(tail -3 "$WORK/agent.log")"
  fi
  kill $PKC 2>/dev/null

  # 자체 백스톱. pkcheck 는 자기 타임아웃이 없어서 폴킷이 25초에 취소해 주지
  # 않는다 — 창을 닫는 것이 우리 30초뿐인 유일한 경우다. 이것이 안 돌면 창이
  # 영영 남는다. 30초를 기다리는 값이 그래서 있다.
  pkcheck --action-id "$MOUNT" --process $$ --allow-user-interaction >/dev/null 2>&1 &
  PKC2=$!
  if wait_window; then
    t0=$(date +%s)
    gone=no
    for i in $(seq 1 80); do [ "$(windows)" = 0 ] && { gone=yes; break; }; sleep 0.5; done
    took=$(( $(date +%s) - t0 ))
    if [ "$gone" = yes ] && [ "$took" -ge 25 ] && [ "$took" -le 38 ]; then
      ok "호출자가 포기하지 않아도 자체 백스톱이 창을 닫는다 (${took}초)"
    else
      bad "백스톱이 예상대로 돌지 않았다" "closed=$gone took=${took}초"
    fi
  else
    bad "두 번째 창이 뜨지 않아 백스톱을 못 봄"
  fi
  kill $PKC2 2>/dev/null
  for p in $(agent_children); do kill "$p" 2>/dev/null; done
  kill "$AGENT" 2>/dev/null; AGENT=""
else
  bad "에이전트가 등록되지 않아 10번을 건너뜀"
fi

# =============================================================================
# 비밀번호가 필요한 케이스. 사람이 있어야 하므로 foot 창을 띄워 맡긴다.
if [ "$WITH_PASSWORD" = 1 ]; then
  head_ "11. 성공 경로 (직접 입력)"
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
