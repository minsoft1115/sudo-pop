# sudo → run0 번역기 — 구현 계획

> **보류됨 (2026-08-19).** [`polkit-agent.md`](../../docs/polkit-agent.md) 방향으로 간다. 이 안은 하드닝을
> 통째로 내주는 거래였는데, 코드를 다 읽고 보니 그 대가로 얻는 것이 작았다 — Omarchy
> 다이얼로그가 우리보다 확실히 나은 것은 레이어셸 서피스 하나뿐이고, 나머지(문구 변환·지문·
> 실패 피드백)는 합쳐서 100줄 남짓이다. 번역 아이디어 자체는 `-E` 가 필요 없는 사람에게
> 여전히 쓸모가 있어 문서는 남긴다.
>
> **이 문서의 역할**: 아직 구현되지 않은 기능의 계획이다. 확정된 사양은
> [`plan.md`](plan.md), 설계 근거와 실측은 [`rationale.md`](rationale.md) 에 있다.
>
> 이 방향을 고르면서 [polkit 에이전트 계획](../../docs/polkit-agent.md)은 **보류**했다. 왜 그랬는지는 §1.
>
> 실측은 전부 2026-08-19, Omarchy 4.0 / systemd 261 / polkit 127 기준이다.

---

## 0. 무엇을 만드는가

`sudo <명령>` 을 **`run0 <명령>` 으로 번역해서 실행한다.** 그러면 인증 창을 우리가 그리지
않는다 — polkit 이 물어보고, 이 환경에서는 **Omarchy 셸의 polkit 다이얼로그**가 뜬다.

번역할 수 없는 것만 지금의 sudo 경로(`sudo -A` + 우리 창)로 남는다.

```
sudo pacman -Syu        → run0 pacman -Syu              → Omarchy polkit 창
sudo -E make install    → run0 --setenv=… make install  → Omarchy polkit 창
sudo -v                 → 번역 불가 → 지금의 sudo -A 경로 → sudo-pop 창
```

---

## 1. 왜 이 방향인가

원래 목표는 "sudo 를 칠 때 팝업이 뜨는 것" 이었고, 그건 이미 **두 갈래로 갈려 있었다** —
sudo 는 sudo-pop 창, polkit 은 Omarchy 창. 이 둘을 하나로 합치는 방법이 두 가지였다.

| | |
|---|---|
| **에이전트를 만든다** ([polkit-agent.md](../../docs/polkit-agent.md)) | polkit 쪽을 우리 창으로 끌어온다. D-Bus·헬퍼 프로토콜·데몬 ≈ 1000줄+ |
| **번역기를 만든다** (이 문서) | sudo 쪽을 polkit 으로 밀어 넣는다. 기존 파서를 재사용하는 수백 줄 |

번역기를 고른 이유는 **얻는 것이 더 크고 비용이 더 작아서**다.

- Omarchy 다이얼로그는 **레이어셸 Overlay + 배타적 키보드 포커스**로 뜬다
  (`PolkitAgent.qml:221-228`). eframe/winit 은 일반 xdg_toplevel 이라 Hyprland 창 규칙으로
  근사할 수밖에 없다. **창의 품질은 그쪽이 확실히 낫다**
- 지문(`pam_fprintd`)과 덮개 감지, `action_id` 를 사람이 읽는 문구로 바꾸는 처리가
  이미 있다. 우리가 만들면 전부 새로 써야 한다
- polkit 프로토콜은 아직 움직인다 (소켓 헬퍼가 최근에 생겼다). 남이 따라가 준다

### 포기하는 것 — 명확히 적어 둔다

**하드닝이 polkit 경로에는 안 걸린다.** Omarchy 다이얼로그는 비밀번호를 QML 문자열로 받아
**바까지 그리는 장수 프로세스(quickshell)의 GC 힙**에 둔다. zeroize 도, mlock 도, 코어덤프
차단도 없다. `omarchy-polkit` 레이어 네임스페이스에 **화면 공유 제외 규칙도 없다**
(Omarchy 는 1Password·Bitwarden 창에만 걸어 뒀다).

