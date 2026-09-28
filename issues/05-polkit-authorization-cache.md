# polkit 인증 재사용과 팝업 승인 범위

- 조사일: 2026-09-28
- 상태: 정책 및 실제 동작 검증 필요 — 기본 정책 확인, 캐시 범위 실험은 하지 않음

## 확인한 사실

설치된 `/usr/share/polkit-1/actions/org.freedesktop.systemd1.policy`의 `org.freedesktop.systemd1.manage-units`에는 다음 기본값이 있었다.

```xml
<allow_any>auth_admin</allow_any>
<allow_inactive>auth_admin</allow_inactive>
<allow_active>auth_admin_keep</allow_active>
```

polkit 공식 문서는 `AUTH_ADMIN_KEEP` 등의 결과를 일정 기간 재사용하며, 같은 action identifier와 subject에 대해서는 세부 변수가 달라도 승인이 재사용될 수 있다고 설명한다. 추가 규칙과 요청 주체에 따라 최종 동작은 달라진다.

## 위험과 전제

사용자가 팝업 한 번을 특정 명령 하나만의 승인으로 이해하면 실제 정책과 기대가 어긋날 수 있다. 캐시나 허용 규칙에 의해 인증이 필요 없다고 판단되면 sudo-pop은 호출되지 않으므로 매번 팝업을 강제하지 못한다.

기본 정책만으로 모든 후속 run0 명령이나 같은 UID의 모든 프로세스가 캐시를 공유한다고 단정할 수 없다. 서로 다른 요청 프로세스에 적용되는 범위는 실제 확인이 필요하다. 인증 캐시는 그 자체로 취약점이라기보다 승인 범위와 사용성의 정책 선택이다.

## 기존 방어와 한계

sudo-pop은 polkit의 인증 에이전트이며 권한 판단의 최종 주체가 아니다. UI만 바꿔서는 이미 승인된 요청에 대해 인증을 다시 요구할 수 없다.

## 개선 방향

- 매 요청 인증이 필요한지, 어떤 범위의 재사용을 허용할지 정한다.
- 적용된 규칙, action, subject, 세션 상태를 기준으로 실제 캐시 범위를 조사한다.
- 변경이 필요하면 polkit 정책에서 해결하고 다른 systemd 관리 작업에 주는 영향도 검토한다.

## 완료 검증

격리 환경에서 같은 요청 주체와 다른 프로세스, 다른 작업 인자, 캐시 만료 전후, 활성·비활성 세션을 비교한다. 팝업 생략이 캐시 때문인지 별도 허용 규칙 때문인지 구분한다.

## 근거

- [polkit 공식 문서: Authorization Rules 및 `*_KEEP` 주의사항](https://polkit.pages.freedesktop.org/polkit/polkit.8.html)
- 조사 당시 설치된 systemd action 정책
