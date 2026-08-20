# 에이전트 보안 비교 — omarchy.polkit · hyprpolkitagent · sudo-pop

> **이 문서의 역할**: Omarchy 4.0 셸에 내장된 polkit 에이전트(`omarchy.polkit`),
> Hyprland 진영의 참고 구현(`hyprpolkitagent`), 그리고 이 리포의 `sudo-pop` 을
> **보안 축으로만** 나란히 놓는다. sudo-pop 자체의 사양은 [`plan.md`](plan.md),
> 설계 근거는 [`rationale.md`](rationale.md) 에 있다.
>
> **근거의 성격을 가른다.** `omarchy.polkit` 과 `sudo-pop` 은 이 머신에서 코드를 읽고
> 실측한 기록([`rationale.md`](rationale.md) §2-1·§2-2·§3-5·§6-2)이다. `hyprpolkitagent` 는
> 이 머신에 **설치돼 있지 않다** — 아래 내용은 2026-08-20 에 main 브랜치 소스
> (`src/core/PolkitListener.cpp`, `src/ui/Dialog.cpp`)를 읽고 확인한 것이고, 동작 실측이
> 아니다. 판단이 섞인 줄은 **평가**라고 적었다.
>
> **그리고 이 문서는 sudo-pop 리포 안에 있다.** 자기 것을 재는 표라는 뜻이다. sudo-pop 열은
> 전부 이 리포의 문서·코드·실측에 근거를 달았으니, 의심스러우면 링크를 따라가서 확인하라.
>
> 참고: rationale.md §12 가 기록한 hyprpolkitagent 는 Qt/QML 시절의 것이다. 지금 main 은
> UI 를 자체 툴킷(hyprtoolkit)으로 다시 썼다. D-Bus·헬퍼 층(sdbus-c++)은 그대로다.

---

## 0. 세 에이전트가 무엇인가

**`omarchy.polkit`** — Omarchy 4.0 셸(Quickshell)의 플러그인.
`/usr/share/omarchy/shell/plugins/polkit/PolkitAgent.qml` 390줄 + `PolkitModel.js` 32줄.
`kinds: ["service"]`, `keepLoaded: true` — **별도 프로세스가 아니라 셸 프로세스 안에서 도는
서비스**다. D-Bus·헬퍼 프로토콜은 `Quickshell.Services.Polkit` 이 대신하고, QML 은 창만 그린다.
창은 레이어셸 Overlay 서피스에 배타 키보드 포커스다.

**`hyprpolkitagent`** — hyprwm 의 독립 에이전트. C++ 로 프로토콜(sdbus-c++)과 UI(hyprtoolkit)를
직접 구현한 **전용 프로세스**다. 창은 `appClass("hyprpolkitagent")` 의 일반 toplevel 창이라
컴포지터 창 규칙의 대상이 된다.

**`sudo-pop`** — 이 리포. Rust 로 프로토콜(zbus)과 UI(egui)를 직접 구현한 전용 데몬인데,
구조가 위 둘과 다르다 — 데몬은 D-Bus·큐·자식 수명만 맡고, **요청마다 짧게 사는 자식
프로세스**(`--agent-prompt`)가 하드닝을 걸고 창을 그리고 헬퍼와 말한다. 비밀번호는 데몬
주소 공간에 들어오지 않는다 ([`rationale.md`](rationale.md) §4). 창은 app-id `sudo-askpass`
의 일반 toplevel 창이고, `--init` 이 창 규칙(`dim_around`·`stay_focused`·`pin`·
`no_screen_share`)을 반드시 깐다.

셋은 같은 자리를 두고 경쟁한다 — polkit 은 **한 세션에 에이전트 하나**만 허용한다.

---

## 1. 기존 두 에이전트가 똑같이 갖고 있는 구멍 — 그리고 sudo-pop 의 자리

비교보다 먼저 적을 것이 있다. 보안에서 가장 무거운 세 축에서 **기존 둘은 다르지 않고**,
sudo-pop 이 만들어진 이유가 정확히 이 세 줄이다.

