# sudo-pop

[English](README.md) · **한국어**

Omarchy 에서 권한이 필요한 모든 순간 — `sudo`·`run0`·디스크 마운트·NetworkManager·
systemctl — 의 비밀번호를 한 창에서 받는다. 그 창은 비밀번호가 코어덤프·스왑·화면 공유·
로그로 새지 않게 만들어져 있다.

**polkit 인증 에이전트**와, 그 앞에 선 **sudo 라우터**다. 모든 길이 같은 창으로 모인다:

```
sudo pacman -Syu   →  run0 pacman -Syu   ─┐
sudo -E make       →  sudo -A -E make    ─┤→  같은 창
디스크 마운트 · NetworkManager · systemctl ─┘
```

<p align="center">
  <img src="screenshots/sudo-pop.png" width="440"
       alt="pacman -Syu 를 비밀번호 입력칸 위에 보여 주는 sudo-pop 창">
</p>

옵션 없는 명령은 `run0` 으로, 옵션이나 `VAR=값` 이 붙으면 `sudo` 에 남아 `-A` 만 더해
우리 창을 쓴다(원래 옵션은 그대로 유지된다). 맨 아랫줄은 sudo 를 안 거치는 네이티브 polkit
액션으로, sudo-pop 이 에이전트 자리를 쥐고 있을 때 이 창으로 온다. 어느 길이든 창 맨 위
줄이 요청 뒤의 실제 명령이다 — **무엇이 묻고 있는지**를 보여 준다. polkit 자신의
밋밋한 문구에는 그게 없다.

---

## 셸이 가진 에이전트가 못 하는 것

Omarchy 는 자기 polkit 에이전트 `omarchy.polkit` 을 갖고 있다 — 셸 프로세스 안에서 도는
QML 서비스다. 그걸 교체하는 건 실제 선택이라, 무엇이 달라지는지 적어 둔다. 아래는 전부
**실측이지 주장이 아니다**:

| | sudo-pop | omarchy.polkit |
|---|---|---|
| 비밀번호 하드닝 — 코어덤프·스왑 차단, RAM 잠금, wipe | ✓ | ✗ — 비밀번호가 오래 사는 셸 프로세스 안에 있다 |
| 화면 공유·녹화에서 제외 | ✓ | ✗ — 레이어 서피스엔 규칙을 못 건다 |
| **실제로 무엇이 묻는지** 명령을 보여줌 | ✓ `pacman -Syu` | 난수 유닛 이름 (`run-p1592…service`) |
| 폴킷이 아닌 호출자를 거절 | ✓ | ✗ — 참고 구현 둘 다 안 한다 |
| 남은 시도 경고 · 잠긴 계정 거부 | ✓ | ✗ |
| `sudo` 와 polkit 프롬프트가 한 창 | ✓ | sudo 는 그대로 |
| 지문 · 테마색 | ✗ | ✓ (지문은 Arch 에서 어차피 안 된다¹) |

이 중 둘 — 폴킷 아닌 호출자 거절, `run0` 요청 뒤의 명령 표시 — 은 omarchy 뿐 아니라
**기존 어떤 에이전트도 하지 않는** 것이다.

¹ omarchy 의 지문 모드는 `/etc/pam.d/polkit-1` 을 읽는데 Arch 엔 그 파일이 없다 —
실제 경로는 `/usr/lib/pam.d/polkit-1` 이다. 그래서 이 플랫폼에서는 그쪽만 가진 그 하나가
영영 켜지지 않는다.

---

## 비밀번호가 가는 곳, 못 가는 곳

요청마다 짧게 살고 죽는 자식이 처리하고, 비밀번호가 메모리에 닿기 전에 스스로를
하드닝한다:

