# sudo-pop

[English](README.md) · **한국어**

Hyprland/Wayland 에서 `sudo` 비밀번호를 GUI 팝업으로 입력받는 단일 Rust 바이너리.
프롬프트만 터미널 밖으로 나가므로 stdin/stdout/stderr 는 그대로 명령에 닿는다.
`pacman` 의 `[Y/n]` 도, `vim` 의 전체 화면 편집도 평소와 똑같이 동작한다.

![sudo-pop 창. 맨 윗줄에 pacman -Syu, 그 아래 sudo 가 넘긴 프롬프트,
마스킹된 입력 필드](screenshots/sudo-pop.png)

> **보안 도구가 아니라 편의 도구다.** 프롬프트가 뜨는 위치를 바꿀 뿐 비밀번호가
> 더 안전해지지는 않는다 —
> [무엇을 지키고 무엇을 지키지 않는가](#무엇을-지키고-무엇을-지키지-않는가) 참조.

---

## 요구사항

| 항목 | 비고 |
|---|---|
| Hyprland | 0.56 이상. 윈도우 룰이 Lua 설정 기준 |
| sudo | askpass(`-A`) 지원. 1.9.17 에서 확인 |
| Rust | 빌드용. `mise.toml` 이 툴체인을 고정한다 |

Wayland·OpenGL 라이브러리(`wayland`, `libxkbcommon`, `mesa`, `libglvnd`)는
데스크톱 환경이면 대개 이미 있다.

---

## 설치

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
```

소스를 받아 빌드하고 `~/.local/bin` 에 설치한 뒤 `--init` 까지 실행한다.
다시 실행하면 그대로 업그레이드된다. `cargo`(또는 `mise`)와 C 링커가 필요하고,
미리 빌드된 바이너리는 제공하지 않는다.

쓰는 것이 전부 `$HOME` 아래이므로 **root 로 실행하지 말 것** — 시도하면 거부한다.

| 플래그 | 환경변수 | 기본값 |
|---|---|---|
| `--prefix DIR` | `SUDO_POP_PREFIX` | `~/.local/bin` |
| `--ref REF` | `SUDO_POP_REF` | `main` |
| `--no-init` | `SUDO_POP_NO_INIT=1` | `--init` 실행 |

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh \
  | bash -s -- --prefix ~/bin --no-init
```

체크아웃 안에서 `./install.sh` 를 실행하면 그 체크아웃을 빌드한다. `--ref` 는 무시.

<details>
<summary>수동 설치</summary>

```bash
cargo build --release
install -Dm755 target/release/sudo-pop ~/.local/bin/sudo-pop   # 별칭이 해결되려면 PATH 에 있어야 한다
sudo-pop --init                                                # 별칭 + Hyprland 룰 등록
source ~/.bashrc                                               # 또는 새 셸을 연다
```

</details>

`--init` 이 하는 일:

| 대상 | 내용 |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | 팝업 창 규칙 (float, center, dim_around, 화면공유 제외) |
| `~/.config/hypr/hyprland.lua` | 위 파일을 `require` 하는 마커 블록 |

멱등하다 — 여러 번 실행해도 중복 추가되지 않고, 스니펫 로더 블록이 이미 있으면
`.bashrc` 는 아예 건드리지 않는다.

## 제거

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
```

직접 지우려면 **이 순서로**:

```bash
sudo-pop --uninit
rm ~/.local/bin/sudo-pop
```

**순서가 중요하다.** 바이너리를 먼저 지우면 alias 가 가리킬 대상이 없어져
`sudo` 가 "command not found" 가 된다. 이미 그렇게 됐다면 `--uninstall` 이
파일을 직접 지워 처리한다.

지우는 것: 설치한 파일, `hyprland.lua` 의 마커 블록, 런타임 심볼릭 링크.
남기는 것: 다른 도구와 공유하는 스니펫 로더 블록, 그리고 이미 열려 있는 셸의
alias (`unalias sudo` 하거나 새 셸을 연다).

---

## 알아둘 것

### 탈출구 — 먼저 기억해 둘 것

별칭만 남고 바이너리가 사라지면 `sudo` 가 "command not found" 가 된다.

```bash
/usr/bin/sudo whoami
```

절대 경로는 별칭도, 셸 함수도, PATH 선점도 전부 우회한다. `\sudo` 와
`command sudo` 는 그보다 약하다 — 백슬래시는 함수를 못 막고, `command` 는 여전히
PATH 를 탄다. 바이너리를 지우기 전에 `--uninit` 을 먼저 실행할 것.

### 별칭은 대화형 셸에서만 확장된다

셸 스크립트, `sh -c`, Makefile 레시피, `xargs sudo`, systemd 유닛, cron 에서는
**원래 sudo 가 실행된다.** sudo-pop 은 사람이 직접 치는 경우만 바꾼다.

### GUI 를 못 띄우면 알아서 비켜난다

다음 중 하나라도 해당하면 평범한 터미널 프롬프트로 넘어간다.

- `WAYLAND_DISPLAY` 와 `DISPLAY` 가 둘 다 없을 때 (SSH, 콘솔 TTY)
- 인자에 이미 `-n` / `-S` / `-A` 가 있을 때
- `XDG_RUNTIME_DIR` 이 없거나 준비에 실패했을 때

**SSH 로 접속해도 sudo 가 잠기지 않는다.**

### faillock — 취소도 실패 1건으로 집계된다

sudo-pop 이 아니라 PAM 의 동작이다. askpass 가 실행되는 시점에는 이미 인증
대화가 시작된 상태라, ESC 는 비밀번호를 틀린 것과 똑같이 1건을 소비한다.

| | PAM 기본값 | 이 머신 (Arch + Omarchy) |
|---|---|---|
| `deny` — 잠기기까지의 실패 건수 | 3 | 10 |
| `fail_interval` — 그 실패가 들어와야 할 창 | 900초 | 900초 |
| `unlock_time` — 잠김 지속 시간 | 600초 | 120초 |
| `passwd_tries` — sudo 자체 재시도 횟수 | 3 | 10 |

```bash
grep faillock /etc/pam.d/system-auth /etc/security/faillock.conf
sudo -l | grep passwd_tries
faillock --user "$USER"     # Valid 열이 V 인 항목만 집계 대상이다
```

`deny` 와 `unlock_time` 은 실행 시점에 읽으므로 각자의 설정을 따라간다.
그 위에서:

- **`sudo` 명령 1회당 팝업은 최대 3회.** sudo 는 askpass 를 `passwd_tries` 만큼
  스스로 재호출하는데, 그것만으로 한 명령이 예산을 다 쓸 수 있다. `passwd_tries`
  가 이미 3 이하면 이 상한은 아무것도 바꾸지 않는다.
- **남은 실패 예산이 3 이하면 창에 경고**를 띄운다.
- **이미 잠긴 상태면 팝업 자체를 띄우지 않는다** — 헛시도로 더 깎지 않기 위해서다.

**정상 인증을 한 번 하면 기록이 초기화된다.** 잠긴 동안은 `unlock_time` 만큼
기다리는 것 외에 방법이 없다.

### 창 사용법

| 키 | |
|---|---|
| Enter | 제출. 입력이 비어 있으면 무시하고 창을 유지한다 |
| Esc | 취소 |
| (그대로 두면) | 90초 후 스스로 취소 |

맨 윗줄은 실행될 명령이다. 예상 못 한 명령이면 눈에 띈다. 알아내지 못하는 경우
(`sudo -v` 등)에는 그 줄을 생략한다.

입력 필드는 256자까지 받는다. 창 문구가 영문인 것은 한글 글리프를 쓰려면 CJK
폰트를 로드해야 하는데, 즉시 뜨는 것이 이 도구의 존재 이유이기 때문이다.

### 색과 폰트는 데스크톱을 따른다

| | |
|---|---|
| 색상 | 현재 Omarchy 테마, `~/.local/state/omarchy/current/theme/colors.toml` |
| 폰트 | `fc-match monospace` 결과. `omarchy-font-set` 으로 바꾼 폰트가 그대로 적용된다 |
| 폰트 크기 | 여기서 고정. Omarchy 에 전역 크기 설정이 없고, 터미널 크기는 이 창에 안 맞는다 |

팝업은 매번 새 프로세스라 테마나 폰트를 바꾸면 **다음 팝업부터 바로 반영된다.**
읽지 못하면 조용히 기본값으로 돌아간다.

### 스크린샷으로는 창을 볼 수 없다

`no_screen_share` 가 걸려 있고 `grim` 같은 스크린샷 도구도 같은 프로토콜을 쓰기
때문에, 창은 **검은 사각형으로만 찍힌다.** 버그가 아니라 비밀번호 창이 녹화·공유에
새지 않는다는 증거다.

이 README 맨 위의 사진은 `~/.config/minsoft1115/hypr/sudo-pop.lua` 에서 그 룰을
잠시 끄고(`hyprctl reload` → 촬영 → 원복) 찍은 것이다.

### 무엇을 지키고 무엇을 지키지 않는가

| | |
|---|---|
| **그대로인 것** | 사용자 권한 악성코드는 alias 도, 바이너리도, `SUDO_ASKPASS` 도 바꿀 수 있다. 하지만 그건 이미 `sudo` 를 alias 로 잡거나 PATH 를 선점하거나 셸 함수로 가짜 프롬프트를 찍을 수 있었다는 뜻이다. 여기서 새로 열리는 경로는 없다. |
| **나빠지는 것 하나** | 피싱. 터미널 프롬프트는 적어도 방금 내가 명령을 친 자리에 뜨는데 팝업은 그것을 포기하고, 똑같이 생긴 창은 아무 권한 없이 그릴 수 있다. 맨 윗줄의 명령 표시가 그에 대한 대응이다 — 단서이지 보증은 아니다. |

실제로 지키는 것은 부주의로 인한 유출이고, 추정이 아니라 측정된 것이다.

- 코어 덤프에 비밀번호가 남지 않는다 — `PR_SET_DUMPABLE=0` + `RLIMIT_CORE=0`.
  릴리스 바이너리를 abort 시켜도 `coredumpctl` 에 항목이 생기지 않는다
- 버퍼를 RAM 에 고정해 스왑·하이버네이션 이미지로 나가지 않는다 — 창이 열려 있는
  동안 `VmLck` 이 0 이 아니다
- 화면 공유·녹화에 창이 잡히지 않는다 (바로 위 항목)
- 로그·명령행·환경변수 어디에도 남지 않는다 — sudo 가 읽는 파이프로 곧장 들어간다

---

## 문제 해결

**팝업이 안 뜨고 터미널에서 물어본다**
비켜나는 조건 중 하나에 걸렸다. `SUDO_POP_DEBUG=1 sudo true` 가 어느 조건인지
stderr 로 알려준다. stdout 은 건드리지 않으므로 켜 둬도 안전하다.

**창이 구석에 뜨거나 배경이 어두워지지 않는다**
Hyprland 룰이 적용되지 않았다. `~/.config/hypr/hyprland.lua` 에
`-- sudo-pop:begin` 블록이 있는지 확인하고 `hyprctl reload`.

**`sudo: command not found`**
바이너리가 PATH 에 없다. 그동안은 `/usr/bin/sudo` 를 쓴다.

**비밀번호가 맞는데 자꾸 실패한다**
계정이 잠겼을 수 있다. `faillock --user "$USER"` 로 확인하고 `unlock_time` 만큼
기다린다.

---

## 문서

| 파일 | |
|---|---|
| `docs/architecture.html` | 구조·흐름 다이어그램 |
| `docs/plan.md` | 구현 사양 |
| `docs/rationale.md` | 설계 근거. 각 결정을 정한 실측이 함께 있다 |

세 문서는 사용법이 아니라 작업 기록이다. 쓰는 데 필요한 것은 이 README 에 다 있다.

---

## 개발

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`SUDO_POP_DEBUG=1` 은 폴백 판단, 하드닝 결과, 재시도 카운터를 stderr 로 알려준다.

---

## 라이선스

MIT. [LICENSE](LICENSE) 참조.
