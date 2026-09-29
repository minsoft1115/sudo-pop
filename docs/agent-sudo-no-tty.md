> 2026-09-28 업데이트: 기본 래퍼는 run0 전용이며 sudo 자동 전환을 하지 않는다.
> 아래의 기존 혼합 라우팅 설명은 `SUDO_POP_MODE=compat`에 해당한다.
> 현재 동작과 호환성은 [요청 표시와 실행 정책](request-display-and-run0.md)을 참고한다.

# 터미널 없는 프로세스의 `sudo` — 조사 기록

> 이전 설치 변경 기록: 설치 기본값이 alias 에서 터미널 PATH 래퍼로 바뀌었다.
> 래퍼는 기존 run0/실제 sudo 분기를 유지한다. `SUDO_POP_RUN0=0` 강제 설정은 제거했다.
> 아래는 PATH 래퍼 도입 전 조사 기록이다.

2026-09-14. "터미널에서 띄운 AI 에이전트가 `sudo` 를 쓰면 sudo-pop 이 뜨나" 에서 출발했다.
결론은 **안 뜬다** 이고, 대신 sudo 가 화면 없이 지문 센서를 30초 기다린다는 것이 드러났다.
설계 쪽 판단은 [`rationale.md`](rationale.md) §26 에 있고, 이 문서는 증거와 선택지를 모아 둔
것이다.

## 1. 실측

Claude Code 의 Bash 도구에서 실행했다. tty 없음, `SUDO_ASKPASS` 없음, `.bashrc` 안 읽음.

```
$ tty
not a tty
$ echo "SUDO_ASKPASS=${SUDO_ASKPASS:-unset}"
SUDO_ASKPASS=unset
$ timeout 5 sudo true
Place your right index finger on the fingerprint reader
Verification timed out
sudo: a terminal is required to read the password; either use the -S option to read from standard input or configure an askpass helper
sudo: a password is required
```

저널 (`journalctl | grep 'sudo\['`):

```
13:40:34.661  sudo[635425]: pam_exec(sudo:auth): /usr/bin/omarchy-hw-laptop-closed failed: exit code 1
13:41:04.928  sudo[635425]: pam_unix(sudo:auth): conversation failed
13:41:04.928  sudo[635425]: pam_unix(sudo:auth): auth could not identify password for [lmh]
```

- 시작부터 실패까지 30.3초. `pam_exec` 게이트 → `pam_fprintd` 30초 대기 → `pam_unix` 가
  비밀번호를 읽으려다 터미널이 없어 실패.
- `timeout 5` 가 끊지 못했다. sudo 는 PAM 인증 중 SIGTERM 을 막는다. 종료 코드 124 는
  `timeout` 이 포기한 것이고 sudo 는 30초를 다 살았다.
- 그 30초 동안 등록된 손가락이 닿으면 sudo 는 **성공한다.** 화면에는 아무것도 없다.

## 2. 왜 sudo-pop 이 안 뜨나

| 조건 | 에이전트의 셸 | 결과 |
|---|---|---|
| `alias sudo='sudo-pop'` (`~/.config/minsoft1115/bash/sudo-pop.sh`, `.bashrc` 로더가 source) | `.bashrc` 를 읽지 않음. 스크립트 안의 `sudo` 는 어차피 alias 를 안 탐 | `/usr/bin/sudo` |
| `sudo -A` + `SUDO_ASKPASS` | 래퍼가 자기 자식 sudo 에게만 넘김 (`src/wrapper.rs` `exec_sudo`) | 없음 |
| `sudo.conf` 의 `Path askpass` | 설정 안 됨 | 없음 |
| polkit | sudo 는 polkit 을 쓰지 않음 | 해당 없음 |

sudo 가 askpass 를 부르는 것은 `-A` 를 받았을 때, 또는 터미널이 없는데 askpass 가 설정돼 있을
때뿐이다. `sudo.conf(5)`:

> askpass — The fully-qualified path to a helper program used to read the user's password
> when no terminal is available. This may be the case when sudo is executed from a graphical
> (as opposed to text-based) application.

## 3. 30초 대기의 정체

`/etc/pam.d/sudo` (Omarchy 지문 설정이 끼워 넣은 두 줄 + 원래 파일):

```
auth      [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth      sufficient pam_fprintd.so
#%PAM-1.0
auth      include   system-auth
```

`pam_fprintd` 는 터미널 유무를 보지 않는다. fprintd 에 verify 를 걸고 기본 `timeout=30`
까지 기다린 뒤, 실패로 돌아가면 `system-auth` 의 `pam_unix` 로 내려간다. 터미널이 없다는
것은 `pam_unix` 가 대화를 시도할 때에야 드러난다. 즉 지문을 sudo 에 붙인 모든 Omarchy 머신에서
**터미널 없는 sudo 는 30초 동안 지문으로 통과될 수 있는 상태**로 있다. sudo-pop 과 무관하다.

## 4. 전역으로 걸 수 있나

### (a) askpass 전역 설정

- `/etc/sudo.conf` 에 `Path askpass <sudo-pop 의 askpass 링크>` (root), 또는
  `~/.config/environment.d/*.conf` 에 `SUDO_ASKPASS=…` (세션 환경).
