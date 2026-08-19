# sudo-pop 테스트 보강 계획

> 점검(`docs/audit.md`)에서 드러난 미테스트 구역을 채운다. **"왜 필요한가"가 없는
> 항목은 넣지 않고**, 못 만들 것/만들면 억지인 것은 이유와 함께 뺀다.
>
> 원칙: 미테스트 로직은 대부분 `-> !`(프로세스 종료)·GUI·D-Bus·syscall 에 묶여 그대로는
> 테스트가 안 된다. 그래서 계획의 절반은 **보안·결함 로직을 순수 함수로 얇게 추출**하는
> 것이고, 그 추출 자체가 audit 가 짚은 지점을 테스트 가능하게 만드는 실질 개선이다.
> 리팩터는 전부 동작 변화 없는 "순수 함수 추출 + 얇은 래퍼" 수준으로 한다.

상태: **[완료]** / **[진행]** / **[대기]**

> **Tier 1 완료** (2026-08-19): 신규 유닛테스트 27개(lib 24→51). 리팩터는 전부
> 순수 함수 추출로 동작 변화 없음. Tier 2 는 대기.

---

## Tier 1 — 꼭 필요 (audit 가 짚은 보안·결함 지점을 직접 덮음)

### 1. `attempts.rs` — faillock 예산/카운터  **[완료]** (8개)
지금 유닛테스트 0. 가장 큰 공백이고, 이번에 고친 C1 게이팅의 근거 로직이다.
- 카운터 왕복: `reset→used()==0`, `record()` 1→2 증가, **stale 60초 경과 시 무시**
  (temp `XDG_RUNTIME_DIR` 로 격리)
- `deny` 파싱: `faillock.conf` 의 `deny = 10` 과 pam 모듈 라인의 `deny=10` 두 형식
  → 순수 `parse_setting(text, key)` 추출
- tally 파싱: **`V` 행만 세고 `I` 는 버림, newest 계산** → 순수 `parse_tally(text)` 추출
  (지금은 `faillock` 셸아웃과 엉켜 있음)
- **`Budget::warning()` / `Budget::refusal()` 신설** — WARN_BELOW(≤3 경고 문구)와 locked
  안내를 여기로 옮겨 askpass·prompt 가 공유. 두 파일의 복붙 `format!` 을 없애고 한 곳에서
  테스트한다
- 예상 ~8개

### 2. `agent.rs` — 발신자 검증 + 종료코드 매핑  **[완료]** (2개)
- `is_polkitd(sender)`: 소유자와 같으면 true, 다르면/`None` 이면 false (그대로 테스트 가능)
- **종료코드→결과 매핑을 `fn result_for(code)` 로 추출**: `0→Ok`, `2(취소)→Ok`, `그 외→Err`.
  틀리면 빈 창 무한 반복(§3-3 함정)이라 못 박을 값어치가 크다
- 예상 ~4개

### 3. `lib.rs` — `choose_identity`  **[완료]** (4개)
- 현재 uid 우선 / 없으면 첫 번째 / `unix-user` 아닌 항목 스킵 / 빈 목록 `None`
- `me: u32` 와 이름 resolver 를 인자로 받게 얇게 파라미터화(지금은 `getuid`/`getpwuid` 직접
  호출이라 비결정적)
- 예상 ~4개

### 4. `paths.rs` — 런타임 디렉터리 검증  **[완료]** (7개)
원래 의심 지점이었다.
- `basename` (askpass 모드 판별의 기반): 경로/심볼릭/빈 값
- `ensure_private_dir`: **0700 아니면 거부, 소유자 다르면 거부, 심볼릭이면 거부, 없으면
  0700 으로 생성** (temp `XDG_RUNTIME_DIR`)
- `ensure_askpass_symlink`: 올바른 링크 재사용 / 잘못된 타깃 교체 / 심볼릭 아님 교체
- 예상 ~6개

### 5. `init.rs` — 남의 파일 다루기  **[완료]** (6개)
원래 의심 지점, 지금은 scenarios 로만 확인된다.
- `add_block`/`remove_block` 을 **순수 문자열 변환으로 추출**(`insert_block(text)->String`,
  `strip_block(text)->Result<Option<String>>`): 멱등(두 번째 no-op), 마커 앞뒤 내용 보존,
  **begin 만 있고 end 없으면 건드리지 않음**
- `omarchy_polkit_enabled`: 손으로 짠 JSON 스캔이라 취약 — `enabled:true`/`false`/부재
  샘플로 검증
- 예상 ~7개

---

## Tier 2 — 있으면 좋음

- **6. `helper.rs` `split_tag`** — 태그/본문 분리(공백 0/1개), 프로토콜 파싱의 기초. ~2개
- **7. `prompt.rs` 재시도 루프** — "3회 후 정지, 오답 사이 `Wrong`". 루프를 mock
  `authenticate` 클로저로 받게 추출하면 테스트 가능. 애매하면 scenarios 로 남긴다
- **8. `scenarios.sh` 에 M3 추가** — 취소된 **대기(큐) 요청**이 차례가 와도 창을 안 띄우는지

---

## 명시적 제외 (만들면 억지라 안 함)

| 파일 | 왜 안 하나 |
|---|---|
| `gui.rs` | winit 이벤트 루프·창. 유닛테스트 부적합 — scenarios 가 창 규칙·Esc·크기를 실물로 봄 |
| `font.rs` | `fc-match` 셸아웃뿐, 분기 없음 |
| `harden.rs` | `prctl`/`setrlimit` 커널 호출 — scenarios 가 `/proc` 로 dumpable·core·VmLck 확인 중 |
| `main.rs` 모드 분기·`session_id` | argv/D-Bus/logind 환경 의존. askpass 판별은 `paths::basename` 으로 커버 |
| `helper.rs` `authenticate` 왕복 | 이미 `tests/helper_protocol.rs` 8개가 덮음 |

---

## 규모/방식/순서

- 새 유닛테스트 **~30개**, 리팩터는 전부 동작 변화 없는 순수 함수 추출.
- 기존 컨벤션: 각 파일 하단 `#[cfg(test)] mod tests`, 이름은 영어 snake_case.
- 순서: **1 → 2 → 4 → 5 → 3 → (Tier 2)**. 1·2 가 이번에 손댄 결함(faillock 게이팅·종료코드)을
  바로 잠근다.
