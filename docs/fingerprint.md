# sudo-pop 지문 인증 계획

> **이 문서의 역할**: 지문을 창에 연계할 때 무엇을 만들고 무엇을 지키는지만 담는다.
> 구현됨. 창·백스톱·파일 목록의 한 줄은 [`plan.md`](plan.md) 에도 옮겨 두었다.
>
> 지문이 어떻게 가능한지, 에이전트가 왜 센서를 직접 읽지 않는지는 이 문서 §0 과
> [`rationale.md`](rationale.md) §2-1·§11, [`agent-comparison.md`](agent-comparison.md) §2-4.

---

## 0. 전제

지문은 에이전트 기능이 아니라 **PAM 기능**이다. Omarchy 의
`omarchy-setup-security-fingerprint` 가 등록하고 `/etc/pam.d/polkit-1` 에
`pam_fprintd.so` 를 넣으면, 에이전트가 누구든 `polkit-agent-helper-1` 이 그 스택을 돈다.

```
권한 요청 → polkitd → sudo-pop 에이전트 → 자식(--agent-prompt)
                                         → 헬퍼(PAM polkit-1)
                                         → pam_exec 덮개 게이트
                                         → pam_fprintd  (sufficient, fprintd)
                                         → pam_unix     (비밀번호 폴백)
```

sudo-pop 은 fprintd 를 부르지 않는다. 헬퍼가 보내는 줄을 받아, **비밀번호를 묻기 전의
대기**를 창에 그릴 뿐이다. 등록·패키지·센서 감지는 Omarchy 셋업에 맡긴다.

이미 되는 것:

| | |
|---|---|
| 지문이 맞으면 프롬프트 없이 `SUCCESS` | `helper.rs` 가 그대로 성공으로 끝낸다 |
| `PAM_TEXT_INFO` ("Place your finger" 등) | 헬퍼는 받는다. 창은 짧은 `Touch the sensor` 만 그린다 |
| 덮개 닫힘이면 지문을 건너뜀 | PAM 의 `omarchy-hw-laptop-closed` 게이트. 창이 아니라 스택이 한다 |
| 지문이 틀리거나 시간 초과하면 비밀번호 | PAM 이 `PAM_PROMPT_ECHO_OFF` 를 보낸다 |

설정 여부가 갈림이다. **읽기만** 하고, 없으면 지금 창을 그대로 둔다.

| | 창 | 인증 |
|---|---|---|
| 지문 설정됨, 덮개 열림 | 지문 아이콘. 칸 없음 | 지문 먼저, 안 되면 비밀번호 |
| 지문 설정됨, 덮개 닫힘 | 처음부터 비밀번호 칸 | PAM 게이트가 지문을 건너뜀 |
| 지문 설정 안 됨 (이 머신) | 지금과 같다 | 비밀번호만 |

이미 고친 것:

| | |
|---|---|
| 창이 처음부터 비밀번호 칸을 띄움 | 설정됨 ∧ 덮개 열림이면 칸 대신 글리프 |
| 대기 중 Esc | 읽기 루프가 `cancelled()` 를 보고 헬퍼를 끊는다 |
| 창 백스톱 30초가 창이 뜨는 순간부터 셈 | `Prompt` 뒤에만 돈다 |
| 지문 전용 화면이 없음 | 지문 글리프. 대기 중 긴 TEXT_INFO 는 안 그림 |

---

## 1. 하지 않는 것

