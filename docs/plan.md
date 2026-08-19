# sudo-pop 구현 사양

> **이 문서의 역할**: 구현할 내용만 담는다. 전부 명령형으로 쓰여 있고,
> 여기 적힌 것은 전부 "만들어야 할 것"이다.
>
> - 설계 근거, 실측 데이터, 검토했으나 채택하지 않은 대안 → `rationale.md`
> - 이 문서가 사양의 유일한 출처다. 최초 요구사항 문서는 삭제했고,
>   그것과의 차이는 `rationale.md` §1 에 이력으로 남아 있다.

---

## 0. 무엇을 만드는가

Hyprland/Wayland 환경에서 `sudo` 비밀번호를 네이티브 GUI 팝업으로 입력받는
단일 Rust 바이너리. 비밀번호 수집만 별도 프로세스로 분리하므로
`pacman`, `vim` 같은 대화형 명령의 터미널 stdin/stdout/stderr가 그대로 보존된다.

**포지셔닝**: 이것은 보안 경계가 아니라 **편의 도구**다. 사용자 권한 악성코드는
alias·바이너리·`SUDO_ASKPASS`를 전부 바꿀 수 있다. README와 커밋 메시지에서
"보안 강화"라고 쓰지 말 것. 보안 조치들(§4)은 "등급 상승"이 아니라
**"부주의로 인한 비밀번호 유출 방지"**로 기술한다.

---

## 1. 금지 사항 — 구현 전 반드시 읽을 것

아래는 전부 **하면 안 되는 것**이다. 근거는 `rationale.md` 참조.

| 하지 말 것 | 대신 할 것 |
|---|---|
| `/tmp` 에 askpass 셸 스크립트 생성 | `$XDG_RUNTIME_DIR/sudo-pop/askpass` 심볼릭 링크 (§3-2) |
| `hyprctl keyword windowrulev2 ...` | Lua 설정에 정적 등록 (§3-1). Hyprland 0.56에서 `keyword` 는 윈도우 룰을 거부한다 |
| `sudo -n true` / `sudo -n -v` 프리체크 | 없음. 무조건 `sudo -A` (§3-2). 캐시만 보므로 NOPASSWD 규칙을 판정하지 못한다 |
| 팝업을 sudo 호출 **이전**에 띄우는 구조 | `sudo -A` 안에서 띄운다. NOPASSWD 규칙 때문에 비밀번호 필요 여부를 사전에 판정할 수 없다 (`rationale.md` §8) |
| `sudo -S` + stdin 으로 비밀번호 파이프 | `sudo -A` (askpass) |
| `Command::spawn()` + `wait()` | `CommandExt::exec()` (§3-2) |
| `SUDO_ASKPASS` 에 인자 붙이기 | 불가능. sudo 는 값을 경로 그대로 exec 한다 |
| `env::current_exe()` 로 askpass 모드 판별 | `env::args().next()` 의 basename (심볼릭 링크가 해석되어 버린다) |
| `println!` / `print!` 로 비밀번호 출력 | 격리해 둔 fd 에 `libc::write()` (§4-2) |
| `egui::Frame::new()` 로 창 바탕 그리기 | `Frame::central_panel(ui.style())`. eframe 0.36 의 `ui()` 는 배경 없는 `Ui` 를 준다 (§3-3) |
| 스크린샷으로 창 외형 검증 | 눈으로 확인. `no_screen_share` 때문에 캡처에는 검게 나온다 (§8) |
| 실패 경로에서 빈 줄(`"\n"`) 출력 | 무출력 + exit 1. 빈 줄은 오답으로 처리돼 10회 재시도를 유발한다 (§4-4) |
| `panic = "abort"` 만 켜고 하드닝 생략 | 하드닝 3종과 세트로만 (§4-1) |
| PATH 에 `sudo` 바이너리 심어 가로채기 | alias 만 사용 (§3-1) |
| `opt-level = "z"` | `opt-level = 3` (§5) |
| 윈도우 룰을 실행할 때마다 주입 | `--init` 에서 1회 등록. `hl.window_rule` 은 호출마다 누적된다 |

---

## 2. 아키텍처

