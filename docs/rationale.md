# sudo-pop 설계 근거 기록

> **이 문서의 역할**: `plan.md` 의 각 결정이 왜 그렇게 정해졌는지, 그리고
> 그 근거가 된 실측 데이터를 남긴다.
>
> **구현할 내용은 여기 없다.** 구현은 같은 디렉터리의 `plan.md` 만 보고 하면 된다.
> 이 문서는 "왜 그렇게 안 했는가"를 다루므로, 여기 나오는 코드·명령 중
> 상당수는 **채택되지 않은 것**이다. 그대로 옮겨 쓰지 말 것.

측정 환경: Hyprland 0.56.2 / sudo 1.9.17p2 / Arch Linux / Omarchy

---

## 1. 최초 요구사항 대비 변경 이력

최초 요구사항 문서(`todo.md`)는 삭제했다. 그 내용 중 유효한 것은 모두 `plan.md` 에
반영됐고, 폐기된 것은 아래 표와 이어지는 각 절에 근거와 함께 남긴다.

| # | 원안 | 최종 | 근거 |
|---|---|---|---|
| 1 | `/tmp/sudo-pop_askpass.sh` 임시 래퍼 | `$XDG_RUNTIME_DIR` 심볼릭 링크 | §3 |
| 2 | `sudo -n true` 프리체크 후 분기 | 삭제. 무조건 `sudo -A` | §4 |
| 3 | `Command::spawn()` + `wait()` | `CommandExt::exec()` | §5 |
| 4 | `panic = "abort"` (RE 방어 목적) | **유지**. 단 하드닝과 세트 | §6 |
| 5 | `opt-level = "z"` | `opt-level = 3` | §6 |
| 6 | `argv[1]` 프롬프트 무시 | 화면에 표시 | §2 |
| 7 | 타임아웃 없음 | 90초 타임아웃 | §2 |
| 8 | (없음) | faillock 대응 추가 | §7 |
| 9 | `hyprctl --batch "keyword windowrulev2 …"` 동적 주입 | Lua 설정에 정적 1회 등록 | §9 |
| 10 | `.bashrc` 직접 수정 | 기존 스니펫 디렉터리 규약 활용 | §9 |

---

## 2. `sudo -A` 호출 규약 실측

`SUDO_ASKPASS` 에 관측용 스크립트를 걸고 `sudo -k -A true` 를 실행한 결과:

```
argv[1] (프롬프트) : [sudo] password for <user>:
인자 개수         : 1
fd 0 (stdin)      : socket:[808143]
fd 1 (stdout)     : pipe:[809248]     터미널? no
fd 2 (stderr)     : 상속됨
```

동작 구조:

```
[ sudo -A pacman -Syu ]   stdin/stdout/stderr = 터미널 (그대로 유지)
      |
      +-- pipe(fds) ; fork()
      |     +-- [자식] dup2(fds[1], 1) ; exec($SUDO_ASKPASS, "프롬프트")
      |     |          write(1, "비밀번호\n") ; exit
      +-- [부모] read(fds[0]) 첫 줄 -> 개행 제거 -> PAM 인증
      v
[ pacman ]   stdin/stdout/stderr = 터미널  <- 아무도 건드리지 않음
```

여기서 도출된 사양 항목:

| 관측 | 사양 반영 |
|---|---|
| 인자 개수 1 | `SUDO_ASKPASS` 에 인자를 붙일 수 없음 → 심볼릭 링크 + argv[0] 판별 (§3) |
| `argv[1]` = 프롬프트 | GUI 에 표시 (아래) |
| `fd 1` = 익명 파이프 | 파일시스템에 이름이 없어 다른 프로세스가 열 수 없고 디스크에 닿지 않음 |
| `fd 2` 상속 | 로그·경고는 stderr 로 내보내면 안전. **stdout 만 오염되면 안 됨** |

**`argv[1]` 을 표시해야 하는 이유** — 원안은 이를 무시했다. 그 경우 다음 상황에서
사용자가 무엇을 입력할지 알 수 없다:

- PAM 다단계(`pam_u2f`, TOTP, 지문) — "Verification code:" 가 온다
- `Defaults targetpw` — "Password for root:" 인데 자기 비밀번호를 친다

**타임아웃이 필요한 이유** — 없으면 Wayland 연결 실패 등으로 창이 뜨지 않을 때
sudo 가 askpass 를 무한정 기다린다. `stay_focused` 까지 걸려 있으면 복구가 어렵다.