| 하지 말 것 | 대신 |
|---|---|
| fprintd D-Bus 를 직접 호출 | 헬퍼가 PAM 으로 한다 |
| 지문 등록·삭제 마법사 | `omarchy setup security fingerprint` / `omarchy remove security fingerprint` |
| `/etc/pam.d/*` 를 쓰거나 고침 | 셋업이 만든 파일을 **읽기만** 한다 |
| 잠금화면 지문 | `omarchy.lock` 의 `omarchy-lock-fingerprint`. polkit 이 아니다 |
| `omarchy.polkit` 과 자리를 나눠 가짐 | 세션에 에이전트는 하나다 |
| askpass(`sudo -A`) 경로에 지문 UI | 그 창은 sudo 가 비밀번호를 물을 때만 뜬다. 지문은 sudo PAM 이 창보다 먼저 시도한다 |
| 라우팅을 바꿔 더 많은 명령을 `run0` 으로 | 지문 때문에 `sudo -E` 의 의미를 버리지 않는다 |
| FIDO2 전용 UI | 같은 PAM 통과로 비밀번호 칸이 미뤄질 수는 있으나, 이 계획의 화면은 지문만 그린다 |
| omarchy 의 아이콘-only 정사각 카드 | 첫 줄은 명령이다. 창이 존재하는 이유라서 지문 모드에서도 남긴다 |

---

## 2. 경로별 범위

| 경로 | 지문 UI | 실제 지문 인증 |
|---|---|---|
| polkit (`run0`, pkexec, 마운트, NM, systemctl) | **한다.** `--agent-prompt` 한곳 | PAM `polkit-1` + 헬퍼 |
| 옵션 없는 `sudo <명령>` | 위와 같다 — 라우터가 `run0` 으로 보낸다 | 위와 같다 |
| `sudo -옵션` / `sudo VAR=값` (`sudo -A`) | **그리지 않는다** | sudo 의 PAM. 맞으면 창이 안 뜨고, 실패하면 비밀번호 창만 뜬다 |
| 터미널에서 `/usr/bin/sudo` | 창이 없다 | sudo PAM 이 터미널에서 한다. 원래부터 그렇다 |
| 잠금화면 | 범위 밖 | `omarchy.lock` |

`sudo -A` 에서 창이 안 뜨고 지문만으로 통과하는 것은 버그가 아니다. sudo 가 askpass 를
부르기 전에 `pam_fprintd` 가 `sufficient` 로 끝난 것이다. 전용 화면이 없는 것을
이 계획에서 고치지 않는다.

---

## 3. 동작

요청 하나에 창 하나. **지문이 켜져 있으면 지문이 먼저**이고, 안 되면 같은 창이
비밀번호 칸으로 바뀐다. 창을 새로 띄우지 않는다.

omarchy.polkit 과 같다: 지문일 때는 입력칸이 없고 아이콘만, 비밀번호일 때는 칸이 있다.
다른 점 하나 — 첫 줄은 명령을 남긴다 (정사각으로 접지 않음).

횟수는 우리가 만들지 않는다. 실패 뒤에는 헬퍼가 보내 준 문구를 **그대로** 그린다.
한도 숫자를 PAM 에서 꺼내 창에 넣지 않는다 (§8).

```
열림
  │  fingerprintConfigured && !laptopClosed
  ├─ yes → 대기: 칸 숨김, 지문 글리프
  └─ no  → 지금과 같다 (칸이 처음부터 있다)
                │
                ├─ SUCCESS / Done            → 창 닫힘 (비밀번호를 묻지 않음)
                ├─ PAM_ERROR_MSG (대기 중)   → 그 문구를 지나가는 줄에. 글리프 실패색. 상태는 대기
                ├─ PAM_TEXT_INFO             → 창 문구는 그대로 `Touch the sensor`. 상태는 대기
                ├─ PAM_PROMPT_ECHO_*         → 비밀번호 칸. 상시 줄은 faillock
                ├─ Wrong 후 헬퍼 재시작      → PAM 이 지문을 다시 시도해도 칸은 유지.
                │                              지문 TEXT_INFO 도 PAM_ERROR_MSG 도 다음 프롬프트
                │                              까지 칸 아래를 덮지 않음 (우리 오류는 보임)
                ├─ Esc / 닫기 / 호출자 취소  → 헬퍼를 끊고 종료 코드 2
                ├─ FAILURE, 프롬프트 없음    → 지금과 같다 (취소로 끝. 잠긴 계정 등)
                └─ 프롬프트 뒤 헬퍼 사망     → 그 문구만, Wrong 없이, 새 헬퍼로 재시도 (HelperGone)
```

