# sudo-pop

[English](README.md) · **한국어**

Omarchy 에서 권한이 필요한 모든 순간 — `sudo`·`run0`·디스크 마운트·NetworkManager·
systemctl — 의 비밀번호를 한 창에서 받는다. 그 창은 메모리·코어덤프·화면 공유·로그를 통한
비밀번호 노출을 줄이도록 설계했다.

**polkit 인증 에이전트**와 **sudo 인증창 연결**을 제공한다. 기본 래퍼는 run0 를 실행하고 에이전트가 인증 창을 표시한다:

```
sudo pacman -Syu   →  run0 pacman -Syu   ─┐
sudo -E make       →  지원하지 않음 (/usr/bin/sudo 명시 호출)
디스크 마운트 · NetworkManager · systemctl ─┘→ 같은 창
```

<p align="center">
  <img src="screenshots/sudo-pop.png" width="440"
       alt="sudo-pop 창 — 입력칸 위에 명령 pacman -Syu, 구석에 남은 시간,
            아래에 잠금까지 남은 횟수">
</p>

설치하면 터미널의 PATH 에 전용 `sudo` 래퍼를 추가한다. **기본값은 run0 전용**이다.
sudo 전용 옵션·선행 환경변수 할당은 거절하며, run0 시작 실패 시 sudo 로 전환하지 않는다.
터미널에서 시작한 CLI 에이전트와 스크립트도 PATH 를 상속한다.
실제 sudo 가 필요하면 `/usr/bin/sudo` 를 명시적으로 호출하거나,
`SUDO_POP_MODE=compat` 으로 기존 혼합 라우팅을 선택한다.
`sudo -v` 를 사용하는 스크립트와 현재 Omarchy 업데이트 경로에 호환성 영향이 있다.
자세한 선택 방법은 [요청 표시와 실행 정책](docs/request-display-and-run0.md)을 참고한다.

창의 명령은 요청 프로세스의 명령행을 참고용으로 표시한다. 상세 보기에서 전체 수집 명령,
구분된 인자, polkit 작업과 설명을 확인한다. 실제 실행이나 스크립트 내용을 보증하지 않는다.
상세 보기를 펼쳐도 지문·비밀번호 인증 및 요청 제한 시간은 계속 진행된다.

---

## Omarchy 기본 에이전트와 다른 점

Omarchy 는 자기 polkit 에이전트 `omarchy.polkit` 을 갖고 있다 — 셸 프로세스 안에서 도는
QML 서비스다. 그걸 교체하는 건 실제 선택이니, 무엇이 달라지는지 적어 둔다. 아래는 전부
직접 재 보고 확인한 것이다:

| | sudo-pop | omarchy.polkit |
|---|---|---|
| 비밀번호 하드닝 — 덤프 방지, RAM 잠금, 버퍼 삭제 | ✓ | ✗ — 비밀번호가 오래 사는 셸 프로세스 안에 있다 |
| 화면 공유·녹화에서 제외 | ✓ | ✗ — 레이어 서피스엔 규칙을 못 건다 |
| **요청 프로세스의 명령행**을 보여줌 | ✓ `pacman -Syu` | 난수 유닛 이름 (`run-p1592…service`) |
| 데스크톱 요청은 무엇을 할지까지 | ✓ `mount the filesystem` | ✓ |
| 폴킷이 아닌 호출자를 거절 | ✓ | ✗ — 참고 구현 둘 다 안 한다 |
| 공유 잠금 예산을 상시 표시 · 잠금 중 비밀번호 차단 | ✓ | ✗ |
| 호출자의 25초 마감을 카운트다운 | ✓ | ✗ |
| `sudo` 와 polkit 프롬프트가 한 창 | ✓ | sudo 는 그대로 |
| 시스템 다이얼로그와 맞춘 테마색 | ✓ | ✓ |
| 지문 | ✓ PAM 통과 + 대기 아이콘 | ✓ |

