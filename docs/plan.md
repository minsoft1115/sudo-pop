# sudo-pop 구현 사양

> **이 문서의 역할**: 무엇을 만들었고 무엇을 지켜야 하는지만 담는다.
> 왜 그렇게 했는지, 무엇을 재 봤는지, 무엇을 기각했는지는
> [`rationale.md`](rationale.md) 에 있다.
>
> 대상 환경은 **Omarchy 4.0+ / Hyprland 0.56+** 하나뿐이다. 버전 분기와 폴백을 두지 않는다.

---

## 0. 무엇인가

**polkit 인증 에이전트**와, 그 앞에 선 **sudo 라우터**.

```
sudo pacman -Syu   →  run0 pacman -Syu  →  polkitd  →  우리 에이전트  →  창
sudo -E make       →  sudo -A make      →                              →  창
디스크 마운트·NetworkManager·systemctl  →  polkitd  →  우리 에이전트  →  창
```

시스템의 모든 권한 프롬프트가 한 창으로 모이고, 그 창은 비밀번호가 코어덤프·스왑·화면
공유로 새지 않도록 만들어져 있다.

**보안 경계가 아니라 편의 도구다.** 사용자 권한 악성코드는 alias·바이너리·유닛을 전부 바꿀
수 있다. 지키는 것은 **부주의로 인한 유출**이다.

---

## 1. 모드

한 바이너리, 다섯 모드. 무엇으로 불렸는지가 정한다.

| 조건 | 모드 | 하는 일 |
|---|---|---|
| `basename(argv[0]) == "askpass"` | askpass | sudo 가 심볼릭 링크로 부른다. 창을 띄우고 답을 fd 1 에 쓴다 |
| `--agent` | 에이전트 | polkit 에 등록하고 요청을 자식에게 넘긴다 (systemd user 유닛) |
| `--agent-prompt` | 자식 | 창 하나 + 헬퍼 대화. 종료 코드로만 말한다 |
| `--init` / `--uninit` | 설치 | 아래 §5 |
| 그 외 | 라우터 | `sudo <args>` 가 여기로 온다 |

askpass 판별은 `argv[0]` 으로 한다. `current_exe` 는 심볼릭 링크를 풀어 버린다.

---

## 2. 지켜야 할 것

구현이 흔들리면 안 되는 지점들이다. 하나하나가 실측이나 사고에서 나왔다.

### 2-1. 비밀번호가 있는 곳

- **데몬은 비밀번호를 보지 않는다.** 요청마다 자식을 띄우고 종료 코드만 받는다
- 자식은 요청 하나만 살고 죽는다. 하드닝은 그 안에서 최초로 실행한다
- 버퍼는 `mlock` 하고 `zeroize` 한다. `panic = "abort"` 라 `Drop` 에 기대지 않는다
- 답은 **원시 두 번 쓰기**로 나간다. `writeln!` 로 포맷하면 지워지지 않는 사본이 생긴다
- 쿠키는 **파이프로** 자식에게 넘긴다. argv 와 환경은 남이 읽는다

### 2-2. 요청을 어떻게 끝내는가

| 상황 | `BeginAuthentication` | 자식 종료 코드 |
|---|---|---|
| 성공 | 정상 리턴 | 0 |
| 사용자가 취소 | **정상 리턴** | 2 |
| 프롬프트 전에 거절됨 (잠긴 계정·깨진 PAM) | **정상 리턴** | 2 |
| 그 외 실패 | `Error.Failed` | 1 |

**취소와 "묻지도 못한 실패" 를 에러로 돌려주면 안 된다.** polkitd 가 요청을 다시 발행해서
빈 창이 무한히 다시 뜬다.

### 2-3. 발신자 검증

`BeginAuthentication` 은 시스템 버스에 열린 메서드다. **폴킷이 부른 것만 받는다** —
발신자가 `org.freedesktop.PolicyKit1` 의 현재 소유자인지 확인하고, 아니면 창을 띄우기 전에
`AccessDenied`. 소유자 이름은 `NameOwnerChanged` 로 따라간다.

### 2-4. 시도 횟수

`cookie` 하나가 sudo 명령 하나다. 자식이 쿠키당 하나 뜨고 재시도를 자기가 돌리므로
상한(`MAX_ATTEMPTS = 3`)은 자연히 쿠키 단위가 된다. 잠긴 계정이면 묻지 않는다.

