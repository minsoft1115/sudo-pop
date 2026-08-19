# polkit 인증 에이전트 — 구현 계획

> **이 방향으로 간다.** 대안이었던 [`run0-router.md`](../old/docs/run0-router.md)(sudo 를 run0 로 번역해
> Omarchy 다이얼로그에 맡기는 안)는 보류했다 — 그쪽은 이 도구의 존재 이유인 메모리 위생을
> 통째로 내주는 거래였고, 그 대가로 얻는 것이 생각보다 작았다 (§2-2).
>
> **이 문서의 역할**: 아직 구현되지 않은 기능의 계획이다. 확정된 사양은
> [`plan.md`](../old/docs/plan.md), 설계 근거와 실측은 [`rationale.md`](../old/docs/rationale.md) 에 있고,
> 여기 있는 것이 구현되면 그 두 문서로 옮긴다.
>
> 실측은 전부 2026-08-19, Omarchy 4.0 / polkit 127 / systemd 261 기준이다.
>
> **저장소 배치가 바뀌었다.** 지금까지의 구현과 그 문서는 전부 `old/` 로 옮겼다 —
> `old/src`, `old/assets`, `old/docs`, 그리고 `old/Cargo.toml`·`old/install.sh`·`old/README*`.
> 루트에는 `docs/`(이 문서), `screenshots/`, `mise.toml`, `LICENSE` 만 남는다.
> 스크린샷은 남겨 둔다 — 창은 `old/src/askpass/gui.rs` 를 그대로 가져올 것이므로 결국 같아진다.
> 아래에서 `plan.md` §n, `rationale.md` §n 으로 참조하는 것은 `old/docs/` 의 그 문서들이고,
> `gui.rs`·`theme.rs` 같은 파일 이름은 `old/src/` 아래를 가리킨다 — 가져다 쓸 코드이지
> 그 자리에 그대로 둘 코드가 아니다.

---

## 0. 무엇을 만드는가

**데몬 모드 (`sudo-pop --agent`)** — sudo-pop 을 polkit 인증 에이전트로 등록해서,
polkit 을 거치는 **모든** 권한 요청의 비밀번호 창을 sudo-pop 이 그린다.

지금 sudo-pop 이 닿는 곳은 `sudo` 하나뿐이다. 에이전트가 되면 `run0`, 디스크 마운트
(udisks2), NetworkManager 의 시스템 설정, PackageKit, D-Bus 로 하는 서비스 조작이
전부 같은 창을 쓴다.

---

## 1. 왜

`run0` (systemd 256+, 이 머신은 261) 은 sudo 를 대체하는데 **sudo 를 전혀 쓰지 않는다.**
setuid 대신 polkit 으로 물어보고 transient 유닛으로 실행한다. 즉 지금 구조의 유일한
진입점인 `SUDO_ASKPASS` 훅이 여기엔 없다.

에이전트가 없으면 polkit 은 `pkttyagent` 를 띄워 **터미널 안에서** 묻는다. 그래서
`alias sudo='run0'` 로 바꾸는 순간 팝업을 잃는다. 반대로 에이전트가 되면 방향이 뒤집힌다 —
`run0` 이 팝업으로 물어보게 되고, **sudo 를 감쌀 이유 자체가 줄어든다.**

> **단, Omarchy 에서는 그 자리가 비어 있지 않다.** Omarchy 셸이 자기 polkit 에이전트를
> 갖고 있고 기본으로 켜져 있다 (§2-1). 그래서 이 작업은 "빈자리를 채우는 것" 이 아니라
> **교체**다. 무엇을 얻고 잃는지를 §2-1 에 적어 뒀고, 교체 여부 자체가 아직 열린 결정이다.

부수 효과가 하나 더 있다. 지금 하드닝(`plan.md` §4 — 코어덤프 차단, mlock, 화면 공유 제외,
zeroize)은 sudo 비밀번호에만 걸린다. 에이전트가 되면 **시스템의 모든 권한 프롬프트**가
그 하드닝을 입는다.

---

## 2. 확인된 사실

| 확인한 것 | 결과 |
|---|---|
| polkit | `127-3`, `polkitd` 실행 중 (backend `js`) |
| Authority D-Bus | `RegisterAuthenticationAgent(in (sa{sv}) subject, in s locale, in s object_path)` · `UnregisterAuthenticationAgent` · `AuthenticationAgentResponse2(in u uid, in s cookie, in (sa{sv}) identity)` |
| setuid 헬퍼 | `/usr/lib/polkit-1/polkit-agent-helper-1` (4755, root) |
| **소켓 헬퍼** | `polkit-agent-helper.socket` 이 **active** — `/run/polkit/agent-helper.socket`, `Accept=yes`, pidfd 기반. setuid 를 안 거치는 경로가 이미 열려 있다 |
| 헬퍼 프로토콜 문자열 | `PAM_PROMPT_ECHO_OFF` · `PAM_PROMPT_ECHO_ON` · `PAM_ERROR_MSG` · `PAM_TEXT_INFO` · `SUCCESS` · `FAILURE` |
| 헬퍼 제약 | `"inappropriate use of helper, stdin is a tty"` — stdin 이 파이프여야 한다 |
| 개발 헤더 | `/usr/include/polkit-1/polkitagent/` 존재 (C 레퍼런스로 참고용) |
| 이 머신의 GUI agent | **있다 — Omarchy 셸 안에서 돈다** (§2-1). `hyprpolkitagent` 는 미설치지만 그건 상대가 아니었다 |
| polkit PAM 스택 | `/usr/lib/pam.d/polkit-1` 이 **`system-auth` 를 include** 한다 — `sudo` 와 같은 스택이다 |
| faillock | `/run/faillock/<user>` **한 파일에 서비스 구분 없이 쌓인다.** 실제 기록에 `SVC polkit-1` 행이 남아 있는 것을 확인했다 (§9) |
| winit | **0.30.13 — 프로세스당 이벤트 루프 1개.** 두 번째 생성은 `EventLoopError::RecreationAttempt` (`event_loop.rs:119`) |
| 참고 구현 | `hyprpolkitagent` 가 polkit-qt 같은 래퍼 없이 **sdbus-c++ 로 프로토콜을 직접** 구현한다. 아래 프로토콜 절은 그 소스로 검증한 것이다 (§12) |

winit 줄이 구조를 결정한다. §4 참고.

### 2-1. Omarchy 는 이미 에이전트를 갖고 있다

```
/usr/share/omarchy/shell/plugins/polkit/PolkitAgent.qml
/usr/share/omarchy/shell/plugins/polkit/manifest.json     id: omarchy.polkit
```