이 중 둘 — 폴킷 아닌 호출자 거절, `run0` 요청 뒤의 명령 표시 — 은 omarchy 도
hyprpolkitagent 도 하지 않는다.

---

## 비밀번호가 가는 곳, 못 가는 곳

요청마다 짧게 살고 죽는 자식이 처리하고, 비밀번호가 메모리에 닿기 전에 스스로를
하드닝한다:

- 하드닝 성공 시 코어덤프와 일반적인 동일 사용자 디버거 접근을 차단한다
- `mlock` 성공 시 비밀번호 버퍼를 **RAM 에 잠그고**, 사용 후 지운다
- 숨김 입력은 egui 에 입력 식별자와 마스킹 문자로만 전달한다. 실행 취소·다시 실행은
  비활성화하고, 문자 삭제 후 보호 버퍼에 남는 바이트도 지운다
- 창이 **화면 공유와 녹화에서 빠진다**
- **로그·명령줄·환경변수 어디에도** 남지 않는다
- **폴킷이 부른 것만** 창을 띄운다 — 버스의 다른 프로세스가 부르면 창이 뜨기 전에 거절한다

메모리 보호 범위는 앱의 raw-input 훅부터다. 그 전에 OS·IME·클립보드·백엔드가 만든
복사본은 범위 밖이며, RAM 잠금은 최대절전 이미지까지 보호하지 않는다. 보호 설정 실패 시
현재는 경고 후 계속 진행한다. 검증과 한계는 [비밀번호 메모리 이슈](issues/01-password-memory-copies.md)에 기록했다.

경계는 담담하게: 이건 보안 벽이 아니라 편의 도구다. 이미 내 권한으로 도는 악성코드는
alias 도 바이너리도 바꿀 수 있다. 막아 주는 것은 **부주의로 인한 유출**이고 — 위 표대로,
셸의 에이전트가 닿지 못하는 곳에서 그걸 막는다.

---

## 요구 사항

| | |
|---|---|
| Omarchy | 4.0+ — 셸이 가진 polkit 에이전트를 비켜 줘야 한다 (아래) |
| Hyprland | 0.56+, Lua 설정. 창 규칙이 그걸 전제한다 |
| systemd | `run0` 때문에 256+. 261 에서 확인 |
| Rust | 빌드용. `mise` 를 쓰면 `mise.toml` 이 툴체인을 핀한다 |

---

## 설치

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash
```

빌드해서 `~/.local/bin` 에 넣고 `sudo-pop --init` 까지 한다. 쓰는 것이 전부 `$HOME` 안이라
**root 로 돌리면 안 된다** — 시도하면 거부한다.

`--init` 은 `~/.local/lib/sudo-pop/bin/sudo` 래퍼, 이를 PATH 에 넣는 셸 스니펫,
Hyprland 창 규칙과 require 블록, systemd user 유닛을 설치한다. 기존 sudo-pop alias 스니펫은
새 PATH 설정으로 교체한다. 래퍼는 실행 정책을 강제하지 않으며, 새 바이너리는 기본적으로 run0 전용 정책을 사용한다.

새 터미널을 열거나 `source ~/.bashrc` 한 뒤 에이전트를 다시 시작한다. 메뉴에서 직접 시작한
앱이나 별도 서비스의 PATH 는 바꾸지 않는다. 기존의 다른 `sudo` alias·함수는 보존한다.

### Omarchy 에게서 자리를 넘겨받기

한 세션에 폴킷 에이전트는 하나이고, Omarchy 셸이 기본으로 자리를 쥐고 있다. 그동안
`--init` 은 유닛을 깔되 **켜지 않고** 그 사실을 알려 준다. 넘겨받으려면:

```bash
omarchy plugin disable omarchy.polkit
sudo-pop --init
```

하드닝·화면 공유 제외·무엇이 묻는지 보여 주는 명령줄을 얻는다. 테마색은 그대로 따라온다 —
sudo-pop 이 셸의 `[polkit]` 팔레트를 읽어 같은 색을 쓴다. 지문 대기는 Omarchy 가 넣어 둔
PAM 스택을 그대로 쓴다. 창은 비밀번호를 묻기 전까지 센서 아이콘을 보여 준다. `--init` 은
지금 어느 에이전트가 자리를 쥐고 있는지 실행할 때마다 알려 준다.

## 제거

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
omarchy plugin enable omarchy.polkit
```

