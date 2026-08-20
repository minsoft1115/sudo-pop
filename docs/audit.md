# sudo-pop 전수 점검 결과 (2026-08-19)

> 점검 범위: `git diff origin/main..main` — polkit 인증 에이전트 + sudo 라우터로
> 방향이 뒤집힌 뒤의 14 커밋. 문서(`plan.md`·`rationale.md`) 전체, `src/` 18파일,
> `tests/` 3파일, `install.sh`, 자산을 `old/` 와 대조. `cargo test` 통과
> (24 단위 + 8 통합). 라이브 상태(`sudo-pop-agent.service active`,
> `omarchy.polkit disabled`)는 **변경하지 않았고** faillock 은 읽기만 했다.

핵심 결론: 가장 큰 문제는 **에이전트 경로에 faillock/attempts 게이팅이 통째로
빠져 있다는 것**이다. 문서는 여러 곳에서 "양쪽 경로에 건다" 고 못 박았는데 코드에는
askpass 경로에만 있다. 사양이 아니라 코드를 고쳐야 하는 어긋남이다.

상태 표기: **[고침]** 은 이 점검에서 수정 완료, **[열림]** 은 미착수.

---

## 심각 (Critical / High)

### C1. 에이전트 경로에 faillock 게이팅이 전혀 없다 — 요청 반복으로 계정 잠금  **[고침]**

`src/prompt.rs` (대조: `src/askpass.rs:43-70`)

`prompt.rs`(자식, `--agent-prompt`)는 `MAX_ATTEMPTS` 만 가져오고 `attempts::budget()`·
`is_locked()`·`WARN_BELOW` 를 한 번도 부르지 않았다. askpass 경로에만 잠긴 계정 거부·
남은 시도 경고가 있었다. 결과:

- **잠긴 계정에서도 창이 떴다.** plan.md §2-4, rationale §4-1·§9·§10 결정표가 모두
  "잠긴 계정이면 묻지 않는다 / 창 대신 안내" 라고 명시했는데 위반.
- **크로스-쿠키 상한이 없었다.** 상한 3 은 자식(쿠키) 안에만 있고 요청마다 새 자식이
  새로 3 회를 받는다. faillock 은 공유(`deny=10`)이므로 polkit 을 태우는 요청을 반복
  유발하면 sudo·로그인까지 잠근다. rationale §3-4 가 경고한 바로 그 시나리오이고,
  §4-1 끝 문장("faillock 예산 조회는 양쪽 경로에서 그대로 쓴다")이 코드에서 지켜지지
  않았다.

**수정.** `prompt.rs` 가 창을 띄우기 전에 `attempts::budget()` 로 `is_locked()` 이면
안내 후 `EXIT_CANCELLED`(2)로 종료하고, `WARN_BELOW` 미만이면 창에 경고를 실었다.
`EXIT_FAILED`(1)가 아니라 취소로 끝내는 것이 §3-3 의 재발행 함정을 피하는 핵심이다.

**크로스-쿠키 카운터를 어디에 두나 (설계 결정).** 자식은 요청마다 새로 뜨므로 자식 안
카운터로는 못 센다. 대신 `budget()` 이 읽는 **살아 있는 faillock tally 자체를 크로스-쿠키
카운터로 쓴다** — 오답이 쌓이면 `remaining` 이 줄고, `deny` 에 닿으면 `is_locked()` 가
참이 되어 다음 요청부터 창 대신 안내가 뜬다. 데몬에 별도 상태를 두지 않아도 된다.

### C2. 시험용 헬퍼 env 변수가 프로덕션에서 무조건 유효 — 비밀번호 리디렉션 통로  **[고침]**

`src/helper.rs:31-42`, 상속 경로 `src/agent.rs`

`socket_path()`·`helper_binary()` 가 `SUDO_POP_HELPER_SOCKET`·`SUDO_POP_HELPER_BIN` 을
모든 빌드에서 읽었고, 자식은 에이전트 환경을 그대로 상속(`env_clear()` 없음)했다.
따라서 에이전트 환경에 `SUDO_POP_HELPER_BIN=/tmp/x` 가 있으면 비밀번호가 그 바이너리의
stdin 으로 평문으로 갔다. "비밀번호가 새지 않는다" 는 핵심 약속과 정면 충돌.

