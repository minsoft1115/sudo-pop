# sudo-pop

[English](README.md) · **한국어**

Hyprland/Wayland 에서 `sudo` 비밀번호를 GUI 팝업으로 입력받는 단일 Rust 바이너리.

비밀번호 수집만 별도 프로세스로 떼어내기 때문에 터미널의 stdin/stdout/stderr 가
그대로 보존된다. `pacman` 의 `[Y/n]` 도, `vim` 의 전체 화면 편집도 평소와 똑같이 동작한다.

```
$ sudo pacman -Syu
   → 화면 가운데 비밀번호 창이 뜨고, 입력하면 pacman 이 평소처럼 이어서 실행된다
```

---

## 이 도구가 약속하는 것과 하지 않는 것

**이것은 보안 도구가 아니라 편의 도구다.**

사용자 권한으로 실행되는 악성코드는 alias 도, 바이너리도, `SUDO_ASKPASS` 환경변수도
전부 바꿀 수 있다. 방어선이 존재하지 않는다. 오히려 "비밀번호를 묻는 GUI 창" 을
일상화시키면 똑같이 생긴 가짜 창을 아무 권한 없이 만들 수 있으므로 피싱은 쉬워진다.

sudo-pop 이 실제로 지키는 것은 **부주의로 인한 유출** 뿐이다:

- 크래시해도 비밀번호가 코어 덤프로 디스크에 남지 않는다
- 비밀번호 버퍼를 RAM 에 고정해 스왑·하이버네이션 이미지로 나가지 않게 한다
- 화면 공유·녹화에 창이 잡히지 않는다
- 로그·명령행·환경변수 어디에도 비밀번호가 남지 않는다

---

## 요구사항

| 항목 | 비고 |
|---|---|
| Hyprland | 0.56 이상에서 확인. 윈도우 룰이 Lua 설정 기준 |
| sudo | askpass(`-A`) 지원. 1.9.17 에서 확인 |
| Rust | 빌드용. `mise` 를 쓰면 `mise.toml` 이 툴체인을 고정한다 |

Wayland·OpenGL 라이브러리는 데스크톱 환경이면 대개 이미 있다
(`wayland`, `libxkbcommon`, `mesa`, `libglvnd`).

---

## 설치

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
```

소스를 받아 빌드한 뒤 `~/.local/bin` 에 설치하고 `sudo-pop --init` 까지 실행한다.
쓰는 것이 전부 `$HOME` 아래이므로 **root 로 실행하지 말 것** — 시도하면 거부한다.
다시 실행하면 그대로 업그레이드된다.

`cargo` 가 필요하다(또는 `mise`. `mise.toml` 의 고정된 툴체인을 쓴다). C 링커도
있어야 한다. 미리 빌드된 바이너리는 제공하지 않는다.

옵션은 `-s --` 뒤에 플래그로 주거나 환경변수로 준다:

| 플래그 | 환경변수 | 기본값 |
|---|---|---|
| `--prefix DIR` | `SUDO_POP_PREFIX` | `~/.local/bin` |
| `--ref REF` | `SUDO_POP_REF` | `main` |
| `--no-init` | `SUDO_POP_NO_INIT=1` | `--init` 실행 |

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh \
  | bash -s -- --prefix ~/bin --no-init
```

체크아웃 안에서 `./install.sh` 를 실행하면 내려받지 않고 그 체크아웃을 빌드한다.
이때 `--ref` 는 무시된다.

<details>
<summary>수동 설치</summary>

```bash
# 1. 빌드
cargo build --release

# 2. PATH 에 놓기 (별칭이 해결되려면 필요하다)
install -Dm755 target/release/sudo-pop ~/.local/bin/sudo-pop

# 3. 셸 별칭 + Hyprland 윈도우 룰 등록
sudo-pop --init

# 4. 새 셸을 열거나
source ~/.bashrc
```

</details>

`--init` 이 하는 일:

| 대상 | 내용 |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | 팝업 창 규칙 (float, center, dim_around, 화면공유 제외) |
| `~/.config/hypr/hyprland.lua` | 위 파일을 `require` 하는 마커 블록 |

여러 번 실행해도 중복 추가되지 않는다. 스니펫 로더 블록이 이미 있으면
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

어느 쪽이든 설치한 파일과 `hyprland.lua` 의 마커 블록, 런타임 심볼릭 링크를
지운다. 스니펫 로더 블록은 다른 도구와 공유하는 것이라 남겨둔다. 이미 열려 있는
셸의 alias 는 `unalias sudo` 하거나 새 셸을 열 때까지 남는다.

