# sudo-pop

[English](README.md) · **한국어**

Omarchy 에서 권한이 필요한 순간의 비밀번호 창. 비밀번호가 코어덤프·스왑·화면 공유로
새지 않게 만들어져 있다.

**polkit 인증 에이전트**와, 그 앞에 선 **sudo 라우터**다.

```
sudo pacman -Syu   →  run0 pacman -Syu   ─┐
sudo -E make       →  sudo -A make       ─┤→  같은 창
디스크 마운트 · NetworkManager · systemctl ─┘
```

비밀번호를 묻는 모든 길이 창 하나로 모이고, 그 창은 **무엇이 묻고 있는지**를 보여 준다 —
polkit 자신의 문구에는 그게 없다.

---

## 무엇을 보장하는가

**보안 경계가 아니라 편의 도구다.** 내 권한으로 도는 악성코드는 alias 도 바이너리도
유닛도 바꿀 수 있다.

막아 주는 것은 **부주의로 인한 유출**이다.

- 크래시가 나도 코어덤프에 비밀번호가 남지 않는다
- 버퍼를 메모리에 잠가서 스왑·최대절전 이미지에 닿지 않는다
- 화면 공유와 녹화에서 창이 빠진다
- 로그·명령줄·환경변수 어디에도 남지 않는다
- **폴킷이 부른 것만** 창을 띄운다. 버스의 다른 프로세스가 부르면 창이 뜨기 전에 거절한다

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

### Omarchy 에는 이미 에이전트가 있다

한 세션에 폴킷 에이전트는 하나이고, Omarchy 셸이 자기 것을 갖고 있다. 그게 자리를 잡고
있는 동안 `--init` 은 유닛을 깔되 **켜지 않고** 그 사실을 알려 준다. 바꾸려면:

```bash
omarchy plugin disable omarchy.polkit
sudo-pop --init
```

그쪽 창의 지문 경로와 테마 연동을 내주고, 하드닝과 "무엇이 묻는지" 를 얻는 거래다.

## 제거

```bash
curl -fsSL https://raw.githubusercontent.com/minsoft1115/sudo-pop/main/install.sh | bash -s -- --uninstall
omarchy plugin enable omarchy.polkit
```

---

## 알아둘 것

**`/usr/bin/sudo` 는 언제나 진짜 sudo 다.** 백슬래시(`\sudo`)는 아니다 — alias 만 막고
셸 함수는 못 막는데, 이 설정의 다른 도구가 `sudo` 를 함수로 만든다.

**옵션 없는 명령은 run0 으로 가고, run0 은 sudo 가 아니다.** `sudoers` 규칙이 안 먹는다 —
`NOPASSWD` 도, `env_keep` 도(그래서 `SSH_AUTH_SOCK`·`DISPLAY` 도) 없다. 명령이 셸의 자식이
아니라 systemd 유닛으로 뜨고, 인증 캐시도 sudo 가 아니라 폴킷 것을 따른다. 옵션이나 환경
할당이 붙으면 sudo 의 의미를 그대로 지키는 쪽으로 간다. `SUDO_POP_RUN0=0` 으로 라우팅을
끌 수 있다.

**25초 안에 입력해야 한다.** 우리 창이 아니라 **호출자**의 D-Bus 타임아웃이다. 넘기면 무엇을
쳐도 "Connection timed out" 으로 끝난다. sudo 쪽에는 이 제한이 없다.

**틀리면 어디서 틀리든 같은 예산을 쓴다.** 폴킷과 sudo 가 faillock 카운터 하나를 공유해서,
이 창에서 틀린 것이 계정을 잠글 수 있다. 잠기기까지 몇 번 남았는지는 창이 알려 준다.

---

## 문서

| | |
|---|---|
| [docs/plan.md](docs/plan.md) | 무엇이며 구현이 무엇을 지켜야 하는가 |
| [docs/rationale.md](docs/rationale.md) | 왜 그렇게 했는지, 무엇을 재 봤는지, 무엇을 기각했는지 |
| `old/` | 옛 구현(sudo askpass 래퍼)을 문서째 그대로 남겨 뒀다 |

## 개발

```bash
cargo test                            # 단위·프로토콜 시험. 환경이 필요 없다
./tests/scenarios.sh                  # polkitd·버스·컴포지터가 필요하다
./tests/scenarios.sh --with-password  # 입력이 필요한 한 케이스를 foot 창으로
```

시나리오는 세션을 원래대로 돌려놓고, 무엇을 되돌렸는지 찍고, 태운 faillock 도 치운다.

## 라이선스

MIT