faillock 카운터는 **sudo·polkit·로그인이 공유한다.** polkit 에서 틀린 것이 sudo 를 잠근다.

### 2-5. 헬퍼

소켓(`/run/polkit/agent-helper.socket`)을 먼저 시도하고, **fork+exec 폴백은 필수다** —
`SO_PEERPIDFD` 가 없는 커널에서 소켓 헬퍼는 묻지도 않고 닫힌다. "프롬프트를 한 번이라도
봤는가" 가 폴백 여부와 요청을 끝내는 방법을 함께 결정한다.

`AuthenticationAgentResponse2` 는 **우리가 부르지 않는다.** 헬퍼가 root 로 보낸다.

### 2-3-1. 소유자 추적

polkitd 의 고유 이름을 **읽기 전에 `NameOwnerChanged` 를 구독한다.** 순서가 뒤집히면 그 사이의
재시작을 놓쳐 죽은 이름으로 진짜 폴킷을 영영 거절한다 (`rationale.md` §17-2). 들고 있는 이름과
같은 신호로는 재등록하지 않는다.

등록 실패는 두 갈래다. `already exists for the given subject` 면 자리를 뺏긴 것이므로 **정상
종료**하고 (`Restart=on-failure` 폭주 방지), 그 밖의 실패는 재시작이 고칠 수 있으므로 **실패로
끝낸다.**

### 2-6. 창

- app-id 는 `sudo-askpass`. `assets/sudo-pop.lua` 의 규칙이 이 이름으로 매칭한다
- 규칙: `float`·`center`·`size 400 200`·`dim_around`·`stay_focused`·`pin`·**`no_screen_share`**
- 요청 하나에 창 하나. PAM 이 여러 번 물어도 창은 그대로 두고 글자만 바꾼다
- 이벤트 루프는 프로세스당 하나만 만들 수 있다. 창이 메인 스레드를 갖고 대화가 옆 스레드로 간다
- 자체 타임아웃 30초. **폴킷 호출자는 25초에 포기한다** — 그 뒤는 백스톱일 뿐이다
- 창에 띄우는 것은 polkit 의 `message` 가 아니라 **`polkit.subject-pid` 의 cmdline** 이다
- 테마 색은 `colors.toml` 에 더해 **`shell.toml` 의 `[polkit]` 섹션**을 읽어 시스템 창과 맞춘다 (실패색 `text-error` 포함)
- 폰트 체인은 Omarchy 의 monospace 면이 앞이고, **우리가 쓰지 않은 글자**(cmdline·polkit
  message·PAM 프롬프트)에 ASCII 밖 문자가 있을 때만 `fc-match :charset=` 으로 한 면을
  꼬리에 더한다. egui 는 글리프 폴백을 하지 않으므로 이것이 없으면 한글이 `◻` 가 된다
  (`rationale.md` §16). 8MB 상한은 **주 폰트에만** 건다

### 2-7. 라우팅

| 친 것 | 가는 곳 |
|---|---|
| `sudo <명령>` | `run0 <명령>` |
| `sudo -옵션 …` | `sudo -A` (우리 창) |
| `sudo VAR=값 <명령>` | `sudo -A` — 환경 할당은 옵션이 아니다. run0 으로 가면 변수가 사라진다 |
| `-A`·`-n`·`-S` 가 이미 있음 | 손대지 않고 sudo |
| 인자 없음 / 디스플레이 없음 / 런타임 디렉터리 없음 | sudo |

판정은 `sudo_args` 가 한다. 셸 alias 나 함수로 흉내 내지 않는다.
되돌릴 스위치는 `SUDO_POP_RUN0=0`.

---

## 3. 하지 말 것

| 하지 말 것 | 대신 |
|---|---|
| `hyprctl keyword windowrulev2 ...` | Lua 규칙 정적 설치. 0.56 은 `keyword` 로 오는 윈도우 룰을 거부한다 |
| 데몬이 자식에게서 비밀번호를 받아 헬퍼에 전달 | 자식이 헬퍼와 직접 말한다 |
| PAM 을 직접 호출해 인증하고 polkitd 에 알리기 | 불가능하다. root 가 부른 응답만 polkitd 가 받는다 |
| 취소를 D-Bus 에러로 리턴 | 정상 리턴 (§2-2) |
| `println!` 로 비밀번호 출력 | 격리한 fd 에 원시 쓰기 |
| `~/.bashrc` 에 alias 를 직접 덧붙이기 | 스니펫 + 공유 로더 블록 |
| polkit 정책(`.rules`) 작성 | 없음. 우리는 묻는 쪽이지 정하는 쪽이 아니다 |