`kinds: ["service"]`, `keepLoaded: true` — **Quickshell 셸 프로세스 안에서 도는 서비스**다.
별도 프로세스가 아니라서 `pgrep` 으로는 안 잡힌다. 이 머신에서
`omarchy-plugin-list` 가 `omarchy.polkit enabled=true` 로 답하고, `shell.json` 의 `disabled`
배열은 비어 있다.

구현도 얕지 않다. `Quickshell.Services.Polkit` 을 쓰고, 테마 색(`shell.toml` 의 `[polkit]`
섹션)을 따르며, **PAM 스택에 `pam_fprintd` 가 있으면 지문 모드**로 뜨고 노트북 덮개가
닫혀 있으면 비밀번호로 되돌린다.

**한 세션에 에이전트는 하나다.** 우리 것을 켜려면 이것을 꺼야 한다.

```bash
omarchy plugin disable omarchy.polkit
```

| 교체하면 얻는 것 | 교체하면 잃는 것 |
|---|---|
| 하드닝 — 코어덤프·스왑·화면 공유·로그로 비밀번호가 새지 않음 | 지문 인증 경로 (이 머신은 `fprintd` 미설치라 지금은 무의미하다) |
| sudo 와 polkit 프롬프트가 **같은 창**이 된다 | 테마 연동 (`shell.toml` 의 `[polkit]` 색) |
| 우리가 고칠 수 있는 코드 | Omarchy 가 계속 손봐 주는 코드 |

**Omarchy 는 sudo 쪽은 건드리지 않는다.** `/usr/share/omarchy` 전체에 `SUDO_ASKPASS` 나
askpass 관련 설정이 없다. 즉 지금도 sudo 는 sudo-pop, polkit 은 Omarchy 로 **이미 갈려
있다.** 교체는 그 갈림을 없애는 일이다.

### 2-2. 그쪽이 가진 것 중 우리가 못 만드는 것은 하나뿐이다

코드를 다 읽고 센 결과다. `PolkitAgent.qml` 390줄, `PolkitModel.js` 32줄.

| 그쪽 기능 | 실체 | 우리 쪽 |
|---|---|---|
| 문구 변환 | **정규식 하나** (`authorizationLabel`) | 5줄. 게다가 `message` 는 polkitd 가 이미 사람이 읽는 문장으로 준다 |
| 지문 감지 | PAM 파일에서 `pam_fprintd.so` 문자열 찾기 | 파일 한 번 읽기 (§2-3) |
| 덮개 게이트 | `omarchy-hw-laptop-closed` 호출 | 같은 CLI 를 부르면 된다 (§2-3) |
| 실패 피드백 | 흔들림 3키프레임 + 1200ms 플래시 | 20줄. **후순위** (§8-1) — 없어도 "Wrong" 글자로 충분하다 |
| ECHO_ON 입력 | `echoMode` 한 줄 | 한 줄 (지금은 `.password(true)` 고정) |
| 신원 선택 | **그쪽도 안 한다** — `identities` 를 받아 두고 UI 는 안 쓴다 | 동일하게 뒤로 |
| **레이어셸 Overlay + 배타 포커스** | 서피스 종류 자체 | **못 만든다.** eframe/winit 은 xdg_toplevel 만 (§2-4) |

반대로 **우리만 가진 것**: 하드닝(`PR_SET_DUMPABLE=0`·`RLIMIT_CORE=0`·버퍼 mlock·zeroize·
stdout 격리), 화면 공유 제외, 실행될 명령 표시, faillock 예산 계산과 잠긴 계정 안내,
90초 타임아웃, 포커스된 모니터에 뜨기. QML 안에서는 이 중 어느 것도 못 한다.

### 2-3. Omarchy 4.0+ 전용으로 못 박는다

이 도구는 **Omarchy 4.0 이상 · Hyprland 0.56 이상**에서만 쓴다. 그러면 분기와 폴백이 사라지고,
그쪽 자산을 그냥 가져다 쓸 수 있다.

| 가져다 쓰는 것 | 어떻게 |
|---|---|
| 덮개 상태 | `/usr/bin/omarchy-hw-laptop-closed` (exit 0 이면 닫힘). 233바이트 스크립트라 내용(`/proc/acpi/button/lid/*/state`)을 직접 읽어도 된다 |
| 테마 색 | `shell.toml` 의 **`[polkit]` 섹션** — `background`·`text`·`text-error`·`border`·`scrim`·`*-alpha`. 지금 `theme.rs` 가 읽는 `colors.toml` 에 이걸 더하면 **시스템 polkit 창과 같은 색**이 된다. 실패 표시색을 우리가 고를 필요가 없다 |
| 창 규칙 | `dim_around`·`stay_focused`·`pin`·`no_screen_share` 가 있다고 전제한다 |
| Lua 설정 | 버전 확인 분기를 없앤다 |

**PAM 경로에는 함정이 있다.** Omarchy 는 `/etc/pam.d/polkit-1` 만 본다.

```qml
FileView { path: "/etc/pam.d/polkit-1"; onLoadFailed: root.fingerprintConfigured = false }
```

그런데 Arch 의 polkit 은 **`/usr/lib/pam.d/polkit-1`** 에 깔고, `/etc/pam.d/polkit-1` 은
**없다**. 즉 그쪽 지문 모드는 이 시스템에서 영영 안 켜진다. 우리는 **두 경로를 다 본다**
(`/etc/pam.d` 가 있으면 그것이 이긴다 — PAM 의 탐색 순서와 같다).
Omarchy 에 올릴 만한 버그이기도 하다.

### 2-4. 레이어셸은 지금 하지 않는다

Hyprland 전용이므로 "다른 컴포지터에서 평범한 창이 된다" 는 논거는 해당 없다. 남는 차이는 둘이다.

- **전체화면 위에 뜨는가** — 테스트 한 번으로 판정된다 (§9). 통과하면 격차 없음
- **보장의 성격** — 배타 포커스는 프로토콜 수준, `stay_focused` 는 컴포지터 규칙.
  규칙은 `--init` 이 반드시 깔므로 "규칙이 없는 상태" 는 이 도구에 존재하지 않는다

보안적으로 이 축이 지키는 것은 **"친 키가 다른 창으로 새지 않는 것"** 하나다. 키로깅은
Wayland 가 이미 막고, 창 위조는 레이어셸로도 못 막는다. 화면 공유 제외는 오히려 **창 규칙
쪽이 되고 레이어 서피스는 안 된다.**

그래서 순서는 **하드닝 먼저, 서피스는 나중**이다. 위 테스트가 실패하면 그때
`smithay-client-toolkit` + `egui-wgpu` 로 백엔드를 갈아 끼우는 일을 꺼낸다.

