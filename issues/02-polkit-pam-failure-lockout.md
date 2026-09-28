# polkit PAM의 계정 단위 실패 누적 부재

- 조사일: 2026-09-28
- 상태: 정책 검토 필요 — 현재 머신 설정 확인, 반복 추측 공격은 재현하지 않음
- 구분: 시스템 PAM 구성과 sudo-pop의 요청별 제한

## 확인한 사실

조사 당시 `/etc/pam.d/polkit-1`의 인증 부분은 다음과 같았다.

```pam
auth      [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth      sufficient pam_fprintd.so
auth      required pam_unix.so
```

해당 파일은 `pam_faillock`을 호출하지 않으며 `system-auth`를 포함하지 않는다. account, password, session에는 각각 `pam_unix.so`가 설정되어 있었다. 이는 이 머신의 조사 시점 설정이며 모든 Omarchy 설치의 상태를 의미하지 않는다.

[`src/prompt.rs`](../src/prompt.rs)의 `run_attempts_with`는 요청 하나당 최대 3회를 허용한다. 새 요청에는 새 제한이 적용된다. [`src/attempts.rs`](../src/attempts.rs)의 `budget`은 해당 PAM 스택에 `pam_faillock`이 없으면 실패 예산을 반환하지 않는다.

## 위험과 전제

polkit 인증 요청을 반복할 수 있는 로컬 주체에 대해 sudo-pop의 요청별 상한만으로 계정 전체의 비밀번호 추측 횟수를 제한할 수 없다. PAM 지연과 다른 시스템 제약의 존재·효과는 별도 검증 대상이다. 현재 설정에 대해 faillock 누적 잠금이 적용된다고 안내하면 잘못된 보안 기대를 만든다.

이 문제는 이번 지문 허용 수정이 만든 것이 아니다. 지문 인증의 허용 순서와 비밀번호 실패 누적 정책은 별도로 결정해야 한다.

## 기존 방어와 한계

요청별 재시도 제한은 실수로 연속 오답을 제출하는 것을 줄인다. 그러나 사용자 프로세스 안의 UI 제한은 시스템 전체의 인증 정책을 대신할 수 없다. 실제 비밀번호·지문 검증은 PAM이 수행한다.

## 개선 방향

- 원하는 계정 잠금 범위, 실패 횟수, 해제 시간, 지문 인증과의 순서를 먼저 정한다.
- 필요하다면 시스템 PAM에 누적 실패 정책을 적용한다. PAM 변경은 별도 작업으로 진행한다.
- 잠금을 악용한 서비스 거부와 로그인·sudo·polkit 간 잠금 공유 영향을 함께 검토한다.

## 완료 검증

격리 환경의 테스트 계정으로 요청 간 실패 누적, 잠금 중 비밀번호 거절, 합의한 지문 동작, 잠금 해제를 확인한다. 실제 사용자 계정을 잠그는 시험을 문서 작성 과정에서 실행하지 않았다.

## 근거

- [`docs/omarchy-polkit-pam.md`](../docs/omarchy-polkit-pam.md)
- [`docs/issue-locked-account-fingerprint.md`](../docs/issue-locked-account-fingerprint.md)
- 조사 당시 읽은 `/etc/pam.d/polkit-1`