즉 이 방향은 **창 품질과 유지보수를 얻고 메모리 위생을 내주는 거래**다. 그 위생이 다시
중요해지면 [polkit-agent.md](../../docs/polkit-agent.md) 가 그대로 남아 있다.

화면 공유 제외는 Omarchy 쪽에 layerrule 한 줄이면 해결된다. **업스트림에 올릴 가치가 있다.**

---

## 2. 확인된 사실

| | |
|---|---|
| `run0` | systemd 261. `--setenv`·`-u`·`-g`·`-i`·`-D`·`--via-shell`·`--no-ask-password`·`--background` |
| `--setenv=NAME` (값 생략) | **호출한 쪽의 값을 물려받는다** (man run0) — `-E` 흉내의 근거 |
| 기본 환경 | *"the session will inherit the system environment from the service manager"* — 호출자 환경이 아니다 |
| 기본 작업 디렉터리 | root 로 전환할 때는 **호출자의 cwd 유지**, 다른 사용자면 그 사용자의 홈 |
| 종료 코드 | 명령의 종료 코드가 그대로 돌아온다 |
| 배경 | 기본으로 **붉게 틴트**된다 (root 기준). `--background=` 로 끌 수 있다 |
| polkit 액션 | `org.freedesktop.systemd1.manage-units`, `allow_active = auth_admin_keep` → **인증이 잠시 캐시된다** |
| 이 환경의 에이전트 | `omarchy.polkit` (`enabled=true`), 셸 안의 서비스라 프로세스 목록엔 안 보인다 |
| faillock | `/usr/lib/pam.d/polkit-1` 이 `system-auth` 를 include → `/run/faillock/<user>` **한 파일에 공유**. 실기록에 `SVC polkit-1` 행 확인 |
| polkit 은 비밀번호를 안 준다 | 그래서 `SUDO_ASKPASS` 로 Omarchy 창을 부르는 것은 **불가능**하다. 번역 말고 길이 없다 |

---

## 3. 번역 규칙

판정과 번역은 **`sudo_args.rs` 가 한다.** 옵션이 값을 따로 받는지, `--` 가 어디서 끝나는지를
아는 코드가 그것뿐이고, 셸 alias 나 함수로 같은 판정을 흉내 내면 반드시 어긋난다.

### 3-1. 그대로 대응되는 것

| sudo | run0 |
|---|---|
| `-u USER` / `--user` | `-u USER` |
| `-g GROUP` / `--group` | `-g GROUP` |
| `-i` | `-i` |
| `-D DIR` / `--chdir` | `-D DIR` |
| `-n` / `--non-interactive` | `--no-ask-password` |
| `VAR=값 <명령>` | `--setenv=VAR=값 <명령>` |
| `-- <명령>` | `-- <명령>` |

`-s` 는 `--via-shell` 로 보내되 **같지 않다** — run0 은 항상 로그인 셸 의미로 띄운다.
문서에 적고, 애매하면 sudo 경로로 남기는 쪽을 택한다.

### 3-2. `-E` — 목록으로 번역한다

`run0` 에 "환경을 통째로 넘기기" 는 없다. 대신 `--setenv=NAME` 이 호출자 값을 물려받으므로,
**넘길 이름의 목록**을 만든다. sudoers 의 `env_keep` 이 하던 일과 같다.

기본 목록 (터미널·에이전트·표시 관련만, 값이 아니라 이름만 넘긴다):

```
TERM  COLORTERM  DISPLAY  WAYLAND_DISPLAY  XAUTHORITY
SSH_AUTH_SOCK  SSH_CONNECTION
LANG  LC_ALL  LC_CTYPE
EDITOR  VISUAL  PAGER
```