### 1-1. 발신자 검증

`BeginAuthentication` 은 에이전트가 시스템 버스에 내놓는 메서드고, polkit 만 부르라는
법이 없다. `hyprpolkitagent` 의 `onBeginAuthentication` 은 인자를 받아 바로 창을 띄운다 —
sender 를 polkitd 의 고유 이름과도, uid 0 과도 비교하지 않는다 (2026-08-20 main 소스로
재확인). `omarchy.polkit` 은 Quickshell 서비스에 맡기고 있어 사정이 같다.

그 결과는 [`rationale.md`](rationale.md) §3-4 에 적힌 그대로 둘 모두에 성립한다:

- 세션 안의 **아무 프로세스나** 진짜 에이전트가 그리는 진짜 인증 창을 띄울 수 있다.
  문구(`message`)를 공격자가 정하고, 위조 창이 아니라서 사용자가 구별할 방법이 없다
- 인증 성공 여부가 `BeginAuthentication` 의 리턴으로 호출자에게 샌다
- 시도마다 PAM 이 돌므로, 반복 유발로 **sudo·로그인과 공용인 faillock 카운터**를 태워
  계정을 잠글 수 있다

**sudo-pop 은 검증한다** — sender 의 고유 이름이 `org.freedesktop.PolicyKit1` 의 현재
소유자와 같거나 uid 0 일 때만 받고, 아니면 **창을 띄우기 전에** 거절하고 로그에 남긴다.
polkitd 재시작으로 고유 이름이 바뀌면 `NameOwnerChanged` 로 기준을 갱신한다. `busctl` 로
직접 불러 거절되는 것까지 실측했다 ([`rationale.md`](rationale.md) §3-4·§3-6).

### 1-2. 비밀번호 메모리

| | omarchy.polkit | hyprpolkitagent | sudo-pop |
|---|---|---|---|
| 저장 | QML/JS 문자열 — GC 힙, 사본 생성을 통제할 수 없다 | `std::string` — 제출 후 `.clear()` 뿐, 메모리를 덮어쓰지 않는다 | mlock 된 고정 버퍼(`Secret`) — 재할당 금지, 원시 쓰기 두 번으로 전송(포맷 사본 없음) |
| mlock (스왑·최대절전 이미지 차단) | ✗ | ✗ | ✓ |
| 코어덤프 차단 | ✗ | ✗ | ✓ `PR_SET_DUMPABLE=0` + `RLIMIT_CORE=0`, `panic = "abort"` 와 한 묶음 |
| zeroize (쓰고 지우기) | 불가능 (GC 언어) | ✗ (`.clear()` 는 지우기가 아니다) | ✓ |
| 비밀번호가 머무는 프로세스의 수명 | 셸과 같다 — 세션 내내 | 데몬과 같다 — 세션 내내 | **요청 하나** — 자식이 끝나면 주소 공간째 사라진다 |

기존 둘은 크래시 덤프·스왑·최대절전 이미지에 비밀번호가 실릴 수 있다. sudo-pop 의
하드닝은 `old/` 시절부터 있던 코드를 그대로 옮긴 것이고 ([`rationale.md`](rationale.md) §14),
자식 프로세스에 유효한 것을 검증 체크리스트로 확인한다 (§9).

### 1-3. faillock 인지와 재시도 상한

`hyprpolkitagent` 는 `FAILURE` 마다 헬퍼를 새로 띄운다 — **횟수를 세지 않는다** (main
소스로 확인). `omarchy.polkit` 도 예산 표시나 잠긴 계정 안내가 없다 (README 비교표, 실측).
polkit 과 sudo 가 **한 faillock 카운터를 공유**한다는 사실([`rationale.md`](rationale.md) §9)을
어느 쪽도 UI 에 반영하지 않는다.