---

## 3. 프로토콜

### 3-1. 등록

**시스템 버스**에 붙어서 부른다.

```
Authority.RegisterAuthenticationAgent(
    subject     = ("unix-session", { "session-id": <logind 세션 id> }),
    locale      = $LANG (없으면 빈 문자열),
    object_path = 우리가 export 한 경로)
```

- `subject` 는 `unix-session` 이다. `unix-process` 는 프로세스가 죽으면 같이 죽어서 데몬에 안 맞는다
- `object_path` 는 우리 네임스페이스로 잡는다 — 예: `/org/minsoft1115/sudo-pop/AuthenticationAgent`
- 종료 시 `UnregisterAuthenticationAgent` 를 반드시 부른다
- **한 세션에 에이전트는 하나다.** 이미 등록된 것이 있으면 등록이 실패한다 →
  `hyprpolkitagent` 와 동시에 못 쓴다. 실패를 삼키지 말고 그대로 알린다

**세션 id 를 구하는 데 세 단계가 필요하다.** 한 방법만 쓰면 환경에 따라 빈손이 된다.

1. `$XDG_SESSION_ID` — `pam_systemd` 가 넣어 주지만, user manager 가 물려주지 않는 경우가 있다
2. logind `GetSessionByPID(getpid())` → 세션의 `Id` 속성. 우리가 `session.scope` 안에 있을 때만 통한다
3. logind `GetUser(getuid())` → `User.Display` 속성. **systemd user 유닛으로 뜨면 여기까지 와야 한다** —
   `user@.service` 의 cgroup 은 세션 scope 밖이라 2번이 실패한다

우리는 user 유닛으로 뜰 계획이므로(§6) **3번이 실제 경로가 될 가능성이 높다.**

### 3-2. 우리가 구현하는 인터페이스

`org.freedesktop.PolicyKit1.AuthenticationAgent`

| 메서드 | |
|---|---|
| `BeginAuthentication(s action_id, s message, s icon_name, a{ss} details, s cookie, a(sa{sv}) identities)` | 인증이 끝날 때까지 **리턴하지 않는다**. 취소·실패도 리턴으로 끝난다 |
| `CancelAuthentication(s cookie)` | 진행 중인 인증을 접는다. 창을 닫고 헬퍼를 죽인다 |

> **응답을 우리가 보내지 않는다.** `AuthenticationAgentResponse2` 는 **헬퍼가 root 권한으로**
> 호출한다. 참고 구현도 그 메서드를 한 번도 부르지 않는다. 우리는 `BeginAuthentication` 의
> 리턴으로만 말한다. 이 사실을 놓치면 "인증은 됐는데 polkitd 가 모른다" 로 하루를 쓴다.

`BeginAuthentication` 을 어떻게 끝내느냐가 곧 결과 통보다.

| 상황 | 리턴 |
|---|---|
| 성공 | 정상 리턴 (빈 결과) |
| 사용자가 취소 | **정상 리턴.** 에러가 아니다 |
| 그 외 실패 | D-Bus 에러 `org.freedesktop.PolicyKit1.Error.Failed` |

**취소를 에러로 돌려주면 안 된다.** 아래 §3-3 의 재발행 함정과 같은 뿌리다.

`identities` 는 인증할 수 있는 신원 목록이다 (보통 `unix-user` 하나 또는 wheel 그룹의 사용자들).
둘 이상이면 고르게 해야 하지만, **1단계에서는 현재 사용자가 목록에 있으면 그것을 쓰고,
없으면 첫 번째를 쓴다.** 선택 UI 는 나중이다.

### 3-3. 헬퍼와의 대화

줄 단위 텍스트 프로토콜이다. **참고 구현 소스로 확인했다** (§12) — 추측이 아니다.

| 경로 | 여는 법 | 첫 바이트 |
|---|---|---|
| **소켓** (먼저 시도) | `/run/polkit/agent-helper.socket` 에 `connect`. 가능 여부는 `access(W_OK)` 로 본다 | `사용자이름\n쿠키\n` 을 그대로 write |
| **fork+exec** (폴백) | `execl(helper, "polkit-agent-helper-1", 사용자이름, NULL)` | **쿠키만** stdin 에 `쿠키\n` |

- 헬퍼 경로는 배포판마다 다르다. `/usr/lib/polkit-1/`, `/usr/libexec/polkit-1/` 등을 훑고,
  **setuid 비트를 확인**한 뒤에만 fork 경로를 쓴다
- stdin 이 tty 면 헬퍼가 거부한다 (`"inappropriate use of helper, stdin is a tty"`) → 파이프여야 한다
- 소켓도 fork 도 안 되면 인증 요청은 무조건 실패한다. **등록 직후에 미리 검사해서 알린다**

```
헬퍼 → PAM_PROMPT_ECHO_OFF Password:     ← 비밀번호 필드
헬퍼 → PAM_PROMPT_ECHO_ON  Username:     ← 에코 켠 필드 (OTP 등)
헬퍼 → PAM_ERROR_MSG  ...                ← 보여주기만
헬퍼 → PAM_TEXT_INFO  ...                ← 보여주기만
우리 → <입력>\n
헬퍼 → SUCCESS | FAILURE
```

태그와 본문 사이의 공백 하나는 있을 수도 없을 수도 있다. 접두사로 잘라내고 앞 공백 하나만 버린다.

#### 반드시 넣어야 하는 세 가지 대응

여기가 참고 구현이 실제로 물려서 넣은 부분이다. 없으면 조용히 망가진다.

**1. 소켓이 프롬프트 없이 닫히면 fork 로 폴백한다.**
`SO_PEERPIDFD` 가 없는 커널에서는 소켓 헬퍼가 아무것도 묻지 않고 EOF 를 준다. 소켓이 열렸다는
사실만으로 성공을 가정하면 안 된다. **프롬프트를 한 번이라도 받았는지**를 기억해 두고,
못 받은 채 끝났으면 fork 경로로 한 번 다시 시도한다.

**2. 프롬프트 전에 온 `FAILURE` 는 취소로 끝낸다.**
계정 잠김이나 깨진 PAM 스택이면 헬퍼가 묻지도 않고 실패한다. 이때 D-Bus 에러로 돌려주면
**polkitd 가 `BeginAuthentication` 을 다시 발행하고, 빈 창이 영원히 다시 뜬다.**
프롬프트를 못 받은 실패는 "취소" 로 리턴해서 고리를 끊는다.

**3. 프롬프트 뒤에 온 `FAILURE` 는 재시도다.**
헬퍼는 시도 하나마다 죽는다. 창은 그대로 두고 **헬퍼를 새로 띄운다.**
신원을 바꿀 때도 마찬가지로 헬퍼를 죽이고 새로 시작한다.