**수정.** 두 env 오버라이드를 `cfg!(debug_assertions)` 안으로 넣었다. install.sh 는 항상
`--release` 로 빌드하므로 프로덕션 바이너리는 이 변수를 아예 읽지 않는다. `cargo test`
는 debug 빌드라 통합 시험은 그대로 돈다.

### C3. fork 헬퍼의 setuid 비트 검증이 없다  **[고침]**

`src/helper.rs:35-42`

plan.md §2-5, rationale §3-3 이 "setuid 비트를 확인한 뒤에만 fork 경로를 쓴다" 고
명시했으나 `helper_binary()` 는 `exists()` 만 봤다. setuid 아닌 동명 파일이 앞자리에
놓이거나 C2 와 결합되면 검증 없이 exec 했다.

**수정.** 프로덕션(release) 경로에서 헬퍼 후보를 `uid==0 && mode & 0o4000` 로 거른다.
debug 오버라이드(fake-helper.sh, setuid 아님)는 시험용이라 그 검사 앞에서 빠진다.

---

## 중간 (Medium)

### M1. logind 세션 조회 실패 시 무한 재시작 루프  **[고침]**
`src/main.rs:187-191` + 유닛 `Restart=on-failure`

`session_id()` 가 `None` 이면 `exit(1)`. 2 초마다 영원히 재시작한다. 등록 충돌은 `Ok`
로 잘 처리(§6)해 놓고 세션 조회 실패는 그 처리에서 빠졌다. non-uwsm 환경이 이 루프에
빠졌다. **수정:** 세션을 못 찾으면 재시작이 고쳐 줄 수 없는 영구 조건이므로
정상 종료(exit 0)로 끝내 루프를 끊는다(`src/main.rs`).

### M2. 취소 시 pid 재사용(TOCTOU)로 엉뚱한 프로세스에 SIGTERM  **[고침]**
`src/agent.rs:120-134`, `ask()` 186-195

`running` 맵에서 pid 제거는 `child.status().await`(reap) 뒤다. reap 직후~맵 제거 사이에
pid 가 재사용될 수 있고, 그 순간 취소가 오면 재사용된 남의 pid 에 SIGTERM. 창은 좁지만
실재했다. **수정:** `running` 맵에 pid 대신 **pidfd**를 저장하고 `pidfd_send_signal`로
보낸다 — pidfd는 그 프로세스만 가리켜 pid 재사용에 면역이다. 취소는 `running` 락을
쥔 채 신호를 보내고, `ask()`도 같은 락 아래에서 fd를 닫아 fd 재사용도 없다(`src/agent.rs`).

### M3. 이미 취소된 대기 요청도 큐 차례가 오면 창을 띄운다  **[고침]**
`src/agent.rs:88-96`

`turn` 락을 요청 전체 동안 잡으므로, 대기 중인 요청에 취소가 와도 자식이 아직 없어
"nothing running". 차례가 오면 죽은 요청에 대해 창이 떴다. **수정:** `cancelled` 쿠키 집합을 두고,
취소가 왔는데 실행 중이 아니면 집합에 넣는다. 대기하던 `begin_authentication`은 턴을
잡은 뒤 그 쿠키면 창 없이 `Ok`로 끝낸다. 집합은 상한 256으로 무한 증가를 막는다.

---

## 낮음 (Low) / 문서-코드 어긋남

- **L1. [고침 — 2026-08-20]** 다시 보니 2순위 자체가 거의 동작하지 않았다. `pgrep -x` 는
  `/proc/<pid>/comm` 과 비교하는데 **커널이 15자에서 자르므로** 목록 5개 중
  `polkit-gnome-...`(35자)·`polkit-kde-...`(33자)는 영영 안 걸렸고, 이 점검이 추가한
  `mate-polkit` 은 패키지 이름이라 역시 안 걸렸다. 이름 표를 버리고 **이름에
  `polkit`/`policykit` 이 있는가**로 바꿨다(잘려도 남는다). 2순위는 `pgrep` 대신 `/proc` 를
  직접 읽어 우리 uid 것만 보고, **3순위(활성 user 유닛)를 신설**했다. 안내도 종류별로
  갈랐다 — 프로세스로 찾았을 때 `systemctl --user disable --now <이름>.service` 는 존재하지
  않는 명령이었다. 유닛테스트 8개 + 시나리오 §8. 자세한 것은 `rationale.md` §17-1.