**"지문이 끝났다" 는 신호는 오지 않는다.** `pam_fprintd` 는 `sufficient` 라서 `max-tries` 를
다 쓰면 조용히 실패로 돌아가고, PAM 은 다음 줄 `pam_unix` 로 내려간다. 그 사이 우리에게
오는 것은 `PAM_ERROR_MSG` 문구뿐이다. 창이 비밀번호 칸으로 바뀌는 계기는 오직 `pam_unix` 가
보내는 첫 `PAM_PROMPT_ECHO_OFF` 다. 그래서 창은 **왜** 비밀번호를 묻는지 모른다 — 지문을 다
틀려서인지, fprintd 가 장치를 못 열어 `pam_fprintd` 가 0.2초 만에 실패해서인지 같은 화면이다
(실측: [`rationale.md`](rationale.md) §22-6). 지문이 성공하면 프롬프트가 아예 오지 않고
`SUCCESS` 로 창이 닫힌다.

`fingerprintMode` 는 다음이 **동시에** 참일 때만이다.

- PAM 스택에 `pam_fprintd.so` 가 있다 (§4) — 설정 안 됨이면 여기까지 안 온다
- 덮개가 열려 있다 (§4)
- 창이 살아 있다
- 아직 `ToUi::Prompt` 를 받지 않았다
- 제출 중이 아니고, 에러 플래시도 아니다

덮개는 **요청이 시작될 때 한 번만** 본다. 대기 중에 덮개가 닫혀도 화면을 비밀번호로 바꾸지
않는다 — PAM 게이트는 스택이 시작될 때 이미 지났고, 칸을 그려 봐야 헬퍼는 아직
`pam_fprintd` 안에 있다.

대기 중에 Enter 는 무시한다. 칸이 없고, `FromUi::Answer` 를 보내서도 안 된다. 나중에
`ask()` 가 그 답을 비밀번호로 집어 넣는 경합이 생긴다.

---

## 4. 감지

순수 함수로 두고 파일 I/O 는 얇은 래퍼만 둔다. 시험이 파일을 만들 필요가 없다.

### 4-1. PAM 파일

읽을 경로, **이 순서**:

1. `/etc/pam.d/polkit-1` — 있으면 이것만 본다 (PAM 탐색 순서와 같다)
2. 없을 때만 `/usr/lib/pam.d/polkit-1`

Omarchy 셋업은 1 을 만든다. 1 이 없는 머신은 지문이 설정되지 않은 것이다. 2 만 고친
수동 구성은 1 이 없을 때 잡히게 한다 — [`rationale.md`](rationale.md) §2-3 의 함정
(omarchy.polkit 은 1 만 봐서 Arch 기본 경로를 놓친다) 을 이쪽에서 피한다.

읽기는 `pam::auth_stack_names("polkit-1", "pam_fprintd")` 하나이고, faillock 검사
(`attempts.rs`, rationale §24) 와 같은 함수다. 두 답이 서로 다른 파일을 보고 어긋날 수 없다.
규칙:

- `#` 줄과 빈 줄을 건너뛴다
- `auth`·`-auth` 줄만 본다 (`account`/`session` 에 같은 문자열이 있어도 무시)
- `include`·`substack`·`@include` 는 그 서비스 파일로 따라 들어간다 (깊이 8 까지). 지문 줄이
  `polkit-1` 이 아니라 `system-auth` 에 있는 수동 구성도 같은 스택이므로 같은 답이다
- `[success=1 default=ignore]` 같은 대괄호 제어는 공백이 있어도 한 필드로 건너뛴다
- 그 줄에 `pam_fprintd` 가 있으면 참
- **첫 줄일 필요는 없다.** 덮개 게이트(`pam_exec` … `omarchy-hw-laptop-closed`) 가 앞에
  오는 것이 정상이다