**`sudo -S` 를 쓰지 않는 이유** — `-S` 는 비밀번호를 stdin 에서 읽는다. 그러면
터미널의 stdin 이 파이프로 대체되고, 그 파이프가 그대로 명령에 전달된다.

```
$ echo "pw" | bash -c 'read -r p; [ -t 0 ] && echo 터미널 || echo 파이프'
파이프
```

→ `pacman` 의 `[Y/n]` 은 EOF 를 만나고, `vim` 은 "Input is not from a terminal" 로 실패한다.

---

## 3. askpass 경로를 어떻게 지정하는가

`SUDO_ASKPASS` 에 인자를 붙일 수 없는 것은 사실이다. sudo 는 그 값을
**경로 그대로 exec** 한다(§2 실측에서 인자 개수 1 확인). 따라서 원안이 임시 래퍼를
고려한 것 자체는 타당한 문제 인식이었다.

### 3-1. `/tmp` 스크립트를 버린 이유

`/tmp` 에 고정 파일명으로 실행 가능한 스크립트를 두는 것은
**비밀번호 탈취 경로를 직접 만드는 것**이다.

```
$ findmnt -no TARGET,FSTYPE,OPTIONS --target /tmp
/tmp tmpfs rw,nosuid,nodev,nr_inodes=1048576,inode64,huge=advise,usrquota
```

world-writable tmpfs 이므로 다른 uid 나 침해된 프로세스가 그 경로를 선점하면
sudo 가 공격자의 프로그램을 실행해 비밀번호를 넘겨준다.
sticky bit 와 `fs.protected_symlinks` 는 완화일 뿐 설계 결함을 덮지 못한다.

### 3-2. 채택안 — `$XDG_RUNTIME_DIR` 심볼릭 링크

```
$ findmnt -no TARGET,FSTYPE,OPTIONS --target /run/user/1000
/run/user/1000 tmpfs rw,nosuid,nodev,relatime,size=1603504k,mode=700,uid=1000,gid=1000
```

| 이점 | 설명 |
|---|---|
| 권한 격리 | `mode=700, uid=1000`. 소유자만 접근 가능 |
| `noexec` 무관 | 심볼릭 링크는 exec 권한을 **타깃**에서 평가한다 |
| 셸 없음 | 중간 인터프리터가 사라져 공격 표면 감소 |
| 정리 불필요 | tmpfs 라 재부팅 시 소멸. 덕분에 §5 의 `exec()` 전환이 가능해짐 |

주의: 모드 판별에 `env::current_exe()` 를 쓰면 안 된다. 심볼릭 링크를
해석해 실제 바이너리 경로를 돌려주므로 판별이 불가능하다.
`env::args().next()` 의 basename 을 써야 한다.

### 3-3. fd 로 넘기는 방법은 불가능하다 (검증 완료)

"파이프 fd 를 상속시켜 비밀번호를 미리 넣어둔다"는 방식은 개념적으로 가장
깔끔해 보인다 — 비밀번호가 커널 파이프 버퍼에만 있고 어느 프로세스 메모리에도
남지 않는다. **그러나 sudo 는 askpass 를 실행하기 전에 호출자의 fd 3 이상을 닫는다.**

```
부모(호출자)가 연 fd 3 : pipe:[844258]
askpass 의 fd 3        : pipe:[839585]   <- 완전히 다른 파이프

askpass 가 실제로 물려받는 fd:
  fd 0 -> socket   (sudo 가 지정)
  fd 1 -> pipe     (비밀번호 회수용. sudo 가 dup2)
  fd 2 -> 상속
```

부모의 파이프 inode 는 askpass 프로세스 어디에도 나타나지 않는다.
fd 3 에 보이는 것은 sudo 자신의 내부 파이프이며, 읽기를 시도하면 EBADF 가 난다.

이는 "sudo 와 우리 프로세스 사이에 별도 채널을 만든다"는 모든 시도에 적용되는
제약이다. §8 의 기각 근거이기도 하다.

---

## 4. 프리체크를 두지 않는 이유

### 4-1. 논리적으로 불필요 (결정적 근거)

`sudo -A` 는 **실제로 비밀번호가 필요할 때만** askpass 를 호출한다.
캐시가 유효하면 팝업은 애초에 뜨지 않는다. 프리체크 유무와 무관하게 동작이 동일하다.