---

## 4. 구조 — 비밀번호는 데몬을 통과하지 않는다

### 제약이 구조를 정한다

winit 0.30.13 은 **프로세스당 이벤트 루프를 하나만** 허용한다. 데몬이 요청마다
`eframe::run_native` 를 부르는 구조는 두 번째 요청에서 실패한다. 선택지는 둘이다.

| | |
|---|---|
| (a) 창을 하나 띄워 두고 보였다 숨겼다 | GUI 코드를 다시 짜야 하고, 컴포지터 연결과 창 상태가 계속 살아 있다. 하드닝의 전제(프로세스가 짧게 살고 죽으면서 메모리가 사라진다)가 깨진다 |
| **(b) 요청마다 짧은 자식 프로세스** | 지금 askpass 모드와 **수명·하드닝이 똑같다.** GUI 코드를 그대로 쓴다 |

**(b) 로 간다.**

### 흐름

```
run0 / udisks / …
 └─ polkitd
     └─ BeginAuthentication(cookie, identities)  →  sudo-pop --agent   (세션당 1, 창 없음)
                                                     │
                                                     ├─ fork: sudo-pop --agent-prompt
                                                     │        ├─ 하드닝 → 창
                                                     │        ├─ 헬퍼 연결 (소켓 → fork 폴백)
                                                     │        ├─ PAM 줄 프로토콜 · 오답 시 헬퍼 재시작
                                                     │        └─ 종료 코드로 결과 보고
                                                     │
                                                     └─ 종료 코드 → BeginAuthentication 리턴
                                                          (성공·취소는 정상 리턴, 그 외는 Error.Failed)
```

**헬퍼와의 대화는 통째로 자식이 갖는다.** 데몬이 헬퍼를 열고 fd 만 넘기는 것도 되지만,
오답 재시도와 신원 변경이 **헬퍼를 새로 띄우는 일**이라(§3-3) 소유권이 왔다 갔다 하게 된다.
자식이 전부 갖고 있으면 그런 게 없고, **비밀번호가 데몬 주소 공간에 한 번도 안 들어온다.**

데몬이 하는 일은 셋뿐이다 — D-Bus, 자식 수명, 그리고 큐.

| 데몬 | 자식 (`--agent-prompt`) |
|---|---|
| 등록·해제, `BeginAuthentication`/`CancelAuthentication` | 하드닝, 창, 헬퍼 연결, PAM 대화, 재시도 |
| 요청 큐 — **한 번에 하나만** 처리하고 나머지는 줄 세운다 | 종료 코드로만 말한다 (0=성공, 1=실패, 2=취소) |
| 취소 요청이 오면 자식을 죽인다 | 비밀번호는 헬퍼로만 보낸다 |

자식에 넘길 것: 프롬프트에 띄울 `message`·`action_id`·`icon_name`, 선택된 신원의 사용자 이름,
그리고 **쿠키**. 쿠키는 `ps` 에 노출되지 않게 **인자가 아니라 파이프나 환경으로** 넘긴다.

> **종료 코드가 §3-3 의 2번과 이어진다.** 프롬프트를 한 번도 못 받고 실패한 경우
> (계정 잠김·깨진 PAM 스택) 자식은 **1 이 아니라 2(취소)** 로 보고해야 한다. 1 로 보고하면
> 데몬이 `Error.Failed` 를 돌려주고, polkitd 가 요청을 다시 발행해 빈 창이 계속 뜬다.
> "프롬프트를 받았는가" 를 아는 것은 헬퍼와 말한 자식뿐이므로, 판단도 자식이 한다.

`gui::prompt()` 와 `theme` 는 손대지 않고 재사용한다. 바뀌는 것은 "비밀번호를 어디로
보내는가" 뿐이고, 그건 이미 `secret::PasswordChannel` 로 추상화돼 있다. 다만 지금은
**한 번 받고 끝**이라, 오답 재시도와 `PAM_TEXT_INFO` 표시를 위해 창이 살아 있는 채로
프롬프트 문구를 갈아 끼우는 경로가 필요하다 (`gui.rs` 의 작은 확장).

## 5. 파일과 모드

```
src/agent/mod.rs       --agent 진입점, D-Bus 서비스, 등록/해제
src/agent/helper.rs    헬퍼 연결(소켓/setuid)과 PAM 줄 프로토콜
src/agent/subject.rs   logind 세션 id 조회, subject 값 구성
src/agent/prompt.rs    --agent-prompt (자식) — 하드닝 + GUI + 헬퍼 대화
```

**창을 그리는 진입점은 하나로 유지한다.** askpass 모드와 `--agent-prompt` 는 "하드닝 → 창 →
받은 것을 어딘가로 보낸다" 가 똑같고, 다른 것은 **보내는 곳**뿐이다 (격리한 fd 냐, 헬퍼냐).
그 차이만 인자로 받는다. 이벤트 루프를 부르는 코드가 두 벌이 되면 §4 의 제약을 두 번
지켜야 한다.

**앱 id 는 `sudo-askpass` 그대로 둔다.** `old/assets/sudo-pop.lua` 의 창 규칙이 그 id 로
매칭하고 있고, 에이전트 창도 같은 규칙(띄우기·가운데·`dim_around`·화면 공유 제외)을
그대로 받아야 한다.

`init.rs` 에는 systemd user 유닛을 다루는 절이 붙는다 (§6). 셸 스니펫·Hyprland 규칙과 같은
규약이다 — 쓰기 전에 같은 내용인지 보고, `--uninit` 이 정확히 자기 것만 걷어낸다.

`main.rs` 의 모드 분기에 `--agent` 와 `--agent-prompt` 를 더한다. 지금 규칙(`--init`·`--uninit`
이 아니면 전부 sudo 로 넘김)을 유지하되, **이 두 개는 sudo 로 넘기지 않는다.**

의존성: D-Bus 클라이언트가 필요하다. `zbus` 를 쓰되 **tokio 를 끌고 오지 않는 구성**으로
넣는다 (`async-io` 기본). 바이너리 크기와 빌드 시간에 영향이 크므로 스파이크에서 실측해
`rationale.md` 에 남긴다.

쓰는 D-Bus 기능은 많지 않다 — 오브젝트 하나 export, 메서드 두 개, 프록시 호출 세 개
(polkit 등록·해제, logind 조회). 참고 구현은 sdbus-c++ 로 660줄에 전부 담았다.

---

## 6. 수명 관리 — systemd user 유닛