`max-tries` 는 파싱하지 않는다. 줄에 있든 없든 시도 횟수는 `pam_fprintd` 가 그 대화
안에서 처리하고, 창은 그 숫자를 모른다. Omarchy 셋업은 지금 인자 없이
`auth sufficient pam_fprintd.so` 만 넣는다.

프롬프트 문구에 `finger` 가 있는지로 감지하지 않는다. omarchy 쪽에 그 함수가 있어도
실제 모드 전환은 PAM 파일을 본다. 문구는 로캘을 타고, 가짜 안내에 화면이 바뀌면 안 된다.

자식이 짧으니 **기동 때 한 번** 읽는다. 파일을 감시하지 않는다.

### 4-2. 덮개

`/usr/bin/omarchy-hw-laptop-closed` — exit 0 이면 닫힘. 실패·부재는 열림으로 본다
(지문 UI 를 꺼서 비밀번호로 두는 쪽이, 없는 센서를 기다리는 쪽보다 낫다).

호출은 `--agent-prompt` 기동 때 한 번. 에이전트 데몬은 부르지 않는다.

---

## 5. 창

### 5-1. 정체

지문 모드여도 바뀌지 않는 것:

- app-id `sudo-askpass`, 폭 400~800 (명령에 맞춤). 높이는 지문 대기 168, 비밀번호 칸 200
- 첫 줄은 명령 (`polkit.subject-pid` 의 cmdline)
- 둘째 줄은 `purpose` (run0 에서는 없음)
- `for <user>`
- 우상단 호출자 카운트다운 (있을 때)

지문 때문에 정사각으로 접지 않는다. 창이 넓은 이유는 명령이 길어서다. omarchy.polkit 은
아이콘만 남기고 카드를 접는다 — 그쪽 창에는 명령이 없다.

### 5-2. 대기 화면이 바꾸는 것

비밀번호 칸과 자물쇠 글리프를 **숨긴다.** 텍스트 입력 창이 아니라 지문 아이콘 창이다.

- 지문 글리프 `U+F0237` (Nerd Font `nf-md-fingerprint`). omarchy.polkit 의
  `\udb80\ude37` 과 같다. `U+F0597` 은 비 아이콘이라 쓰지 않는다
- 대기 중 안내는 짧은 ASCII `Touch the sensor`. `PAM_TEXT_INFO` 문장은 그리지 않는다.
  실패 (`PAM_ERROR_MSG`) 만 그 자리를 대신한다
- 색은 강조색 (자물쇠와 같다). 오답 안내가 오면 실패색
- 대기 중 상시 줄에 횟수를 두지 않는다. 비밀번호 faillock 도 이 구간에는 안 그린다 —
  지문 실패는 그 카운터를 보통 안 올리므로, 띄우면 거짓이 된다

조작 안내를 우리가 쓰지 않는다. Enter·Esc 안내도 띄우지 않는 지금 규칙과 같다.

### 5-3. 비밀번호로 돌아옴

`ToUi::Prompt` 가 오는 순간 `fingerprintMode` 는 꺼진다. 글리프는 자물쇠, 칸이 나타나고,
상시 줄은 **그때** faillock 을 다시 읽어서 그린다 (지금 오답 뒤 `attempts::budget()` 과
같다). 입력칸 폭은 여전히 `FIELD_ROW_WIDTH` 고정.

### 5-4. askpass

손대지 않는다. 그 모드는 뜨는 순간 이미 비밀번호를 묻는 중이고, `ToUi::Prompt` 를
기동과 함께 보낸다.

---

## 6. 헬퍼 대화와 취소

지금 취소는 `ask()` 의 `from_ui.recv()` 가 `FromUi::Cancel` 을 받을 때만 헬퍼로
이어진다. 지문 대기는 `read_line` 에 있고 `ask()` 에 없다.

**대기 중 취소는 헬퍼를 즉시 끊어야 한다.** 창만 닫고 자식이 센서 타임아웃까지 남는 것은
실패다. 데몬의 요청당 자식 하나가 그렇게 묶이면 다음 요청이 큐에서 기다린다.

