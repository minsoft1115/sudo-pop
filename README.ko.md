# sudo-pop

[English](README.md) · **한국어**

Hyprland/Wayland 에서 `sudo` 비밀번호를 GUI 팝업으로 입력받는 단일 Rust 바이너리.
비밀번호 프롬프트만 터미널 밖으로 나간다.

![sudo-pop 창. 맨 윗줄에 pacman -Syu, 그 아래 sudo 가 넘긴 프롬프트,
마스킹된 입력 필드](screenshots/sudo-pop.png)

> **보안 도구가 아니라 편의 도구다.** 프롬프트가 뜨는 위치를 바꿀 뿐 비밀번호가
> 더 안전해지지는 않는다 — [한계](#한계) 참조.

---

## 무엇을 보장하는가

| 항목 | 보장 내용 |
|---|---|
| 터미널 | stdin/stdout/stderr 가 그대로 명령에 닿는다. `pacman` 의 `[Y/n]` 도, `vim` 의 전체 화면 편집도 평소와 똑같다 |
| 코어 덤프 | 크래시해도 비밀번호가 디스크에 남지 않는다 |
| 스왑 | 버퍼를 RAM 에 고정해 스왑·하이버네이션 이미지로 나가지 않는다 |
| 화면 공유 | 공유·녹화 스트림에 창이 잡히지 않는다 |
| 로그 | 로그·명령행·환경변수 어디에도 남지 않는다 |

전부 추정이 아니라 측정한 것이다 — 터미널은 `docs/rationale.md` §2, 하드닝은 §6,
화면 공유 차단은 §10 에 있다.

스크린샷 도구도 화면 공유와 같은 프로토콜을 쓰므로 창은 검은 사각형으로 찍힌다.
위 사진은 `~/.config/minsoft1115/hypr/sudo-pop.lua` 에서 `no_screen_share` 를
잠시 끄고 찍은 것이다.

---

## 요구사항

| 항목 | 비고 |
|---|---|
| Hyprland | 0.56 이상. 윈도우 룰이 Lua 설정 기준 |
| sudo | askpass(`-A`) 지원. 1.9.17 에서 확인 |
| Rust | 빌드용. `mise.toml` 이 툴체인을 고정한다 |

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

`--init` 은 멱등하고, 다음 세 곳만 건드린다.

| 대상 | 내용 |
|---|---|
| `~/.config/minsoft1115/bash/sudo-pop.sh` | `alias sudo='sudo-pop'` |
| `~/.config/minsoft1115/hypr/sudo-pop.lua` | 팝업 창 규칙 |
| `~/.config/hypr/hyprland.lua` | 위 파일을 `require` 하는 마커 블록 |

## 제거

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
```

직접 지우려면 **이 순서로**:

```bash
sudo-pop --uninit
rm ~/.local/bin/sudo-pop
```

**순서가 중요하다.** 바이너리를 먼저 지우면 alias 가 가리킬 대상이 없어진다.
이미 그렇게 됐다면 `--uninstall` 이 파일을 직접 지워 처리한다.

다른 도구와 공유하는 스니펫 로더 블록, 그리고 이미 열려 있는 셸의 alias 는
남는다 (`unalias sudo` 하거나 새 셸을 연다).

---

## 알아둘 것

### 창 사용법

| 키 | 동작 |
|---|---|
| Enter | 제출. 입력이 비어 있으면 무시하고 창을 유지한다 |
| Esc | 취소 |
| (그대로 두면) | 90초 후 스스로 취소 |

맨 윗줄은 실행될 명령이다. 예상 못 한 명령이면 눈에 띈다. 알아내지 못하는 경우
(`sudo -v` 등)에는 그 줄을 생략한다. 창 문구는 영문이다.

### Omarchy 테마를 따라간다

색은 현재 테마에서, 폰트는 `fc-match monospace` 에서 가져온다. 팝업은 매번 새
프로세스라 둘 중 무엇을 바꾸든 **다음 팝업부터 바로 반영된다** — 리로드가 없다.
Omarchy 가 아니거나 팔레트를 읽지 못하면 기본값으로 돌아간다.

### 별칭은 대화형 셸에서만 확장된다

셸 스크립트, `sh -c`, Makefile 레시피, `xargs sudo`, systemd 유닛, cron 에서는
**원래 sudo 가 실행된다.** sudo-pop 은 사람이 직접 치는 경우만 바꾼다.

### GUI 를 못 띄우면 터미널로 물러난다

다음 중 하나라도 해당하면 팝업을 건너뛰고 평범한 터미널 프롬프트를 쓴다.

- `WAYLAND_DISPLAY` 와 `DISPLAY` 가 둘 다 없을 때 (SSH, 콘솔 TTY)
- 인자에 이미 `-n` / `-S` / `-A` 가 있을 때
- `XDG_RUNTIME_DIR` 이 없거나 준비에 실패했을 때

**SSH 로 접속해도 sudo 가 잠기지 않는다.**

### 잠기기 전에 창이 알려준다

PAM 은 sudo 인증 실패를 세고, 일정 수가 쌓이면 계정을 잠근다. sudo-pop 이 있든
없든 마찬가지다. sudo-pop 이 더하는 것은 그 상황을 보여주는 것과 상한이다.

- **남은 시도가 3회 이하로 떨어지면 창이 알려준다**
- **`sudo` 명령 1회당 팝업은 최대 3회** — sudo 혼자서는 `passwd_tries` 만큼
  재시도하므로 한 명령이 예산을 다 쓸 수 있다
- **이미 잠긴 상태면 그 사실을 알려준다.** 통하지 않을 비밀번호를 묻지 않는다

기준값은 각자의 PAM 설정에서 온다. `deny` 와 `unlock_time` 을 실행 시점에 읽는다.
**정상 인증을 한 번 하면 기록이 초기화된다.**

> [!NOTE]
> **ESC 도 시도 1회로 친다.** 팝업이 뜬 시점에는 이미 인증 대화가 시작된
> 상태라, 그 안에서 취소하는 것도 다른 시도와 똑같이 기록된다.

```bash
faillock --user "$USER"     # Valid 열이 V 인 항목만 집계 대상이다
```

### sudo 가 안 될 때

별칭이 남은 채로 바이너리를 지우면 `sudo` 가 "command not found" 가 된다.
절대 경로는 언제나 통한다.

```bash
/usr/bin/sudo whoami
```

절대 경로는 별칭도, 셸 함수도, PATH 선점도 우회한다. `\sudo` 와 `command sudo`
는 그렇지 못하다. 바이너리를 지우기 전에 `--uninit` 을 실행하면 이 상황 자체가
생기지 않는다.

### 한계

**그대로인 것.** 사용자 권한 악성코드는 alias 도, 바이너리도, `SUDO_ASKPASS` 도
바꿀 수 있다. 하지만 그건 이미 `sudo` 를 alias 로 잡거나 PATH 를 선점하거나 셸
함수로 가짜 프롬프트를 찍을 수 있었다는 뜻이다. 여기서 새로 열리는 경로는 없다.

**나빠지는 것 하나 — 피싱.** 터미널 프롬프트는 적어도 방금 명령을 친 자리에
뜨는데 팝업은 그것을 포기하고, 똑같이 생긴 창은 아무 권한 없이 그릴 수 있다.
맨 윗줄의 명령 표시가 그에 대한 대응이다 — 단서이지 보증은 아니다.

---

## 문제 해결

**팝업이 안 뜨고 터미널에서 물어본다**
물러나는 조건 중 하나에 걸렸다. `SUDO_POP_DEBUG=1 sudo true` 가 어느 조건인지
stderr 로 알려준다.

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

| 파일 | 내용 |
|---|---|
| `docs/architecture.html` | 구조·흐름 다이어그램 |
| `docs/plan.md` | 구현 사양 |
| `docs/rationale.md` | 설계 근거. 각 결정을 정한 실측이 함께 있다 |

세 문서는 작업 기록이다. 쓰는 데 필요한 것은 이 README 에 다 있다.

---

## 개발

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`SUDO_POP_DEBUG=1` 은 폴백 판단, 하드닝 결과, 재시도 카운터를 stderr 로 알려준다.
stdout 은 건드리지 않으므로 켜 둬도 안전하다.

---

## 라이선스

MIT. [LICENSE](LICENSE) 참조.