---

## 4. 파일

```
src/main.rs        모드 분기, 세션 id, 등록/해제, NameOwnerChanged
src/agent.rs       D-Bus 서비스 — 발신자 검증, 신원 선택, 큐, 자식 실행
src/helper.rs      헬퍼 연결(소켓/fork)과 PAM 줄 프로토콜
src/prompt.rs      --agent-prompt — 하드닝 + 창 + 대화 + 종료 코드
src/askpass.rs     askpass 모드 — 같은 창, 목적지만 sudo 의 fd
src/wrapper.rs     라우터
src/gui.rs         창 (요청당 하나, 채널로 갱신)
src/init.rs        설치 모드
src/secret.rs      mlock + zeroize 버퍼, sudo 용 fd 격리
src/harden.rs · theme.rs · font.rs · invocation.rs · attempts.rs · paths.rs · sudo_args.rs
assets/sudo-pop.lua · assets/sudo-pop.sh
```

`src/lib.rs` 는 이것들을 시험에서 쓸 수 있게 내놓는다.

---

## 5. 설치

`--init` 이 쓰는 것. 전부 `$HOME` 안이고, 전부 마커 안이라 정확히 되돌릴 수 있다.

| 경로 | |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | 창 규칙 |
| `~/.config/hypr/hyprland.lua` | `-- sudo-pop:begin/end` 마커와 require 한 줄 |
| `~/.config/systemd/user/sudo-pop-agent.service` | `ExecStart=<절대경로> --agent`, `WantedBy=graphical-session.target` |

- **다른 폴킷 에이전트가 있으면 유닛을 깔되 enable 하지 않는다.** 감지는 셋이다 —
  ① `omarchy.polkit`(셸 안의 서비스라 프로세스 목록에 안 보인다) ② 우리 uid 의 `/proc/*/comm`
  ③ 활성 user 유닛. **이름은 표가 아니라 `polkit`/`policykit` 포함 여부로 본다** — comm 은
  커널이 15자에서 자르므로 정확한 이름 표는 긴 이름을 영영 놓친다 (`rationale.md` §17-1)
- 등록 충돌은 **실패가 아닌 정상 종료**로 끝낸다. `Restart=on-failure` 가 무한히 되살린다
- `--uninit` 은 우리 것만 지운다. 공유 로더 블록은 남긴다
- `ExecStart` 에는 `--init` 을 실행한 바이너리의 절대 경로가 박힌다

---

## 6. 시험

```
cargo test                            단위 + 통합. 환경 없이 돈다
tests/scenarios.sh                    polkitd·버스·컴포지터가 필요한 것들
tests/scenarios.sh --with-password    위 + 사람이 비밀번호를 넣는 한 케이스
tests/scenarios.sh --restart-polkitd  위 + polkitd 를 실제로 재시작한다 (비밀번호 1회)
cargo run --release --example font-cost   폰트 체인 비용 실측 (rationale §16-3)
```

`tests/fake-helper.sh` 가 헬퍼 행세를 하고, `SUDO_POP_HELPER_BIN`·`SUDO_POP_HELPER_SOCKET`
으로 두 문을 다른 곳에 건다. 시나리오는 Hyprland 를 시험 장비로 쓴다 —
`hl.dsp.send_shortcut` 으로 Esc 를 주입하고, `grim` 으로 화면 캡처 제외를 센다.

무엇을 잡는지는 [`rationale.md`](rationale.md) §15.

---

## 7. 아직 아닌 것

| | |
|---|---|
| 실패 피드백 (흔들림·색 플래시) | `Wrong` 한 줄로 대신하고 있다 |
| 지문 (`pam_fprintd`) | 이 머신에 `fprintd` 가 없다. PAM 파일은 `/etc` 와 `/usr/lib` 둘 다 봐야 한다 |
| 신원 선택 UI | 관리자가 여럿인 환경에서만 의미가 있다 |
| 레이어셸 서피스 | 전체화면 위 동작이 문제가 될 때 (`rationale.md` §2-4) |
| polkitd 재시작 재등록 | **실측 완료.** `tests/scenarios.sh --restart-polkitd` (비밀번호 1회) |
