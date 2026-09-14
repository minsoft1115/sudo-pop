# Omarchy 지문 설정이 polkit 에서 faillock 을 빼는 문제 — 조사 기록

2026-09-14. sudo-pop 창의 "N attempt(s) left before the account locks" 가 polkit 오답에
줄어들지 않는다는 관찰에서 출발했다. 결론은 sudo-pop 의 결함이 아니라 Omarchy 의
`omarchy-setup-security-fingerprint` 가 만든 `/etc/pam.d/polkit-1` 때문이고, 의도가 아니라
실수로 보인다. 설계 쪽 판단은 [`rationale.md`](rationale.md) §24 에 있고, 이 문서는 증거와
경위를 upstream 에 보고할 수 있는 형태로 모아 둔 것이다.

## 1. 증상

- 창에서 비밀번호를 틀리면 `Wrong password` 는 뜨는데 잔여 횟수 줄이 그대로다.
- `faillock --user lmh` 표가 비어 있다. polkit 오답이 기록되지 않는다.
- sudo 에서 틀리면 정상적으로 쌓인다.

## 2. 타임라인 (이 머신, Omarchy 4.0.3, polkit 127-3)

| 언제 | polkit 이 쓰는 PAM 파일 | faillock |
|---|---|---|
| 2026-08-19, 08-20 | `/usr/lib/pam.d/polkit-1` (polkit 패키지 소유) | 저널에 `pam_faillock(polkit-1: …)` 3건 + 6건 |
| 2026-09-11 09:36 | `omarchy-setup-security-fingerprint` 실행 (`fprintd-enroll` 09:36:25) | |
| 2026-09-11 10:04 | `/etc/pam.d/polkit-1` 생성 (`pacman -Qo`: 어느 패키지 소유도 아님) | 이후 저널에 한 건도 없음 |

PAM 은 `/etc/pam.d/<service>` 를 먼저 보고 없을 때만 `/usr/lib/pam.d/<service>` 를 본다.
9월 11일부터는 새 파일이 polkit 의 전부다.

## 3. 두 파일

`/usr/lib/pam.d/polkit-1` — polkit 패키지가 주는 것:

```
#%PAM-1.0
auth       include      system-auth
account    include      system-auth
password   include      system-auth
session    include      system-auth
```

`/etc/pam.d/polkit-1` — 스크립트가 만든 것:

```
auth      [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth      sufficient pam_fprintd.so
auth      required pam_unix.so

account   required pam_unix.so
password  required pam_unix.so
session   required pam_unix.so
```

`system-auth` 가 어디에도 없다. 거기에 있던 `pam_faillock` (preauth / authfail / authsucc),
`pam_systemd_home`, `pam_env`, `pam_permit` 이 모두 빠진다. 이 머신의
`/etc/pam.d/system-auth` 3·8·11행이 그 세 줄이고, `deny=10 unlock_time=120` 이다.

## 4. 원인 — 스크립트의 두 갈래

`/usr/share/omarchy/bin/omarchy-setup-security-fingerprint` 의 polkit 부분:

```sh
if [[ -f /etc/pam.d/polkit-1 ]]; then
  # 있으면 맨 위에 pam_fprintd 한 줄과 덮개 게이트를 끼워 넣는다
  sudo sed -i '1i auth      sufficient pam_fprintd.so' /etc/pam.d/polkit-1
  sudo sed -i "/pam_fprintd\.so/i $fprintd_gate" /etc/pam.d/polkit-1
else
  # 없으면 새로 만든다
  sudo tee /etc/pam.d/polkit-1 >/dev/null <<EOF
$fprintd_gate
auth      sufficient pam_fprintd.so
auth      required pam_unix.so

account   required pam_unix.so
password  required pam_unix.so
session   required pam_unix.so
EOF
fi
```

Arch 는 polkit-1 을 `/usr/lib/pam.d` 에만 두므로 **모든 Omarchy 설치가 둘째 갈래를 탄다.**
sudo 쪽은 `/etc/pam.d/sudo` 가 원래 있어서 첫째 갈래(끼워 넣기)만 있고, 그 파일의
`include system-auth` 가 살아 있다. 그래서 sudo 는 faillock 이 되고 polkit 은 안 된다.

## 5. 버그인가 의도인가

의도라는 근거가 없고, 실수라는 정황이 셋이다.