```
$ sudo pacman -Syu
   |  alias sudo='sudo-pop'
   v
[ sudo-pop <args> ]   래퍼 모드
   |  1. 폴백 조건 검사 -> 해당하면 exec sudo <args> 로 끝
   |  2. $XDG_RUNTIME_DIR/sudo-pop/askpass 심볼릭 링크 보장
   |  3. SUDO_ASKPASS=<링크 경로>
   |  4. exec sudo -A <args>          <- 프로세스 대체
   v
[ sudo ]   터미널 fd 3종을 그대로 보유
   |  캐시가 유효하면 여기서 바로 명령 실행 (askpass 호출 없음)
   |  캐시 만료 시에만 v
   |     pipe() + fork() + dup2(pipe_w, 1)
   v
[ askpass 모드 ]   argv[0] basename == "askpass"
   |  1. 프로세스 하드닝  (§4-1)
   |  2. stdout 격리      (§4-2)
   |  3. eframe 팝업, argv[1] 프롬프트 표시
   |  4. 격리 fd 에 raw write -> 즉시 zeroize -> exit 0
   |     ESC / 타임아웃 -> 무출력 exit 1
   v
[ sudo ]   파이프 첫 줄로 PAM 인증 -> 명령 실행 (터미널 그대로)
```

---

## 3. 실행 모드 사양

바이너리는 세 모드로 동작한다. 판별 순서:

```
1. argv[0] 의 basename 이 "askpass"        -> askpass 모드 (§3-3)
2. argv[1] 이 "--init" / "--uninit"        -> 설치 모드 (§3-1)
3. 그 외                                   -> 래퍼 모드  (§3-2)
```

### 3-1. 설치 모드 (`--init` / `--uninit`)

기존 저장소 규약(`setup-korean.sh`, `install-workspaces-widget.sh`)과 동일한
마커 블록 방식을 따른다. **모든 쓰기는 멱등이어야 한다** — 마커 존재 여부를
먼저 검사하고, 두 번 실행해도 중복 추가하지 않는다.

**(a) 셸 별칭**

`~/.config/minsoft1115/bash/sudo-pop.sh` 를 생성한다:

```sh
alias sudo='sudo-pop'
```

`~/.bashrc` 에는 이 디렉터리의 `*.sh` 를 전부 source 하는 블록이 이미 있다.
따라서 **블록이 존재하면 `.bashrc` 를 건드리지 않는다.** 확인할 마커:

```
# minsoft1115-bash:begin
```

블록이 없으면(다른 머신) 그때만 마커 블록째로 `.bashrc` 에 추가한다.
zsh 는 위 루프가 없으므로 `~/.zshrc` 에 동일 마커 블록을 추가한다.
fish/nushell 은 미지원 — 감지 시 수동 설정 안내를 출력하고 종료한다.

**(b) Hyprland 윈도우 룰**

`~/.config/minsoft1115/hypr/sudo-pop.lua` 를 생성한다:

```lua
-- sudo-pop askpass popup
o.window("^(sudo-askpass)$", {
  float = true,
  center = true,
  size = { 400, 200 },
  dim_around = true,
  stay_focused = true,
  pin = true,
  no_screen_share = true,
})
```

속성명은 위 표기가 정확하다. `dimaround`(언더스코어 없음)는 Hyprland 가
`unknown field` 로 거부한다. `no_screen_share` 는 필수 — 화면 공유·녹화 중
비밀번호 창이 스트림에 나가는 것을 막는다.

`~/.config/hypr/hyprland.lua` 에 마커 블록을 추가한다:

```lua
-- sudo-pop:begin
require("minsoft1115.hypr.sudo-pop")
-- sudo-pop:end
```

마지막에 `hyprctl reload` 를 실행한다.

**(c) `--uninit`**

마커 블록 제거 + 생성한 파일 삭제 + `hyprctl reload`.
바이너리만 사라지고 alias 가 남으면 sudo 를 쓸 수 없게 되므로 제거 경로는 필수다.

### 3-2. 래퍼 모드 (`sudo-pop <args>`)

```
1. args 가 비어 있으면                     -> exec sudo            (usage 출력)
2. args 에 -n / -S / -A 가 이미 있으면     -> exec sudo <args>     (호출자 의도 존중)
3. WAYLAND_DISPLAY 와 DISPLAY 가 둘 다 없으면 -> exec sudo <args>  (SSH/TTY 폴백)
4. XDG_RUNTIME_DIR 이 비어 있으면          -> exec sudo <args>
5. ensure_askpass_symlink() 실패 시        -> exec sudo <args>
6. env SUDO_ASKPASS = <심볼릭 링크 절대경로>
7. exec sudo -A <args>
```

- **3번 폴백이 없으면 SSH 접속 시 락아웃된다.** 반드시 구현한다.
- 5번: GUI 를 못 띄우는 것이 sudo 자체를 못 쓰는 것보다 낫다. 실패는 항상 폴백.
- `-k`, `-K`, `-l`, `-v`, `-e`(sudoedit) 는 그대로 통과시키면 정상 동작한다.
- `exec()` 를 쓰면 종료 코드 전파(시그널 사망 128+N 포함), SIGINT 전달,
  프로세스 그룹, 잡 컨트롤, TTY 소유권이 전부 자동으로 해결된다.