---

## 알아둘 것

askpass 가 아니라 polkit 에이전트라서 따라 나오는 것들이다:

- **설치된 `sudo` 래퍼는 기본적으로 run0 전용이다.** polkit 정책을 사용하므로 sudoers 가
  적용되지 않는다. sudo 전용 옵션은 거절하며 자동 sudo fallback 은 없다.
  `SUDO_POP_RUN0=0` 은 `SUDO_POP_MODE=compat` 을 명시한 경우에만 허용한다.
- **run0 경로에서는 25초 안에 입력한다** — 우리 창이 아니라 **호출자**의 D-Bus 타임아웃이다.
  창이 구석에서 1초 단위로 세어 주고, 마지막 5초는 경고색이다. sudo 경로에는 이 제한이
  없어서 아무것도 세지 않는다.
- **폴킷과 sudo 가 faillock 카운터 하나를 공유**해서, 이 창에서 틀린 것이 양쪽에 쌓인다.
  잠기기까지 몇 번 남았는지를 창이 **띄워 두는 내내** 보여 주고, 셋 이하로 떨어지면
  경고색으로 바뀐다. 해당 PAM 스택이 `pam_faillock` 을 사용할 때만 적용되며,
  이를 사용하지 않는 polkit 스택에는 횟수 표시나 잠금 차단이 없다.
- **비밀번호가 잠겨도 설정된 지문은 PAM 에서 먼저 시도합니다.** 덮개가 열려 있고 지문이
  설정된 polkit 요청에 해당합니다. 잠긴 상태에서 입력 단계로 넘어오면 답을 보내지 않고
  최대 5초 동안 잠금 안내를 보여 줍니다(Esc 로 닫기). 지문을 시도할 수 없으면 안내만
  표시합니다. askpass 경로의 지문은 sudo 가 창을 부르기 전에 처리합니다.
  faillock 기록을 초기화하거나 시스템 PAM 정책을 바꾸지는 않습니다.
- **`/usr/bin/sudo` 는 언제나 진짜 sudo 다.** `\sudo` 는 아니다 — alias 만 막을 뿐 PATH 래퍼나 셸 함수를 우회하지 않는다.

---

## 문서

| | |
|---|---|
| [docs/plan.md](docs/plan.md) | 무엇이며 구현이 무엇을 지켜야 하는가 |
| [docs/fingerprint.md](docs/fingerprint.md) | 지문: PAM 통과와 대기 UI |
| [docs/rationale.md](docs/rationale.md) | 왜 그렇게 했는지, 무엇을 재 봤는지, 무엇을 기각했는지 |
| [docs/audit.md](docs/audit.md) | 지금 코드 전수 점검과 무엇을 고쳤는지 |
| `old/` | 옛 구현(sudo askpass 래퍼)을 문서째 그대로 남겨 뒀다 |

---

## 개발

```bash
cargo test                            # 단위·프로토콜 시험. 환경이 필요 없다
./tests/scenarios.sh                  # polkitd·버스·컴포지터가 필요하다
./tests/scenarios.sh --with-password  # 입력이 필요한 한 케이스를 foot 창으로
./tests/scenarios.sh --restart-polkitd  # polkitd 를 재시작하고 에이전트가 따라가는지 본다
```

시나리오는 세션을 원래대로 돌려놓고, 무엇을 되돌렸는지 찍고, 태운 faillock 도 치운다.

## 라이선스

MIT