- `SUDO_POP_KEEP_ENV` 로 **더하거나 뺄 수 있게** 한다 (`+NAME`, `-NAME`)
- 목록에 없는 변수는 안 넘어간다. **`-E` 를 완전히 재현하지 않는다** — 그 사실을 문서에 적는다
- `PATH` 는 넘기지 않는다. root 로 실행되는 명령의 `PATH` 를 사용자 값으로 바꾸는 것은
  이 도구가 할 일이 아니다

### 3-3. 번역하지 않고 sudo 로 남기는 것

| sudo | 왜 |
|---|---|
| `-v` / `-k` / `-K` | polkit 에 타임스탬프를 미리 갱신하거나 지우는 개념이 없다 |
| `-l` / `-ll` | 권한 목록 조회. 대응 없음 |
| `-b` | 백그라운드 실행. run0 은 어차피 유닛으로 뜬다 |
| `-S` / `-A` / `-p` | 비밀번호 입력 경로 자체를 지정하는 옵션들 |
| `-h` / `-H` / `-P` / `-C` / `-r` / `-t` / `-T` | 대응 없음 |
| 해석 실패 | 모르는 옵션이 하나라도 있으면 **전부 sudo 로 보낸다** |

**모르면 sudo 로 보낸다** 가 원칙이다. 잘못 번역해서 조용히 다르게 실행되는 것보다,
번역을 포기하고 원래 도구로 보내는 쪽이 언제나 낫다.

---

## 4. 구조

```
sudo-pop <args>
 └─ sudo_args::parse
     ├─ 번역 가능 → run0 인자 조립 → exec run0        → polkit → Omarchy 창
     └─ 번역 불가 → 지금 그대로 SUDO_ASKPASS + exec sudo -A → sudo-pop 창
```

- `exec()` 로 갈아탄다. 지금 래퍼가 그렇게 하고 있고 (`rationale.md` §5), 종료 코드·시그널·
  stdin/stdout 이 그대로 유지된다
- **두 경로 다 창을 우리가 미리 띄우지 않는다.** NOPASSWD 든 polkit 캐시든, 물어볼지 말지는
  아래쪽이 정한다 (`rationale.md` §8 의 결론이 그대로 적용된다)
- `run0` 이 없거나(systemd 256 미만) exec 에 실패하면 **sudo 경로로 폴백**한다

새 파일은 하나면 된다.

```
src/run0.rs    번역 규칙, 인자 조립, 환경 목록
```

`wrapper.rs` 에 갈림길 한 줄, `sudo_args.rs` 에 "이 옵션이 번역 가능한가" 를 답하는 함수가
붙는다. **askpass 모드·하드닝·창 코드는 손대지 않는다.**

---

## 5. 되돌릴 수 있게 한다

의미가 바뀌는 변경이므로 스위치를 둔다.

| | |
|---|---|
| `SUDO_POP_RUN0=0` | 번역을 끄고 전부 지금의 sudo 경로로 |
| `SUDO_POP_KEEP_ENV` | `-E` 목록 조정 (`+NAME` / `-NAME`) |
| `SUDO_POP_RUN0_BACKGROUND` | run0 의 배경 틴트. 비우면 `--background=` 로 끈다 |

`command sudo …` 는 지금처럼 언제나 진짜 sudo 다.

---

## 6. 사용자에게 달라지는 것 — README 에 반드시 적는다

번역은 **의미까지 같게 만들지 못한다.** 아래는 규칙으로 못 막는 차이다.

| | |
|---|---|
| `sudoers` 의 `NOPASSWD` | **안 먹는다.** 인증 여부는 polkit 정책이 정한다 |
| `env_keep` | 안 먹는다. §3-2 의 목록만 넘어간다 |
| 인증 캐시 | polkit 의 `auth_admin_keep` (짧다). sudo 의 15분 타임스탬프와 별개 |
| 잡 컨트롤 | transient 유닛이라 **셸의 자식이 아니다.** `&`·Ctrl-Z·`$!`·`wait` 가 다르게 논다 |
| 로그 | 실행이 systemd 저널에 남는다 |
| 화면 | 배경이 붉게 틴트된다 (끌 수 있다) |
| **faillock** | polkit 에서 틀린 것이 **sudo 도 잠근다.** 카운터가 한 파일에 공유된다 |