**심볼릭 링크 관리 (`ensure_askpass_symlink`)**

```
$XDG_RUNTIME_DIR/sudo-pop/            디렉터리, 모드 0700 으로 생성
$XDG_RUNTIME_DIR/sudo-pop/askpass  ->  (symlink)  실제 sudo-pop 바이너리 절대경로
```

- 디렉터리는 반드시 `0700`.
- 링크가 이미 존재하면 **타깃을 검증한 뒤 재사용**한다. 무조건 unlink 후
  재생성하지 않는다.
- 타깃이 다르면 교체한다.
- 심볼릭 링크는 exec 권한을 타깃에서 평가하므로 `noexec` 마운트 영향을 받지 않는다.

### 3-3. askpass 모드

sudo 가 이 프로세스를 다음과 같이 호출한다 (실측 확인됨):

- `argv[1]` = 프롬프트 문자열 (예: `[sudo] password for <user>: `). 인자는 이것 하나뿐.
- `fd 1` = sudo 로 연결된 **익명 파이프**. 여기 쓴 첫 줄이 비밀번호가 된다.
- `fd 2` = 상속됨(터미널). 로그·경고는 여기로 내보내도 안전하다.

실행 순서:

```
1. 프로세스 하드닝                 (§4-1) — 다른 무엇보다 먼저
2. stdout 격리                     (§4-2)
3. 재시도 가드 검사                (§4-4b) — 3회 초과면 무출력 exit 1
4. faillock 잔여 예산 조회         (§4-4c) — 잠김 상태면 안내 후 무출력 exit 1
5. prompt = argv[1]  (없으면 "Password:")
6. eframe 팝업 실행 (블로킹)
7. 결과 분기
   - Enter, 입력이 비어 있음  -> 제출하지 않고 팝업 유지 (§4-4d)
   - Enter, 입력 있음         -> 격리 fd 에 write, 즉시 zeroize,
                                 attempts 파일 삭제, exit 0
   - ESC                      -> 무출력, exit 1
   - 90초 타임아웃            -> 무출력, exit 1
   - 그 외 모든 오류          -> 무출력, exit 1   (절대 빈 줄을 쓰지 않는다)
```

**GUI 요구사항**

- Wayland app-id 를 정확히 `"sudo-askpass"` 로 설정
  (`ViewportBuilder::with_app_id`). Hyprland 가 이 값으로 §3-1 룰을 매칭한다.
- 장식 없음(`with_decorations(false)`), 400x200.
- `argv[1]` 프롬프트를 라벨로 표시한다. PAM 다단계(2FA, 지문)에서
  "Verification code:" 같은 질문이 올 수 있으므로 원문을 그대로 보여준다.
- 비밀번호 입력은 `TextEdit::password(true)` 로 마스킹. 입력은 **256자로 제한**한다
  (버퍼 재할당을 막아 `mlock` 한 페이지를 벗어나지 않게 하기 위해, §4-3).
- 창이 뜨는 즉시 입력 필드에 포커스.
- ESC 경로는 어떤 상태에서도 살아 있어야 한다.

**창 구성** — 위에서부터:

| 요소 | 내용 | 스타일 |
|---|---|---|
| 명령 | 실행될 명령 (§3-4) | 11.5px, 모노스페이스, **테마 accent 색** |
| 프롬프트 | `argv[1]` 원문 | 12.5px, **불투명도 0.5** |
| 입력 필드 | 마스킹 | 테두리 accent |
| 힌트 / 경고 | 조작 안내, 또는 faillock 경고(§4-4c) | 11px |

명령을 알아내지 못하면 그 줄만 생략한다. 추측해서 표시하지 않는다.

**바탕 그리기** — eframe 0.36 의 `App::ui()` 가 넘겨주는 `Ui` 에는 **배경도 여백도
없다.** `egui::Frame::central_panel(ui.style())` 로 명시적으로 칠하지 않으면 창이
빈 화면으로 보인다.

**색과 폰트는 데스크톱을 따른다** (§3-5).

### 3-4. 실행될 명령 표시

sudo 는 askpass 에 프롬프트만 넘긴다. 명령은 **부모 프로세스에서 읽는다** —
sudo 가 우리를 포크하므로 부모의 명령행이 `sudo -A <명령>` 이고,
`/proc/<pid>/cmdline` 은 setuid 프로세스여도 world-readable 이다.