**순서가 중요하다.** 바이너리를 먼저 지우면 alias 가 가리킬 대상이 없어져
`sudo` 가 "command not found" 가 된다. 이미 그렇게 됐다면 `--uninstall` 이
처리한다 — 물어볼 바이너리가 없으면 파일을 직접 지운다.

---

## 알아둘 것

### 탈출구 — 먼저 기억해 둘 것

별칭만 남고 바이너리가 사라지면 `sudo` 가 "command not found" 가 된다.
그때는 다음 중 아무거나 쓰면 원래 sudo 가 실행된다.

```bash
command sudo whoami   # 별칭도 함수도 무시
/usr/bin/sudo whoami
\sudo whoami          # 별칭만 무시한다 — 아래 주의
```

`--uninit` 을 실행하기 전에 바이너리를 지우지 않는 편이 안전하다.

**`\sudo` 는 확실한 탈출구가 아니다.** 백슬래시는 별칭 확장만 막고 **셸 함수는 못 막는다.**
같은 스니펫 폴더를 쓰는 다른 도구가 `sudo` 를 함수로 잡아 둘 수 있다 — omarchy-setup 의
패키지 가드(`zz-pkg-guards.sh`)가 그렇고, 그건 이 별칭을 로드 시점에 넘겨받아 자기 안에서
호출한다. 그런 셸에서 `\sudo` 는 그 함수로 들어간다. 어느 경우에도 원래 sudo 로 가려면
`command sudo` 나 절대 경로를 쓴다.

### 별칭은 대화형 셸에서만 확장된다

셸 별칭의 성질이라 어쩔 수 없다. 아래에서는 **원래 sudo 가 그대로 실행**된다.

- 셸 스크립트, `sh -c "..."`
- Makefile 의 레시피
- `xargs sudo ...`
- systemd 유닛, cron

즉 sudo-pop 은 사람이 직접 명령을 치는 상황만 바꾼다. 자동화된 경로는 건드리지 않는다.

### GUI 를 못 띄우는 상황에서는 알아서 비켜난다

다음 중 하나라도 해당하면 팝업을 포기하고 평범한 터미널 프롬프트로 넘어간다.

- `WAYLAND_DISPLAY` 와 `DISPLAY` 가 둘 다 없을 때 (SSH 접속, 콘솔 TTY)
- 인자에 이미 `-n` / `-S` / `-A` 가 있을 때
- `XDG_RUNTIME_DIR` 이 없거나 준비에 실패했을 때

**SSH 로 접속해도 sudo 가 잠기지 않는다.**

### faillock — 취소도 실패 1건으로 집계된다

이 부분은 sudo-pop 이 아니라 PAM 의 동작이다. askpass 가 실행되는 시점에는
이미 인증 대화가 시작된 상태라서, 그 안에서 취소하면 실패 1건으로 기록된다.

```
Esc 로 취소       → 실패 +1
비밀번호 오입력   → 실패 +1
```

기본 설정에서는 **15분 안에 10건이 쌓이면 120초 동안 계정이 잠긴다**
(`/etc/security/faillock.conf` 의 `deny`, `/etc/pam.d/system-auth` 의 `unlock_time`).

sudo-pop 은 이를 세 가지로 완화한다.

- 한 번의 `sudo` 명령에서 팝업은 **최대 3회**까지만 뜬다. sudo 자체는 10회를
  허용하지만 그대로 두면 한 명령이 예산을 다 쓸 수 있다.
- 남은 실패 예산이 **3 이하로 떨어지면 창에 경고**를 띄운다.
- 이미 잠긴 상태면 비밀번호를 묻지 않고 안내만 띄운다. 헛시도로 예산을 더
  깎지 않기 위해서다.

**정상 인증을 한 번 하면 기록이 초기화된다.** 실패가 쌓였다면 비밀번호를
정확히 한 번 입력하는 것으로 정리된다. 잠긴 동안은 120초를 기다리면 풀린다.

현재 상태는 언제든 확인할 수 있다.

```bash
faillock --user "$USER"    # Valid 열이 V 인 항목만 집계 대상이다
```

### 창 사용법

| 키 | 동작 |
|---|---|
| Enter | 제출. 비어 있으면 무시하고 창을 유지한다 |
| Esc | 취소 |
| (방치) | 90초 후 자동 취소 |

창은 위에서부터 이렇게 구성된다.