마지막 줄은 `plan.md` §4-4 의 대응이 **번역 경로에도 유효하다**는 뜻이다. 다만 이제 시도를
세는 주체가 우리가 아니라 Omarchy 다이얼로그이므로, 우리 `attempts` 게이팅은 sudo 경로에만
남는다. 번역 경로에서 몇 번까지 틀릴 수 있는지는 §8 에서 실측한다.

---

## 7. 단계

1. **번역 표.** `sudo_args.rs` 에 "번역 가능/불가" 판정과 run0 인자 조립을 붙이고
   **단위 테스트로 굳힌다.** 실행은 아직 안 한다 — `--dry-run` 으로 조립된 명령줄만 찍는다
2. **exec 연결.** 번역 가능한 경우를 실제로 run0 로 보낸다. 폴백(run0 없음·exec 실패) 포함
3. **`-E` 목록.** 기본 목록과 `SUDO_POP_KEEP_ENV` 확장
4. **문서.** README 의 "알아둘 것" 에 §6 표를 넣는다. `plan.md`·`rationale.md` 갱신
5. **정리.** 번역이 자리를 잡으면 sudo 경로에 남는 것이 무엇인지 다시 본다

---

## 8. 검증 체크리스트

```bash
sudo pacman -Q                 # 조회 — 번역되어 Omarchy 창이 뜨는가
sudo -E env | grep SSH_AUTH    # 목록에 있는 변수가 넘어가는가
sudo -v                        # 번역 불가 → sudo-pop 창이 뜨는가
sudo -u lmh whoami             # -u 가 그대로 옮겨지는가
sudo -- ls -l                  # -- 뒤가 명령으로 붙는가
sudo FOO=1 env | grep FOO      # 환경 할당이 --setenv 로 가는가
SUDO_POP_RUN0=0 sudo pacman -Q # 스위치가 도는가
```

- `vim`·`pacman -Syu` 같은 **대화형 명령에서 터미널이 그대로 붙는가** (이 프로젝트의 존재 이유)
- 종료 코드가 그대로 돌아오는가 (`sudo false; echo $?`)
- 파이프가 사는가 (`echo x | sudo tee /tmp/x`, `sudo ls | head`)
- Ctrl-C 가 명령에 닿는가
- **오답 3번 뒤 `faillock` 카운터가 얼마나 올라갔는지** 실측 (§6 마지막 줄)
- `run0` 이 없는 환경(또는 `PATH` 에서 가린 상태)에서 sudo 로 폴백하는가

---

## 9. 하지 않는 것

- **polkit 정책을 쓰지 않는다.** `NOPASSWD` 를 흉내 내려고 `.rules` 를 까는 것은 권한
  경계를 바꾸는 일이고, 이 도구가 할 일이 아니다
- `-E` 를 완전히 재현하려 들지 않는다. 목록 방식의 한계를 문서에 적는 것으로 끝낸다
- Omarchy 다이얼로그를 고치지 않는다. 필요하면 업스트림에 올린다
  (화면 공유 제외 layerrule 이 첫 후보다)

---

## 10. 보류한 것

[polkit-agent.md](../../docs/polkit-agent.md) — polkit 프롬프트까지 우리 창으로 가져오는 계획.
조사와 프로토콜 검증은 끝나 있고, 다시 꺼낼 조건은 둘이다.

1. **메모리 위생을 polkit 경로까지 넓혀야 할 이유가 생길 때**
2. **레이어셸 문제를 풀 방법이 생길 때** — eframe/winit 으로는 레이어셸 창을 못 띄운다.
   그대로 만들면 지금보다 나쁜 창으로 좋은 창을 교체하게 된다