### 4-2. 이식성 문제

sudoers 가 제한적이면(`user ALL=(ALL) /usr/bin/pacman` 등) `true` 는 허용
명령이 아니므로 캐시가 멀쩡해도 프리체크가 실패한다.
(테스트 환경에서는 해당 없었지만, 배포를 고려하면 결함이다.)

### 4-3. 저널 오염

캐시 만료 시 매 호출마다 한 줄씩 쌓인다:

```
sudo[673099]: <user> : a password is required ; PWD=… ; USER=root ; COMMAND=/usr/bin/true
```

### 4-4. faillock 을 막아주지도 않는다

askpass 가 무출력 종료하면 faillock 카운터가 오른다(§7). 이 때문에 프리체크로
사전 차단할 수 있는지 검토했으나 **한 건도 막지 못한다.**

| 상황 | 프리체크 있음 | 프리체크 없음 |
|---|---|---|
| 캐시 유효 | 통과 → `sudo` 실행, 팝업 없음 | `sudo -A` 가 askpass 를 호출하지 않음, 팝업 없음 |
| 캐시 만료 + 정상 입력 | 팝업 → 성공 | 팝업 → 성공 |
| 캐시 만료 + 취소 | 팝업 → **+1** | 팝업 → **+1** |
| 캐시 만료 + 오입력 | 팝업 → **+최대 10** | 팝업 → **+최대 10** |

faillock 소비는 전부 "팝업이 뜬 뒤"에 발생하며, 그 시점은 프리체크가 끝난 다음이다.
캐시가 유효할 때는 애초에 팝업이 없으므로 막을 대상이 존재하지 않는다.
실측상 `sudo -n true` 자체는 faillock 을 올리지 않지만(저널 라인만 1건),
**단 한 건도 줄이지 못하면서 저널만 더럽힌다.**

---

## 5. `spawn` + `wait` → `exec()`

프로세스를 통째로 대체하면 다음이 전부 자동으로 해결된다:

- 종료 코드 전파 (시그널 사망 시 128+N 포함)
- Ctrl-C / SIGINT 전달
- 프로세스 그룹, 잡 컨트롤 (`sudo pacman` 중 Ctrl-Z)
- TTY 소유권

spawn+wait 는 위를 전부 직접 처리해야 하며, 하나라도 빠지면 스크립트가 깨진다.
§3-2 의 심볼릭 링크 방식이 사후 정리를 없애준 덕분에 `exec()` 가 가능해졌다.

---

## 6. 하드닝과 `panic = "abort"`

원안은 "gdb/ptrace 등 메모리 디버깅 방어를 극대화"할 목적으로 릴리스 프로파일
하드닝을 요구했다. **의도는 반영하되 수단은 교체했다.**

### 6-1. 컴파일 플래그는 메모리 디버깅을 막지 못한다

- `strip = true` — 심볼만 제거한다. 디버거 attach 도 메모리 읽기도 막지 못한다.
- `lto = true` — 난독화가 아니라 최적화다. "코드 경계 병합"은 부수 효과일 뿐이다.

두 플래그는 크기·성능 이점 때문에 유지하되 보안 효과는 기대하지 않는다.
메모리 검사에 대한 실제 방어는 전부 런타임 조치에서 나온다.

### 6-2. 코어 덤프 실측

이 머신은 systemd-coredump 가 활성이고 `ulimit -c` 가 `unlimited` 이다.

```
$ cat /proc/sys/kernel/core_pattern
|/usr/lib/systemd/systemd-coredump %P %u %g %s %t %c %h %d %F %I
```

가짜 비밀번호 `CANARY_PASSWORD_9f3a2b` 를 힙에 올린 뒤 `abort()` 를 호출하는
C 프로그램으로 측정 (SIGABRT 는 `panic = "abort"` 가 내는 것과 동일한 시그널):

| 조건 | 코어 덤프 | 비밀번호 추출 |
|---|---|---|
| 하드닝 없음 | 21.8K 저장됨 | **성공** (`strings core \| grep CANARY`) |
| `RLIMIT_CORE = 0` | `Storage: none` | 실패 (메타데이터 항목만 남음) |
| `+ PR_SET_DUMPABLE = 0` | 항목 자체 없음 | 실패 (`No coredumps found`) |

### 6-3. 그래서 왜 `abort` 를 유지하는가