```
/proc/<getppid()>/cmdline  →  "sudo\0-A\0pacman\0-Syu\0"
```

- argv[0] 의 basename 이 `sudo` 가 아니면 표시하지 않는다.
- sudo 자신의 옵션을 건너뛰고 명령 시작점을 찾는 데 `sudo_args::command_start`
  를 쓴다. 래퍼의 `-n/-S/-A` 감지와 같은 옵션 표를 공유한다.
- 120자를 넘으면 말줄임한다.

**목적은 정보 제공이자 약한 안티피싱이다.** 예상치 못한 명령이 비밀번호를
요구할 때 알아챌 수 있는 유일한 단서다. polkit 다이얼로그가 액션 설명을
보여주는 것과 같은 취지.

### 3-5. 데스크톱 색상·폰트 연동

askpass 는 매 프롬프트마다 새 프로세스이므로 **매번 다시 읽는다.**
테마나 폰트를 바꾸면 다음 팝업부터 반영되고, 리로드 로직이 필요 없다.

**색상** — Omarchy 팔레트를 직접 파싱한다:

```
$XDG_STATE_HOME/omarchy/current/theme/colors.toml   (기본 $HOME/.local/state)
```

`key = "#rrggbb"` 줄만 훑으면 되므로 `toml` 크레이트가 필요 없다.
`omarchy-theme-color` 를 호출하지 않는 이유는 프로세스 하나 값이 아까워서다.

| colors.toml 키 | egui |
|---|---|
| `mode` | `Visuals::dark()` / `light()` |
| `background` | `panel_fill`, `window_fill` |
| `lighter_background` → `selection` → `dark_background` | `extreme_bg_color` (입력 필드) |
| `foreground` | 본문 텍스트 |
| `bright_foreground` | 강조 텍스트 |
| `accent` | `text_cursor`, 활성 테두리, **`hyperlink_color`** (명령 줄이 읽어 씀) |
| `red` → `orange` → `yellow` | `warn_fg_color` |

`background` 를 못 읽으면 팔레트를 아예 적용하지 않는다. 반쯤 칠해진 창보다 낫다.

**폰트** — `fc-match monospace -f '%{family}\n%{file}'` 로 조회한다.
Omarchy 의 `omarchy-font-set` 이 fontconfig 규칙을 쓰고
`omarchy-font-current` 가 문자 그대로 `fc-match monospace` 이므로, fontconfig 에
묻는 것이 곧 Omarchy 에 묻는 것이고 Omarchy 가 아닌 환경에서도 동작한다.

- Proportional·Monospace **두 패밀리 모두**에 앞세운다. 터미널 폰트이고
  프롬프트도 터미널에서 온 문자열이다.
- egui 기본 폰트를 폴백 체인에 남긴다. 없는 글리프가 빈칸이 되지 않도록.
- **8MB 를 넘으면 건너뛴다.** monospace 가 CJK 폰트로 지정되면 수십 MB 라
  기동이 무너진다. 실측상 JetBrainsMono Nerd Font(2.5MB)는 기동 시간에
  영향이 없었다(58ms → 약 50ms).

**폰트 크기는 연동하지 않는다.** Omarchy 에 전역 크기 설정이 없고, 터미널의
`size = 9` 는 400x200 팝업에 맞지 않는다.

---

## 4. 필수 보안 조치

### 4-1. 프로세스 하드닝 — askpass 진입 최초 지점

비밀번호가 메모리에 들어오기 전에, GUI 초기화 이전에 실행한다.
**이 세 줄과 `panic = "abort"` 는 세트다. 하나라도 빠지면 안 된다.**

```rust
// SAFETY: called at process start, before any other thread exists.
unsafe {
    libc::prctl(libc::PR_SET_DUMPABLE, 0);   // 코어덤프 핸들러 무력화 + 동일 uid ptrace 차단
    let lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    libc::setrlimit(libc::RLIMIT_CORE, &lim);
}
```

- 순서를 지킨다. `PR_SET_DUMPABLE=0` 이 가장 강력하다.
- 이 조치는 GUI 프로세스에만 적용된다. `pacman` 본체에는 아무 제약이 걸리지 않는다.

**스왑 차단은 `mlockall` 이 아니라 비밀번호 버퍼 단위로 한다.**

이 머신의 `RLIMIT_MEMLOCK` 은 8 MB 인데, eframe 이 매핑된 뒤의 주소 공간은 그보다
훨씬 크다. `mlockall(MCL_CURRENT | MCL_FUTURE)` 은 ENOMEM 으로 실패하며
**아무것도 보호하지 못한다**(실측 확인).

