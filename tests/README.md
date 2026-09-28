# 테스트 실행과 검증 범위

## 자동 회귀 테스트

```sh
cargo test --offline
cargo fmt --check
bash -n tests/scenarios.sh tests/input-scenario.sh tests/fake-helper.sh
git diff --check
```

helper 통합 테스트는 로컬 Unix 소켓을 생성한다. 소켓 생성을 금지하는 샌드박스에서는 해당 테스트가 권한 오류로 실패하므로 이를 제품 결함이나 SKIP으로 숨기지 말고 실행 환경을 확인한다.

비밀번호 입력 관련 검증:

| 경로 | 검증 내용 |
|---|---|
| `secure_input::tests` | egui 이벤트 마스킹, 실행 취소 기록, 한글·이모지, 삭제·선택·붙여넣기, IME 조합·취소·확정, 길이 제한, 입력 순서, 지연된 입력 재전달 |
| `gui::tests::real_input_hook_*` | 실제 `Window::raw_input_hook`과 입력 필드, 창 포커스 상실·복귀, Enter 제출, 인증 대기 중 입력 거절, 재시도. 매 프레임 포커스를 강제로 주지 않음 |
| `gui::tests::changing_to_an_echoed_prompt_*` | 숨김 입력이 echo-on 프롬프트에 노출되지 않도록 기존·대기 입력 폐기 |
| `secret::tests` | 삭제한 바이트 덮어쓰기, 버퍼 주소 유지, 공유 페이지 잠금 참조 관리, 다른 버퍼 해제 후 `/proc/self/smaps`의 실제 커널 잠금 플래그 확인 |

## 격리된 GUI 입력 시나리오

```sh
./tests/input-scenario.sh
# 같은 시험의 진입점:
./tests/scenarios.sh --input-only
```

Hyprland 세션에서 `cargo`, `hyprctl`, `jq`, `wtype`이 필요하다. 실행 전 현재 소스로 debug 바이너리를 항상 빌드한다. 필요한 도구나 세션이 없으면 `SKIP`을 출력하고 **77**로 종료한다. 성공은 0, 시험 실패는 1이다.

이 시험은 실제 창을 띄우고 잠시 키보드 포커스를 사용한다. 직접 `--agent-prompt`를 실행해 가짜 helper에 연결하므로 실제 계정 비밀번호를 입력할 필요가 없다. 입력할 문자열은 테스트 전용이며 로그에는 답변 내용 대신 `MATCH` 또는 `MISMATCH`만 기록한다.

1. 앞뒤 공백을 포함한 `  sudo-pop 한🔐 test  `를 입력하고, 추가 문자를 Backspace로 삭제한 뒤 Enter로 제출한다. helper가 문자열 전체를 정확히 비교하고 프로세스의 성공 종료를 확인한다.
2. 틀린 테스트 문자열을 입력해 `MISMATCH`를 확인한다. 같은 창에서 정답 테스트 문자열을 다시 입력하고 `MISMATCH → MATCH`와 성공 종료를 확인한다.

테스트 창은 PID와 app-id로 식별하며, 해당 창만 종료하고 이전 창으로 포커스를 돌린다. polkit 에이전트 등록, 서비스·PAM·설치 설정 변경, 실제 faillock 초기화, 클립보드 변경은 하지 않는다. 현재 Hyprland의 Lua focus dispatcher를 사용한다.

`wtype`의 Unicode 입력 성공은 실제 사용자 IME의 동작까지 입증하지 않는다. 실제 IME 조합은 별도 수동 검증 대상이고, egui IME 이벤트 처리 자체는 자동 테스트로 확인한다.

## 전체 데스크톱 시나리오

```sh
./tests/scenarios.sh
./tests/scenarios.sh --with-password
./tests/scenarios.sh --restart-polkitd
```

이 기존 종합 스크립트는 에이전트 전환, 설치·제거, 실제 polkit 요청 등을 시험하며 사용자 서비스와 설정에 영향을 준다. 격리된 입력 시험과 달리 실제 데스크톱 상태를 사용하므로 전용 테스트 세션에서 실행하는 것이 적절하다. `--with-password`는 사용자 인증을 요구하고 `--restart-polkitd`는 시스템 polkitd 재시작을 포함한다.

- 세션에 영향을 주기 전에 debug/release 바이너리를 모두 현재 소스로 빌드한다. 빌드 실패 시 시험을 시작하지 않는다.
- PASS·FAIL·SKIP을 구분한다. 도구 부재나 옵션 미지정으로 생략한 시험을 성공으로 집계하지 않는다.
- 종료·중단 시 **faillock 기록을 초기화하지 않는다.** 기존 기록과 테스트 도중 다른 요청이 만든 기록을 구분할 수 없기 때문이다.
- 기존 서비스 복구 절차가 있어도 모든 사용자 상태가 완벽히 되돌아간다고 가정하지 않는다. 실제 PAM 인증 실패가 발생하면 해당 기록은 유지된다.
- 새 격리 GUI 입력 시나리오도 포함하며, 실패와 환경 부재를 각각 FAIL과 SKIP으로 집계한다.

## 2026-09-28 보완 검증

전체 Cargo 테스트 169개와 격리 GUI 시나리오 2개가 통과했다. 전체 데스크톱 시나리오는 셸 문법과 코드를 점검했으며 이번 보완 과정에서 실행하지 않았다. 실제 비밀번호·지문 인증과 실제 IME 조합에 대한 수동 검증은 별도로 남는다.