한때 `panic = "unwind"` + `catch_unwind` 로 바꾸는 안을 검토했으나 철회했다.

1. **`unwind` 는 SIGSEGV 를 전혀 막지 못한다.** eframe 아래 Mesa/GPU 드라이버가
   죽으면 그것은 Rust 패닉이 아니라 SIGSEGV 이고, `catch_unwind` 는 손도 못 댄다.
   즉 `PR_SET_DUMPABLE=0` + `RLIMIT_CORE=0` 은 panic 설정과 **무관하게 필수**이며,
   그것이 들어가는 순간 `abort` 의 유일한 단점이 사라진다.
2. 하드닝이 있으면 `abort` 가 오히려 유리하다. 문제 발생 후 실행되는 코드가
   최소화되는데, **이 설계는 stdout 이 곧 비밀 채널**이므로 망가진 상태에서
   eframe/egui 스택의 소멸자를 도는 것보다 즉시 죽는 편이 안전하다.
3. `ZeroizeOnDrop` 미실행 문제는 명시적 zeroize 로 대체한다 (§6-4).

**전제 조건**: 하드닝 없이 `abort` 만 켜면 위 표 1행 그대로 비밀번호가
디스크에 남는다. 둘은 반드시 세트로 취급한다.

**실제 바이너리로 재확인** — 하드닝된 릴리스 askpass 에 시그널을 보낸 결과:

| 시그널 | 프로세스 | 코어 덤프 |
|---|---|---|
| SIGABRT (`panic = "abort"` 가 내는 것) | 사망 (wait=134) | **생성 안 됨** |
| SIGILL | 사망 (wait=132) | **생성 안 됨** |
| 같은 시각 대조군(하드닝 없음) SIGABRT | 사망 | 21.8K 저장됨 |

**시험 방법 주의 — `kill -SEGV` 로는 Rust 프로그램의 크래시를 흉내낼 수 없다.**
Rust 런타임이 스택 오버플로 감지를 위해 SIGSEGV 핸들러를 설치하므로
(`/proc/<pid>/status` 의 `SigCgt` 에 비트 11 이 켜져 있다), `kill` 로 보낸 SEGV 는
가드 페이지와 무관해 핸들러가 삼켜버리고 프로세스가 죽지 않는다.
실제 폴트가 아닌 인위적 시그널에만 해당하는 현상이므로 §6-3 의 논거는 그대로다.
검증에는 SIGABRT 를 쓴다.

### 6-4. zeroize 의 실제 효력 범위

과신하지 않기 위해 한계를 명시해 둔다.

- **egui 가 내부 복사본을 만든다.** `TextEdit` 은 `&mut String` 을 받고 레이아웃 /
  galley 캐시 / IME 버퍼로 문자열이 복제된다. 이 복사본은 지울 수 없다.
  (`password(true)` 모드는 마스킹된 문자열로 galley 를 만들므로 평문이
  galley 에 남지는 않는다.)
- **`String` 재할당이 흔적을 남긴다.** capacity 를 넘기면 기존 버퍼가 zeroize 없이
  free 되고, 잠가 둔 페이지에서도 벗어난다 → `with_capacity(2048)` + 입력 256자
  제한으로 재할당 자체를 불가능하게 만들었다.
- **`abort` 에서는 `Drop` 이 호출되지 않는다.** 다만 손실은 크지 않다 —
  프로세스가 죽으면 커널이 페이지를 회수하고 다른 프로세스에 넘기기 전에
  0으로 채우므로 프로세스 간 노출은 없다.

→ zeroize 의 실질 가치는 **프로세스가 살아 있는 동안**이다. 그래서 `Drop` 이 아니라
  파이프에 쓴 직후 명시적으로 지우는 방식을 택했다.

### 6-5. `mlockall` 과 `opt-level`

```
$ swapon --show
NAME           TYPE       SIZE
/swap/swapfile file      15.3G
/dev/zram0     partition 15.3G
```

디스크 스왑파일이 활성이다. 하이버네이션 시에는 확정적으로 기록된다.

**다만 `mlockall` 은 이 머신에서 쓸 수 없다.** `RLIMIT_MEMLOCK` 이 8 MB 인데
eframe 이 매핑된 주소 공간이 그보다 훨씬 커서 `MCL_CURRENT | MCL_FUTURE` 가
ENOMEM 으로 실패한다:

```
sudo-pop: mlockall failed (Cannot allocate memory (os error 12))
sudo-pop: hardening — dumpable=0 core_limit=0 locked=0 kB
```

`locked=0 kB` — 경고만 남기고 넘어가면 **아무것도 보호되지 않는다**.
그래서 비밀번호 버퍼 하나만 `mlock` 하는 방식으로 바꿨다. 한 페이지면 되므로
한도 안에서 항상 성공하고, 창이 열린 동안 외부에서 확인된다:

```
$ grep VmLck /proc/<pid>/status
VmLck:	       4 kB
```

부작용으로 버퍼 재할당을 막아야 해서(재할당되면 잠긴 페이지를 벗어난다)
입력 글자 수를 256 자로 제한했다.

`opt-level` 은 `"z"` → `3` 으로 바꿨다. 이 도구의 가치는 팝업이 즉시 뜨는 것인데
eframe 에 `"z"` 는 그 가치를 정면으로 해친다. 바이너리는 어차피 10~20MB 대라
크기 이득은 비율상 미미하고 GPU 컨텍스트 생성 지연만 남는다. 보안 이점은 0이다.
크기를 우선한다면 `"s"` 가 타협점이다.

---

## 7. faillock — 실측 기록

원안에 없던 항목이며, 이 프로젝트에서 발견된 가장 위험한 실패 모드다.

### 7-1. 이 머신의 설정

```
$ sudo -n -l
    Defaults … passwd_tries=10

$ grep faillock /etc/pam.d/system-auth
auth  required       pam_faillock.so preauth silent deny=10 unlock_time=120
auth  [default=die]  pam_faillock.so authfail deny=10 unlock_time=120
```

`fail_interval` 은 어디에도 명시돼 있지 않으므로 기본값 **900초(15분)** 이 적용된다.

| 파라미터 | 값 | 의미 |
|---|---|---|
| `fail_interval` | 900초 | 이 창 안에 누적된 실패를 센다 |
| `deny` | 10 | 누적이 10이면 잠근다 |
| `unlock_time` | 120초 | 잠긴 뒤 해제까지 |
| `passwd_tries` | 10 | sudo 가 askpass 를 재호출하는 최대 횟수 |

`passwd_tries=10` 은 배포판 기본값으로 sudoers 에 설정돼 있다. **`deny=10` 과
정확히 같은 숫자**라는 점이 문제의 핵심이다. 그리고 판정 창이 15분으로 넓다.

### 7-2. 취소 비용 — 1건, 0 으로는 만들 수 없다

askpass 가 **아무것도 출력하지 않고** 종료하면 sudo 는 `no password was provided`
로 **즉시 포기**한다. 재시도하지 않는다.

```
sudo -k -A true  (askpass 무출력 exit 1)  x 2회
  -> faillock 2건 (실행당 정확히 +1)
```

저널에는 다음이 남는다:

```
sudo[667100]: pam_unix(sudo:auth): conversation failed
sudo[667100]: pam_unix(sudo:auth): auth could not identify password for [<user>]
```

즉 askpass 가 실행되는 시점에는 PAM 인증 대화가 이미 시작된 상태이므로,
그 안에서의 취소는 **정의상 실패 1건**이다.

부모 sudo 에게 시그널을 보내 PAM 기록 전에 종료시키는 방법도 검토했으나 기각했다 —
PPID 가 sudo 라는 보장이 없고, 기록 시점과의 경쟁이라 재현성이 없으며,
자기 부모를 죽이는 동작은 예측 가능성을 해친다.

### 7-3. 오입력 비용 — sudo 명령 1회가 한도 전체를 소진할 수 있다

틀린 비밀번호를 반환하는 askpass 로 `sudo -A true` 를 **단 1회** 실행한 결과:

```
--- askpass 호출 횟수 ---
09:29:50 / 09:29:52 / 09:29:54 / 09:29:56 / 09:29:58
09:30:00 / 09:30:03 / 09:30:05 / 09:30:07 / 09:30:09
총 10 회 (약 2초 간격)

sudo: 10 incorrect password attempts

--- faillock ---
2 -> 10   (한도 도달)
```

**이 측정은 "사람이 1번 틀린" 경우가 아니다.** 스크립트가 사람 개입 없이 틀린 값을
10회 반환한 결과다. 사람이 GUI 앞에 있으면 1회 오입력은 1건이고, 잠기려면 10회
연속 틀려야 한다(터미널 sudo 와 동일).

