# 이슈: 잠긴 계정에 지문 기회를 주지 않는다 (리뷰 F-7)

> **상태**: 열림. 결정 보류. 2026-09-14 자체 리뷰 [`review-2026-09-14.md`](review-2026-09-14.md) 의
> F-7 에서 나왔고, 코드는 바꾸지 않았다.
> **적용 조건**: 답하는 PAM 스택이 `pam_faillock` 을 돌릴 때만 해당된다 (rationale §24). 이 머신은
> `/etc/pam.d/polkit-1` 을 고치기 전에는 polkit 경로에서 체감되지 않는다. sudo 의 askpass 경로는
> 지금도 해당된다.

## 1. 지금 동작

`src/prompt.rs` 와 `src/askpass.rs` 는 창을 열기 전에 faillock 표를 읽고, 남은 횟수가 0 이면
그 요청을 종료 코드 2(취소)로 끝낸다. 헬퍼를 띄우지 않으므로 센서도 켜지지 않고 창도 없다.
이유 문구 `account locked, Ns to go` 는 stderr 로만 나가 저널에 남는다.

## 2. PAM 이 했을 것

Omarchy 가 만드는 스택 순서:

```
auth  [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth  sufficient pam_fprintd.so
auth  include    system-auth          ← pam_faillock preauth 는 여기 안에 있다
```

`sufficient` 는 앞선 required 실패가 없으면 성공 즉시 체인을 끝낸다. faillock 의 `preauth` 는
지문 다음이다. 즉 **PAM 은 잠긴 계정이라도 등록된 손가락이면 통과시킨다.** Omarchy 의 sudo 와
잠금 화면도 같은 순서라, 시스템의 다른 곳은 모두 그렇게 동작한다.

## 3. 갈리는 결과

| 잠금 2분 동안 | 결과 |
|---|---|
| 터미널의 `sudo` | 손가락을 받는다 |
| 잠금 화면 | 손가락을 받는다 |
| sudo-pop 을 거치는 run0 / pkexec | 창 없이 `Access denied`. 화면에 이유가 없다 |
| sudo-pop askpass (`sudo -A`) | 창 없이 실패. 안내는 저널에만 |

## 4. 거절의 근거는 무엇이었나

rationale §4-1: "잠긴 계정에 창을 띄우면 예산만 태운다." 비밀번호에 대해서는 맞다.

- 잠금 중의 오답은 `authfail` 이 기록해 잠금을 뒤로 민다.
- 정답도 `preauth` 가 이미 실패했으므로 통과하지 못하고, `authsucc` 는 잠긴 계정에 대해
  재설정하지 않는다.

지문에 대해서는 맞지 않다. 매치 실패는 `pam_fprintd` 안에서 끝나고 faillock 은 건드리지
않는다. "지문만 받고 비밀번호는 막는다" 가 PAM 과 같은 답이면서 예산도 지키는 형태다.

## 5. 보안 관점

우리 쪽이 더 보수적이다. 그러나 그 엄격함은 우리 창을 거치는 경로에만 있고 sudo·잠금 화면에는
없으므로, 공격자에게 장벽이 아니라 사용자에게만 걸리는 불일치다. 지문은 추측으로 뚫는 것이
아니고, 지문을 잠금보다 앞에 둔 것은 Omarchy 의 결정이다. 권한이 새는 방향의 문제는 아니다.

## 6. 선택지

1. **그대로 두고 문서에 한 줄.** 가장 작다. 불일치와 "화면 없이 실패" 는 남는다.
2. **잠긴 계정에도 창을 열되 지문 단계만 허용.** 스택에 `pam_fprintd` 가 있고 계정이 잠겼으면
   창을 열고 헬퍼를 띄운다. 지문이 맞으면 PAM 이 통과시킨다. 첫 프롬프트가 오면(지문 실패 뒤)
   비밀번호 칸 대신 `account locked, Ns to go` 를 보이고 입력을 받지 않으며 취소로 끝낸다.
   PAM 과 같은 답이고 예산도 안 태운다. 창의 상태 하나(F-5 의 `pam_muted` 옆에 `locked`)와
   시험이 늘고, 지문이 없는 스택에서는 지금과 같다.
3. **거절은 유지하되 창으로 알린다.** 잠금 안내를 작은 창으로 띄우고 닫는다. 불일치는 남지만
   "왜 실패했는지 모른다" 는 사라진다. 지문과 무관하게 지금도 있는 빈틈이다.

## 7. 제안

이 머신의 PAM 파일이 고쳐져 faillock 이 실제로 도는 상태가 되기 전에는 어느 쪽도 체감되지
않는다. 지금은 1 로 두고, 2 를 열린 항목으로 남긴다. 2 를 할 때는 3 의 안내도 같은 창에서
해결된다.

## 8. 참고

- 코드: `src/prompt.rs` (`budget` → `refusal` → `EXIT_CANCELLED`), `src/askpass.rs` 같은 자리,
  `src/attempts.rs` `Budget::refusal`
- 문서: rationale §4-1 (쿠키 단위 상한), §24 (세지 않는 스택), [`omarchy-polkit-pam.md`](omarchy-polkit-pam.md)
- pam_faillock(8): `preauth` / `authfail` / `authsucc` 의 의미