```
      pacman -Syu               ← 실행될 명령 (테마 강조색)
  [sudo] password for you:      ← sudo 가 준 프롬프트 (흐리게)
  ┌────────────────────────┐
  └────────────────────────┘
   Enter to confirm  Esc to cancel
```

**맨 윗줄이 지금 무엇을 실행하려는지 알려준다.** 예상치 못한 명령이 비밀번호를
요구하면 여기서 눈에 띈다. 명령을 알아낼 수 없는 경우(`sudo -v` 등)에는 그 줄만
생략된다.

비밀번호는 256자까지 입력된다. 창의 문구는 영문이다 — 한글 글리프를 넣으려면
CJK 폰트를 로드해야 하는데, 팝업이 즉시 뜨는 것이 이 도구의 핵심이라
시작 시간을 내주지 않았다. 화면에서 가장 중요한 문구는 어차피 sudo 가 그대로
넘겨주는 프롬프트다.

### 색과 폰트는 데스크톱을 따라간다

**색상**은 현재 Omarchy 테마에서 가져온다
(`~/.local/state/omarchy/current/theme/colors.toml`). 배경, 입력 필드, 강조색,
경고색이 모두 테마 팔레트에서 나온다.

**폰트**는 `fc-match monospace` 가 알려주는 것을 쓴다. `omarchy-font-set` 으로
바꾼 폰트가 그대로 적용된다는 뜻이고, Omarchy 가 아닌 환경에서도 시스템 기본
고정폭 폰트를 따라간다.

팝업은 매번 새 프로세스라 **테마나 폰트를 바꾸면 다음 팝업부터 바로 반영된다.**
재시작도 리로드도 필요 없다. 어느 쪽이든 읽지 못하면 조용히 기본값으로 돌아간다.

폰트 크기만은 직접 정해 두었다. Omarchy 에 전역 크기 설정이 없고, 터미널 기준
크기는 이 창에 맞지 않는다.

### 스크린샷으로는 창을 볼 수 없다

화면 공유 차단(`no_screen_share`)이 걸려 있어서, `grim` 같은 스크린샷 도구도
같은 프로토콜을 쓰는 탓에 창이 **검은 사각형으로만 찍힌다.** 버그가 아니라
의도된 동작이며, 비밀번호 창이 녹화·공유에 새지 않는다는 증거다.

외형을 캡처해야 한다면 `~/.config/minsoft1115/hypr/sudo-pop.lua` 에서
`no_screen_share` 를 잠시 끄고 `hyprctl reload` 한 뒤, 반드시 되돌린다.

---

## 문제 해결

**팝업이 안 뜨고 터미널에서 물어본다**
GUI 를 포기하는 조건 중 하나에 걸렸다. 원인을 보려면:

```bash
SUDO_POP_DEBUG=1 sudo true
```

어느 관문에서 비켜났는지 stderr 에 나온다. 이 변수는 stdout 을 절대 건드리지 않으므로
켜둔 채로 써도 안전하다.

**창이 화면 구석에 뜨거나 배경이 안 어두워진다**
Hyprland 룰이 적용되지 않았다. `~/.config/hypr/hyprland.lua` 에
`-- sudo-pop:begin` 블록이 있는지 확인하고 `hyprctl reload` 를 실행한다.

**`sudo: command not found`**
바이너리가 PATH 에 없다. 위의 탈출구로 `command sudo` 를 쓰고, `~/.local/bin` 이
PATH 에 있는지 확인한다.

**비밀번호가 맞는데 자꾸 실패한다**
계정이 잠겼을 수 있다. `faillock --user "$USER"` 로 확인하고 120초 기다린다.

---

## 문서

| 파일 | 내용 |
|---|---|
| `docs/architecture.html` | 구조와 흐름 다이어그램 |
| `docs/plan.md` | 구현 사양 |
| `docs/rationale.md` | 설계 근거와 실측 기록 |

`sudo -A` 를 쓰는 이유, `/tmp` 대신 `$XDG_RUNTIME_DIR` 심볼릭 링크를 쓰는 이유,
`panic = "abort"` 를 유지하면서 코어 덤프를 막는 방법 같은 결정의 근거는
전부 `docs/rationale.md` 에 실측과 함께 남아 있다.

---

## 개발

```bash
cargo test           # 단위 테스트
cargo clippy --all-targets
cargo fmt
```

`SUDO_POP_DEBUG=1` 을 켜면 폴백 판정, 하드닝 적용 결과, 재시도 카운터가
stderr 로 출력된다.