- 크래시가 나도 **코어덤프가 안 남는다** — 비밀번호가 디스크에 떨어지지 않는다
- 버퍼를 **RAM 에 잠가서** 스왑·최대절전 이미지에 닿지 않는다
- 창이 **화면 공유와 녹화에서 빠진다**
- **로그·명령줄·환경변수 어디에도** 남지 않는다
- **폴킷이 부른 것만** 창을 띄운다 — 버스의 다른 프로세스가 부르면 창이 뜨기 전에 거절한다

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

`--init` 은 네 가지를 깐다. 전부 마커 안이라 정확히 되돌릴 수 있다: `sudo` alias,
Hyprland 창 규칙, `hyprland.lua` 의 require 한 줄, 에이전트용 systemd user 유닛.

### Omarchy 에게서 자리를 넘겨받기

한 세션에 폴킷 에이전트는 하나이고, Omarchy 셸이 기본으로 자리를 쥐고 있다. 그동안
`--init` 은 유닛을 깔되 **켜지 않고** 그 사실을 알려 준다. 넘겨받으려면:

```bash
omarchy plugin disable omarchy.polkit
sudo-pop --init
```

하드닝·화면 공유 제외·무엇이 묻는지 보여 주는 명령줄을 얻고, 그쪽 창의 테마색을 내준다
(지문 경로도 내주지만 Arch 에서는 어차피 안 된다). `--init` 은 지금 어느 에이전트가 자리를
쥐고 있는지 실행할 때마다 알려 준다.

## 제거

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
omarchy plugin enable omarchy.polkit
```

---

## 알아둘 것

askpass 가 아니라 polkit 에이전트라서 따라 나오는 것들이다:

- **옵션 없는 명령은 `run0` 으로 간다.** `sudo pacman -Syu` 는 폴킷 인증을 거쳐 systemd
  유닛으로 실행되는데, 그게 바로 에이전트가 창을 띄울 수 있게 하는 지점이다. 그 길에서는
  `sudoers` 규칙이 안 먹고 (`NOPASSWD` 도, `env_keep` 도 — 그래서 `SSH_AUTH_SOCK`·`DISPLAY`
  도 없다), 인증 캐시도 sudo 가 아니라 폴킷 것을 따른다. 옵션이나 `VAR=값` 이 붙으면 sudo 의
  의미를 그대로 지킨다. `SUDO_POP_RUN0=0` 으로 라우팅을 끌 수 있다.
- **run0 경로에서는 25초 안에 입력한다** — 우리 창이 아니라 **호출자**의 D-Bus 타임아웃이다.
  창이 그렇게 알려 준다. sudo 경로에는 이 제한이 없다.
- **폴킷과 sudo 가 faillock 카운터 하나를 공유**해서, 이 창에서 틀린 것이 양쪽에 쌓인다.
  잠기기까지 몇 번 남았는지는 창이 알려 준다.
- **`/usr/bin/sudo` 는 언제나 진짜 sudo 다.** `\sudo` 는 아니다 — alias 만 막고, 이 설정의
  다른 도구가 만드는 셸 함수는 못 막는다.

---

## 문서

| | |
|---|---|
| [docs/plan.md](docs/plan.md) | 무엇이며 구현이 무엇을 지켜야 하는가 |
| [docs/rationale.md](docs/rationale.md) | 왜 그렇게 했는지, 무엇을 재 봤는지, 무엇을 기각했는지 |
| [docs/audit.md](docs/audit.md) | 지금 코드 전수 점검과 무엇을 고쳤는지 |
| `old/` | 옛 구현(sudo askpass 래퍼)을 문서째 그대로 남겨 뒀다 |

---

## 개발

```bash
cargo test                            # 단위·프로토콜 시험. 환경이 필요 없다
./tests/scenarios.sh                  # polkitd·버스·컴포지터가 필요하다
./tests/scenarios.sh --with-password  # 입력이 필요한 한 케이스를 foot 창으로
```

시나리오는 세션을 원래대로 돌려놓고, 무엇을 되돌렸는지 찍고, 태운 faillock 도 치운다.

## 라이선스

MIT