의미 있는 결론은 이것이다: **askpass 는 사람이 타이핑하지 않아도 sudo 가 자동으로
재호출하므로, 구현 실수 하나가 20초 만에 한도를 소진시킬 수 있다.**

### 7-4. 빈 줄과 무출력의 결정적 차이

| askpass 의 행동 | sudo 의 해석 | 소비 |
|---|---|---|
| 무출력 + exit | "no password was provided" → 즉시 포기 | **1회** |
| 빈 줄 `"\n"` 출력 | 빈 비밀번호 → 오답 → 재시도 | **최대 10회** |

실패 경로에서 실수로 개행 하나를 쓰면 **한 번의 취소가 계정을 잠근다.**
가장 현실적인 사고 경로이며, `plan.md` §1 금지 목록과 §4-4(a) 에 반영했다.

### 7-5. 채택한 대응

`plan.md` §4-4 참조. 요약:

| 조치 | 효과 |
|---|---|
| (a) 실패 경로 무출력 강제 | 취소 비용을 최대 10 → 1 로 고정 |
| (b) `attempts` 파일 기반 자체 재시도 제한(3회) | 오입력 비용을 최대 10 → 3 으로 축소 |
| (c) `faillock --user` 조회 후 경고·차단 | 잠긴 상태에서 헛시도를 쌓지 않음 |
| (d) 빈 비밀번호 제출 차단 | 실수로 인한 소비 방지 |

`faillock --user <id>` 는 root 없이 자기 계정 기록을 읽을 수 있음을 확인했다.
단 `fail_interval` 이 지난 기록은 목록에 남은 채 `Valid` 열이 `V`→`I` 로 바뀌므로
**`V` 인 행만 세야 한다.**

**해결하지 않는 것**: 사용자가 실제로 비밀번호를 틀리는 경우. faillock 이
정상 동작하는 것이므로 우회하려 하지 않는다. (b) 로 세션당 3건으로 제한될 뿐이다.

---

## 8. "팝업을 sudo 호출 이전에 띄우면 되지 않나?" — 기각

**아이디어**: `sudo` 를 부르기 전에 래퍼가 직접 팝업을 띄우고, 사용자가 취소하면
sudo 를 아예 실행하지 않는다. PAM 대화가 시작되기 전이므로 **취소 비용이 0** 이 된다.
이 부분은 맞다. 그러나 두 개의 벽에 막힌다.

### 8-1. 벽 1 (결정타) — 비밀번호가 필요한지 미리 알 수 없다

팝업을 선행하려면 "지금 비밀번호가 필요한가"를 먼저 판정해야 하는데,
그 판정이 불가능하다. 캐시를 비우고 측정한 결과:

```
$ sudo -k
$ sudo -n -v
sudo: a password is required          rc=1   -> "비밀번호 필요" 로 판정

$ sudo -n /usr/bin/<nopasswd-cmd> --help
<nopasswd-cmd> 0.6.0 …                rc=0   -> 비밀번호 없이 실행됨
```

프리체크는 **타임스탬프 캐시만** 본다. 명령별 NOPASSWD 규칙, `-u` 대상 사용자,
`targetpw`, PAM 모듈 조건은 반영하지 못한다. 테스트 머신 sudoers 에는 아래 형태의
NOPASSWD 항목이 있었다:

```
(ALL) NOPASSWD: /usr/bin/<nopasswd-cmd>
(root) NOPASSWD: /usr/bin/<another-cmd> <subcommand> *
```

따라서 팝업 선행 방식은 **NOPASSWD 로 설정해 둔 명령에 비밀번호를 요구하는
기능 회귀**를 일으킨다. `sudo -n -l <명령>` 으로 규칙을 조회하는 변형도 검토했으나
sudoers 출력 파싱에 의존하고 `targetpw`·PAM 조건을 잡지 못해 불완전하다.

`sudo -A` 는 이 판단을 sudo 자신이 하므로 오탐이 원천적으로 없다.

### 8-2. 벽 2 — 비밀번호를 넘길 방법이 마땅치 않다

`exec()` 는 프로세스 이미지를 교체하므로 래퍼의 메모리(=비밀번호)가 사라진다.
sudo 가 askpass 자식을 포크하는 것은 그 이후이므로 건네줄 주체가 남아 있어야 한다.