sudo-pop 은 셋 다 한다: **쿠키(요청) 하나당 3회 상한** — 발신자 검증이 "요청을 만들 수
있는 쪽"을 좁히고 이 상한이 "요청 하나가 태울 수 있는 양"을 좁히는 짝이다 (§4-1) —,
창이 열려 있는 내내 **남은 잠금 예산 표시**(3회 이하면 에러 색), 그리고 **잠긴 계정이면
창 대신 안내** (물어봐야 실패만 쌓인다).

---

## 2. 갈리는 축

### 2-1. 프로세스 모델 — 비밀번호가 어느 주소 공간에 들어가는가

**`omarchy.polkit` 의 가장 무거운 열위다.** 비밀번호가 **셸 프로세스** — 세션에서 가장
크고, 가장 오래 살고, 가장 많은 입력(알림·트레이·테마·플러그인)을 처리하는 프로세스 —
의 주소 공간에 들어간다. 셸의 어느 구석이 크래시해도 그 덤프에 비밀번호가 실릴 수 있고,
셸에 물린 어떤 취약점이든 비밀번호와 같은 방에 있게 된다.

`hyprpolkitagent` 는 그 일만 하는 작은 전용 프로세스다. 지키는 코드는 없지만(§1-2),
**노출 반경 자체가 작다.** 다만 데몬이 창을 보였다 숨겼다 하는 구조라 프로세스는 오래
살고, 인증 동안 비밀번호가 그 데몬의 주소 공간을 통과한다.

`sudo-pop` 은 한 단계 더 간다 — 오래 사는 데몬에는 비밀번호가 **아예 들어오지 않는다.**
요청마다 뜨는 자식이 하드닝을 건 뒤에 받아서 헬퍼 fd 로만 보내고, 종료 코드로만 데몬에
말한다. 쿠키조차 argv·환경이 아니라 파이프로 넘긴다 ([`rationale.md`](rationale.md) §4·§4-2).

**평가: sudo-pop > hyprpolkitagent > omarchy.polkit.** 반경이 "요청 하나" < "전용 데몬" <
"셸 전체" 순으로 넓어진다.

### 2-2. 서피스 종류 — 포커스 보장과 화면 공유 제외는 맞바꿈이다

여기가 **omarchy.polkit 이 유일하게 구조적 우위를 갖는 축**이다.

| | omarchy.polkit (레이어셸 Overlay) | hyprpolkitagent (일반 창) | sudo-pop (일반 창 + 필수 규칙) |
|---|---|---|---|
| 친 키가 다른 창으로 새지 않는 보장 | **프로토콜 수준** — 배타 키보드 포커스 | 컴포지터 규칙 의존. 기본 배포에 규칙 없음 | 컴포지터 규칙 수준(`stay_focused`·`pin`) — 단, `--init` 이 규칙을 반드시 깔므로 "규칙 없는 상태"가 없다 |
| 화면 공유·녹화에서 창 제외 | **구조적으로 불가** — 레이어 서피스에는 규칙을 걸 수 없다 | 가능하나 기본 미설정 | ✓ `no_screen_share` — grim 스크린샷에서 검은 사각형으로 **실측** (§6-2) |
| 전체화면 위에 뜨는 것 | 보장 (Overlay 레이어) | 규칙 의존 | 규칙 의존 — 판정 시험이 체크리스트에 있고(§9), 실패하면 레이어셸 백엔드로 간다는 조건부 계획이 §2-4 에 있다 |

화면 공유에 비밀번호 글자가 찍히는 것은 아니다(마스킹된 필드다). 새는 것은 **인증 창의
존재와 문구** — 무엇에 권한을 올리고 있는지가 방송·녹화에 남는다. 반대로 포커스 축이
지키는 것은 "비밀번호 타이핑이 옆 창으로 가는 사고" 하나다. 키로깅은 Wayland 가 이미
막고, 창 위조는 레이어셸로도 못 막는다 ([`rationale.md`](rationale.md) §2-4).

**평가:** 포커스 하나만 보면 omarchy.polkit. 화면 공유까지 합치면 sudo-pop — 두 성질을
동시에 가진 것은 sudo-pop 뿐이고(규칙 수준 포커스 + 실측된 공유 제외), omarchy 는 한쪽을
**영영** 갖지 못하며, hyprpolkitagent 는 둘 다 사용자 몫으로 남긴다.