대신 `Secret` 이 자기 버퍼만 `mlock` 한다. 한 페이지면 충분하므로 항상 성공한다.

```rust
let buffer = String::with_capacity(2048);
libc::mlock(buffer.as_ptr().cast(), 2048);   // Drop 에서 munlock
```

- 재할당이 일어나면 잠긴 페이지가 아닌 곳으로 옮겨가므로, 입력 위젯에서
  **글자 수를 256 자로 제한**해 용량을 넘길 수 없게 한다(4바이트/글자 최대 1024바이트).
- 검증: 창이 열린 동안 `grep VmLck /proc/<pid>/status` → `4 kB`

### 4-2. stdout 격리

sudo 는 fd 1 의 첫 줄을 비밀번호로 읽는다. 라이브러리 경고나 디버그 출력이
섞이면 그것이 비밀번호로 전송된다.

```
1. 시작 시 fd 1 을 dup() 해서 보관              -> pw_fd
2. fd 1 을 /dev/null 로 덮어씀 (dup2)
3. 이후 프로그램 전체에서 stdout 사용 금지. 모든 로그는 stderr
4. 종료 시 pw_fd 에만 libc::write() 로 직접 기록 (비밀번호 + "\n")
```

`println!` 을 쓰지 않는 이유는 `BufWriter` 가 비밀번호 복사본을 힙에 남기기 때문이기도 하다.

### 4-3. zeroize

`Zeroize` / `ZeroizeOnDrop` 을 사용하되, **`Drop` 에 의존하지 않는다.**
`panic = "abort"` 에서는 `Drop` 이 호출되지 않는다.

- 비밀번호 버퍼는 `String::with_capacity(2048)` 로 시작하고 입력을 256자로 제한한다.
  재할당되면 기존 버퍼가 zeroize 없이 free 되고, `mlock` 해 둔 페이지에서도 벗어난다.
- **파이프에 write 한 직후 정상 경로에서 명시적으로 zeroize** 한다.
  GUI 종료 처리 같은 느린 구간에 비밀번호가 살아 있는 시간을 없애는 것이 목적이다.
- egui 내부 복사본까지는 지울 수 없다. 이 한계를 알고 설계한다.

### 4-4. faillock 대응 — 이 시스템에서 가장 위험한 실패 모드

측정된 이 머신의 설정:

```
/etc/sudoers           : Defaults passwd_tries=10
/etc/pam.d/system-auth : pam_faillock.so deny=10 unlock_time=120
                         (fail_interval 미지정 -> 기본 900초)
```

| 파라미터 | 값 | 의미 |
|---|---|---|
| `fail_interval` | 900초 (기본값) | 이 창 안에 누적된 실패를 센다 |
| `deny` | 10 | 누적이 10이면 잠근다 |
| `unlock_time` | 120초 | 잠긴 뒤 이만큼 지나면 풀린다 |
| `passwd_tries` | 10 | sudo 가 askpass 를 재호출하는 최대 횟수 |

**즉 15분 창 안에 실패 10건이면 잠긴다.** 창이 넓으므로 소비를 아끼는 것이 중요하다.

비용의 단위는 "오입력 횟수"가 아니라 **sudo 명령 1회의 재시도 시퀀스 전체**다.

| 상황 | 팝업 | faillock 소비 |
|---|---|---|
| 1번 틀리고 맞춤 | 2회 | 1건 |
| 3번 틀리고 맞춤 | 4회 | 3건 |
| ESC 취소 (무출력) | 1회 | **1건** — 0 으로 만들 수 없다 (아래 참조) |
| 사람이 10번 연속 틀림 | 10회 | **10건 → 잠김** |
| **askpass 가 사람 개입 없이 틀린 값을 반환** | 10회 (약 20초) | **10건 → 잠김** |

사람이 조작하는 앞의 세 줄은 터미널 sudo 와 동일하다. 문제는 **마지막 줄**이다.
askpass 방식은 사람이 타이핑하지 않아도 sudo 가 자동으로 재호출하므로,
구현 실수 하나가 20초 만에 한도를 소진시킨다. 가장 현실적인 경로:

```
ESC → 실수로 write(fd, "\n") → sudo: 빈 비밀번호 = 오답 → 팝업 재출현
    → 사용자가 "왜 안 닫히지?" 하며 ESC 연타 → 20초 후 계정 잠김
```

무출력으로 종료했다면 첫 ESC 에서 sudo 가 포기했을 상황이다.