- 효과: 터미널 없는 `sudo` 가 실패하는 대신 sudo-pop 의 askpass 창(rationale §7 의 sudo 경로)이
  뜬다.
- 한계 1: askpass 는 `pam_unix` 차례에야 불린다. 지문 30초는 여전히 먼저, 조용히 지나가고
  창은 그 뒤에 뜬다. 줄이려면 `/etc/pam.d/sudo` 의 `pam_fprintd` 줄에 `timeout=` 이 필요하다.
- 한계 2: tty 가 있는 sudo 는 지금처럼 터미널에서 묻는다. 대화형 셸의 alias 경로와 겹치지
  않는다.
- 주의: sudo-pop 의 askpass 링크는 런타임 디렉터리 안에 있고 (`src/paths.rs`
  `ensure_askpass_symlink`) 래퍼가 실행될 때 만든다. 전역으로 쓰려면 고정 경로의 링크가 따로
  필요하다.

### (b) PATH 에 `sudo` 심

- `~/.local/bin/sudo → sudo-pop` 을 PATH 에서 `/usr/bin` 보다 앞에 둔다.
- 효과: 비대화형 셸도 래퍼를 타고, 래퍼는 run0 로 보내 polkit 창이 뜬다. 지문 단계가 있고
  tty 가 필요 없다.
- 대가: 그 PATH 를 물려받는 모든 스크립트의 `sudo` 가 run0 가 된다. sudoers 대신 polkit 규칙,
  환경 초기화, cwd 등 의미가 다르고 Omarchy 업데이트 스크립트(rationale §25)까지 걸린다.
  래퍼가 모르는 플래그는 진짜 sudo 로 넘긴다 (rationale §7-1).
- 이 머신의 현재 PATH 앞부분: `~/.cargo/bin`, mise 의 claude·codex 경로. `~/.local/bin` 은 뒤에
  있다 (`which -a sudo` 는 `/usr/bin/sudo` 하나).

둘 다 사용자가 알고 켜는 옵트인이다. 설치는 대화형 alias 만 깐다.

## 5. 참고

- 실측 셸: Claude Code Bash 도구 (pty 없음). 다른 에이전트가 pty 를 주면 sudo 는 그 pty 에
  터미널 프롬프트를 내고, 지문도 같은 30초를 기다린다.
- 관련: `docs/omarchy-polkit-pam.md` (polkit-1 쪽 PAM 파일), rationale §22-5·§22-6 (센서 점유,
  fprintd 정지), §25 (`omarchy update` 의 인증 경로).


## 6. 세션 공통 적용 검토 (2026-09-28)

현재 이 에이전트의 실행 환경에서 `sudo` 는 `/usr/bin/sudo` 로 해석되고,
설치된 연결은 대화형 Bash alias 뿐이다. Hyprland 시작 시 세션 PATH 앞에 전용
`sudo` 래퍼 디렉터리를 두면 그 환경을 상속한 비대화형 프로세스에도 적용할 수 있다.
단순한 autostart 자식의 `export` 는 부모 Hyprland 환경을 바꾸지 않는다. 세션 시작 환경과
systemd 사용자 서비스의 환경을 함께 고려해야 하며, 이미 실행 중인 에이전트는 다시 시작해야 한다.

선택과 영향:

- 현재 sudo-pop 을 그대로 연결: 일반 명령은 run0 / polkit 으로 가므로 지문 대기부터 창을
  표시할 수 있다. 대신 sudoers / NOPASSWD, 인증 캐시, 전달 환경, 터미널·입출력 동작이 달라질 수 있다.
  `sudo -v` 로 sudo 캐시를 준비해도 이후 run0 인증을 준비한 것이 아니다.
- 래퍼에서 `SUDO_POP_RUN0=0` 지정: sudo 정책과 실행 경로를 유지하고 askpass 를 붙인다.
  지문은 sudo PAM 이 먼저 처리하므로 센서 대기 중에는 창이 없을 수 있다.
- 세션 `SUDO_ASKPASS` 만 지정: sudo 자체는 유지하며 TTY 가 없을 때 비밀번호 창을 연결한다.
  현재 런타임 링크 대신 basename 이 `askpass` 인 고정 링크가 필요하다. 또 현재 재시도 카운터는
  래퍼가 요청마다 초기화하므로, 직접 askpass 를 공통 연결하려면 요청별 카운터 분리도 검토해야 한다.

어느 방식도 `/usr/bin/sudo` 절대 경로, 별도 PATH, 환경 제거, 컨테이너·SSH·별도 서비스까지
일괄 대체하지 않는다. `-n` 은 비대화형, `-S` 는 표준입력을 선택한 것이므로 팝업으로 덮어쓰지 않는다.
에이전트의 승인 및 샌드박스 제약도 별개다. 전역 PATH 변경은 업데이트·설치 스크립트에도 영향을 주므로
우선 에이전트 실행 환경에만 적용해 검증하는 편이 범위를 관리하기 쉽다.

이 검토에서는 세션 환경이나 설치 설정을 변경하지 않았다.

출처: [sudo 의 askpass 선택 코드](https://github.com/sudo-project/sudo/blob/main/src/tgetpass.c),
[run0 공식 매뉴얼 소스](https://github.com/systemd/systemd/blob/main/man/run0.xml).