**`--init` 이 유닛까지 깔고 켠다.** 안내만 하고 손으로 하게 두지 않는다. 지금도 `--init` 이
셸 스니펫과 Hyprland 규칙을 끝까지 넣어 주므로, 여기만 반쪽으로 두면 규약이 갈린다.

```
~/.config/systemd/user/sudo-pop-agent.service
```

```ini
[Unit]
Description=sudo-pop polkit authentication agent
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=exec
ExecStart=%h/.local/bin/sudo-pop --agent
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
```

`graphical-session.target` 이 실제로 도는지는 **확인했다** — Omarchy 는 `uwsm` 으로 Hyprland 를
띄우고(`uwsm start -g -1 -e -D Hyprland hyprland.desktop`), 그 타깃이 active 다.
uwsm 을 안 쓰는 환경에서는 이 타깃이 안 켜질 수 있으므로, `--init` 이 **깔기 전에 확인하고**
안 켜져 있으면 그 사실을 알린다 (유닛은 깔되 "로그인 시 자동 시작은 이 환경에서 보장되지
않는다" 를 출력).

| | |
|---|---|
| `--init` | 유닛 파일 작성 → `systemctl --user daemon-reload` → `enable --now` |
| `--uninit` | `disable --now` → 유닛 파일 삭제 → `daemon-reload` |
| 재실행 | 지금 규칙 그대로 idempotent. 내용이 같으면 다시 쓰지 않는다 |

`ExecStart` 는 `--init` 을 실행한 바이너리의 **절대 경로**를 박는다 (`$PATH` 에 기대지 않는다).
`~/.local/bin` 이 아닌 곳에 깔았어도 맞게 들어간다.

### 다른 에이전트가 이미 있으면 설치하지 않는다

**한 세션에 polkit 에이전트는 하나다** (§3-1). 이미 다른 것이 자리를 잡고 있으면 우리 유닛을
켜 봐야 등록에 실패하고, 실패한 서비스가 `Restart=on-failure` 로 계속 되살아난다.

그래서 `--init` 은 **켜기 전에 찾아보고, 있으면 건너뛴다.**

찾는 순서가 중요하다. **1순위는 `omarchy.polkit`** 이다 — 이 환경에서 실제로 자리를 잡고 있는
것이 그것이고, 프로세스 목록에는 **안 나온다**(셸 안의 서비스라서). 플러그인 목록으로 봐야 한다.

| 순위 | 찾는 법 |
|---|---|
| 1 | `omarchy-plugin-list --json` 에 `omarchy.polkit` 이 `enabled` 인가 |
| 2 | 도는 프로세스 — `hyprpolkitagent`, `polkit-gnome-authentication-agent-1`, `polkit-kde-authentication-agent-1`, `lxpolkit`, `mate-polkit` |
| 3 | 활성화된 user 유닛 중 위 이름들 |

```
[!] Omarchy 셸의 polkit 에이전트(omarchy.polkit)가 켜져 있다.
    한 세션에 에이전트는 하나뿐이라 sudo-pop 에이전트는 켜지 않았다.
    바꾸려면:  omarchy plugin disable omarchy.polkit
               sudo-pop --init
    (지문 인증과 테마 연동은 그쪽 기능이다. docs/polkit-agent.md §2-1 참고)
```

유닛 파일은 **깔아 두되 enable 하지 않는다.** 나중에 마음이 바뀌면 한 줄로 켤 수 있고,
설치와 활성화를 갈라 두면 "왜 안 도는지" 를 상태로 설명할 수 있다.

### 그 밖

- `polkitd` 가 재시작되면 등록이 날아간다 → `NameOwnerChanged` 를 구독해 재등록
- 세션이 끝나면 unregister 후 종료 (`PartOf` 가 정지까지 묶어 준다)
- 등록이 실패하면 **조용히 죽지 않는다.** 이유를 로그와 종료 코드로 남긴다.
  `Restart=on-failure` 가 무한 재시도하지 않도록 등록 충돌은 **실패가 아닌 정상 종료**로 끝낸다

---

## 7. 기존 기능은 어떻게 되나 — 스마트 라우터

sudo 래퍼는 **남긴다.** `sudo -E`, `-v`, `-k`, `sudoers` 의 `NOPASSWD`·`env_keep` 처럼
polkit 에 대응이 없는 것들이 있고, 스크립트가 부르는 sudo 도 그대로다.

에이전트가 돌기 시작하면 래퍼는 **갈림길**이 된다.

| 친 것 | 가는 곳 | 왜 |
|---|---|---|
| `sudo <명령>` | `run0 <명령>` | polkit 을 타므로 우리 에이전트가 창을 띄운다. setuid 를 안 거치고 실행 기록이 systemd 세션에 남는다 |
| `sudo -옵션 …` | 지금의 `sudo -A` 경로 | run0 에 대응이 없거나 의미가 다른 것들 |
| `sudo VAR=값 <명령>` | 지금의 `sudo -A` 경로 | **옵션이 아니라 환경 할당이다.** run0 으로 보내면 변수가 조용히 사라진다 |

판정은 이미 있는 `sudo_args.rs` 가 한다 — 옵션이 값을 따로 받는지, `--` 가 어디서 끝나는지를
아는 코드가 그것뿐이다. 셸 alias 나 함수로 같은 판정을 흉내 내지 않는다. 두 곳에 갈리면
반드시 어긋난다.

주의할 것 두 가지.

- **`-u`·`-i` 는 run0 에도 있다.** 그래도 1차에서는 "옵션이면 sudo" 로 단순하게 간다.
  나중에 개별로 넘길 수 있지만, 규칙이 늘수록 어긋날 곳도 는다
- **옵션 없는 호출도 sudo 와 완전히 같지는 않다.** `env_keep`(`SSH_AUTH_SOCK`·`DISPLAY`),
  `NOPASSWD`, 잡 컨트롤(transient 유닛이라 셸의 자식이 아니다)이 다르다. 이건 라우팅 규칙으로
  못 막는다. README 에 명시하고, 되돌릴 스위치(`SUDO_POP_RUN0=0`)를 둔다

**이 갈림길은 에이전트가 실제로 도는 것을 본 뒤에 켠다** (§8 의 5단계 이후). 에이전트 없이
켜면 흔한 경우가 팝업을 잃고 터미널 프롬프트(`pkttyagent`)로 떨어진다.

---

## 8. 단계

각 단계가 끝날 때마다 실제로 확인 가능한 것을 남긴다.

1. **스파이크 — 등록만.** zbus 로 시스템 버스에 붙어 등록하고 `BeginAuthentication` 을
   **로그만** 찍는다. `run0 true` 를 쳐서 호출이 오는지, `identities` 와 `details` 에 뭐가
   들어오는지 눈으로 본다. 세션 id 3단계(§3-1)도 여기서 확인한다
2. **헬퍼 왕복.** 소켓 경로로 PAM 대화를 붙이고 비밀번호를 **stdin 으로 읽어** 성공/실패를
   확인한다. fork 폴백과 §3-3 의 세 가지 대응까지 넣는다. 프로토콜 리스크는 여기서 끝난다
3. **GUI 연결.** 자식 프로세스를 분리하고 `gui::prompt()` 를 붙인다. 창을 살린 채 프롬프트를
   갈아 끼우는 경로를 만든다
4. **가장자리.** 취소, 신원이 여럿일 때, 요청 큐, polkitd 재시작 후 재등록
5. **포장.** user 유닛과 `--init`/`--uninit` 연동(§6), 다른 에이전트 감지, README·`plan.md`·
   `rationale.md` 정리
6. **스마트 라우터.** 에이전트가 도는 것을 확인한 뒤에 §7 의 갈림길을 켠다

### 8-1. 후순위 — 하고 싶지만 나중에

동작에 필요 없고, 없어도 쓰는 데 지장이 없는 것들. 순서대로 꺼낸다.

| | 왜 나중인가 |
|---|---|
| 실패 피드백 (흔들림·에러 색 플래시) | 재시도 프롬프트에 `Wrong` 한 줄이면 뜻은 전달된다. 20줄짜리 장식 |
| 지문 모드 | 이 머신에 `fprintd` 가 없다. 감지 코드는 §2-3 대로 두 경로를 보되, **UI 는 센서가 실제로 생겼을 때** |
| 신원 선택 UI | 관리자가 여럿인 환경에서만 의미가 있다. Omarchy 도 안 한다 |
| 레이어셸 서피스 | §2-4 의 전체화면 테스트가 실패했을 때만 |

## 9. 검증 체크리스트

- `run0 true` — 팝업이 뜨고, 성공 / 오답 / 취소(Esc) 세 경로가 각각 맞게 끝나는가
- `run0` 이 아닌 액션에서도 뜨는가 — USB 마운트, `systemctl` D-Bus 조작
- `auth_admin_keep` 캐시가 도는가 (연속 호출 시 두 번째는 안 물어봐야 한다)
- 에이전트를 죽이면 `pkttyagent` 로 폴백되는가
- `hyprpolkitagent` 를 띄운 상태에서 등록이 **실패하고 그 사실이 보이는가**
- 하드닝이 자식에 유효한가 — 코어덤프 없음, 스왑에 안 남음, 화면 공유에서 제외
- **오답을 세 번 넣어도 창이 살아 있고** 매번 헬퍼가 새로 뜨는가
- **잠긴 계정으로 요청했을 때 빈 창이 무한히 다시 뜨지 않는가** (§3-3 의 2번)
- 소켓 헬퍼가 프롬프트 없이 닫히는 커널에서 fork 폴백이 도는가 (`polkit-agent-helper.socket` 을
  꺼서 재현 가능)
- 인증 중에 요청이 하나 더 오면 줄을 서는가
- **전체화면 앱 위에서 창이 보이고 키가 들어가는가** (§2-4 의 판정 시험). 실패하면
  레이어셸을 후순위에서 끌어올린다
- polkit 이 **현지화된 메시지**를 보낼 때 글자가 깨지지 않는가 — 등록 시 `locale` 을 넘기므로
  한글 문장이 올 수 있고, 지금 폰트 로더는 8MB 상한 때문에 CJK 면을 건너뛴다 (`font.rs`).
  깨지면 폴백 폰트를 하나 더 붙인다
- `ECHO_ON` 프롬프트에서 입력이 **가려지지 않고** 보이는가
- `--init` → 로그아웃 → 로그인에서 에이전트가 **자동으로 떠 있는가**
- `--init` → `--uninit` 왕복 뒤 유닛 파일과 `enable` 링크가 **둘 다** 사라지는가
- `hyprpolkitagent` 를 켜 둔 채 `--init` 하면 **안내를 내고 enable 하지 않는가**
- 등록 충돌로 끝났을 때 `Restart=on-failure` 가 재시도 고리를 만들지 않는가
- **faillock — 카운터는 공유된다.** `/usr/lib/pam.d/polkit-1` 이 `system-auth` 를 include 하고,
  그 스택의 `pam_faillock` 은 `/run/faillock/<user>` **한 파일**에 쌓는다. 서비스 이름은
  기록만 될 뿐 칸이 갈리지 않는다. 실제로 이 머신의 기록에 `SVC polkit-1` 행이 남아 있다.
  즉 **polkit 에서 틀린 것이 sudo 를 잠근다.** `plan.md` §4-4 의 대응이 에이전트 경로에도
  그대로 필요하다 — 재측정은 확인용이지 판단 근거가 아니다

---

## 10. 결정과 남은 것

### 결정됨

| | |
|---|---|
| 헬퍼 경로 | 소켓 먼저, fork+exec 폴백 **필수** (§3-3) |
| `--init` | systemd user 유닛을 **깔고 enable 까지** 한다 (§6) |
| 다른 에이전트가 있을 때 | 유닛은 깔되 **enable 하지 않고 안내**한다 (§6) |
| zbus 의존성 | **받아들인다.** 빌드 시간 증가는 감수한다. 실측치는 남겨 `rationale.md` 에 기록 |
| 스마트 라우터 | 에이전트가 도는 것을 본 뒤에 켠다. 되돌릴 스위치 `SUDO_POP_RUN0=0` 을 둔다 (§7) |
| 신원이 여럿일 때 | 1차는 현재 사용자 우선, 없으면 첫 번째. 선택 UI 는 후순위 (§8-1) |
| 대상 환경 | **Omarchy 4.0+ / Hyprland 0.56+ 전용.** 버전 분기와 폴백을 두지 않는다 (§2-3) |
| 창 서피스 | 지금의 xdg_toplevel + Hyprland 창 규칙 그대로. 레이어셸은 후순위 (§2-4) |
| 덮개·테마 | Omarchy 것을 그대로 쓴다 — `omarchy-hw-laptop-closed`, `shell.toml` 의 `[polkit]` (§2-3) |

| 시도 횟수 정책 | **`attempts` 게이팅을 에이전트 경로에도 건다.** 카운터가 공유된다는 것이
설정과 실제 기록으로 확인됐다 (§9). 값은 `plan.md` §4-4 를 따르고, 재측정은 확인용으로만 한다 |

### 남은 결정 — 사람이 정할 것

**`omarchy.polkit` 을 교체할 것인가** (§2-1). 기술 문제가 아니라 취향과 우선순위 문제다.
하드닝과 창 일원화를 얻고, 지문 경로와 테마 연동을 잃는다. 이 결정 전까지 §8 의 5단계
(유닛 enable)는 **의미가 없다** — 켜도 등록이 거부된다. 1~4단계는 그와 무관하게 진행할 수
있다: 다른 에이전트가 있는 채로도 **등록 시도까지는** 확인할 수 있고, 그 실패를 보는 것이
곧 §6 의 충돌 처리 검증이다.

---

## 11. 하지 않는 것

- polkit **정책 파일**(`.rules`, `.policy`)을 쓰거나 고치지 않는다. 우리는 물어보는 쪽이지
  누가 무엇을 할 수 있는지 정하는 쪽이 아니다
- 지문·FIDO 등 비밀번호가 아닌 PAM 모듈의 UI 는 1차 범위 밖이다. `PAM_TEXT_INFO` 로
  안내만 하고 통과시킨다
- 시스템 데몬으로 만들지 않는다. 사용자 세션 전용이다

---

## 12. 참고 구현

[`hyprpolkitagent`](https://github.com/hyprwm/hyprpolkitagent) — Hyprland 용 polkit 에이전트.
**polkit-qt 같은 래퍼를 쓰지 않고 sdbus-c++ 로 프로토콜을 직접 구현한다.** 위 §3 의 사실은
전부 이 소스에서 확인한 것이다.

| 파일 | 줄 | 볼 것 |
|---|---|---|
| `src/core/PolkitListener.cpp` | 661 | 등록, 헬퍼 소켓/fork, PAM 줄 처리, 큐, 세션 id 3단계 |
| `src/core/PolkitListener.hpp` | | `SAuthRequest`·`SHelperProc`·`SActiveAuth` 자료구조 |
| `src/core/Agent.cpp` | 68 | 리스너와 UI 사이의 얇은 층 |
| `src/ui/Dialog.cpp` | 441 | 프롬프트·에러·정보 표시, 신원 선택 |

특히 `tryHelperSocket` / `tryHelperFork` / `onHelperReadable` / `handleLine` / `completeAuth`
다섯 함수가 프로토콜의 전부다.

**구조가 다른 점** — 저쪽은 툴킷 백엔드를 프로세스 안에 계속 띄워 두고 창을 보였다 숨겼다
한다. 우리는 winit 제약(§4) 때문에 그렇게 못 하고, 그럴 필요도 없다. 비밀번호를 데몬 밖으로
빼는 것이 우리 쪽 하드닝의 전제이기 때문이다.

**베끼지 않는다.** GPL 여부와 무관하게, 프로토콜은 사실이고 구현은 우리 구조에 맞춰 새로 쓴다.

### `omarchy.polkit` — 같은 자리에서 이미 도는 것

`/usr/share/omarchy/shell/plugins/polkit/PolkitAgent.qml`. QML 이고 `Quickshell.Services.Polkit`
이 프로토콜을 대신 해 주므로 D-Bus·헬퍼 코드는 없다. 대신 **다이얼로그가 무엇을 보여줘야
하는지**의 참고가 된다 — `action_id` 를 사람이 읽는 문구로 바꾸는 `PolkitModel.js`,
실패 시 흔들림·테두리 색, 지문과 비밀번호 사이의 전환 규칙.

---

## 13. 다른 제안에서 거른 것

밖에서 들어온 제안(전체 생성을 노린 사양서 초안)을 검토했다. 가져온 것은 위에 녹였고(§4 의 모드 분리, §5 의 GUI
진입점 단일화, §7 의 스마트 라우터), 아래는 **넣지 않는다.** 이유를 남겨 둔다.

| 제안 | 왜 안 되는가 |
|---|---|
| 데몬이 **자식의 stdout 에서 비밀번호를 읽어** 헬퍼에 넘긴다 | 비밀번호가 **오래 사는 프로세스**의 주소 공간을 통과한다. §4 의 전제를 정면으로 깬다. 자식이 헬퍼와 직접 말하면 데몬은 볼 일이 없다 |
| PAM 크레이트로 **직접 인증**하고 D-Bus 핸드셰이크를 끝낸다 | 불가능하다. `AuthenticationAgentResponse2` 는 **root 가 부른 것만** polkitd 가 받는다. 우리 권한으로는 shadow 도 못 읽는다. 그 일을 하라고 있는 것이 setuid/소켓 헬퍼다 |
| `hyprctl keyword windowrulev2 ...` 로 창 규칙을 주입 | Hyprland 0.56 은 `keyword` 로 오는 윈도우 룰을 거부한다. `plan.md` §1 의 금지 사항이고, 정적 Lua 규칙이 이미 그 자리를 대신한다 |
| `--init` 이 `~/.bashrc`·`~/.zshrc` 에 `alias sudo=...` 를 직접 덧붙인다 | 지금은 스니펫을 `~/.config/minsoft1115/bash/` 에 두고 **공유 로더 블록**만 건드린다. 다른 도구와 같은 폴더를 쓰기 때문이고, 직접 덧붙이면 그 규약이 깨진다 |
| `exec-once = sudo-pop --polkit` 을 `hyprland.conf` 에 안내 | 두 가지가 틀렸다. 이 환경의 Hyprland 설정은 **Lua**(`hyprland.lua`)이고, 에이전트는 **systemd user 유닛**으로 떠야 한다 — 세션 id 를 `User.Display` 로 찾는 경로(§3-1)가 거기에 맞다 |
| 앱 id 를 `sudo-pop-gui` 로 | 설치된 창 규칙이 `^(sudo-askpass)$` 로 매칭한다. 바꾸면 규칙이 통째로 안 걸린다 |
| `opt-level = "z"` 로 바꾸고 "역공학 방지" | 최적화 수준은 역공학과 무관하다. 지금 `opt-level = 3` 은 GUI 응답성 때문에 고른 값이고, `strip`·`lto`·`panic = "abort"` 는 이미 들어가 있다 (`panic = "abort"` 는 하드닝과 한 묶음이다 — `rationale.md` §6) |
| 창이 비밀번호를 **stdout 에 출력** | askpass 모드는 이미 stdout 을 격리하고 따로 떼어 둔 fd 에 쓴다 (`plan.md` §4-2). 에이전트 자식은 아예 stdout 으로 안 보내고 헬퍼로 보낸다 |

---

## 14. `old/` 읽기 지도

새로 쓰는 게 아니라 **옮겨 오는 것**이 대부분이다. 어디를 열어야 하는지, 그리고 무엇이
그대로 오고 무엇이 갈리는지 미리 적어 둔다. 줄 수는 실제 파일 기준이다.

### 그대로 가져오는 것

| 파일 | 줄 | 무엇이 들어 있나 |
|---|---|---|
| `old/src/askpass/harden.rs` | 71 | `PR_SET_DUMPABLE=0` + `RLIMIT_CORE=0`. **`mlockall` 을 쓰지 않는 이유**가 주석에 있다 — 주소 공간 전체는 `RLIMIT_MEMLOCK`(8MB)을 넘겨 `ENOMEM` 으로 아무것도 못 지킨다 |
| `old/assets/sudo-pop.lua` | 13 | 창 규칙. app-id `sudo-askpass` 로 매칭하며 `no_screen_share` 가 여기 있다 |
| `old/src/paths.rs` | 103 | `$XDG_RUNTIME_DIR/sudo-pop/` 을 0700 으로 만들고 검증하는 코드. 에이전트는 askpass 심볼릭 링크가 필요 없지만, **시도 카운터 파일이 같은 디렉터리를 쓴다** |
| `old/src/wrapper.rs` · `sudo_args.rs` | 93 · 195 | sudo 경로는 손대지 않는다. 폴백 5단계(인자 없음 / `-A`·`-n`·`-S` / 디스플레이 없음 / 런타임 디렉터리 없음 / 링크 실패)의 판단 기준이 여기 |

### 고쳐서 가져오는 것 — 여기가 실제 작업이다

| 파일 | 줄 | 무엇이 갈리나 |
|---|---|---|
| `old/src/askpass/secret.rs` | 194 | **가장 중요한 갈림.** `Secret` 은 그대로(mlock + zeroize + 재할당 금지, 테스트까지 있다). 하지만 `PasswordChannel` 은 "sudo 가 읽는 stdout" 에 묶여 있다 — 에이전트에서는 목적지가 **헬퍼 fd** 다. **목적지 fd 를 받는 형태로 일반화**하면 양쪽이 같은 코드를 쓴다. `send()` 가 두 번의 raw write 인 이유(합치면 zeroize 안 되는 사본이 생긴다)는 그대로 유효하다 |
| `old/src/askpass/mod.rs` | 161 | `--agent-prompt` 의 뼈대. 순서가 사양이다 — 하드닝 → 채널 확보 → 예산 확인 → 창 → 전송 → wipe. 에이전트에서는 "채널 확보" 가 헬퍼 연결로 바뀐다 |
| `old/src/askpass/gui.rs` | 312 | 창 자체는 그대로. 세 곳을 고친다 — ① `prompt()` 가 **1회용**이라 다회 대화를 못 한다 ② `.password(true)` 고정이라 `ECHO_ON` 을 못 받는다 ③ 90초 타임아웃은 에이전트에서도 유지 |
| `old/src/attempts.rs` | 232 | faillock 예산 계산(`deny` 읽기, tally 파싱, 잠금까지 남은 횟수)은 **polkit 경로에도 그대로 유효하다** (§9 — 카운터가 공유된다). 갈리는 것은 **리셋 시점**뿐이다: 지금은 `wrapper.rs` 가 sudo 명령마다 지우는데, 에이전트에는 그 자리가 없다. **인증 요청 하나 = 한 세션**으로 다시 정의해야 한다 |
| `old/src/askpass/theme.rs` | 206 | `colors.toml` 만 읽는다. **`shell.toml` 의 `[polkit]` 섹션을 더한다** (§2-3) — 그러면 에러 색·스크림·테두리를 시스템과 맞출 수 있다 |
| `old/src/askpass/font.rs` | 78 | `fc-match monospace` 로 Omarchy 폰트를 싣고 **8MB 넘으면 건너뛴다**. polkit 이 현지화 메시지를 보내므로 **CJK 폴백을 하나 더** 붙여야 한다 (§9) |
| `old/src/init.rs` | 355 | **systemd 유닛 설치의 본보기.** `add_block`/`remove_block`(마커 사이만 정확히 넣고 빼기), `write_snippet`, `reload_hyprland` 가 있다. §6 의 유닛 설치도 같은 규약을 따른다 |
| `old/src/main.rs` | 41 | 모드 분기. `--agent`·`--agent-prompt` 를 여기 더하되 **sudo 로 넘기지 않도록** 한다 (지금은 `--init`·`--uninit` 외 전부 sudo 행이다) |

### 대체되는 것

| | |
|---|---|
| `old/src/askpass/invocation.rs` (125) | `/proc/<ppid>/cmdline` 을 읽어 **실행될 명령**을 창에 띄운다. 에이전트에서는 부모가 sudo 가 아니라 우리 데몬이라 이 경로가 없다. 대신 polkit 이 주는 `message`·`action_id` 를 쓴다. **읽을 가치는 있다** — "무엇이 묻는지 보여준다" 가 왜 한 줄을 쓸 값어치인지가 주석에 있다 |

### 문서에서 볼 곳

| | |
|---|---|
| `old/docs/plan.md` §4-1~4-4 | 하드닝·stdout 격리·zeroize·faillock 대응의 **사양**. 새 사양은 이걸 옮겨 적는 것으로 시작한다 |
| `old/docs/rationale.md` §6 | 하드닝과 `panic = "abort"` 가 한 묶음인 이유 |
| `old/docs/rationale.md` §7 | **faillock 실측 기록.** §9 의 재측정은 이것과 비교하는 것이다 |
| `old/docs/rationale.md` §5 | `spawn`+`wait` 대신 `exec()` 를 쓰는 이유 — 자식 프로세스 설계(§4)에 그대로 걸린다 |
| `old/docs/rationale.md` §2·§3·§4·§8 | `sudo -A` 호출 규약, askpass 경로 지정, 프리체크를 두지 않는 이유. **에이전트에는 해당 없다** — sudo 경로를 건드릴 때만 본다 |

### 미리 답해 둘 질문 셋

읽으면서 결론을 내야 하고, 코드를 쓰기 전에 정해지는 것들이다.

1. **`PasswordChannel` 을 어떻게 일반화할 것인가** — 목적지 fd 를 인자로 받는가, 트레이트로 가르는가
2. **`attempts` 의 리셋 경계** — 인증 요청 하나가 곧 한 세션인가, 아니면 `cookie` 단위인가
3. **`gui::prompt()` 를 다회로 바꾸는 방법** — 창을 살린 채 프롬프트만 갈아 끼우는 API 모양

### 참고

루트의 `install.sh` 는 **`old/install.sh` 로 넘기는 한 줄짜리**다. `omarchy-setup` 이 이 파일을
이름으로 부르기 때문이고, 새 빌드가 자리를 잡으면 지운다.