- **L2. [설계상 정상]** 라우터 display 검사가 run0 라우팅보다 뒤인 것은 의도된 것이다.
  래퍼의 display env 는 래퍼 자신의 askpass 창(`sudo -A` 경로)에만 의미가 있고, run0
  경로의 창은 **세션 유닛인 에이전트**가 그린다. 스크립트/cron 이 비운 env 로 sudo 를
  불러도 사용자가 그래픽 로그인 상태면 run0 팝업이 맞다. 코드 변경 없음.
- **L3. [고침 — 2026-08-20]** 한글이 `◻` 로 나왔다. 원인은 상한이 아니라 **egui 에
  글리프 폴백이 없다는 것**이었고, 조건도 현지화가 아니라 **cmdline 에 든 한글**이라
  `LANG=en_US` 에서도 재현됐다 (Omarchy 의 모노스페이스 후보 5개·egui 번들 4개 전부
  한글 없음). `font::Chain` 이 ASCII 밖 문자를 만났을 때만 `fc-match :charset=` 으로
  면을 하나 더 붙인다. ASCII 경로는 비용 변화 없음, 한글 경로는 창이 12ms 늦게 뜬다.
  유닛테스트 12개 추가. 자세한 것은 `rationale.md` §16.
- **L4. [고침]** 요청별 상세 로깅을 `SUDO_POP_DEBUG` 뒤로 넣었다(`agent.rs`·`main.rs`).
  프로덕션 journal 에는 거절(REJECTED)·오류만 남는다. scenarios.sh 가 보는 REFUSED
  줄은 항상 남긴다.
- **L5. [고침 — 2026-08-20]** 창이 좁고 fail-closed 인 것은 맞지만 **회복이 자동이 아니다** —
  프로세스가 안 죽으니 `Restart=on-failure` 도 안 걸리고, polkitd 가 한 번 더 재시작하지
  않는 한 낡은 고유 이름으로 진짜 폴킷을 계속 거절한다. 고침은 순서 하나: **소유자를 읽기
  전에 구독한다.** 곁가지로 (a) 이미 아는 소유자로는 재등록하지 않고, (b) 등록 실패를 갈라
  `already exists for the given subject` 일 때만 정상 종료한다 — 지금까지는 polkitd 가 잠깐
  없어서 실패한 경우까지 삼켜 세션이 조용히 에이전트 없이 남았다. 유닛테스트 1개 +
  시나리오 §9, 그리고 진짜 재시작은 `--restart-polkitd` 뒤에 뒀다. `rationale.md` §17-2.

---

## 시험이 보장하는 것 / 못 잡는 구멍

`cargo test`(helper_protocol.rs)는 줄 프로토콜의 각 갈래와 쿠키가 stdin 으로 가는 것을
실제로 보장한다. scenarios.sh 는 발신자 거절·창 미표시·dumpable=0·VmLck·화면캡처
제외(1101↔4색)를 진짜 polkitd/Hyprland 로 본다. 이름값을 한다.

**못 잡는 구멍:**

1. **잠긴 계정 게이팅(C1) — [보강함].** `tests/scenarios.sh` §6 을 추가했다. 진짜
   faillock 을 태우지 않고, tally 만 10 건으로 흉내 내는 가짜 `faillock` 을 PATH 앞에 두어
   `--agent-prompt` 가 창 없이 종료 코드 2 로 끝나는지 본다. 실제 카운터는 건드리지 않는다.
2. **setuid 검증(C3)** 시험 없음 — fake-helper 가 setuid 아닌데 통과하므로 오히려
   "검증 안 함" 을 방증했다.
3. **크로스-쿠키 반복 유발** 시험 없음.
4. fork 폴백은 이 커널에서 실물 재현 안 됨(문서 인정) — 소켓 침묵만 단위 시험으로 대체.
5. **polkitd 실제 재시작(L5) — [닫힘]** `tests/scenarios.sh --restart-polkitd` 로
   실제 재시작까지 돌려 통과했다 (45/45, 2026-08-20). 비밀번호가 한 번 필요해 기본
   실행에는 없을 뿐이다.