지켜야 할 것:

| 상황 | `BeginAuthentication` | 자식 종료 코드 |
|---|---|---|
| 지문 성공 (프롬프트 없음) | 정상 리턴 | 0 |
| 대기 중 사용자가 취소 | **정상 리턴** | 2 |
| 대기 중 polkitd 가 취소 (25초 등) | 지금과 같다 — 데몬이 pidfd 로 자식을 죽인다 | (자식이 죽는다) |
| 지문 실패 후 비밀번호 성공/실패/취소 | 지금과 같다 | 지금과 같다 |

구현이 흔들리면 안 되는 지점:

- `attempt()` 의 읽기는 **취소와 동시에** 깨어날 수 있어야 한다. 읽기 전에 디스크립터를
  `poll()` 로 100ms 씩 기다리고, 그 사이마다 `Conversation::cancelled()` 를 본다.
  **소켓 읽기 타임아웃(`SO_RCVTIMEO`)으로 하면 안 된다** — fork 헬퍼의 stdout 은 파이프라
  `setsockopt` 가 `ENOTSOCK` 으로 실패하고, 취소가 영영 안 깨어난다. 그러면 Esc 뒤에도
  센서가 살아 있어 **취소한 명령이 손가락 한 번으로 실행된다.** 취소는 `Channel` 을
  떨어뜨리는 것으로 끝난다: 소켓은 닫히고 fork 자식은 `Drop` 이 `kill` 한다
- `poll()` 전에 `BufReader` 버퍼를 먼저 본다. 헬퍼가 두 줄을 한 번에 쓰면 둘째 줄은
  이미 버퍼에 있고, 디스크립터는 조용하다
- **소켓 헬퍼는 우리가 끊어도 바로 죽지 않는다.** root 의 `polkit-agent-helper@*.service`
  가 `pam_fprintd` 안에 있으면 그 타임아웃(기본 30초)까지 센서를 쥔다. 권한은 안 생긴다 —
  자식이 exit 2 로 끝난 순간 polkitd 의 세션이 사라져 늦은 응답은 버려진다. 그러나 그
  30초 안의 **다음 요청은 fprintd 가 `Device was already claimed` 로 거절**해 지문 없이
  곧장 비밀번호 칸이 뜬다. 창은 지문 대기로 열렸다가 첫 프롬프트에 칸으로 바뀌니 168 이
  잠깐 보인다. 우리 쪽에서 고칠 수 없다 (`rationale.md` §22-5). fork 헬퍼는 우리 자식이라
  `kill` 로 즉시 놓는다
- 취소 플래그는 창 스레드가 쓰고 헬퍼 스레드가 본다. `ask()` 가 아닌 읽기 루프도 그것을
  본다
- 대기 중(오답 뒤 PAM 이 센서를 다시 도는 동안)에 보낸 `FromUi::Answer` 는 보관했다가
  다음 `ask()` 에 넘긴다. 단 **echo-off 프롬프트에만** — 사용자명이나 OTP 같은 echo-on
  프롬프트에 비밀번호가 답으로 나가면 그 모듈이 로그에 남길 수 있다. 그 경우는 지운다
- 소켓 헬퍼가 프롬프트 없이 닫히는 기존 폴백(`RefusedWithoutPrompt` → fork) 은 유지한다.
  지문 `SUCCESS`(프롬프트 없음) 와 구분하는 기준은 지금과 같다: 태그 `SUCCESS` 면 성공,
  EOF/`FAILURE` 이고 프롬프트를 못 봤으면 거절

`Conversation` 에 취소 폴링을 더하는 쪽이, 헬퍼가 GUI 채널을 직접 아는 쪽보다 낫다.
가짜 대화는 폴링이 항상 거짓이라 기존 프로토콜 시험이 그대로 돈다.

---

## 7. 타임아웃

두 시계는 역할이 다르다. 섞지 않는다.

