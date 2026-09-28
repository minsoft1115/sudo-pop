# run0 사용 의도와 실제 sudo 실행 경로의 불일치

- 조사일: 2026-09-28
- 상태: 일부 해결 — PATH 래퍼의 강제 설정 제거. 기존 sudo 호환 분기는 검토 대상으로 유지

## 확인한 사실

최초 조사 당시 [`assets/sudo-wrapper.sh`](../assets/sudo-wrapper.sh)는 `SUDO_POP_RUN0=0`을 설정해 일반 명령도 실제 sudo로 실행했다. 이후 사용자 요청으로 강제 설정을 제거했다. 현재 래퍼는 기존 sudo-pop 분기를 사용하며 호출자가 명시한 환경변수는 그대로 전달한다. 이는 alias를 PATH 래퍼로 교체하는 요청에 구현 과정에서 추가된 동작이며, 사용자는 run0 사용이 목적이라고 명확히 밝혔다.

이 강제 설정과 별개로 [`src/wrapper.rs`](../src/wrapper.rs)에는 기존부터 다음 sudo 경로가 있다.

- sudo 옵션이나 선행 환경변수 할당 등 run0로 그대로 전달하지 않는 호출
- `-A`, `-n`, `-S` 등 인증 입력 방식이 지정된 호출
- 인자가 없는 호출
- run0를 `exec`하지 못했을 때의 fallback

마지막 항목은 run0 실행 파일 시작 실패에 대한 처리다. run0가 정상 시작한 뒤 인증을 거절하거나 명령이 실패한 경우까지 sudo로 재시도한다는 뜻은 아니다.

## 위험과 전제

사용자가 run0를 통해 setuid sudo 실행을 피한다고 기대해도 실제 호출은 sudo로 갈 수 있다. 두 경로는 sudoers와 polkit, 인증 캐시, 실행 환경 등 정책이 다르므로 호출 형태에 따라 보안 기대가 달라진다.

이는 현재 sudo에서 악용 가능한 취약점을 확인했다는 뜻이 아니다. 또한 PATH를 run0로 연결해도 시스템에 남은 setuid sudo를 직접 실행할 수 있는 상태라면 해당 공격 표면이 제거되는 것은 아니다.

## 개선 방향

- 완료: PATH 래퍼의 `SUDO_POP_RUN0=0` 강제 설정을 제거한다.
- 기존의 sudo 호환 분기를 유지할지, 명시적으로 선택하게 할지, run0 전용 모드에서 거절할지 결정한다.
- run0 실행 불가 시 조용히 sudo로 전환할지, 실패로 종료할지 정책을 정한다.
- 시스템 sudo 제거 또는 setuid 변경은 업데이트·복구 경로에 영향이 있으므로 래퍼 변경과 분리한다. 이 문서는 해당 작업을 승인하거나 수행하지 않는다.

## 완료 검증

실제 권한 상승 없이 가짜 실행 파일 또는 테스트용 실행 추상화로 일반 명령·옵션·환경변수·run0 시작 실패 경로를 확인한다. 인증 거절 시 sudo 재시도가 없음을 구분해 검증한다. 합의한 정책과 설치 문서, 테스트, 실제 래퍼가 일치해야 한다.

## 근거

- [`src/wrapper.rs`](../src/wrapper.rs): `plain_command`, `exec_run0`, `run`
- [`assets/sudo-wrapper.sh`](../assets/sudo-wrapper.sh)
- [systemd run0 공식 설명](https://github.com/systemd/systemd/blob/main/man/run0.xml)