**취소 비용을 0 으로 만들 수는 없다.** askpass 가 실행되는 시점에는 sudo 가 이미
PAM 인증 대화 중이므로(저널: `pam_unix(sudo:auth): conversation failed`),
그 안에서의 취소는 정의상 인증 실패 1건이다. 1건은 "공짜"가 아니라 **달성 가능한
최선**이며, (a) 를 어기면 같은 취소가 최대 10건이 된다.

아래 네 가지를 모두 구현한다.

**(a) 실패 경로에서 절대 빈 줄을 쓰지 않는다 — 가장 중요**

| askpass 의 행동 | sudo 의 해석 | 소비되는 시도 |
|---|---|---|
| 아무것도 쓰지 않고 exit | "no password was provided" → **즉시 포기** | **1회** |
| 빈 줄(`"\n"`)을 씀 | 빈 비밀번호 → 오답 → **10회까지 재시도** | **최대 10회** |

취소·타임아웃·오류·빈 입력 등 **모든 실패 경로는 무출력 + exit 1** 이어야 한다.
실수로 `write(fd, "\n")` 하는 순간 한 번의 취소가 계정을 잠근다.

**(b) 자체 재시도 제한 — sudo 의 10회를 3회로 줄인다**

askpass 는 자신이 연속 몇 번째 호출인지 알 수 없으므로 상태를 직접 남긴다.

```
$XDG_RUNTIME_DIR/sudo-pop/attempts     ← "<unix_ts> <count>" 한 줄
```

- **래퍼가 sudo 를 exec 하기 직전에 카운터를 초기화한다.** 허용량은 "분당" 이 아니라
  **sudo 명령 1회당** 3회다. sudo 의 재시도는 같은 명령 안에서 일어나므로 이렇게 하면
  의도한 단위가 정확히 맞는다.
- **카운트는 "팝업이 뜬 횟수" 가 아니라 "비밀번호를 제출한 횟수" 다.** 취소는 세지
  않는다 — 취소하면 sudo 가 즉시 포기하므로 애초에 반복될 수 없고, 취소를 세면
  다음 sudo 명령이 부당하게 막힌다.
- 마지막 기록이 **60초를 넘으면 무시**한다. 래퍼를 거치지 않고 sudo 가 호출된
  경우(수동 `SUDO_ASKPASS` 등)를 위한 안전장치다.
- 카운트가 **3 이상**이면 팝업을 띄우지 않고 즉시 무출력 exit 1 한다.
  → sudo 가 포기하므로 faillock 소비가 3에서 멈춘다.
  사람이 틀리든 버그로 자동 반복되든 **sudo 명령 1회의 상한이 10 → 3 으로 내려간다.**
- 트레이드오프: 비밀번호를 3번 틀리면 sudo 가 종료되므로 명령을 다시 입력해야 한다.
  `deny=10` 이므로 3 이면 잠기기 전에 sudo 명령을 3회 실행할 여유가 남는다.
  5 로 올리면 두 번만에 한도에 닿으므로 3 을 택한다.
- 파일은 0600. 비밀번호는 절대 기록하지 않는다.

**(c) 남은 실패 예산을 GUI 에 표시한다**

`faillock --user <id>` 는 **root 권한 없이** 자기 계정 기록을 읽을 수 있다.
askpass 시작 시 이를 파싱해 `남은 예산 = deny - 유효_실패건수` 를 계산한다.

| 남은 예산 | GUI 동작 |
|---|---|
| 4 이상 | 표시하지 않음 (평소와 동일) |
| **3 이하** | 팝업에 경고 표시 — 예: "실패 시 잠김까지 2회 남음" |
| 0 (한도 도달) | 팝업을 띄우지 않고 "계정이 잠겨 있습니다. 약 N초 후 해제" 안내 후 무출력 exit 1 |

**파싱 시 주의 — `Valid` 열을 반드시 확인한다.**

```
$ faillock --user <user>
When                Type  Source     Valid
2026-08-19 09:30:00 SVC   sudo       I     <- fail_interval 밖. 세지 않는다
2026-08-19 09:47:11 SVC   sudo       V     <- 유효. 이것만 센다
```

`fail_interval`(기본 900초)을 지난 기록은 목록에 **남아 있지만** `Valid` 열이
`I` 로 바뀌며 잠금 판정에서 제외된다. 전체 행 수를 세면 이미 만료된 기록까지
포함해 남은 예산을 과소 계산하게 되므로, **`V` 인 행만 센다.**

`deny` 값은 `/etc/security/faillock.conf` 또는 `/etc/pam.d/*` 에서 읽되,
읽지 못하면 안전하게 기본값 3(가장 보수적인 흔한 설정)이 아니라 **경고 표시를
생략**한다. 잘못된 숫자를 보여주는 것보다 아무것도 안 보여주는 편이 낫다.