| 시계 | 누구 것 | 지문 대기 |
|---|---|---|
| 호출자 25초 (`SUDO_POP_LEFT_MS`) | run0 등 D-Bus 호출자 | **그대로 센다.** 센서 앞에서 더 급하다 |
| 창 백스톱 30초 (`gui::TIMEOUT`) | 취소가 안 올 때 | **`ToUi::Prompt` 가 온 뒤에만** 돌린다. 지금은 창이 뜨는 순간부터 돈다 |

호출자가 없는 경로(`pkcheck`)는 25초 배지가 원래 없다. 지문 대기가 그것을 만들지 않는다.
sudo 경로(askpass) 는 이 계획에서 지문 UI 가 없다.

지문 전용 타임아웃을 두지 않는다. `pam_fprintd` 와 호출자가 각자 포기한다.

---

## 8. 시도 횟수 — 지문은 세지 않는다

| | 지문 | 비밀번호 |
|---|---|---|
| 한도 | `pam_fprintd` 의 `max-tries` (기본 3). 모듈이 자기 대화 안에서 처리. 창은 모름 | 쿠키당 `MAX_ATTEMPTS = 3` |
| 누가 세나 | 그 모듈만 | 오답 뒤 `attempts::budget()` 으로 faillock 을 **다시 읽음** |
| 창에 보이는 것 | 글리프 + `Touch the sensor`. 실패 뒤엔 헬퍼가 준 `PAM_ERROR_MSG` 문구 그대로 | 칸 아래 faillock 잔여 (공유 카운터). polkit-1 스택에 `pam_faillock` 이 없으면 줄 없음 (rationale §24) |
| 한도를 넘기면 | PAM 이 비밀번호를 묻고 칸이 나타남 | 그 요청을 끝냄 (exit 2) |

지문 횟수를 창이 셀 수 없는 이유는 `pam_fprintd` 소스에 있다 (`rationale.md` §22).
매치 실패("Failed to match fingerprint")는 시도를 소모하며 `PAM_ERROR_MSG` 로 오지만,
"swipe too short"·"finger not centered"·"remove and retry" 같은 재스캔 안내도 **같은
`PAM_ERROR_MSG`** 이면서 시도를 소모하지 않는다. 타임아웃은 `PAM_TEXT_INFO` 이고 그 자리에서
포기한다. 헬퍼 프로토콜은 정수를 싣지 않는다. 그래서 `PAM_ERROR_MSG` 를 세면 실제보다
빨리 0 에 닿고, `max-tries` 를 읽어 M 을 만들어도 N 이 거짓이 된다. 숫자를 안 만드는 것이
맞다. `max-tries` 는 읽지 않는다.

지문 재시도는 PAM 이 한 번의 `authenticate()` 안에서 한다. sudo-pop 이 헬퍼를 세 번 열어
지문을 재시도하지 않는다. 프롬프트 없이 오는 `FAILURE` 는 잠긴 계정과 구분이 안 되어,
그걸 재시도로 치면 빈 창이 다시 뜬다.

- 지문 성공은 `authenticate()` 한 번, `SUCCESS`, 루프 종료. `MAX_ATTEMPTS` 를 쓰지 않는다
- 지문 단계가 끝나면 비밀번호 단계로만 간다. 센서로 돌아가지 않는다. 오답이면
  `Wrong password` 가 지나가는 줄에 남고 (다음 제출까지), 상시 줄의 faillock 잔여가
  다시 읽힌다. 쿠키당 3회 상한은 창에 숫자로 띄우지 않는다.
  호출자 25초는 **두 단계 모두** 그린다 (§7)
- `pam_fprintd` 실패는 보통 unix faillock 을 올리지 않는다
- 잠긴 계정은 지금처럼 창 전에 거절 (exit 2)

---

## 9. 파일

새 모듈 하나. 나머지는 대화와 창만 고친다.

