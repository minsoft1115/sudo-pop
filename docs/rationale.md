# sudo-pop — 설계 근거와 실측

> **이 문서의 역할**: 왜 그렇게 만들었는지, 무엇을 재 봤는지, 무엇을 기각했는지.
> 구현 사양은 [`plan.md`](plan.md) 에 있다.
>
> 이것은 **작업 중에 쌓인 기록**이다. 계획으로 시작해 단계마다 실측을 덧붙였고, 틀린
> 가정은 지우지 않고 무엇이 뒤집혔는지 남겼다. 절 번호는 그때의 것이라 순서가 사양과
> 나란하지 않다.
>
> 실측은 전부 2026-08-19, Omarchy 4.0 / Hyprland 0.56.2 / polkit 127 / systemd 261 기준이다.
>
> **`plan.md` §n, `rationale.md` §n 으로 참조하는 것은 옛 구현의 문서다** —
> `old/docs/` 아래에 있고, 링크가 걸린 곳은 그쪽을 가리킨다. 새 사양은 언제나
> [`plan.md`](plan.md) 다.
>
> 옛 구현(sudo askpass 래퍼)의 소스와 문서는 `old/` 에 그대로 있다.

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

부수 효과가 하나 더 있다. 지금 하드닝(`old/docs/plan.md` §4 — 코어덤프 차단, mlock, 화면 공유 제외,
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

### 3-4. 발신자를 검증한다 — 참고 구현 둘 다 안 한다

`BeginAuthentication` 은 **우리가 시스템 버스에 내놓은 메서드**다. 그 자리에 폴킷만
오라는 법이 없다. `hyprpolkitagent` 의 `onBeginAuthentication` 은 인자를 받아 바로 창을
띄운다 — 누가 불렀는지 보지 않는다. Omarchy 쪽도 Quickshell 에 맡기고 있어 사정이 같다.

검증하지 않으면 **세션 안의 아무 프로세스나** 우리 창을 띄울 수 있다.

| | |
|---|---|
| 문구를 공격자가 정한다 | `message` 가 그대로 화면에 뜬다. 위조 창이 아니라 **진짜 에이전트가 그리는 진짜 창**이라 사용자가 구별할 방법이 없다 |
| 성공 여부가 샌다 | 결과가 `BeginAuthentication` 의 리턴으로 호출자에게 간다. 비밀번호는 안 가지만 "방금 친 것이 맞았다" 는 간다 |
| faillock 을 태운다 | 시도마다 PAM 이 돌고, 카운터는 sudo·로그인과 **공용**이다 (§9). 반복 유발로 계정을 잠글 수 있다 |

**그래서 폴킷이 부른 것만 받는다.**

- 발신자의 고유 이름이 `org.freedesktop.PolicyKit1` 의 현재 소유자와 같은가, 또는
- `GetConnectionUnixUser(sender) == 0` 인가

둘 중 하나로 확인하고, 아니면 **창을 띄우기 전에** 거절한다. 거절은 로그에 남긴다 —
이게 걸린다는 것은 정상 상황이 아니다.

소유자 이름은 캐시하지 말고 `NameOwnerChanged` 로 따라간다. polkitd 가 재시작하면 고유
이름이 바뀌고, 그때 §6 의 재등록도 같이 걸린다.

**실측(스파이크 1).** 진짜 요청의 `sender` 는 `:1.12` 로 왔고, 시작할 때
`GetNameOwner("org.freedesktop.PolicyKit1")` 로 얻은 값과 **같았다.** 비교 한 줄로 성립한다.

### 3-5. 실제로 오는 값 — 스파이크 1 기록

`omarchy.polkit` 을 잠깐 내리고 우리 스텁으로 `run0 true` 를 받아 본 결과다.

```
sender     : :1.12                     ← polkitd 의 고유 이름과 일치
action_id  : org.freedesktop.systemd1.manage-units
message    : Authentication is required to start transient unit
             'run-p1592228-i1586931.service'.
icon_name  : (빈 문자열)
cookie     : 71자
details    : { "polkit.caller-pid": "1", "polkit.subject-pid": "1592228" }
identity   : unix-user { uid }
```

여기서 나오는 것이 셋이다.

**1. `message` 를 그대로 띄우면 안 된다.** run0 의 문구에는 **무엇을 실행하는지가 없다** —
유닛 이름은 난수다. 사용자가 "지금 무엇에 비밀번호를 주는가" 를 판단할 근거가 화면에
하나도 없게 된다.

**2. 대신 `polkit.subject-pid` 가 온다.** 그 pid 의 `/proc/<pid>/cmdline` 을 읽으면 실행될
명령을 복원할 수 있다. `old/src/askpass/invocation.rs` 가 이미 그 일을 한다 — 지금은 부모
프로세스를 보지만, **보는 대상을 `subject-pid` 로 바꾸면 그대로 살아난다.**
읽기 전에 그 pid 가 아직 살아 있는지와 **소유자가 우리 uid 인지**는 확인한다.

> Omarchy 다이얼로그는 `command_line`·`cmdline` 키를 찾아본다. **run0 은 그 키를 안 보낸다** —
> 그래서 그쪽 창에는 유닛 이름만 뜬다. 우리는 `subject-pid` 로 한 단계 더 간다.

**3. `icon_name` 은 비어 있고 `identities` 는 하나뿐이다.** 아이콘 자리는 없어도 되는 설계로
두고, 신원 선택 UI 를 후순위로 둔 판단(§8-1)은 이 관측과 맞는다.

거절했을 때 `run0` 은 `Failed to start transient service unit: Access denied` 로 즉시
끝났다(exit 1). 매달리지 않는다. 비밀번호를 묻지 않았으므로 faillock 도 건드리지 않았다.

### 3-6. 헬퍼 왕복 — 스파이크 2 기록

터미널에서 묻는 스텁으로 `run0 true` 를 끝까지 돌려 봤다. **오답만 넣었다** — 성공 경로는
사람이 직접 쳐야 해서 남겨 뒀다.

```
chosen : lmh (uid 1000)
-- attempt 1/3 --  [hidden] Password:
-- attempt 2/3 --  [hidden] Password:
-- attempt 3/3 --  [hidden] Password:
out of attempts                      → Error.Failed → run0: "Access denied"
```

| 확인된 것 | |
|---|---|
| 소켓 헬퍼 | `/run/polkit/agent-helper.socket` 에 `사용자이름\n쿠키\n` 을 보내니 **프롬프트가 왔다.** 이 커널은 pidfd 를 주므로 fork 폴백까지 가지 않았다 — 폴백은 그대로 두되, **이 머신에서는 검증되지 않은 경로**다 |
| 줄 프로토콜 | `PAM_PROMPT_ECHO_OFF Password:` → 답 → `FAILURE`. 태그와 본문 사이 공백 하나 규칙대로 |
| 신원 | `unix-user` 의 `uid` → `getpwuid` → `lmh`. 현재 사용자 우선(§3-2)이 그대로 맞았다 |
| 재시도 | 헬퍼는 시도마다 죽는다. 새로 띄우면 다시 묻는다. **3회에서 정확히 멈춘다** (§4-1) |
| 발신자 거절 | `busctl` 로 폴킷이 아닌 곳에서 직접 부르니 **창을 띄우기 전에 `Access denied`** (§3-4) |
| `details` 키 순서 | 실행마다 다르다. 맵이므로 당연하지만 **순서에 의존해 파싱하지 말 것** |

**성공 경로도 확인했다** (사람이 직접 입력).

```
== BeginAuthentication ==  20:39:47
  SUCCESS  (2 초 경과, 20:39:49)
unregistered
run0 exit=0        ← 인증만 통과한 게 아니라 명령이 실제로 실행됐다
```

`AuthenticationAgentResponse2` 를 **우리는 한 번도 부르지 않았다.** 헬퍼가 root 로 보냈고
polkitd 가 그것으로 허가했다 — §3-2 의 전제가 실물로 확인됐다.

#### 호출자는 25초만 기다린다

첫 시도는 실패했는데, 이유가 프로토콜이 아니라 **시간**이었다. 답이 늦으면 이렇게 끝난다.

```
run0: Failed to start transient service unit: Connection timed out
→ 호출자가 포기하기까지 25초        (일부러 늦게 답해 실측)
```

거절과 구별된다 — 거절은 `Access denied` 다. 25초는 sd-bus 의 기본 메서드 타임아웃이고,
**우리가 늘릴 수 없는 값이다.** 포기한 뒤 polkitd 는 `CancelAuthentication` 을 보내 온다
(그것도 실물로 받았다).

여기서 두 가지가 따라 나온다.

- **창의 타임아웃을 90초로 두는 것은 의미가 없다** (`old/src/askpass/gui.rs` 의 `TIMEOUT`).
  25초가 지나면 사용자가 무엇을 입력하든 호출자는 이미 떠났다. 창은 `CancelAuthentication`
  을 받는 즉시 닫고, 자체 타임아웃은 그보다 조금 길게만 둔다
- **`CancelAuthentication` 이 성공 뒤에 올 수 있다.** 처리는 멱등이어야 한다 — 첫 시도에서
  `SUCCESS` 다음에 취소가 도착했고, 그때 아무 일도 일어나지 않아야 맞다
- sudo 경로에는 이 제한이 없다. sudo 는 askpass 를 기다린다. **같은 창인데 경로에 따라
  주어진 시간이 다르다**는 사실을 UI 가 알고 있어야 한다

**faillock 산수를 바로잡는다.** 오답 3회가 `SVC polkit-1` 로 공용 tally 에 쌓였고,
`unlock_time=120` 은 **잠긴 뒤 풀리기까지의 시간이지 실패가 쌓이는 창이 아니다.** 실측에서
3분이 지나도 항목이 `V`(유효)로 남아 있었다. 즉 테스트로 태운 실패는 한참 남고,
`deny=10` 까지의 거리도 그만큼 오래 좁아진 채로 있다. 쿠키 단위 상한(§4-1)이 없으면
요청 몇 번으로 계정이 잠긴다는 뜻이다.

정리는 `faillock --reset` 으로 된다 — tally 파일이 사용자 소유라 root 가 필요 없다.

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

### 4-1. 시도 횟수는 쿠키 단위로 센다

`old/src/attempts.rs` 는 **sudo 명령 하나**를 단위로 삼는다 — `wrapper.rs` 가 sudo 를 exec
하기 전에 카운터를 지우고, askpass 가 제출할 때마다 올린다. 에이전트에는 그 "명령 시작"
지점이 없다. 그대로 옮기면 두 가지로 망가진다.

- **리셋을 안 하면** 예전 요청의 카운트가 남아 정상 재시도가 일찍 막힌다
- **요청마다 무조건 리셋하면** 3회 상한이 요청 단위로 리셋되므로, 요청을 반복 유발해
  `deny=10` 을 태우는 것을 아무것도 막지 못한다

**`cookie` 하나를 sudo 명령 하나로 본다.** 폴킷은 인증 요청마다 새 쿠키를 준다.

| | |
|---|---|
| 리셋 | 새 쿠키에 대한 `BeginAuthentication` 을 받았을 때 |
| 증가 | 자식이 비밀번호를 헬퍼에 넘길 때마다 |
| 상한 | `MAX_ATTEMPTS = 3` (그대로) |
| 잠긴 계정 | 창을 띄우지 않고 `notice()` 로 알린다 — 물어봐야 실패만 쌓인다 |

**세는 자리는 자식 프로세스다.** 자식이 쿠키 하나당 하나 뜨고 재시도 루프를 자기가 돌리므로,
그 안의 카운터가 곧 쿠키 단위다. 데몬은 상태를 하나도 들고 있지 않아도 된다 — 계획 단계에서는
"데몬 메모리" 라고 적었는데, 3단계에서 자식이 대화를 통째로 갖는 구조가 되면서 더 단순해졌다.
`$XDG_RUNTIME_DIR/sudo-pop/attempts` 파일은 sudo 가 askpass 를 매번 새 프로세스로 부르기
때문에 필요했던 것이라 **sudo 경로 전용으로 남긴다.**

faillock 예산 조회(`attempts::budget()` — `deny` 읽기, tally 파싱)는 시스템 상태를 보는 것이라
양쪽 경로에서 그대로 쓴다.

> 이 결정은 §3-4 와 짝이다. 발신자 검증이 **요청을 만들 수 있는 쪽**을 폴킷으로 좁히고,
> 쿠키 단위 상한이 **요청 하나가 태울 수 있는 양**을 좁힌다. 둘 중 하나만으로는 부족하다.

### 4-2. 창을 붙이고 나서 — 스파이크 3 기록

`--agent-prompt` 자식을 만들어 실제 인증까지 돌렸다.

```
== BeginAuthentication ==  20:53:22
  chosen : lmh (uid 1000)
  SUCCESS  (6 초 경과)
unregistered
run0 exit=0
```

창은 이렇게 뜬다 (`sleep 300` 을 subject 로 놓고 찍은 것).

```
        sleep 300            ← subject-pid 의 cmdline, 강조색
        Password:            ← 헬퍼가 보낸 PAM 프롬프트
   [                    ]
  Enter to confirm  Esc to cancel
```

`hyprctl clients` 로 확인한 창은 `class=sudo-askpass size=[400,200] floating=true` —
옛 app-id 그대로라 `old/assets/sudo-pop.lua` 의 규칙이 그대로 걸린다.

| 성립한 것 | |
|---|---|
| 비밀번호가 데몬을 통과하지 않는다 | 데몬은 자식을 띄우고 **종료 코드만** 본다. 쿠키는 argv·환경이 아니라 **파이프**로 넘긴다 |
| 다회 대화 | 헬퍼 스레드 ↔ 창을 채널로 잇는다. winit 이 프로세스당 이벤트 루프를 하나만 주므로 창이 메인 스레드를 갖고, 대화가 옆 스레드로 간다 |
| 답을 쓰는 방식 | `Secret` 을 그대로 헬퍼 fd 에 **원시 두 번 쓰기**로 보낸다. `writeln!` 로 포맷하면 지워지지 않는 사본이 생긴다 |
| 창 타임아웃 | 90초 → **30초**. 호출자가 25초에 포기하므로 그 뒤를 받는 백스톱일 뿐이다 |

**알려진 구멍이었던 것.** `BeginAuthentication` 이 블로킹하는 동안 `CancelAuthentication`
이 처리되지 않았다 — zbus 블로킹 API 는 메서드를 직렬로 돌린다. 4단계에서 async 로 바꿔
해결했다 (§4-3).

### 4-3. 취소·큐·재등록 — 스파이크 4 기록

zbus 를 async 로 바꿨다. 요청 하나가 사람을 기다리는 동안(최대 25초) 그 요청에 대한
취소가 들어올 수 있으니, 두 메서드가 동시에 돌아야 한다.

```
== BeginAuthentication ==  21:08:44      ← 자식을 기다리는 중인데
== CancelAuthentication ==  21:08:52     ← 취소가 들어와 처리된다
  closed the prompt (pid 1681556)
  exit 2  (8 초 경과)
```

| 확인된 것 | |
|---|---|
| 취소가 창을 닫는다 | 쿠키 → 자식 pid 를 들고 있다가 `SIGTERM`. **30초 백스톱을 기다리지 않는다** |
| 창은 한 번에 하나 | 요청 둘을 1초 간격으로 넣고 세어 보니 `창 수: 1`. 두 번째 자식은 첫 번째가 끝난 뒤에 떴다 |
| 종료 코드 매핑 | 취소는 `exit 2` → `Ok()`. 에러로 돌려주면 polkitd 가 요청을 되던진다 (§3-3 의 함정) |

**큐는 잠금 하나다.** `turn` 을 요청 전체 동안 잡으므로 두 번째 요청은 자연히 기다린다.
자료구조를 따로 두지 않았다 — 어차피 화면에 한 번에 하나만 띄울 것이라면 대기열의 순서를
우리가 관리할 이유가 없다.

**`SIGTERM` 을 쓰는 이유**: 자식은 창과 헬퍼 대화를 갖고 있고, 죽으면 둘 다 정리된다.
데몬이 자식의 내부를 알 필요가 없다는 §4 의 분리가 여기서도 그대로 적용된다.

**polkitd 재시작 대응도 같이 넣었다.** `NameOwnerChanged` 를 구독해서, 폴킷이 돌아오면
**발신자 검증 기준(고유 이름)을 갱신하고 다시 등록한다.** 이 둘은 한 몸이다 — 이름이
바뀌었는데 검증 기준만 옛것으로 남으면 진짜 폴킷의 요청을 우리가 거절하게 된다.
(재시작 자체는 나중에 실측했다 — §17-3.)

---

## 5. 파일과 모드

실제로 이렇게 됐다 (계획보다 평평하다 — 나눌 만큼 크지 않았다).

```
src/main.rs        모드 분기, 세션 id 3단계, 등록/해제, NameOwnerChanged
src/agent.rs       D-Bus 서비스 — 발신자 검증, 신원 선택, 큐, 자식 실행
src/helper.rs      헬퍼 연결(소켓/fork)과 PAM 줄 프로토콜
src/prompt.rs      --agent-prompt (자식) — 하드닝 + 창 + 대화 + 종료 코드
src/gui.rs         창 (요청당 하나, 채널로 갱신)
src/secret.rs · harden.rs · theme.rs · font.rs · invocation.rs   old/ 에서
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
이 문서에 남긴다.

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

### 6-1. 설치 실측 — 스파이크 5 기록

`--init` / `--uninit` 을 만들어 돌렸다.

```
$ sudo-pop --init
wrote ~/.config/minsoft1115/hypr/sudo-pop.lua
added the window rules to ~/.config/hypr/hyprland.lua
reloaded Hyprland
wrote ~/.config/systemd/user/sudo-pop-agent.service

omarchy.polkit (the Omarchy shell's own agent) already holds this session's polkit seat.
The unit is installed but not enabled. To switch:
  omarchy plugin disable omarchy.polkit
  sudo-pop --init
```

| 확인된 것 | |
|---|---|
| 충돌 시 | 유닛은 깔되 **enable 하지 않는다.** `systemctl --user is-enabled` 가 `disabled` |
| 감지 순서 | `omarchy.polkit` 이 1순위. 프로세스 목록에 안 보이므로 플러그인 목록을 본다 (§6) |
| 멱등성 | 두 번째 `--init` 은 `already current` 만 찍고 파일을 안 건드린다 |
| 왕복 | `--uninit` 뒤 `hyprland.lua` 에서 **마커 4줄만** 빠진다 |

**셸 스니펫은 없앴다.** 옛 `--init` 은 `~/.bashrc` 의 로더 블록과 `alias sudo='sudo-pop'` 을
다뤘는데, 에이전트는 사용자가 부르는 것이 아니라 polkitd 가 부른다. `~/.bashrc` 는 이제 우리
일이 아니고, omarchy-setup 의 bash 단계와 겹칠 일도 없다.

**`ExecStart` 는 `--init` 을 실행한 바이너리의 절대 경로가 박힌다.** 개발 트리에서 그대로
`--init` 하면 `cargo build` 가 도는 동안 에이전트가 죽거나 옛 코드로 돈다. 상시로 쓸 것이면
`~/.local/bin` 에 복사한 뒤 그쪽에서 `--init` 한다.

---

### 6-2. 화면 공유 제외가 실측됐다

`--init` 이 창 규칙을 깔고 나니 `grim` 으로 찍은 스크린샷에서 창 자리가 **검은 사각형**으로
나온다. 규칙이 없던 3단계 스모크 테스트에서는 내용이 그대로 찍혔다.

```
창 규칙 없음  → 스크린샷에 "sleep 300 / Password:" 가 보인다
창 규칙 설치  → 같은 자리에 검은 사각형
```

`no_screen_share` 가 wlr-screencopy 를 막는 것이고, 사용자 눈에는 정상으로 보인다. §2-2 에서
"우리만 가진 것" 으로 센 항목이 실물로 확인됐다 — Omarchy 의 레이어 서피스에는 이 규칙을
걸 방법이 없다.

개발 중에 창 내용을 봐야 하면 규칙을 잠깐 빼야 한다는 뜻이기도 하다.

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

### 7-1. 구현됨 — 스파이크 6 기록

```
$ SUDO_POP_DEBUG=1 sudo-pop -n true
sudo-pop: caller passed -A/-n/-S, leaving arguments untouched   → 평범한 sudo

$ SUDO_POP_DEBUG=1 sudo-pop true
sudo-pop: plain command, routing to run0                        → polkit → 우리 에이전트 창
```

판정은 `plain_command()` 하나다 — `command_start()` 가 0 이고 첫 인자가 `NAME=값` 이 아니면
run0. 단위 테스트로 굳혔다 (옵션·환경 할당·`--`·`-name=x` 같은 헷갈리는 인자까지).

폴백 순서도 그대로다: 인자 없음 → sudo, `-A`/`-n`/`-S` → sudo, `SUDO_POP_RUN0=0` → sudo,
디스플레이 없음 → sudo, 런타임 디렉터리·심볼릭 링크 실패 → sudo. **run0 이 없거나 exec 에
실패해도 sudo 로 떨어진다** — 팝업을 못 띄우는 것보다 sudo 를 못 쓰는 것이 훨씬 큰 문제다.

askpass 모드도 돌아왔다. 창 코드는 에이전트와 **같은 것을 쓴다** (§5) — 대화가 한 번뿐이고
답이 헬퍼 대신 sudo 가 읽는 fd 로 간다는 것만 다르다. 잠긴 계정이면 묻지 않고, 남은 시도가
적으면 창에 경고를 띄운다.

---

## 8. 단계

각 단계가 끝날 때마다 실제로 확인 가능한 것을 남긴다.
**여섯 단계 모두 끝났다** (§3-5, §3-6, §4-2, §4-3, §6-1, §7-1). 남은 것은 후순위(§8-1)뿐이다.

1. **스파이크 — 등록만.** zbus 로 시스템 버스에 붙어 등록하고 `BeginAuthentication` 을
   **로그만** 찍는다. `run0 true` 를 쳐서 호출이 오는지, `identities` 와 `details` 에 뭐가
   들어오는지 눈으로 본다. 세션 id 3단계(§3-1)도 여기서 확인한다
2. **헬퍼 왕복.** 소켓 경로로 PAM 대화를 붙이고 비밀번호를 **stdin 으로 읽어** 성공/실패를
   확인한다. fork 폴백과 §3-3 의 세 가지 대응까지 넣는다. 프로토콜 리스크는 여기서 끝난다
3. **GUI 연결.** 자식 프로세스를 분리하고 `gui::prompt()` 를 붙인다. 창을 살린 채 프롬프트를
   갈아 끼우는 경로를 만든다
4. **가장자리.** 취소, 신원이 여럿일 때, 요청 큐, polkitd 재시작 후 재등록
5. **포장.** user 유닛과 `--init`/`--uninit` 연동(§6), 다른 에이전트 감지, README 와 문서 정리
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
- **폴킷이 아닌 곳에서 `BeginAuthentication` 을 부르면 거절되는가** (§3-4).
  `busctl --system call <우리 고유이름> <경로> org.freedesktop.PolicyKit1.AuthenticationAgent
  BeginAuthentication ...` 로 직접 쳐 본다. 창이 뜨면 실패다
- 오답 3번 뒤 **네 번째 프롬프트가 안 뜨는가**, 그리고 같은 요청에서만 그런가 (§4-1)
- 계정이 잠긴 상태에서 요청이 오면 **창 대신 안내가 뜨는가**
- `--init` → 로그아웃 → 로그인에서 에이전트가 **자동으로 떠 있는가**
- `--init` → `--uninit` 왕복 뒤 유닛 파일과 `enable` 링크가 **둘 다** 사라지는가
- `hyprpolkitagent` 를 켜 둔 채 `--init` 하면 **안내를 내고 enable 하지 않는가**
- 등록 충돌로 끝났을 때 `Restart=on-failure` 가 재시도 고리를 만들지 않는가
- **faillock — 카운터는 공유된다.** `/usr/lib/pam.d/polkit-1` 이 `system-auth` 를 include 하고,
  그 스택의 `pam_faillock` 은 `/run/faillock/<user>` **한 파일**에 쌓는다. 서비스 이름은
  기록만 될 뿐 칸이 갈리지 않는다. 실제로 이 머신의 기록에 `SVC polkit-1` 행이 남아 있다.
  즉 **polkit 에서 틀린 것이 sudo 를 잠근다.** `old/docs/plan.md` §4-4 의 대응이 에이전트 경로에도
  그대로 필요하다 — 재측정은 확인용이지 판단 근거가 아니다

---

## 10. 결정과 남은 것

### 결정됨

| | |
|---|---|
| 헬퍼 경로 | 소켓 먼저, fork+exec 폴백 **필수** (§3-3) |
| `--init` | systemd user 유닛을 **깔고 enable 까지** 한다 (§6) |
| 다른 에이전트가 있을 때 | 유닛은 깔되 **enable 하지 않고 안내**한다 (§6) |
| zbus 의존성 | **받아들인다.** 빌드 시간 증가는 감수한다. 실측치는 남겨 이 문서에 기록 |
| 스마트 라우터 | 에이전트가 도는 것을 본 뒤에 켠다. 되돌릴 스위치 `SUDO_POP_RUN0=0` 을 둔다 (§7) |
| 신원이 여럿일 때 | 1차는 현재 사용자 우선, 없으면 첫 번째. 선택 UI 는 후순위 (§8-1) |
| 대상 환경 | **Omarchy 4.0+ / Hyprland 0.56+ 전용.** 버전 분기와 폴백을 두지 않는다 (§2-3) |
| **발신자 검증** | **필수.** 폴킷이 부른 것만 받는다 (§3-4). 참고 구현 둘 다 안 하는 부분이다 |
| **시도 횟수 경계** | **쿠키 단위.** 데몬 메모리에서 세고, 파일 카운터는 sudo 경로에 남긴다 (§4-1) |
| 창에 무엇을 띄우나 | polkit 의 `message` 가 아니라 **`polkit.subject-pid` 의 cmdline** 을 앞세운다 (§3-5) |
| 창 타임아웃 | polkit 경로는 **호출자가 25초에 포기한다** (§3-6). 90초 타임아웃은 그 뒤에 의미가 없다 |
| 창 서피스 | 지금의 xdg_toplevel + Hyprland 창 규칙 그대로. 레이어셸은 후순위 (§2-4) |
| 덮개·테마 | Omarchy 것을 그대로 쓴다 — `omarchy-hw-laptop-closed`, `shell.toml` 의 `[polkit]` (§2-3) |

| 시도 횟수 정책 | **`attempts` 게이팅을 에이전트 경로에도 건다.** 카운터가 공유된다는 것이
설정과 실제 기록으로 확인됐다 (§9). 값은 `old/docs/plan.md` §4-4 를 따르고, 재측정은 확인용으로만 한다 |

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
| `hyprctl keyword windowrulev2 ...` 로 창 규칙을 주입 | Hyprland 0.56 은 `keyword` 로 오는 윈도우 룰을 거부한다. `old/docs/plan.md` §1 의 금지 사항이고, 정적 Lua 규칙이 이미 그 자리를 대신한다 |
| `--init` 이 `~/.bashrc`·`~/.zshrc` 에 `alias sudo=...` 를 직접 덧붙인다 | 지금은 스니펫을 `~/.config/minsoft1115/bash/` 에 두고 **공유 로더 블록**만 건드린다. 다른 도구와 같은 폴더를 쓰기 때문이고, 직접 덧붙이면 그 규약이 깨진다 |
| `exec-once = sudo-pop --polkit` 을 `hyprland.conf` 에 안내 | 두 가지가 틀렸다. 이 환경의 Hyprland 설정은 **Lua**(`hyprland.lua`)이고, 에이전트는 **systemd user 유닛**으로 떠야 한다 — 세션 id 를 `User.Display` 로 찾는 경로(§3-1)가 거기에 맞다 |
| 앱 id 를 `sudo-pop-gui` 로 | 설치된 창 규칙이 `^(sudo-askpass)$` 로 매칭한다. 바꾸면 규칙이 통째로 안 걸린다 |
| `opt-level = "z"` 로 바꾸고 "역공학 방지" | 최적화 수준은 역공학과 무관하다. 지금 `opt-level = 3` 은 GUI 응답성 때문에 고른 값이고, `strip`·`lto`·`panic = "abort"` 는 이미 들어가 있다 (`panic = "abort"` 는 하드닝과 한 묶음이다 — `old/docs/rationale.md` §6) |
| 창이 비밀번호를 **stdout 에 출력** | askpass 모드는 이미 stdout 을 격리하고 따로 떼어 둔 fd 에 쓴다 (`old/docs/plan.md` §4-2). 에이전트 자식은 아예 stdout 으로 안 보내고 헬퍼로 보낸다 |

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
| `old/src/askpass/invocation.rs` | 125 | **살린다.** `/proc/<pid>/cmdline` 을 읽어 실행될 명령을 창에 띄우는 코드. 보는 대상만 부모 프로세스 → `polkit.subject-pid` 로 바꾼다 (§3-5). polkit 의 `message` 에는 명령이 없으므로, 이 한 줄이 창에서 가장 중요한 정보가 된다 |
| `old/src/main.rs` | 41 | 모드 분기. `--agent`·`--agent-prompt` 를 여기 더하되 **sudo 로 넘기지 않도록** 한다 (지금은 `--init`·`--uninit` 외 전부 sudo 행이다) |

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

---

## 15. 시험

세 층으로 나뉜다. 위로 갈수록 환경을 더 요구하고, 아래로 갈수록 자주 돌린다.

```
cargo test                  99 단위 + 8 통합 — 환경 없이 돈다
tests/scenarios.sh          49 시나리오 — polkitd·버스·컴포지터가 필요하다
tests/scenarios.sh --with-password    위 + 사람이 비밀번호를 넣는 한 케이스
tests/scenarios.sh --restart-polkitd  위 + polkitd 를 실제로 재시작한다
cargo run --release --example font-cost   폰트 비용 실측 (§16-3)
```

### 단위·통합 (`cargo test`)

`tests/fake-helper.sh` 가 `polkit-agent-helper-1` 행세를 한다. `SUDO_POP_HELPER_BIN` 과
`SUDO_POP_HELPER_SOCKET` 으로 두 문을 다른 곳에 걸 수 있어서, **PAM 도 비밀번호도 root 도
없이** 대화의 갈래를 전부 돌린다.

> **디버그 빌드에서만 걸린다** (audit C2). 릴리스 바이너리는 이 변수를 아예 안 읽으므로
> 손시험에 쓰면 진짜 PAM 이 돌아 faillock 을 태운다 — §21-6 에서 두 번 겪었다.

| 잡는 것 | |
|---|---|
| 성공·오답·`ECHO_ON`·`TEXT_INFO`·`ERROR_MSG` | 줄 프로토콜의 각 태그 |
| **프롬프트 전 `FAILURE` 는 실패가 아니라 거절** | 이걸 틀리면 창이 무한히 다시 뜬다 (§3-3) |
| **창을 닫으면 취소** | 오답으로 세면 faillock 을 태운다 |
| **소켓이 조용히 닫히면 fork 로 폴백** | 이 커널에서는 실물로 재현이 안 되는 경로다 (§3-6) |
| 헬퍼에 사용자·쿠키가 전달되고, 쿠키는 argv 가 아니라 stdin 으로 | |
| 라우팅 판정 — 옵션·환경 할당·`--`·`-name=x` | `sudo VAR=1` 이 run0 로 새면 변수가 사라진다 |

### 시나리오 (`tests/scenarios.sh`)

Hyprland 와 실제 polkitd 를 시험 장비로 쓴다. 세션을 원래대로 돌려놓고 끝나며 —
중간에 실패하거나 끊겨도 마찬가지다 — **무엇을 되돌렸는지 찍는다.** 조용히 실패하면
세션이 인증을 못 하는 채로 남기 때문이다.

```
되돌리기
  sudo-pop-agent: active
  omarchy.polkit: false
  faillock: 3건 정리
```

**faillock 도 치운다.** 가짜 쿠키로 띄운 창을 닫으면 헬퍼가 실패로 세므로 한 번 돌 때마다
서너 건이 쌓인다. 공유 카운터라(§9) 남겨 두면 진짜로 필요할 때의 여유가 줄어든다.

`hl.dsp.send_shortcut` 으로 **Esc 를 창에 주입**할 수 있어서, 사용자가 취소하는 경로까지
자동으로 돈다 (비밀번호가 필요 없다). `hyprctl clients` 로 창 규칙(floating·pin·크기)을
확인하고, `grim` + `identify` 로 화면 캡처 제외를 센다.

보안 쪽에서 자동으로 확인하는 것:

| | 어떻게 |
|---|---|
| 폴킷이 아닌 발신자는 거절되고 **창도 안 뜬다** | `busctl` 로 직접 호출 |
| 쿠키가 argv 에 없다 | 자식의 `/proc/<pid>/cmdline` |
| **자식의 `environ` 을 읽을 수 없다** | `PR_SET_DUMPABLE=0` 이면 `/proc` 항목이 잠긴다 |
| 코어덤프 한도가 0 | `/proc/<pid>/limits` |
| 비밀번호 버퍼가 메모리에 잠겨 있다 | `/proc/<pid>/status` 의 `VmLck` |
| **화면 캡처에 안 찍힌다** | 같은 자리를 찍어 색 수를 센다 — 규칙을 빼면 **1101색**, 걸면 **4색** |
| 요청이 겹쳐도 창은 하나 | 두 요청을 동시에 넣고 센다 |
| `--uninit` 이 남의 것을 안 지운다 | 공유 로더 블록이 남아 있는지, `hyprland.lua` 의 다른 줄이 그대로인지 |

### 사람이 있어야 하는 것

성공 경로 하나뿐이다. `--with-password` 를 주면 **foot 창을 띄워** 거기서 입력받고,
`run0` 이 실제로 명령을 실행했는지(`exit=0`)까지 본다.

---

## 16. 한글이 두부로 나오던 것 — 실측과 고침

`audit.md` 의 L3 는 "CJK 폰트 8MB 상한으로 현지화 메시지가 깨질 수 있음" 한 줄이었다.
파고 보니 **상한은 원인이 아니었고, 조건도 현지화가 아니었다.**

### 16-1. 무엇이 깨졌나

egui 에는 **글리프 단위 폴백이 없다.** 체인은 우리가 `FontDefinitions` 에 넣은 목록이
전부다. 터미널·GTK·브라우저가 한글을 잘 그리는 것은 못 그리는 글리프마다 fontconfig 에
다시 물어보기 때문인데, `font.rs` 는 `fc-match monospace` 를 **한 번** 묻고 그 답만 들고 있었다.

체인에 무엇이 있었는지 세어 봤다.

| | 한글 |
|---|---|
| `omarchy-font-list` 의 다섯 후보 — Adwaita Mono, iA Writer Mono S, JetBrainsMono NF, Liberation Mono, Nimbus Mono PS | **전부 없음** |
| egui 번들 넷 — Hack, Ubuntu-Light, NotoEmoji, emoji-icon-font | **전부 없음** |

`omarchy-font-set` 은 터미널 모노스페이스를 고르는 도구라 CJK 는 애초에 후보가 아니다.
시스템에 `noto-cjk` 는 깔려 있다 — Omarchy 가 그것을 `monospace` 로 가리키지 않을 뿐이다.
그래서 **시스템 전체에서 한글이 멀쩡한데 우리 창에서만 깨지는** 모양이 된다.

깨지는 모양은 epaint 의 대체 글리프다.

```rust
const PRIMARY_REPLACEMENT_CHAR: char = '◻';  // epaint-0.36.1/src/text/fonts.rs:643
```

즉 `sudo vim 계획서.md` 는 창 맨 윗줄에 `vim ◻◻◻.md` 로 떴다. 하필 그 줄이 "지금 무엇에
비밀번호를 주는가" 를 알려 주는 정보고, README 가 omarchy.polkit 대비 우위로 내세운 항목이다.

**로케일과 무관하다.** L3 가 상정한 것은 `locale` 을 넘겨 받는 polkit 의 번역 메시지였는데,
`gui.rs` 는 `message` 보다 **subject-pid 의 cmdline** 을 앞세운다 (§3-5). cmdline 은 사용자가
친 것이라 `LANG=en_US.UTF-8` 인 이 머신에서도 한글 파일명 하나로 바로 재현된다.

### 16-2. 고친 방식 — 필요할 때만 늘어나는 체인

`font::Chain` 이 생겼다. Omarchy 면이 맨 앞에 서고, **우리가 쓰지 않은 글자**(cmdline,
polkit 의 message, PAM 프롬프트)에 ASCII 밖 문자가 있을 때만 뒤에 한 면이 더 붙는다.

```
foreign_chars("vim 계획서.md")  →  ['계','획','서']       ASCII 는 버린다
charset_query(..)              →  ":charset=ACC4 D68D C11C"
fc-match                       →  Noto Sans CJK JP        체인 꼬리에
```

결정 세 가지.

- **스크립트를 우리가 판정하지 않는다.** 어떤 Nerd Font 가 무엇을 담는지 표를 들고 있느니
  fontconfig 에 문자를 그대로 넘긴다. 한글·일본어·아랍어가 같은 코드로 처리된다
- **fontconfig 이 이미 가진 파일을 답하면 아무것도 안 한다.** `café` 는 Omarchy 면이 이미
  덮으므로 fc-match 가 그 파일을 되돌려 주고, 바이트를 **읽기 전에** 걸러진다
- **폴백에는 8MB 상한을 걸지 않는다.** 설치된 한글 면 중 가장 작은 것이 16.7MB 고
  (`NotoSansCJK-Thin.ttc`), 상한을 걸면 두부가 그대로 남는다. 상한은 매 실행 파싱하는
  **주 폰트에만** 남긴다

**함정 하나.** 한글 면을 찾는 뻔한 질의가 이 시스템에서 거짓말을 한다.

```
fc-match ":lang=ko"          → Liberation Sans     ← 한글 없음
fc-match "monospace:lang=ko" → JetBrainsMono NF    ← omarchy 의 monospace 규칙이 :lang 을 이긴다
fc-match ":charset=AC00"     → Noto Sans CJK JP    ← 맞다
```

`:charset=` 으로 물어야 한다. 반환된 것이 `KR` 이 아니라 `JP` 면인 것은 상관없다 —
Noto CJK 의 지역별 면은 한글 자모를 공통으로 싣고, 한자 기본 자형만 다르다.

### 16-3. 얼마나 드나 — 실측

`cargo run --release --example font-cost` 로 부분별 비용을, Hyprland 의 `openwindow`
이벤트로 실제 창이 뜨는 시각을 쟀다 (2026-08-20, 같은 머신).

| | min | mean |
|---|---|---|
| `fc-match monospace` | 6.28 ms | 6.53 ms |
| `fc-match :charset=<한글 3자>` | 6.24 ms | 6.38 ms |
| 주 폰트 읽기 (2.5MB, warm) | 0.12 ms | 0.36 ms |
| 폴백 읽기 (19.5MB, warm) | 1.84 ms | 4.19 ms |
| 폴백 읽기 (19.5MB, **cold**) | 4.81 ms | 5.09 ms |
| egui 파싱 — 번들만 | 0.07 ms | 0.11 ms |
| egui 파싱 — **+ 주 폰트 (지금까지)** | 0.20 ms | 0.34 ms |
| egui 파싱 — **+ 폴백까지** | 1.24 ms | 1.60 ms |
| 한 줄 배치 — 한글, 폴백 있음 | 1.51 ms | 1.56 ms |

**19.5MB 를 싣는 값이 겁났는데 파싱은 1.2ms 였다.** epaint 는 harfrust 로 테이블만 읽고
글리프는 쓸 때 굽는다. 진짜 비용은 폰트가 아니라 **`fc-match` 프로세스 하나(6.3ms)** 다.

실제 창까지 (`--agent-prompt`, 7회, `openwindow` 이벤트 기준):

```
ascii  (pacman -Syu)    min 37.9 ms   median 39.4 ms
korean (vim 계획서.md)   min 49.8 ms   median 51.5 ms
```

**한글일 때 12ms 늦게 뜬다. ASCII 경로는 이전과 완전히 같다.**

### 16-4. 비동기로 먼저 띄우지 않는 이유

"창을 먼저 띄우고 폰트는 뒤에서 싣자" 를 검토했고, **하지 않기로 했다.**

- 차이가 **12ms** 다. 60Hz 한 프레임(16.7ms)보다 짧다. 창은 어느 쪽이든 40~50ms 에 뜬다
- 얻는 것은 12ms 이르게 뜨는 창인데, 그 12ms 동안 화면에 있는 것은 **`vim ◻◻◻.md`** 다.
  그 줄은 이 창의 존재 이유라, 틀린 것을 먼저 보여 주고 고치는 것은 손해다
- 파싱(1.2ms)은 어차피 UI 스레드다. `FontsImpl::new` 가 체인의 모든 면을 한꺼번에 만들고,
  그것이 `set_fonts` 다음 `begin_pass` 에서 돈다. 옆 스레드로 뺄 수 있는 것은 `fc-match`
  와 파일 읽기뿐이고, 그마저 스레드·채널·대기 상태·리플로 한 프레임을 새로 들여야 한다

**창이 뜬 뒤에 오는 글자는 이미 그 경로로 돈다.** PAM 이 나중에 보내는 프롬프트·안내는
`drain()` 이 받을 때 `cover()` 를 거치고, 체인이 바뀌면 다음 프레임에 반영된다. 비동기가
실제로 필요한 자리는 거기였고, 거기에는 이미 들어가 있다.

### 16-5. 시험

`cargo test` 에 12개가 붙었다 (65 → 77). 대부분 순수 함수라 환경이 필요 없다.

| | |
|---|---|
| `foreign_chars` | ASCII 는 fc-match 를 부르지 않는다 / 한글만 추려낸다 / 중복 음절은 한 번 / 32자 상한 / 한·일·중 한 질의 |
| `charset_query` | `계획` → `:charset=ACC4 D68D` |
| `Chain::cover_with` | 폴백이 **꼬리**에 붙는다 (주 폰트가 계속 이긴다) / ASCII 는 조회조차 없다 / 이미 가진 파일이면 안 싣는다(`café`) / 두 번째 한글 프롬프트는 다시 안 싣는다 / 못 찾으면 체인 그대로 |
| **`a_korean_command_line_becomes_drawable`** | 진짜 fontconfig·진짜 폰트로 `Fonts` 를 만들어 `has_glyphs("vim 계획서.md")` 를 묻는다. 프레임이 쓰는 것과 같은 경로라 **실제로 그려진다는 것**을 증명한다. 한글 면이 없는 머신에서는 건너뛴다 |

실물로도 한 번 돌렸다 — cmdline 이 `vim 계획서.md` 인 프로세스를 subject 로 놓고
`--agent-prompt` 를 띄우니 `sudo-pop: fell back to Noto Sans CJK JP (19484784 bytes) for 계획서`
가 찍히고 (`SUDO_POP_DEBUG`) 창이 규칙대로 떴다.

> **여기서 audit C2 가 실물로 확인됐다.** 처음에 `target/release` 로 시간을 재다가
> `SUDO_POP_HELPER_BIN` 이 무시되는 바람에 **진짜 PAM 이 돌아 faillock 에 10건이 쌓였다.**
> 릴리스 빌드가 시험용 env 를 안 읽는다는 C2 의 고침이 그대로 동작한 것이다.
> `faillock --reset` 으로 치웠고, 측정은 `RUSTFLAGS="-C debug-assertions=yes"` 로
> 최적화는 유지한 채 오버라이드만 살린 빌드로 다시 했다.

---

## 17. 감지와 재등록 — L1·L5 를 닫다

`audit.md` 의 마지막 두 열린 항목이다. 하나는 감지가 **거의 아무것도 못 잡고 있었고**,
하나는 폴킷이 재시작하면 **진짜 폴킷을 영영 거절할 수 있는** 창이었다.

### 17-1. L1 — `pgrep -x` 는 15자에서 잘린다

`--init` 은 유닛을 켜기 전에 다른 에이전트가 자리를 쥐고 있는지 본다 (§6). 2순위 판정이
`pgrep -x <이름>` 이었는데, **`pgrep -x` 는 `/proc/<pid>/comm` 과 비교하고 comm 은 커널이
15자에서 자른다.** pgrep 이 직접 거부한다.

```
$ pgrep -x polkit-kde-authentication-agent-1
pgrep: pattern that searches for process name longer than 15 characters
       will result in zero matches
```

목록 다섯 개를 이 기준으로 세면 이렇게 된다.

| 이름 | 길이 | |
|---|---|---|
| `hyprpolkitagent` | 15 | 걸린다 (딱 경계) |
| `polkit-gnome-authentication-agent-1` | 35 | **절대 안 걸린다** |
| `polkit-kde-authentication-agent-1` | 33 | **절대 안 걸린다** |
| `lxpolkit` | 8 | 걸린다 |
| `mate-polkit` | 11 | 길이는 되지만 **패키지 이름이지 프로세스 이름이 아니다** (실행 파일은 `polkit-mate-authentication-agent-1`) |

즉 감지가 실제로 잡던 것은 `omarchy.polkit`(1순위)과 `hyprpolkitagent`·`lxpolkit` 뿐이었다.
`mate-polkit` 은 audit 가 L1 **[고침]** 으로 추가한 항목인데, 아무것도 안 하고 있었다.

**놓쳤을 때의 값이 싸지 않다.** `--init` 은 "enabled and started" 를 찍고, 에이전트는 등록에
실패해 종료 코드 0 으로 죽고, `Restart=on-failure` 는 걸리지 않는다. 사용자는 됐다는 말을
듣고 창은 안 뜨며 이유는 journal 한 줄에만 남는다.

**고친 방식 — 이름 표를 버린다.** 정확한 이름 목록은 유지도 안 되고 잘림에도 안 죽는다.
대신 **이름에 `polkit`/`policykit` 이 들어 있는가**로 본다. 잘려도 남는 부분이고, 새 에이전트가
나와도 목록을 고칠 일이 없다. polkit 자신(`polkitd`·`polkit-agent-helper`)과 우리는 뺀다.

| 순위 | 어떻게 | 왜 이것도 필요한가 |
|---|---|---|
| 1 | `omarchy-plugin-list --json` | 셸 안의 서비스라 프로세스도 유닛도 없다 |
| 2 | **`/proc` 를 직접 읽어** 우리 uid 의 comm 을 본다 | XDG autostart 로 뜬 것은 유닛이 없다. `pgrep` 을 안 쓰므로 잘림이 **보이고**, uid 로 거르니 root 의 `polkitd` 가 자동으로 빠진다 |
| 3 | 활성 user 유닛 이름 (**신설**) | 유닛 이름은 안 잘린다. 프로세스가 잠깐 없는 순간에도 자리는 그쪽 것이다 |

안내도 갈렸다. 옛 코드는 찾은 것이 무엇이든 `systemctl --user disable --now <이름>.service` 를
찍었는데, 프로세스로 찾았을 때 그 명령은 **존재하지 않는다.** 이제 유닛으로 찾았을 때만
그 명령을 내고, 프로세스면 그렇게 말한다.

### 17-2. L5 — 소유자를 먼저 읽고 구독을 나중에 했다

발신자 검증(§3-4)은 요청의 고유 이름을 polkitd 의 현재 고유 이름과 비교한다. 그 이름을
시작할 때 한 번 읽고 이후 `NameOwnerChanged` 로 따라가는데, **둘 사이가 벌어져 있었다.**

```
get_name_owner()  →  owner = ":1.12"
    ↓  logind 세션 조회 3단계 · 연결 구축 · 등록      ← 이 사이에 polkitd 가 재시작하면
receive_name_owner_changed()                          ← 그 신호는 아무도 안 듣는다
```

놓치면 검증 기준이 죽은 `:1.12` 로 남고 **진짜 폴킷의 요청을 계속 `AccessDenied` 로 거절한다.**
고유 이름은 재사용되지 않으므로 사칭은 불가능하다 — 보안이 아니라 가용성 문제다. 다만
**회복이 자동이 아니다**: 프로세스는 죽지 않으니 `Restart=on-failure` 도 안 걸리고, polkitd 가
한 번 더 재시작하지 않는 한 낡은 채로 남는다.

**고침은 순서 하나다.** 읽기 전에 구독한다. 표준 패턴이고, 창이 통째로 사라진다.

곁가지 둘을 같이 닫았다.

- **일찍 구독하면 이미 아는 소유자에 대한 신호가 올 수 있다.** 그걸로 재등록하면 polkitd 가
  `already exists` 를 돌려주고 에러 로그가 남는다. 들고 있는 이름과 다를 때만 움직인다
- **등록 실패를 두 갈래로 가른다.** 지금까지는 어떤 실패든 종료 코드 0 이었다 — 자리를 뺏긴
  경우에 `Restart` 폭주를 막으려던 것인데(§6), polkitd 가 잠깐 없어서 실패한 경우까지
  삼켜 세션이 조용히 에이전트 없이 남았다. **`already exists for the given subject` 일 때만**
  정상 종료하고, 나머지는 재시작이 고칠 수 있으므로 실패로 끝낸다

**실측 — 구독이 정말 살아 있는가.** 구독을 옮긴 연결(`probe`)이 오브젝트를 내놓는 연결과
다르므로 신호가 진짜 오는지 확인했다. 임시 로그를 넣고 에이전트를 띄운 뒤 시스템 버스를
흔들었다.

```
watching org.freedesktop.PolicyKit1 for owner changes     ← 구독 (1줄)
polkitd owns org.freedesktop.PolicyKit1 as :1.12          ← 읽기 (2줄)
our bus name: :1.2387
REGISTERED.
TEMPSIGNAL :1.2387        ← 우리 자신의 버스 이름이 생긴 신호. 구독이 그것보다 앞섰다
TEMPSIGNAL :1.2388 …      (11건)
```

**`:1.2387` 은 우리가 나중에 만든 연결의 이름이다.** 그보다 앞서 건 구독이 그 신호를 받았다는
것이, 곧 "구독 이후에 벌어지는 일은 잃지 않는다" 는 L5 가 필요로 하는 성질 자체다.
임시 로그는 빼고 이 기록만 남긴다 — 상시로 켜면 버스의 모든 연결이 줄로 찍힌다.

### 17-3. 시험

`cargo test` 에 9개 (77 → 86), `tests/scenarios.sh` 에 12개 (30 → 42).

**유닛** — 판정을 순수 함수로 뽑아 두었다.

| | |
|---|---|
| `looks_like_agent` | 7개 에이전트를 이름으로 알아본다 / **커널이 15자로 자른 이름도 알아본다**(옛 결함을 못으로 박는다) / 유닛 이름도 / `polkitd`·헬퍼·우리 자신은 아니다 / `polkit.service` 는 통짜로만 제외(부분 문자열로 빼면 `xfce-polkit.service` 가 같이 죽는다) / 대소문자·개행 무관 |
| `Seat::hint` | 종류마다 다른 안내. 프로세스일 때 `.service` 를 붙이지 않는다 |
| `seat_is_taken` | 자리를 뺏긴 경우에만 참. 타임아웃·이름 없음 등은 거짓 |

**시나리오 §8 (L1)** — 결함을 실물로 재현한다. `sleep` 을
`polkit-kde-authentication-agent-1` 이라는 이름으로 복사해 띄우면 comm 이 실제로
`polkit-kde-auth` 로 잘린다. 그 자리에서 `pgrep -x` 가 못 잡는 것을 먼저 보이고, 우리 감지는
잡는 것을 본다. 이어서 `systemd-run --user --unit=scenario-polkit-agent.service` 로
**프로세스 이름에는 단서가 없는**(comm 이 `sleep`) 경우를 만들어 3순위만 따로 시험하고,
마지막으로 아무도 없을 때는 평소대로 enable 되는지 — 감지가 과하게 걸리지 않는지 — 를 본다.

**시나리오 §9 (L5)** — 결함이 곧 순서였으므로 순서를 로그의 줄 번호로 못 박는다: 구독이
소유자 읽기보다, 그리고 등록보다 앞에 찍히는가. 재등록이 헛돌지 않는지도 같이 본다.

**`--restart-polkitd` — 돌렸고 통과했다 (2026-08-20).**

```
PASS  polkitd 를 재시작했다
PASS  새 소유자를 보고 다시 등록한다
PASS  재시작 뒤에도 진짜 폴킷 요청에 창이 뜬다 (검증 기준이 따라갔다)

결과: 45 통과, 0 실패
```

마지막 줄이 L5 의 실물 확인이다. 재시작으로 polkitd 의 고유 이름이 바뀌었는데도 그 뒤의
진짜 요청에 **창이 떴다** — 검증 기준이 따라갔다는 뜻이고, 낡은 채였다면 거절되어 창이
안 떴다. §4-3 이 "재시작 자체는 아직 실측하지 않았다" 로 남겨 둔 것이 여기서 닫힌다.

polkitd 를 진짜로 재시작하는 케이스를 새 플래그 뒤에 뒀다.
`run0 systemctl restart polkit.service` 에 **비밀번호가 한 번** 필요하고 세션 전체에 영향이
있어 기본으로 켜지 않는다. 하는 일은 셋이다: 재시작 뒤 `came back as` 가 찍히는가,
그리고 — 여기가 진짜 시험이다 — **그 뒤에 온 진짜 폴킷 요청에 창이 뜨는가.** 검증 기준이
낡았으면 거절되어 창이 안 뜬다. polkitd 재시작이 인증 캐시도 지우므로 반드시 물어보게 되고,
창이 뜬 것만 보면 되니 **비밀번호는 재시작 때 한 번뿐이다.**
기본 실행에 넣지 않는 이유는 비밀번호와 세션 영향 때문이지, 못 미더워서가 아니다.

**되돌리기를 하나 고쳤다.** 시나리오는 `--init` 을 여러 번 부르는데, `$BIN`(개발 트리)으로
되돌리면 유닛의 `ExecStart` 가 개발 트리를 가리킨 채 남는다 — `cargo build` 가 도는 동안
에이전트가 죽는다(§6-1). 되돌릴 때는 `command -v sudo-pop` 이 찾은 **설치본**으로 `--init`
하고, 무엇으로 되돌렸는지 경로까지 찍는다.

---

## 18. 창의 아랫줄을 바꾸다 — 안내 대신 예산

조작 안내(`Enter to confirm    Esc to cancel`)를 빼고, 그 자리에 **faillock 잔여 횟수를
상시로** 둔다. 셋 이하이면 경고색이다.

### 18-1. 왜

안내 줄이 가르치는 것은 Enter 와 Esc 다. 둘 다 어느 입력칸에서나 같은 뜻이고, 이 창을
두 번째로 보는 사람에게는 아무것도 알려 주지 않는다. 그 자리를 매번 차지할 값어치가 없다.

반대로 잔여 횟수는 **이 창에서만 알 수 있는 것**이다. 카운터는 sudo·polkit·로그인이 공유하고
(§9), 여기서 틀리면 로그인까지 잠긴다. 지금까지는 `WARN_BELOW = 4` 미만일 때만 나왔는데,
그건 이미 늦다 — 여유가 얼마나 남았는지는 다 쓰기 전에 알아야 쓸모가 있다.

### 18-2. 두 줄로 나눈 이유

한 줄에 두 가지를 넣을 수는 없었다. `Wrong` 과 PAM 메시지는 **지나가는 것**이고 잔여 횟수는
**계속 참인 것**이라, 한 슬롯에 넣으면 오답 직후 — 잔여가 방금 하나 줄어 가장 알고 싶은
순간 — 에 숫자가 가려진다.

```
pacman -Syu                    ← subject-pid 의 cmdline
for lmh
  [                    ]
Wrong                          ← 지나가는 줄 (없으면 빈 줄로 높이만 유지)
7 attempt(s) left before ...   ← 상시. 3 이하면 경고색
```

빈 줄로 높이를 잡아 두는 것은 `Wrong` 이 뜰 때 아래 줄이 튀지 않게 하기 위해서다.

### 18-3. 어디에 실어 보내나

`ToUi::Error` 로 한 번 보내던 것을 **`Subject` 의 필드로** 옮겼다. 창이 떠 있는 동안 값이
바뀌지 않으므로 채널에 흘릴 메시지가 아니라 요청의 속성이다. 덤으로 `Wrong` 이 그것을
덮어쓸 수 없게 된다.

`Budget::warning() -> Option<String>` 은 `Budget::status() -> Option<(String, bool)>` 이
됐다. `None` 은 이제 "경고할 만큼 낮지 않다" 가 아니라 **"잠겨서 창 자체가 안 뜬다"** 뿐이다
(`refusal()` 이 그 경우를 맡는다). 상수도 `WARN_BELOW = 4`(미만) 에서
`WARN_AT_OR_BELOW = 3`(이하) 로 바꿨다 — 화면에 보이는 숫자와 상수가 같아야 읽을 때 헷갈리지
않는다.

### 18-4. 확인

`no_screen_share` 를 잠깐 빼고 실물을 찍었다 (§6-2 가 말한 그 방법). 잔여 3 은 가짜
`faillock` 을 PATH 앞에 두어 만들었다 — 진짜 카운터는 건드리지 않는다 (§15 의 시나리오 §6 과
같은 수법).

| | |
|---|---|
| 잔여 10 | 약한 색으로 상시 표시. Enter/Esc 줄 없음 |
| 잔여 3 | 같은 줄이 경고색(`text-error`) |
| 규칙 복구 뒤 | 캡처 색 5개 — 화면 공유 제외가 다시 걸린 것 확인 |

`screenshots/sudo-pop.png` 도 새 창으로 갈았다. 옛것은 사라진 안내 줄을 달고 있었다.

### 18-5. 곁에서 나온 것 — README 가 사실이 아니었다

README 두 벌이 25초 제한을 두고 **"창이 그렇게 알려 준다"** 고 적고 있었다. 창이 그린 문자열
전부를 세어 보니 그런 문구는 없다 — 처음부터 없었다. 제한은 진짜지만(§3-6) 화면에 나온 적이
없으므로 그 문장만 걷어냈다.

---

## 19. 남은 시간을 창이 센다

`run0` 경로에는 25초 마감이 있다 (§3-6). 그 사실을 알고 있는 것과 화면에서 보는 것은
다르다 — 지금까지는 문서에만 있었고, 창은 아무 말도 하지 않았다 (§18-5).

### 19-1. 기준점을 어디로 잡나

카운트다운이 틀리면 없느니만 못하다. 기준으로 삼을 수 있는 시각이 셋 있다.

| 후보 | 오차 |
|---|---|
| 자식이 뜬 시각 | 평소엔 작지만 **큐에 밀리면 통째로 틀린다** |
| 에이전트가 `BeginAuthentication` 을 받은 시각 | 폴킷이 우리를 부르기까지의 시간만큼 낙관적 |
| 호출자가 D-Bus 호출을 낸 시각 | 우리가 볼 수 없다 |

먼저 실측했다. `run0` 호출부터 창이 뜨기까지 **48ms** (4회, min 47 / median 48 / max 49,
Hyprland `openwindow` 이벤트 기준). 1초 단위 표시에는 무시할 수 있는 값이라 자식 시각으로도
평소엔 충분하다.

**그래도 에이전트 시각으로 잡았다.** 요청이 줄을 서면(§4-3) 그 대기 시간 동안 호출자의
시계는 계속 간다. 자식 시각을 쓰면 대기로 흘린 만큼을 창이 **다시 내주게** 되고, 그건
없는 여유를 약속하는 것이다. 에이전트는 `begin_authentication` 맨 위에서 이미
`started` 를 찍고 있었으므로, 큐를 통과한 **뒤에** 남은 값을 계산해 `SUDO_POP_LEFT_MS`
로 자식에게 넘긴다. 비밀이 아니라 env 로 간다 (쿠키만 파이프다 — §2-1).

25초라는 값은 **우리 것이 아니다.** sd-bus(`run0`·`systemctl`)와 GDBus(udisks·
NetworkManager)의 기본 메서드 타임아웃이고, 자기 타임아웃을 지정하는 호출자는 이 값과
다르다. 그래서 창이 그리는 것은 약속이 아니라 카운트다운이다 — 자체 백스톱 30초는 그대로
두고, 요청이 실제로 끝나는 것은 polkitd 의 취소다.

### 19-2. 실측 — 표시가 맞는가

진짜 `run0` 로 확인했다. 화면에 **`4s`(경고색)** 이 떠 있던 시각이 호출 후 21,281ms,
`run0` 이 실제로 죽은 것이 25,028ms.

```
21,281ms  화면: 4s      →  예측 만료 25,281ms
25,028ms  run0: Failed to start transient service unit: Connection timed out
```

**약 0.25초 낙관적이다.** 호출자의 시계가 폴킷이 우리에게 닿기 전에 이미 돌고 있었고,
그 구간은 우리가 볼 수 없다. 1초 단위 안이라 표시로는 드러나지 않는다.

반올림은 **올림**으로 한다. 내림이면 마지막 1초 동안 `0s` 가 떠 있게 되는데, 아직 남은
시간을 없다고 말하는 셈이다. 이미 낙관적인 쪽으로 0.25초 기울어 있으므로 거기서 더
깎지 않는다.

### 19-3. 어디에 두나

**우상단 여백 띠**에, 흐름에 얹지 않고(`Ui::put`) 그린다. 세로 배치에 넣으면 아래 구성이
통째로 내려가고, 가운데 정렬된 명령 줄과 부딪힐 수도 있다. 여백에 얹으면 둘 다 없다.

성격도 그 자리가 맞다. 아래 두 줄은 글이고(지나가는 것 + 상시), 카운트다운은 **재촉**이다.
같은 덩어리에 넣으면 매 초 바뀌는 숫자가 읽어야 할 글을 흔든다.

5초 이하이면 경고색으로 바뀐다 — 시도 횟수 줄이 3 이하에서 그러는 것과 같은 규약이다.

### 19-4. 시험

- **유닛 2개** (88개로). `ceil_secs` 를 순수 함수로 뽑아 올림 경계(0·1·999·1000·1001ms)와
  경고 문턱(5000ms 이하)을 못 박는다. 창 자체는 여전히 유닛테스트 대상이 아니지만
  (`test-plan.md` 의 제외 목록), 매 초 눈에 보이는 산수는 뽑아낼 값어치가 있다
- **시나리오 2개** (44개로). 에이전트가 남은 시간을 로그에 남기게 하고 §3 에서 본다:
  첫 요청이 25초에 붙어 있는가(실측 **24,999ms**), 그리고 **어떤 요청도 25초를 넘겨
  받지 않는가** — 후자가 큐에서 흘린 시간을 다시 내주지 않는다는 보장이다
- 실물은 §19-2. 스크린샷도 카운트다운이 보이는 것으로 갈았다

---

## 20. GUI 유래 요청 — 첫 줄만으로는 부족했다

에이전트가 되면서 창에 오는 요청이 `run0` 만은 아니게 됐는데, 창은 여태 `run0` 만 상정하고
있었다. 실제로 데스크톱 액션을 걸어 보고 나서야 드러났다.

### 20-1. 폴킷이 보내는 것이 경로마다 반대다

`pkcheck` 로 udisks 마운트 인증을 quickshell 을 주체로 걸었다. 온 것은 이렇다.

```
action_id : org.freedesktop.udisks2.filesystem-mount-system
message   : Authentication is required to mount the filesystem
icon_name : drive-removable-media
details   : {"polkit.subject-pid": "1061797", "polkit.caller-pid": "1857905"}
```

`run0` 과 나란히 놓으면 **쓸모가 정확히 뒤집혀 있다.**

| | `subject-pid` 의 cmdline | polkit 의 `message` |
|---|---|---|
| run0 | `run0 pacman -Syu` — **전부** | `start transient unit 'run-p1592…service'` — 난수 |
| 데스크톱 | `quickshell -n -p /usr/share/omarchy/shell` — **누가**만 | `mount the filesystem` — **무엇을** |

§3-5 는 run0 만 보고 "`message` 를 그대로 띄우면 안 된다" 로 결론지었고 그건 맞다. 하지만
데스크톱 경로에서는 첫 줄이 **그 앱의 바이너리 이름**일 뿐이라, `quickshell` 만 보고
마운트인지 네트워크 설정인지 알 방법이 없었다. 우리가 우위로 내세운 "무엇이 묻고 있는지"
가 정작 그 경로에서 반쪽이었던 셈이다.

### 20-2. 고침 — 둘째 줄을 둔다

`invocation::purpose(message, action_id, have_command)` 를 뒀다.

- 상투구 `"Authentication is required to "` 와 끝의 마침표를 걷어낸다. 창은 이미 비밀번호를
  묻고 있으므로 그 앞부분은 아무것도 더하지 않는다
- **번역된 문장은 통째로 둔다.** 등록할 때 로케일을 넘기므로 한국어로 올 수 있고, 못 알아본
  접두사를 억지로 자르느니 그대로 보여 주는 편이 낫다
- **`manage-units` 이면 띄우지 않는다** — 첫 줄이 이미 `run0 …` 라 그 문장은 난수 유닛
  이름밖에 남지 않는다. 예외는 이 액션 하나이고, 이유가 실측이라 표에 적어 뒀다
- 첫 줄이 없으면(주체를 못 읽으면) 이 문장이 첫 줄이 된다 — 그때는 그것이 전부이므로
  억제하지 않는다

```
quickshell -n -p /usr/share/omarchy/shell     ← 누가
mount the filesystem                          ← 무엇을        (새로 생긴 줄)
for lmh                                       ← 누구의 비밀번호
```

`run0` 쪽은 한 줄 그대로다. 창 높이는 400×200 안에 그대로 들어갔다.

### 20-3. 곁에서 드러난 것 — 25초는 모두의 것이 아니다

`pkcheck` 요청은 25초에 취소되지 않았다. 로그가 `exit 2 (30 초 경과)` 였다 — polkitd 가
취소를 보낸 것이 아니라 **우리 자체 백스톱**이 창을 닫은 것이다. `pkcheck` 는 자기 D-Bus
타임아웃을 두지 않으므로 §3-6 의 25초가 애초에 해당되지 않는다.

그러면 §19 의 카운트다운이 그 경로에서 **0 에 닿고 빨갛게 5초를 버틴다.** 아직 멀쩡히 살아
있는 요청인데 다 됐다고 말하는 셈이다. 게다가 sd-bus 호출자에게는 0 이 보일 일이 없다 —
그쪽은 폴킷이 즉시 취소해 창이 같이 닫힌다. **즉 `0s` 가 화면에 보이는 경우는 그것이 틀렸을
때뿐이다.**

그래서 0 이 되면 숫자를 지운다. 세는 동안은 뜻이 있고, 다 쓰고 나면 우리가 아는 것이 없다.

### 20-4. 동시 요청 — 실측

터미널 셋이 동시에 친 상황(0.8초 간격 3건)을 만들어 봤다.

| | 대기 | 창에 뜬 시간 | 끝 |
|---|---|---|---|
| req1 | 없음 | 25s | Esc → `Access denied` |
| req2 | req1 뒤 | **19s** (`left=21,360ms`) | 기다리다 만료 → `Method call timed out` |
| req3 | req2 뒤 | **1s** (`left=801ms`) | 곧바로 `Connection timed out` |

창은 내내 하나였다. 확인된 것 둘.

- **대기 중에도 호출자의 시계는 간다.** §19-1 이 기준점을 자식이 아니라 에이전트 수신 시각으로
  잡은 이유가 그대로 화면에 나왔다 — 둘째 창이 25 가 아니라 19 에서 시작했다
- 큐에서 만료된 요청은 polkitd 가 취소를 보내고, 에이전트가 그 쿠키의 창을 닫는다
  (`closed the prompt for that cookie`)

셋째 요청은 0.8초만 남은 채 창이 떴다 사라진다. **그렇다고 남은 시간이 적을 때 창을 건너뛰지는
않는다** — 위에서 본 대로 25초는 우리가 확신할 수 있는 값이 아니고, 없는 만료를 근거로 창을
안 띄우면 `pkcheck` 처럼 계속 기다리는 호출자를 우리가 죽이게 된다.

### 20-5. 시험

- **유닛 8개** (88 → 95 + main 1). `purpose` 의 갈래를 전부 덮는다: 데스크톱 문장에서
  상투구가 빠지는가, run0 문장이 억제되는가(그리고 첫 줄이 없을 때는 억제되지 **않는가**),
  번역문이 온전한가, 빈 문장·상투구만 있는 문장, 길이 상한. `countdown` 은 0 이면 `None`
- **시나리오 5개** (44 → 49). **§10 은 이 저장소에서 run0 을 거치지 않는 첫 시나리오다** —
  여태 모든 시나리오가 run0 하나로만 에이전트를 두드렸다. `pkcheck` 로 udisks 액션을 걸어
  창이 뜨는지, 액션과 문장이 그대로 오는지, Esc 로 닫히는지를 본다
- **자체 백스톱도 여기서 처음 시험한다.** `pkcheck` 는 호출자 타임아웃이 없어서 창을 닫는
  것이 우리 30초뿐이다 — 그것이 안 돌면 창이 영영 남는다. 30초를 기다리는 값이 그래서 있고,
  실측도 정확히 **30초**였다

---

## 21. 창 폭을 명령에 맞추다, 그리고 동시 요청 조사

### 21-1. 왜 가변인가

창의 첫 줄은 "무엇이 묻고 있는가" 이고, 그것이 이 도구가 셸의 에이전트보다 낫다고 내세운
이유다 (§3-5). 그런데 400 고정 폭에서는 조금만 길어도 잘렸다.

```
systemctl restart NetworkManager-dispatcher.service      373pt — 400 안에 안 들어간다
/usr/lib/chromium/chromium --ozone-platform=wayland …    828pt
```

잘린 명령은 없느니만 못하다. 뒤가 안 보이면 **판단 근거가 아니라 짐작 근거**가 된다.

400~800 으로 잡았다. 아래는 지금까지의 모양 그대로이고, 위는 그 이상 넓히면 눈이 줄을
읽지 않고 훑기 시작하는 지점이다. 비밀번호 상자가 화면을 채울 이유는 없다.

### 21-2. 창을 만들기 전에 실제로 잰다

폭은 창을 만들 때 정해야 하는데, egui 의 `Fonts` 는 첫 프레임 전에는 존재하지 않는다
(§19-3 에서 `ctx.fonts()` 가 패닉하는 것과 같은 이유다). 그래서 `font::Chain` 을 창보다
**먼저** 만들고, 그 체인으로 `Fonts` 를 한 번 세워 실제 레이아웃 폭을 잰 뒤 버린다.

글자 수로 추정하지 않는 이유는 둘이다 — 비례 폰트에서 틀리고, CJK 에서는 두 배로 틀린다.
비용은 §16-3 실측 기준 0.2ms(라틴)~1.2ms(CJK), 40ms 짜리 실행 대비 무시할 수준이다.

실측(`SUDO_POP_DEBUG` 로 찍는다):

```
text  97pt -> window 400pt     pacman -Syu
text 373pt -> window 429pt     systemctl restart NetworkManager-dispatcher.service
text 828pt -> window 800pt     /usr/lib/chromium/chromium --ozone-platform=… (상한에 걸림)
```

`invocation` 의 120자 상한이 먼저 자르므로, 폭은 **이미 잘린 문자열**에 맞춰진다. 둘이
따로 놀지 않도록 재는 문자열과 그리는 문자열을 `Subject::headline()`/`detail()` 한 곳에서
낸다 — 재는 크기와 그리는 크기가 어긋나는 버그는 긴 줄에서만 드러나서 눈에 잘 안 띈다.

### 21-3. Hyprland 규칙에서 `size` 를 뺀다

`assets/sudo-pop.lua` 의 `size = { 400, 200 }` 이 클라이언트가 요청한 폭을 덮어쓴다.
규칙을 비활성화하고 재 보니 같은 요청이 800 으로 떴다. 그래서 규칙에서 크기만 뺐고,
나머지(`float`·`center`·`dim_around`·`stay_focused`·`pin`·`no_screen_share`)는 그대로다 —
전부 켠 채로도 800 이 나오는 것을 확인했다.

`--init` 을 다시 돌려야 반영된다. `install.sh` 가 그 일을 한다.

### 21-4. 입력칸은 같이 늘어나지 않는다

폭을 키우니 비밀번호 입력칸이 함께 늘어났다. 되돌렸다 — 창이 넓어지는 이유는 **명령이
길어서**지 비밀번호가 길어서가 아니다. 입력칸이 명령을 따라 늘어나면 그 명령이 거기
들어갈 것처럼 읽힌다. 칠 것의 길이는 무엇을 인증하든 같다.

그래서 자물쇠+입력칸 한 줄은 **400 창에서 갖던 폭(352pt)을 그대로 유지하고 가운데**에
선다. 넓은 창에서는 양옆이 비고, 시선이 명령 → 입력칸으로 자연스럽게 좁아진다.

### 21-5. 조사 — 동시 요청은 바로 셀 수밖에 없나

두 층으로 나뉜다.

**카운트 자체는 피할 수 없다.** 25초는 호출자의 시계다. 우리가 큐에 넣든 말든 이미 가고
있고, 우리는 그것을 **보여 줄 뿐**이다. 안 보여 주면 사용자만 모른다. §20-4 의 실측이
그대로다 — 둘째 요청은 차례가 왔을 때 이미 21.4초, 셋째는 0.8초만 남아 있었다.

**직렬화는 그러나 우리 정책이다.** 자식 둘을 `turn` 락 밖에서 동시에 띄워 봤다.

```
자식 2개 동시 → 창 수: 2
   at [568, 345] size [400, 200] pid 1924941
   at [568, 345] size [400, 200] pid 1924939
```

winit 의 "프로세스당 이벤트 루프 하나" 는 요청마다 자식을 띄우는 구조(§4)에서 이미
우회돼 있다. §4-3 이 "화면에 한 번에 하나만 띄울 것이라면 대기열을 관리할 이유가 없다"
고 적은 대로, 직렬화는 단순화를 위한 선택이었다.

**그리고 같은 실측이 그 선택이 옳았음도 보여 준다.** 두 창이 **정확히 같은 자리**에 겹쳐
떴다. 똑같이 생긴 비밀번호 상자 둘이 포개져 있고 둘 다 `stay_focused` 라 포커스를 서로
뺏는다. 어느 창이 어느 명령의 것인지 모르는 채로 비밀번호를 치게 된다 — 이 창이 막으려는
바로 그것이다.

| | 얻는 것 | 잃는 것 |
|---|---|---|
| **직렬화 유지 (지금)** | 창은 언제나 하나, 무엇에 답하는지 명확 | 뒤 요청이 기다리다 만료될 수 있다 |
| 동시 표시 | 아무도 기다리지 않음 | 겹친 비밀번호 창 |
| 마감 임박 순 | 더 많이 구제 | 창 뜨는 순서를 예측할 수 없다 |
| 남은 시간 없으면 건너뛰기 | 헛된 창이 안 뜸 | **불가** — 만료가 없는 호출자(§20-3)를 우리가 죽인다 |

**유지한다.** 남은 개선 여지는 "N개 더 대기 중" 을 창에 알리는 정도이고, 그건 의미를
바꾸지 않으면서 서두를 이유를 준다 — 후순위로 남긴다.

### 21-6. 시험, 그리고 재면서 두 번 속은 것

유닛 3개 (95 → 98): `clamp_width` 의 하한·상한, 그리고 입력칸 폭이 창 폭과 무관하다는 것.
시나리오는 크기 단언을 `400 ≤ 폭 ≤ 800 && 높이 = 200` 으로 넓혔다 (49/49 통과).

측정하면서 두 번 헛짚었고, 둘 다 도구가 아니라 **내 하네스**가 문제였다.

- **이전 창이 안 닫힌 채로 크기를 읽었다.** 연속으로 세 번 찍는데 앞 창이 남아 있어서
  계속 첫 번째(400)를 보고 "폭이 안 변한다" 고 결론 냈다. 창이 0인 것을 확인하고 다시
  재니 400/429/800 이 그대로 나왔다
- **릴리스 빌드로 가짜 헬퍼를 쓰려 했다.** C2 의 고침대로 릴리스는 `SUDO_POP_HELPER_BIN`
  을 읽지 않으므로 진짜 PAM 이 돌았고, 반복하다 **faillock 이 10건에 닿아 계정이 잠겼다**
  (`account locked, 119s to go`). `faillock --reset` 으로 풀었다. 창 모양만 볼 때는
  디버그 빌드를 쓸 것 — 같은 화면이고 카운터를 안 태운다