| 전달 방법 | `exec()` | 대가 |
|---|---|---|
| `spawn` + `wait` | 포기 | 종료 코드·시그널·잡 컨트롤 직접 처리 (§5) |
| 헬퍼 `fork` 후 부모 `exec` | 유지 가능 | 평문 보유 프로세스가 고아로 남을 위험 |
| 파이프 fd 상속 | — | **불가능. §3-3 에서 검증** |
| 환경변수 | 유지 가능 | `/proc/PID/environ` 노출. 채택 불가 |

헬퍼 방식은 sudo 가 죽거나 사용자가 Ctrl-C 를 치면 평문을 든 프로세스가 고아로
남는다. 자체 하드닝·자폭 타이머·소켓 생명주기 관리가 모두 필요하고, 현재 설계
(단명 GUI 프로세스 + 커널 파이프 버퍼)보다 명백히 나빠진다.
게다가 재시도 시 채널에 값이 없어 **오입력 케이스는 여전히 해결되지 않는다.**

### 8-3. 결론

| | 얻는 것 | 잃는 것 |
|---|---|---|
| 팝업 선행 | 취소 비용 1건 → 0건 | NOPASSWD 명령에 팝업 오출현, 평문 보유 프로세스, 오타 시 명령 재입력 |

기각. 결정타는 §8-1 이다. 전달 채널을 아무리 잘 설계해도 "비밀번호가 필요한지
미리 알 수 없다"는 문제는 해결되지 않는다. 취소 1건은 정상 인증 한 번으로 기록이
초기화되므로 실질 부담도 작고, 최대 소비는 §7-5(b) 로 10 → 3 으로 낮춘다.

---

## 9. Hyprland 룰과 설치 경로

### 9-1. `hyprctl keyword windowrulev2` 는 이 버전에서 작동하지 않는다

Hyprland 0.5x 가 Lua 설정 파서로 이행하면서 윈도우 룰이 legacy 파서 밖으로 나갔다.

```
$ hyprctl keyword windowrulev2 "float,class:^(x)$"
keyword can't work with non-legacy parsers. Use eval.

$ hyprctl keyword windowrule "float,class:^(x)$"      # 신문법도 동일하게 거부
keyword can't work with non-legacy parsers. Use eval.
```

동적 주입이 꼭 필요하면 `hyprctl eval` 로는 가능하다(동작 확인함).
**그럼에도 정적 등록을 택한 이유**: `hl.window_rule` 은 호출할 때마다 룰이
**누적**된다. sudo 를 하루 200회 쓰면 세션 동안 룰이 1200개 쌓여 윈도우 생성 시
매칭 비용이 계속 늘어난다. 나타나지 않는 클래스의 룰은 비용이 0이므로
`--init` 에서 한 번 등록하는 편이 모든 면에서 낫다.

**속성명 검증** — 잘못된 이름은 Hyprland 가 거부한다:

```
$ hyprctl eval 'hl.window_rule({ match={class="^(x)$"}, dimaround = true })'
error: hl.window_rule: unknown field 'dimaround'
```

| 원안 표기 | 실제 Lua 속성 |
|---|---|
| `dimaround` | `dim_around` |
| `stayfocused` | `stay_focused` |
| `size 400 200` | `size = { 400, 200 }` |

### 9-2. `.bashrc` 를 수정할 필요가 없다

기존 저장소가 이미 쓰는 구조를 그대로 따른다.

```
$ grep -n minsoft1115 ~/.bashrc
16:# minsoft1115-bash:begin
17:for __minsoft1115_rc in "$HOME/.config/minsoft1115/bash"/*.sh; do
18:  [ -r "$__minsoft1115_rc" ] && . "$__minsoft1115_rc"
21:# minsoft1115-bash:end
```

bash 스니펫은 `$HOME/.config/minsoft1115/bash/` 에 파일만 떨구면 자동 로드된다.
Lua 는 `$HOME/.config/minsoft1115/hypr/` 에 두고 `hyprland.lua` 에 마커 블록으로
`require` 를 추가한다(`korean-input.lua` 등과 동일한 방식).

윈도우 룰 헬퍼는 Omarchy 의 `o.window(match, rules)` 를 쓴다
(`/usr/share/omarchy/default/hypr/helpers.lua:131`). 내부적으로
`hl.window_rule(rules)` 를 호출한다.

---