```
src/fingerprint.rs   PAM 에 `pam_fprintd.so` 가 있는지 (읽기만), 경로 순서, 덮개 CLI
src/helper.rs        읽기 루프가 취소를 본다. Channel 을 취소로 닫는다
src/prompt.rs        기동 때 감지·덮개를 읽고 Subject 에 넘긴다
src/gui.rs           아이콘 대기, PAM 실패 문구 그대로, Prompt 전 백스톱 정지, 대기 중 Enter 무시
src/lib.rs           fingerprint 모듈을 내놓는다
src/askpass.rs       `fingerprint_wait: false` 한 줄. 지문 UI 없음

만지지 않음
src/wrapper.rs       라우팅 불변
src/init.rs          PAM 을 설치하지 않음
src/agent.rs         비밀번호와 마찬가지로 지문을 보지 않음. 자식에게 맡긴다
```

`Subject` 에 넘기는 것:

- `fingerprint_wait: bool` — 기동 때 설정됨 ∧ 덮개 열림

한도 숫자와 PAM 경로 문자열은 GUI 까지 들이지 않는다. `max-tries` 도 파싱하지 않는다.
실패 문구는 헬퍼 → `ToUi::Error` / `Info` 로 이미 오고, 창은 그걸 그린다.

---

## 10. 시험

센서와 `fprintd` 없이 대화와 판별을 닫는다. 이 머신에는 `fprintd` 가 없다.

### 10-1. 단위 — `fingerprint.rs`

- `auth … pam_fprintd.so` 가 있으면 참
- 게이트 줄이 앞에 있어도 참
- 주석 처리된 `pam_fprintd.so` 는 거짓
- `account`/`session` 줄만 있으면 거짓
- 빈 파일·없는 파일은 거짓
- `/etc/pam.d/polkit-1` 이 있으면 `/usr/lib` 는 안 본다 (경로 선택 함수)
- `max-tries=` 인자가 있어도 판별은 `pam_fprintd.so` 존재만 본다. 숫자는 읽지 않는다

### 10-2. 프로토콜 — `tests/fake-helper.sh` + `tests/helper_protocol.rs`

모드를 더한다. 기존 `info`(TEXT_INFO 후 비밀번호) 는 유지한다.

| 모드 | 헬퍼가 하는 일 | 기대 |
|---|---|---|
| `finger-ok` | `PAM_TEXT_INFO` 후 프롬프트 없이 `SUCCESS` | `Outcome::Success`, `ask` 0회, info 1회 |
| `finger-retry` | TEXT_INFO, ERROR, TEXT_INFO, 프롬프트 없이 `SUCCESS` | info 2, error 1, ask 0, 성공 |
| `finger-then-pw` | TEXT_INFO, ERROR×n, `PAM_PROMPT_ECHO_OFF`, 답 받아 `SUCCESS` | 지문 한도 소진 후 ask 1회 |
| `finger-hang` | TEXT_INFO 후 입을 다물고 있다가 stdin/신호로 죽음 | 취소가 끝나면 `Cancelled`, 헬퍼가 남지 않음 |

취소 시험은 `Conversation` 이 몇 번째 `info` 뒤에 취소를 켜는 스크립트로 한다. 실제
창이 필요 없다.

프롬프트 없는 `SUCCESS` 가 `RefusedWithoutPrompt` 로 접히지 않는 것을 못 박는다 — 지금
코드가 이미 그렇게 가므로 회귀 방지다.

### 10-3. 창

이벤트 루프는 단위시험하지 않는다 (지금과 같다). 대신:

- 대기 중 백스톱 시계가 돌지 않는 것 — `Prompt` 수신 전에는 `TIMEOUT` 기한을 두지 않는
  순수 조건으로 추출해 시험
- 대기 중 Enter 가 submit 으로 안 가는 것 — `fingerprintMode` 일 때 submit 가드
- 대기 중 `Error` 는 문구만 바꾸고 횟수를 만들지 않는다. `Info` 는 대기 화면을 바꾸지 않는다
- 비밀번호 단계에서 `Prompt` 는 지나가는 줄을 지우지 않는다 — `Wrong password` 다음
  프롬프트는 ms 안에 오므로, 지우면 한 프레임도 못 그린다. 지우는 것은 제출뿐이다