### 2-3. 사용자가 무엇에 비밀번호를 주는지 아는가

동의(consent)의 질 문제다. polkit 의 `message` 는 run0 경로에서 **무엇을 실행하는지 말하지
않는다** — 유닛 이름은 난수다 ([`rationale.md`](rationale.md) §3-5 실측).

| | omarchy.polkit | hyprpolkitagent | sudo-pop |
|---|---|---|---|
| details 의 `command_line`·`cmdline` 키 | 본다 | 본다 (+ `command` 키, + `message` 의 `"to run …"` 문구 파싱) | — (아래가 있어 필요 없다) |
| `polkit.subject-pid` → `/proc/<pid>/cmdline` | ✗ | ✗ | ✓ — pid 생존과 **소유자 uid 확인** 뒤에 읽는다 |
| run0 요청에서 보이는 것 | 난수 유닛 이름 | 난수 유닛 이름 | **실제 명령** (`pacman -Syu`) |
| pkexec 요청에서 보이는 것 | 명령 | 명령 (`$ …` 모노스페이스 박스) | 명령 |
| 한글 등 비 ASCII 명령 | (셸 폰트 체계) | (hyprtoolkit 팔레트) | ✓ fontconfig `:charset=` 폴백으로 실측 (§16) — 이 줄이 창의 존재 이유라서 |

run0 은 그 키들을 보내지 않으므로 기존 둘은 run0 앞에서 장님이고, hyprpolkitagent 의
문구 파싱은 §1-1 때문에 세션 내 임의 프로세스가 정할 수 있는 `message` 를 근거로 삼는다.
sudo-pop 의 subject-pid 경로는 그 한계를 우회하지만, 공짜는 아니다 — **cmdline 은 그
프로세스가 스스로 바꿀 수 있는 값**이라 이것도 절대 신뢰는 아니고, "polkit 의 문구보다
훨씬 나은 근거" 까지다.

**평가: sudo-pop 우위** — run0 라우팅이 이 도구의 기본 경로라서 더욱 그렇다.

### 2-4. 지문 — 있는 쪽이 오히려 안 켜진다

`omarchy.polkit` 은 지문 전용 UI 모드가 있다 — PAM 파일에 `pam_fprintd.so` 가 있으면
지문 화면으로 뜨고, 덮개가 닫히면 비밀번호로 되돌린다. 그런데 감지가
**`/etc/pam.d/polkit-1` 만** 본다. Arch 는 `/usr/lib/pam.d/polkit-1` 에 깔고 `/etc/pam.d`
쪽은 없으므로, **이 배포판에서 그 모드는 영영 안 켜진다** ([`rationale.md`](rationale.md) §2-3).

`hyprpolkitagent` 는 지문 UI 가 따로 없다. 대신 PAM 대화를 그대로 통과시킨다 —
`PAM_TEXT_INFO` 를 표시하고, PAM 이 비밀번호를 물을 때까지 입력 필드를 숨긴다. 즉
fprintd 가 PAM 스택에 있으면 **전용 UI 없이도 지문이 동작한다.**

`sudo-pop` 도 전용 UI 는 없다 — 비밀번호가 아닌 PAM 모듈은 1차 범위 밖이고,
`PAM_TEXT_INFO` 로 안내만 하고 통과시킨다 (§11). 감지 코드는 두 PAM 경로를 다 보도록
계획돼 있고(§2-3), UI 는 이 머신에 센서가 생겼을 때로 미뤄 뒀다 (§8-1).

**평가: 실사용 기준 hyprpolkitagent ≈ sudo-pop (둘 다 PAM 통과), omarchy 는 기능표의 ✓ 가
Arch 에서 동작하지 않는다.**

### 2-5. 코드 반경과 견고성