1. heredoc 은 2025-08-24 PR #635 "Fix fido2 and fprint auth flow" 에서 생겼다. 그 전의
   스크립트는 `/etc/pam.d/polkit-1` 을 `rm -rf` 로 지워 vendor 파일로 되돌렸다. PR 설명은
   (a) 지문이 비밀번호 뒤에 붙어 순서가 틀렸다, (b) fido2 가 polkit 에 없었다, (c) 제거가
   파일을 통째로 지웠다 — 셋이다. 리뷰의 유일한 논의는 fido2 의 물리 접근 우려다. faillock,
   system-auth, 계정 잠금은 어디에도 등장하지 않는다.
2. 같은 PR 이 sudo 는 끼워 넣기만 했다. polkit 만 `pam_unix` 로 새로 쓴 이유가 설명된 곳이
   없다. 파일이 없으니 "일단 동작하는 최소 스택" 을 적은 모양이다. 같은 PR 의 fido2 스크립트와
   마이그레이션(`migrations/1754860578.sh`)도 같은 heredoc 을 쓴다.
3. 2026-09 현재 upstream master 의 같은 스크립트도 이 heredoc 그대로이고, 이슈·PR 검색
   (`polkit-1`, `pam`, `faillock`, `fingerprint`)에 이 문제를 다룬 것이 없다. 가까운 것은
   #10254 (덮개 게이트가 pam_exec 오류를 남긴다) 뿐이다.

부수 효과가 하나 더 있다. faillock 이 sudo·로그인에서 계정을 잠가도 **polkit 은 그 계정의
비밀번호를 받는다.** 잠금이 polkit 경로에서 우회된다는 뜻이다.

## 6. 고치는 모양

첫째 갈래가 만들었을 것과 같게, vendor 파일을 복사한 위에 두 줄을 끼우면 된다. 이 머신에서
직접 고치려면 (root):

```sh
sudo cp /etc/pam.d/polkit-1 /etc/pam.d/polkit-1.omarchy-orig
sudo tee /etc/pam.d/polkit-1 >/dev/null <<'EOF'
auth      [success=1 default=ignore] pam_exec.so quiet /usr/bin/omarchy-hw-laptop-closed
auth      sufficient pam_fprintd.so
auth      include system-auth
account   include system-auth
password  include system-auth
session   include system-auth
EOF
```

확인: run0 창에서 오답 한 번 → `faillock --user <name>` 에 `polkit-1` 행이 생긴다 →
`faillock --user <name> --reset`. Omarchy 의 sudo 와 같은 성질이 하나 남는다:
`pam_fprintd` 가 faillock 보다 앞이라 잠긴 계정도 지문으로는 통과한다.

upstream 에서 고친다면 스크립트의 둘째 갈래를 `cp /usr/lib/pam.d/polkit-1 /etc/pam.d/polkit-1`
뒤 첫째 갈래로 합치는 것이 가장 작다. fido2 스크립트와 마이그레이션도 같은 heredoc 이다.

## 7. sudo-pop 쪽 대응 (커밋 f830e7b)

숫자의 출처는 틀리지 않았다 — `deny` 는 `/etc/security/faillock.conf`, 실패 건수는
`faillock --user`. 틀린 것은 "이 비밀번호가 그 표를 움직인다" 는 전제였다. 그래서
`attempts::budget(service)` 가 답하는 PAM 스택의 `auth` 줄을 `include`·`substack`·`@include`
를 따라 읽고, `pam_faillock` 이 없으면 줄도 잠금 거절도 내지 않는다. 스택은 요청마다 디스크에서
다시 읽으므로, 파일이 고쳐지면 다음 창부터 줄이 자동으로 돌아온다. 재설치·재시작은 필요 없다.

## 8. 참고

- 스크립트: `/usr/share/omarchy/bin/omarchy-setup-security-fingerprint` (polkit 부분 33~56행)
- upstream: https://github.com/basecamp/omarchy/pull/635,
  https://github.com/basecamp/omarchy/blob/master/bin/omarchy-setup-security-fingerprint
- 저널 확인: `journalctl | grep "pam_faillock(polkit-1"`
- Omarchy 자체 에이전트(`/usr/share/omarchy/shell/plugins/polkit/PolkitAgent.qml`)는 횟수를
  세지도 보여 주지도 않으므로 이 문제가 눈에 띌 곳이 sudo-pop 밖에 없었다.