- 비밀번호 단계의 `Error` 는 전부 그린다. `Wrong password` 만 칸을 다시 연다 — 다른
  에러는 답이 보관된 채 센서를 도는 헬퍼에서도 올 수 있다

### 10-4. 시나리오

`tests/scenarios.sh` 11–12번. 손가락이 없어도 되는 것만 기본으로 돈다.

- polkit PAM 에 `pam_fprintd.so` 가 있는지, 덮개가 열려 있는지 — 바이너리와 같은 순서
- `--agent-prompt` 디버그 로그의 `fingerprint_wait=` 가 그 판별과 같음
- 대기 중 Esc → 종료 코드 2
- 지문 대기가 켜진 머신에서 `run0` + Esc → 명령 실패, 빈 창이 다시 안 뜸
- 가짜 헬퍼로 비밀번호 3회 오답 → 창이 닫히고 다시 안 뜸 (지문으로 재발행 안 함)
- 지문 대기 중에는 창 30초 백스톱 시험을 건너뜀 (`Prompt` 전에 안 돌므로)
- 지문이 없으면 `fingerprint_wait=false`, 높이 200, 처음부터 칸 — 기존 비밀번호 창과 같다

맞춤·틀림·안 댐은 `./tests/scenarios.sh --with-password` (12번). 창이 뜨면 지문, 안 되면
비밀번호.

손시험으로 남는 것:

- 틀린 손가락 — PAM 실패 문구 후 칸으로 바뀜
- 센서에 안 대고 기다림 — 칸으로 바뀜
- 덮개를 실제로 닫고 `run0` — 처음부터 칸
- `sudo -E true` — 지문 UI 없이 sudo PAM 이 먼저 시도

---

## 11. 구현 순서

한 단계가 시험을 그린 뒤에 다음으로 간다. 창을 먼저 그리지 않는다.

1. **`fingerprint.rs` + 단위시험** — `pam_fprintd.so` 판별과 경로 순서. 동작 변화 없음.
   `max-tries` 는 파싱하지 않는다
2. **가짜 헬퍼 모드 + 프로토콜 시험** — `finger-ok` / `finger-retry` / `finger-then-pw` 는
   지금 `helper.rs` 로도 통과해야 한다. 안 되면 3 보다 먼저 고친다
3. **취소가 읽기를 깨움** — `finger-hang` 이 `Cancelled` 로 끝나고 헬퍼가 안 남는다.
   여기까지는 창이 안 바뀐다
4. **`Subject` + `gui.rs` 대기 화면** — 칸 숨김, 글리프, 대기 중 TEXT_INFO 숨김, Prompt 전
   백스톱 정지, Enter 무시. 비설정이면 `fingerprint_wait == false` 라서 지금과 같다
5. **`prompt.rs` 가 감지·덮개를 넘김** (`fingerprint_wait` 만. 한도 숫자는 없음)
6. **문서** — `plan.md` §2 창 / §7 표, README 비교표의 지문 칸.
   이 문서는 계획이 아니라 이 기능의 사양으로 남기거나, 내용이 `plan.md` 에 흡수되면
   링크로만 남긴다

5 가 끝나기 전에는 설정 머신에서도 지문 성공은 이미 창을 닫는다(1·2). 보이는 것만
비밀번호 칸이 잠깐 떠 있는 상태다.

---

## 12. `plan.md` 와의 관계

[`plan.md`](plan.md) §7 은 지문을 구현된 것으로 두고 이 문서를 가리킨다. `plan.md` 의
지켜야 할 것(자식만 비밀번호를 본다, 취소는 D-Bus 성공, 발신자 검증, 헬퍼 두 문) 은
지문 대기에도 그대로 적용한다. 예외를 만들지 않는다.

데몬은 지문 결과를 비밀번호와 마찬가지로 보지 않는다. 종료 코드만 받는다.