- **TCB 크기.** omarchy.polkit 은 QML 422줄이지만 그 아래에 Quickshell 전체 + Qt/QML
  런타임이 선다 — 비밀번호가 그 스택 전부와 한 프로세스에 있다. hyprpolkitagent 는
  sdbus-c++ + hyprtoolkit 위의 수백 줄. sudo-pop 은 zbus(tokio 없는 구성) + egui 위의
  Rust 이고, 비밀번호를 만지는 부분은 짧게 사는 자식 하나에 갇혀 있다
- **프로토콜 견고성.** hyprpolkitagent 는 실전에서 물린 자국이 코드에 있다 — 소켓 헬퍼가
  프롬프트 없이 닫히면 fork 로 폴백, 프롬프트 전 `FAILURE`(계정 잠김·깨진 PAM)는 취소로
  리턴해서 polkitd 의 재발행 고리를 끊는다. **sudo-pop 은 그 세 가지 대응을 그대로
  구현했고**(§3-3 — 자식의 exit 2 가 그 고리 차단이다) 잠긴 계정·취소·재시도를 실측했다
  (§3-6·§4-3). omarchy 쪽은 그 층이 Quickshell 안이라 이 문서 범위에서 검증하지 못했다
- **호출자의 25초 마감** ([`rationale.md`](rationale.md) §3-6): run0 등 D-Bus 호출자는 25초
  뒤 포기한다. 기존 둘은 세어 주지 않는다 — 늦게 치면 성공하고도 `Connection timed out`
  으로 끝난다. sudo-pop 은 창 구석에서 카운트다운하고 마지막 5초는 에러 색으로 바꾼다
- **`ECHO_ON`(OTP 등) 프롬프트**: 셋 다 처리한다 (sudo-pop 은 `gui.rs` 의
  `.password(!echo)`, omarchy 는 `echoMode`, hypr 는 프롬프트 종류별 필드)

**평가: sudo-pop ≥ hyprpolkitagent > omarchy.polkit** (검증된 범위 안에서 — sudo-pop 열은
자기 실측이라는 §0 의 주의를 함께 읽어라).

---

## 3. 정리 표

✓/✗ 는 확인된 사실, **(평가)** 는 이 문서의 판단이다. sudo-pop 열의 근거는 괄호의 문서 절.

| 축 | omarchy.polkit (Omarchy 4.0) | hyprpolkitagent (main, 2026-08-20) | sudo-pop (이 리포) |
|---|---|---|---|
| 실행 형태 | 셸(Quickshell) 프로세스 **안의** 서비스 | 전용 프로세스 | 전용 데몬 + **요청마다 짧게 사는 자식** (§4) |
| 비밀번호가 머무는 곳 | 셸 전체의 주소 공간 (GC 힙) | 데몬의 주소 공간 (`std::string`) | 자식의 mlock 버퍼 — **데몬에는 안 들어온다** |
| mlock · zeroize · 코어덤프 차단 | ✗ | ✗ | ✓ (§14, old/harden.rs·secret.rs) |
| 노출 반경 | 세션 최대 프로세스 | 작은 상주 프로세스 | 요청 하나짜리 프로세스 — **(평가) 최소** |
| `BeginAuthentication` 발신자 검증 | ✗ | ✗ | ✓ polkitd 이름 또는 uid 0, 창 전에 거절 — busctl 실측 (§3-4) |
| 재시도 상한 | ✗ | ✗ (무제한 헬퍼 재시작) | ✓ 쿠키당 3회 (§4-1) |
| faillock 예산 표시 · 잠긴 계정 안내 | ✗ | ✗ | ✓ 상시 표시 · 잠기면 창 대신 안내 |
| 키 입력이 다른 창으로 새지 않는 보장 | ✓ 프로토콜 수준 (배타 포커스) | 규칙 의존, 기본 없음 | 규칙 수준 — `--init` 이 필수 설치 |
| 화면 공유·녹화에서 창 제외 | ✗ **구조적으로 불가** | 가능하나 기본 미설정 | ✓ `no_screen_share` **실측** (§6-2) |
| 전체화면 위 표시 | ✓ (Overlay) | 규칙 의존 | 규칙 의존 (§2-4 — 실패 시 레이어셸 전환 계획) |
| run0 요청에서 실행될 명령 표시 | ✗ (난수 유닛 이름) | ✗ (난수 유닛 이름) | ✓ subject-pid 의 cmdline (§3-5) |
| pkexec 계열에서 명령 표시 | ✓ | ✓ (+ 문구 파싱) | ✓ |
| 호출자 25초 마감 표시 | ✗ | ✗ | ✓ 카운트다운 |
| 프롬프트-전 FAILURE 재발행 고리 차단 | 미확인 (Quickshell 내부) | ✓ (소스 확인) | ✓ (exit 2 규약, §3-3·§4) + 실측 |
| ECHO_ON·`PAM_TEXT_INFO` | ✓ | ✓ | ✓ |
| 지문 | 전용 UI — **Arch 에서 감지 버그로 안 켜짐** | PAM 통과로 동작 | PAM 통과 (전용 UI 없음, §11) |
| sudo 경로까지 같은 창 | ✗ | ✗ | ✓ 라우터가 run0/`sudo -A` 로 보낸다 (§7) |
| 테마 연동 | ✓ (`shell.toml` `[polkit]`) | hyprtoolkit 팔레트 | ✓ 같은 `[polkit]` 팔레트를 읽는다 |