**(d) 빈 비밀번호 제출 차단**

입력이 비어 있으면 Enter 를 무시한다(팝업 유지). (a) 와 함께 동작해야 한다.

**해결되지 않는 잔여 위험**: 사용자가 실제로 비밀번호를 틀리는 경우는 faillock 이
정상 동작하는 것이므로 막지 않는다. (b) 에 의해 한 세션당 최대 3건으로 제한된다.

## 5. Cargo.toml

```toml
[profile.release]
strip = true
lto = true
codegen-units = 1
panic = "abort"       # §4-1 하드닝과 반드시 세트
opt-level = 3
```

```toml
[dependencies]
eframe = { version = "0.36", default-features = false,
           features = ["glow", "wayland", "x11", "default_fonts"] }
libc = "0.2"
zeroize = "1"
```

- eframe 백엔드는 **`glow`** 를 선택한다(wgpu 보다 콜드 스타트가 빠르다).
  이 도구의 가치는 팝업이 즉시 뜨는 것이므로 시작 지연이 가장 중요한 지표다.
- 크레이트는 최소로 유지한다. 팔레트·폰트 조회는 파일 읽기와 `fc-match` 호출로
  해결되므로 `toml` 파서나 fontconfig 바인딩을 들이지 않는다.
- 실측 기동 시간: **약 50ms** (릴리스, 테마·폰트 적용 포함).

---

## 6. 파일 구조

```
sudo-pop/
├── Cargo.toml
├── mise.toml                  rust 툴체인 고정
├── install.sh                 curl | bash 설치 스크립트 (빌드 → 설치 → --init, --uninstall 로 역순 제거)
├── README.md                  영문
├── README.ko.md               한글
├── docs/
│   ├── plan.md                이 문서 (구현 사양)
│   ├── rationale.md           설계 근거·실측 기록
│   └── architecture.html      구조·흐름 다이어그램
├── src/
│   ├── main.rs                모드 판별 및 디스패치
│   ├── init.rs                --init / --uninit
│   ├── wrapper.rs             폴백 판정, 심볼릭 링크, exec
│   ├── sudo_args.rs           sudo 옵션 스캐너 (래퍼·askpass 공용)
│   ├── askpass/
│   │   ├── mod.rs             하드닝 -> GUI -> 출력 오케스트레이션
│   │   ├── harden.rs          PR_SET_DUMPABLE / RLIMIT_CORE
│   │   ├── secret.rs          zeroize 버퍼(+mlock), stdout 격리, raw write
│   │   ├── theme.rs           Omarchy 팔레트 → egui Visuals (§3-5)
│   │   ├── font.rs            fc-match → egui FontDefinitions (§3-5)
│   │   ├── invocation.rs      /proc/<ppid>/cmdline → 실행될 명령 (§3-4)
│   │   └── gui.rs             eframe 창
│   ├── paths.rs               XDG_RUNTIME_DIR, 설정 경로 규약
│   └── attempts.rs            재시도 가드 + faillock 조회 (§4-4)
└── assets/
    ├── sudo-pop.lua           --init 이 설치할 Hyprland 룰
    └── sudo-pop.sh            --init 이 설치할 셸 별칭
```

`gui.rs` 는 반드시 독립 모듈로 격리한다. 추후 GUI 백엔드 교체 가능성이 있다.

**언어 규약**: 코드·식별자·코드 주석은 영어. 문서(`README.md`, `docs/` 이하)는 한글.

---

## 7. 구현 순서

| 단계 | 내용 | 완료 기준 |
|---|---|---|
| 1 | 모드 판별 + 래퍼 모드 (`exec sudo -A`) + 폴백 5종 | GUI 없이 `sudo -A` 동작, SSH 폴백 확인 |
| 2 | 심볼릭 링크 관리 (`paths.rs`) | 링크 생성·재사용, 디렉터리 0700 검증 |
| 3 | askpass 하드닝 + stdout 격리 + 하드코딩 값 반환 스텁 | 팝업 없이 인증 성공 |
| 4 | eframe GUI (app-id, 마스킹, 프롬프트, Enter/ESC, 타임아웃) | 실제 팝업으로 인증 |
| 5 | `--init` / `--uninit` | 멱등성, 마커 블록, `hyprctl reload` |
| 6 | 재시도 가드 + faillock 조회 (§4-4b,c) | 오입력 시 팝업 3회에서 중단 확인 |
| 7 | 명시적 zeroize 마감 + 하드닝 검증 | 강제 패닉 후 코어덤프 미생성 확인 |
| 8 | README(한글) | §9 항목 전부 포함 |