## 10. 검증하다 두 번 속은 것

둘 다 코드 버그가 아니라 **검증 방법의 함정**이었다. 다시 겪지 않도록 남긴다.

### 10-1. `no_screen_share` 는 스크린샷에도 걸린다

`grim` 으로 창을 찍었더니 완전히 검게 나와 렌더링 버그로 판단했고, 원인을
`dim_around` 로 잘못 지목했다. 속성을 분리해 재측정한 결과:

| 설정 | grim 캡처 |
|---|---|
| `dim_around = true`, `no_screen_share = false` | **정상 표시** |
| `dim_around = false`, `no_screen_share = true` | **완전 검정** |

`grim` 은 화면 공유와 **같은 wlr-screencopy 프로토콜**을 쓴다. 화면 공유에 안
잡히는 창은 스크린샷에도 안 잡힌다. 즉 **검게 나온 것이 기능이 작동한다는 증거**였다.

외형을 확인하려면 `no_screen_share` 를 잠시 꺼야 한다. 그때는 중단되더라도
설정이 원복되도록 `trap` 을 걸 것.

### 10-2. `kill -SEGV` 로는 Rust 크래시를 흉내낼 수 없다

§6-3 참조. Rust 런타임이 스택 오버플로 감지용 SIGSEGV 핸들러를 설치하므로
`kill` 로 보낸 SEGV 는 핸들러가 삼키고 프로세스가 죽지 않는다. 하드닝 검증에는
`panic = "abort"` 가 실제로 내는 **SIGABRT** 를 쓴다.

---

## 11. 한계와 향후

### 11-1. 보안 경계가 아니다

사용자 권한으로 실행되는 악성코드는 alias 도, 바이너리도, `SUDO_ASKPASS` 도
전부 바꿀 수 있다. 방어선이 존재하지 않는다.

오히려 **피싱을 쉽게 만든다.** "비밀번호를 묻는 GUI 창"을 일상화시키면
똑같이 생긴 가짜 창을 아무 권한 없이 만들 수 있고, 사용자가 진짜와 구별할
방법이 없다. 터미널 sudo 는 최소한 "내가 방금 명령을 친 그 터미널 안에"
프롬프트가 뜬다는 약한 출처 보증이 있는데 그것을 버리는 셈이다.

따라서 이 도구가 실제로 지킬 수 있는 약속은 "보안 등급 상승"이 아니라
**"부주의로 인한 비밀번호 유출 방지"**(코어 덤프, 스왑, 화면 공유, 로그)다.

참고로 이 머신에는 polkit 은 설치돼 있으나 인증 에이전트가 없다
(`hyprpolkitagent` 미설치). GUI 앱 권한 상승의 정석은 polkit 이지만
polkit 은 터미널 `sudo` 를 커버하지 않는다. sudo-pop 의 니치는 유효하되,
그것은 UX 니치이지 보안 니치가 아니다.

alias 는 대화형 셸에서만 확장되므로 스크립트·`sh -c`·Makefile·systemd 유닛에서는
원래 sudo 가 실행된다. 동작이 반만 일관되지만, **PATH 에 `sudo` 를 심는 것은
채택하지 않았다** — 전형적 악성코드 패턴이고 `/usr/bin/sudo` 하드코딩은 어차피
우회되므로 위험만 늘고 이득이 없다.

### 11-2. layer-shell 오버레이 (향후 재검토)

Hyprland 윈도우 룰 대신 `wlr-layer-shell` 의 overlay 레이어 +
`keyboard_interactivity = exclusive` 로 표면을 직접 만드는 방법.
실제 잠금화면과 인증 프롬프트가 쓰는 방식이다.

| 장점 | 단점 |
|---|---|
| 윈도우 룰 주입 불필요 (컴포지터 설정 의존 0) | `smithay-client-toolkit` + 직접 렌더링, 난도 상승 |
| 키보드 그랩이 프로토콜 차원에서 보장 | egui 대비 UI 작성 비용 증가 |
| Hyprland 업그레이드에 안 깨짐 (§9-1 재발 방지) | |
| 콜드 스타트가 빠르고 공격 표면이 작음 | |

**판단**: 1단계는 eframe 으로 완성한다. 룰 의존이 실제 문제를 일으키면 그때 옮긴다.
그래서 `plan.md` §6 에서 `gui.rs` 를 독립 모듈로 격리했다.