### 총평 — 이 문서의 판단

**기존 둘만 보면 hyprpolkitagent 가 근소 우위다.** 결정적인 근거는 기능이 아니라
**반경**이다 — 어느 쪽도 비밀번호를 지키는 코드가 없는 이상(§1-2), 그것이 세션에서 가장
큰 프로세스에 들어가느냐(omarchy) 작은 전용 프로세스에 들어가느냐(hypr)의 차이가 남는다.
omarchy.polkit 의 유일한 구조적 우위인 배타 포커스는, 화면 공유 제외가 영영 불가능하다는
구조적 열위와 한 몸이다.

**sudo-pop 을 넣으면 표의 무게중심이 옮겨 간다.** 무거운 축 — 발신자 검증, 메모리 하드닝,
faillock 인지, run0 명령 표시 — 은 기존 둘이 똑같이 비워 둔 자리고, sudo-pop 은 그 네 칸을
채우려고 만들어졌다. 대신 잃는 것도 표에 있다: 배타 포커스라는 프로토콜 수준 보장(규칙
수준으로 대체), omarchy 의 지문 UI(Arch 에서는 어차피 안 켜지지만), 그리고 Omarchy 가
계속 손봐 주는 코드라는 유지보수 안심.

**경계도 그대로 적는다.** 이 비교의 어느 열도 "보안 벽" 이 아니다 — 이미 내 권한으로 도는
멀웨어는 별칭이든 바이너리든 에이전트든 바꿔치기할 수 있다. 세 에이전트가 갈리는 것은
**부주의한 누출** — 덤프·스왑·녹화·로그·잘못된 동의 — 을 어디까지 막느냐이고, 그 범위
안에서의 순서가 위 표다.

### 한계

- hyprpolkitagent 는 이 머신에 설치돼 있지 않다. 해당 열은 **소스 읽기**지 동작 실측이
  아니며, 창 규칙 적용 여부 같은 런타임 성질은 검증하지 못했다
- omarchy.polkit 의 D-Bus·헬퍼 층은 Quickshell 내부라 코드로 확인하지 못했다. "발신자
  미검증" 은 QML 층에 검증이 없고 Quickshell 문서가 검증을 언급하지 않는 데서 온 판단이다
- sudo-pop 열은 **자기 리포의 실측**이다. 제3자 검증이 아니라는 뜻이고, 근거 링크를 단
  이유가 그것이다. 전체화면 위 표시는 아직 체크리스트 항목이지 실측 기록이 아니다
- 세 프로젝트 모두 움직인다. hyprpolkitagent 는 Qt → hyprtoolkit 전환 직후라 위 사실은
  2026-08-20 의 main 기준이다