---

## 8. 테스트 체크리스트

**터미널 무결성 — 이 도구의 존재 이유**
- [ ] `sudo pacman -Syu` — 진행률 표시, `[Y/n]` 프롬프트 정상
- [ ] `sudo vim /etc/fstab` — 전체 화면 편집, 종료 후 터미널 복구
- [ ] `sudo -i` / `sudo -s` — 대화형 루트 셸
- [ ] `sudo cat /etc/shadow | head -1` — 파이프 출력
- [ ] `echo hi | sudo tee /tmp/x` — stdin 파이프

**시그널·종료 코드**
- [ ] `sudo false` → 종료 코드 1
- [ ] `sudo sleep 100` 중 Ctrl-C → 즉시 종료
- [ ] `sudo sleep 100` 중 Ctrl-Z → 잡 서스펜드, `fg` 복귀

**폴백·경계**
- [ ] `env -u WAYLAND_DISPLAY -u DISPLAY sudo-pop true` → 터미널 프롬프트
- [ ] SSH 세션에서 `sudo true`
- [ ] `sudo-pop` (인자 없음) → sudo usage
- [ ] `sudo -n true` / `sudo -v` / `sudo -k` — 인자에 `-n` 등이 있으면 손대지 않고 통과 (§3-2 2번)
- [ ] `sudo -e /etc/hosts` (sudoedit)
- [ ] 바이너리 삭제 후 `\sudo true` 로 복구 가능

**GUI**
- [ ] ESC → 종료 코드 1, stdout 무출력
- [ ] 빈 비밀번호로 Enter → 제출되지 않음
- [ ] 비밀번호 3회 오입력 → 팝업 3회, 각 프롬프트 표시
- [ ] 90초 방치 → 타임아웃 종료
- [ ] 멀티모니터에서 포커스된 모니터 중앙에 표시
- [ ] `dim_around` / `stay_focused` / `pin` 적용 확인
- [ ] 창 배경·입력 필드·테두리가 현재 Omarchy 테마 색인지
- [ ] 폰트가 터미널과 같은지 (`fc-match monospace` 결과와 일치)
- [ ] 테마를 바꾼 뒤 다음 팝업에 바로 반영되는지
- [ ] 맨 윗줄에 실행될 명령이 accent 색으로 나오는지
- [ ] `sudo -v` 처럼 명령이 없으면 그 줄이 생략되는지
- [ ] 릴리스 기동 시간이 100ms 이내인지

**보안**
- [ ] askpass 프로세스에 `gdb -p` attach 실패
- [ ] `grep Dumpable /proc/<pid>/status` → `0`
- [ ] `grep VmLck /proc/<pid>/status` → 0 이 아님
- [ ] 강제 패닉 후 `coredumpctl list` 에 항목 미생성
- [ ] 화면 공유 중 팝업이 스트림에 안 잡힘
      (`grim` 도 같은 wlr-screencopy 를 쓰므로 **스크린샷으로는 창을 볼 수 없다**.
       외형을 눈으로 확인하려면 `no_screen_share` 를 잠시 꺼야 한다)
- [ ] ESC 취소 → faillock 정확히 +1 (10 이 아님)
- [ ] 틀린 비밀번호 → 팝업이 **3회까지만** 뜨고 중단 (§4-4b)
- [ ] 실패 경로에서 stdout 바이트 수 0 인지 (`sudo -A ... | wc -c`)
- [ ] faillock 한도 도달 상태에서 → 팝업 대신 안내 표시
- [ ] 남은 예산 3 이하일 때 경고 문구 표시 (§4-4c)
- [ ] `Valid=I` 인 만료 기록이 남은 예산 계산에 포함되지 않는지

**설치**
- [ ] `--init` 2회 실행 → 중복 추가 없음
- [ ] `--uninit` 후 마커 블록·파일 완전 제거

---

## 9. README 에 반드시 포함할 것

- **탈출구**: `\sudo` / `command sudo` / `/usr/bin/sudo`.
  바이너리가 사라지고 alias 만 남았을 때의 유일한 복구 경로.
- **alias 의 적용 범위**: 대화형 셸에서만 확장된다. 스크립트, `sh -c`,
  Makefile, `xargs sudo`, systemd 유닛에서는 원래 sudo 가 실행된다.
- **faillock**: ESC 취소도 실패 1건으로 집계된다(§4-4). 15분 창에 10건이면
  120초 잠김. 정상 인증 1회로 기록이 초기화된다는 점도 안내한다.
- **포지셔닝**: 보안 도구가 아니라 편의 도구라는 점 (§0).
