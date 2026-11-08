# valtrack

Overwolf 없는 VALORANT 라이브 매치 트래커. Rust, 단일 exe 3.6MB.

## 다운로드 / 설치

1. [Releases](../../releases)에서 `valtrack-v*-win64.zip`을 받는다.
2. 압축을 풀고 `valtrack.exe`를 더블클릭한다. 설치 프로그램·관리자 권한·런타임 설치가 없다
   (Rust 정적 링크, VC++ 재배포 불필요).
3. 더블클릭(=인자 없음)은 **오버레이**를 띄운다. 콘솔 출력은 `valtrack.exe live`로 실행한다.
4. 해시 확인: `Get-FileHash .\valtrack-v*-win64.zip -Algorithm SHA256` 값이 릴리스의
   `SHA256SUMS.txt`와 같아야 한다.

서명되지 않은 바이너리라 첫 실행 시 **SmartScreen**(“Windows에서 PC를 보호했습니다”)이 뜰 수
있다. `추가 정보` → `실행`을 누르면 되고, Defender가 격리했다면 복원 후 제외 목록에 폴더를
추가한다(아래 [실행이 막힐 때](#실행이-막힐-때-smartscreen--defender)).

## 왜 만들었나

Tracker.gg(Valorant Tracker)는 Overwolf 확장으로 설치되는데, 상시 실행되는
CEF 프로세스 여러 개 + 광고 라이브러리를 함께 끌고 온다. 이 도구는 게임 중에만
필요한 정보(현재 로비 10명의 Riot ID / 레벨 / 티어/RR / 요원)를 콘솔에 바로 출력한다.

## 역공학 결과: API 키 없이 왜 동작하는가

설치된 확장(`%LOCALAPPDATA%\Overwolf\Extensions\ipmlnnogholfmdmenfijjifldcpjoecappfccceh\1.32.14`)을 뜯은 결과:

| 계층 | 정체 | 근거 |
|---|---|---|
| 집계 전적 | `api.tracker.network` REST + **하드코딩된 키** | `js/Cu_khBDY.js`에 `KEY:'<REDACTED>'`, 헤더 `TRN-API-Key` |
| 라이브 로비 | Riot 로컬 API | `js/windows/BSj0SAG_.js`의 `Riot Client Service` |
| 토큰 | `plugins/Overwolf.Plugins.Valorant.dll` (.NET) | 문자열 `https://127.0.0.1:{0}/entitlements/v1/token`, `riot:`, `Basic` |
| 게임 이벤트 | Overwolf GEP, game id `21640` | `manifest.json`의 `game_events` |

즉 "API 키가 없다"가 아니라 **키를 클라이언트에 박아 넣었다**. 사용자 키가 필요 없을 뿐이다.

토큰/로비 파이프라인(앱 로그 `D:\GameData\Overwolf\Log\Apps\Valorant Tracker`에서 확인):

```
lockfile  %LOCALAPPDATA%\Riot Games\Riot Client\Config\lockfile
          name:pid:port:password:protocol
   ↓  GET https://127.0.0.1:{port}/entitlements/v1/token
      Authorization: Basic base64("riot:{password}")
   ↓  { accessToken, token(entitlement), subject(PUUID), issuer }
core-game  GET https://glz-{shard}-1.{region}.a.pvp.net/core-game/v1/players/{puuid}
           GET https://glz-{shard}-1.{region}.a.pvp.net/core-game/v1/matches/{matchId}
pregame    GET .../pregame/v1/players/{puuid}  ,  .../pregame/v1/matches/{matchId}
rank       GET https://pd.{region}.a.pvp.net/mmr/v1/players/{puuid}
names      PUT https://pd.{region}.a.pvp.net/name-service/v2/players   [puuid,...]  (v1은 404)
content    GET https://shared.{region}.a.pvp.net/content-service/v3/content
```

헤더: `Authorization: Bearer {accessToken}`, `X-Riot-Entitlements-JWT: {entitlement}`,
`X-Riot-ClientPlatform`, `X-Riot-ClientVersion`.

`region`/`shard`/`clientVersion`는 게임 로그에서 뽑는다(앱과 동일한 정규식):

- `%LOCALAPPDATA%\VALORANT\Saved\Logs\ShooterGame.log`
- `CI server version: ([\w.-]+)`
- `glz-(\w+)-\d+\.(\w+)\.a\.pvp\.net` → shard, region

기타 관찰:

- 2026년 클라이언트는 게임이 꺼져 있으면 로컬 REST가 404(`Invalid URI format`)이고
  `/`에 JSON-RPC 2.0(`error.code -32601`)만 응답한다. 게임 실행 중에만 REST가 열린다.
- `app.asar`(155MB) 안에 RCP 함수명 평문은 없었다. 함수명 규명은 미완.

### 파티(듀오/트리오/5인큐) 감지 — 앱이 실제로 하는 방법

먼저 확인한 사실: **Riot은 라이브 파티를 API로 주지 않는다.**

| 경로 | 라이브 매치 중 |
|---|---|
| `GET /parties/v1/players/{남의puuid}` | 403 `NONSELF_OPERATION` |
| `chat/v4/presences` | 친구만 (매치 중 19개 중 로비 인원 2개 = 나) |
| `pd.../match-details/v1/matches/{id}` | 404 `MATCH_NOT_FOUND` (경기 종료 후에만 200) |
| `core-game` / `pregame` 매치 JSON | `party` 필드 없음 |

그럼 앱은 어떻게 아나? 번들 `BSj0SAG_.js`의 `#x()` 함수가 전부다:

```js
// 1) 로비 전원의 match-history를 받아
// 2) 2명 이상이 공유하는 matchId만 추린 뒤
// 3) 그 매치의 match-details에서 partyId로 묶고
// 4) 1명짜리 그룹은 버리고, 겹치는 그룹은 병합
n[this.#s.uuid] = this.#s.puuids;              // 내 파티를 시드로
... t.players.filter(p => participantsIds.includes(p.subject))
for (...) { n[p.partyId] ||= []; n[p.partyId].push(p.subject) }
for (let k in n) if (n[k].length <= 1) delete n[k];
```

즉 **"최근에 같이 뛴 매치" + 그 매치의 `partyId`** 로 추론한다. 라이브 파티를 직접
조회하는 게 아니다. 그래서 한 번도 같이 플레이한 적 없는 파티는 앱에서도 안 잡힌다.

이 도구도 같은 방식을 쓴다(`Riot::shared_match_parties`):

1. 로비 10명 `match-history` (각 20경기, 10스레드 병렬)
2. 공유 matchId 추출 → 최대 20개
3. 해당 매치 `match-details`의 `partyId`로 그룹핑 (순차)
4. 1명 그룹 제거 + 겹침 병합 + 내 파티 시드
5. 2명 이상이면 `duo` / `trio` / `4-stack` / `5-stack` 뱃지

비용: 히스토리 10(병렬) + 매치 상세 최대 20(순차). 로스터가 바뀔 때만 다시 계산하고
그 사이에는 캐시를 쓴다. 랭크 조회(`mmr_all`)는 순차 + 60ms 간격 + 429 1회 재시도.
히스토리를 병렬로 쏘는 부분은 429가 나면 해당 인원 히스토리가 비어 뱃지가 빠질 수 있고,
이때는 순차 + 백오프로 바꾸는 게 다음 개선 대상이다.

### 티어 / 승률 — `LatestCompetitiveUpdate`를 쓰면 안 되는 이유

`mmr/v1/players/{puuid}`에는 두 종류의 랭크 정보가 있다.

| 필드 | 의미 | 문제 |
|---|---|---|
| `LatestCompetitiveUpdate` | 마지막 경쟁전 **이후 갱신이 멈춘 스냅샷** | 이번 act를 안 뛴 사람은 지난 시즌 값이 그대로 남음 |
| `QueueSkills.competitive.SeasonalInfoBySeasonID[actId]` | act별 집계 | 클라이언트가 실제로 렌더링하는 값 |

앱(`BSj0SAG_.js`)도 후자를 쓴다:

```js
let s = mmr[player.Subject]?.QueueSkills?.competitive
          ?.SeasonalInfoBySeasonID?.[this.#a.actId] ?? CompetitiveTier
```

`actId`는 `content-service`에서 `Type=="act" && IsActive`인 항목. 이 엔트리에는
`NumberOfWins` / `NumberOfGames`도 있어서 **승률은 Riot API만으로 계산된다**
(tracker.gg도, UA 위장도 필요 없음).

## 아이콘 — 파일로 임베드

요원 30종(64×64) + 티어 26종(32×32) PNG, 총 350KB를 `assets/`에 받아 두고
`include_bytes!`로 바이너리에 넣는다(`src/assets.rs`는 생성 파일).
렌더링 시에는 PNG → RGBA → 32bpp DIB → `AlphaBlend`만 하고 **네트워크를 타지 않는다**.
요원 이름표도 같은 스냅샷(`assets/agents.json`)에서 뽑아 임베드하므로 첫 프레임 지연이 없다.

```powershell
# 최초 1회 (아이콘/이름 갱신 시)
powershell -File scripts\fetch_icons.ps1
node scripts/gen_assets.mjs
powershell -File scripts\make_icon.ps1   # res/app.ico 재생성 (선택)
```

## 빌드 (WSL Ubuntu → Windows exe)

소스는 Windows에 두고, **cargo는 ext4에서** 돌린 뒤 exe만 복사한다
(`/mnt/d`는 9p 마운트라 빌드가 3~5배 느리다).

```bash
# WSL 안에서, 저장소 루트 기준
wsl -d Ubuntu -e bash -lc 'cd /mnt/d/projects/trackergg && bash scripts/build.sh'
# ~/valtrack-build 로 rsync → cargo build → dist/valtrack.exe
```

산출물: `D:\projects\trackergg\dist\valtrack.exe` (3.6MB)

배포 패키지(체크섬 포함)는 Windows에서 만든다:

```powershell
powershell -File scripts\package.ps1
# dist\release\valtrack-v*-win64.zip + SHA256SUMS.txt
```

## 사용

```powershell
valtrack.exe            # 현재 로비 출력
valtrack.exe whoami     # puuid / region / 클라이언트 버전 / 내 티어
valtrack.exe watch      # 로비가 바뀔 때만 다시 출력
valtrack.exe overlay    # 게임 위에 항상-위 오버레이 (클릭 통과, Alt+T 토글)
valtrack.exe overlay --demo   # 게임 없이 오버레이 렌더 확인
valtrack.exe agent      # VALORANT 실행 중에만 오버레이를 띄우는 감시자
valtrack.exe agent --stop
valtrack.exe autostart  # 로그인 시 agent 자동 시작 (HKCU Run)
valtrack.exe autostart --remove
valtrack.exe --json live
```

### 게임 실행/종료에 맞추기

`agent`는 창·트레이·HTTP가 없는 감시자다. 2초마다 프로세스 목록만 확인해서
`VALORANT.exe`가 뜨면 `valtrack.exe overlay`를 자식으로 띄우고, 게임이 끝나면 그 자식을
종료시킨다. Riot API를 직접 호출하지 않으므로 유휴 비용은 프로세스 스냅샷 1회/2초뿐이다.
`agent --stop`으로 끄고, `autostart`로 로그인 시 자동 시작을 등록한다(레지스트리
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`).

### 업데이트 확인

하루 1회 `api.github.com`에서 최신 릴리스 태그를 확인하고, 새 버전이 있으면 트레이
알림과 메뉴 항목(`새 버전 vX.Y.Z 열기`)으로 알린다. 자동 교체는 하지 않는다(실행 중인 exe를
덮어쓰려면 별도 헬퍼가 필요하고, 검증되지 않은 바이너리를 받아 실행하는 편이 더 위험하다).
끄려면 `VALTRACK_NO_UPDATE=1`. 이 확인은 로컬 Riot API 외에 유일한 외부 요청이다.

오버레이는 매치가 잡히면 자동으로 뜨고, **Alt+T** 로 숨김/복원한다.
매치가 끝나면 자동으로 숨는다. 마우스 입력은 통과시킨다(`WS_EX_TRANSPARENT`).

상태 변화는 Riot 로컬 WebSocket(`wss://127.0.0.1:{port}`)의
`OnJsonApiEvent_chat_v4_presences` 이벤트로 즉시 감지한다(`src/ws.rs`). WS가 막히면
폴백 타이머가 대신 돌고, 그 주기는 매치 중 5초 / 대기 중 10초라 유휴 시 토큰 요청이
분당 12회에서 6회로 줄어든다.

출력 예:

```
YOUR TEAM
  #   Riot ID                        Lvl  Rank                   Agent   WR    Party
  1   Hide on bush#KR1               214  Diamond 2 (54 RR)      Jett    43%   trio
```

오버레이는 요원/티어 아이콘 + `Lv` + `WR` + 파티 뱃지를 한 줄에 보여준다.
스트리머 모드(인코그니토) 플레이어는 이름 옆에 `[S]`가 붙고, 계정 레벨을 숨긴
플레이어는 `레벨 -`로 표시한다(클라이언트가 실제로 제공하는 값만 그대로 반영).

## 한계 / 안전

- REST 토큰 경로는 **VALORANT 실행 중에만** 열린다(게임 중에 쓰는 도구라 실사용엔 문제 없음).
  게임이 꺼져 있으면 명확한 에러를 낸다.
- Tracker Network의 하드코딩 키는 **사용하지 않는다**. 남의 서비스 키를 재사용하는 것은
  약관 위반이고, 키가 로테이션되면 죽는다. 필요하면 본인 키를 직접 붙이는 방향으로 확장할 것.
- 파티 추론은 "최근 함께 뛴 매치" 기반이라 **한 번도 같이 플레이한 적 없는 파티는 안 잡힌다**
  (원본 앱도 동일). 내 파티만 라이브에서 정확하다.
- 승률은 이번 act 기준. 배치 중이거나 이번 act 경기가 없으면 `-`.
- 라이브 로비/티어 조회만 한다. 전적 집계·MMR 추정·광고는 범위 밖.
- Riot 로컬 API 사용은 Riot 약관의 회색지대다. 계정 제재 사례는 드물지만 보장은 없다.

## 다음 후보

1. 최근 N경기 폼(연속 승/패, K/D) 표시
2. 오버레이 위치/크기 설정 저장
3. 파티 후보를 presence 친구 목록으로 보강

## 실행이 막힐 때 (SmartScreen / Defender)

이 exe는 **코드 서명이 없다**. 서명 인증서(연 $100~400)가 없으면 Windows는 낯선 바이너리를
휴리스틱으로 막는다. 바이너리 자체에 악성 동작은 없지만, 오탐을 0으로 만드는 방법은 서명뿐이다.

| 증상 | 조치 |
|---|---|
| “Windows에서 PC를 보호했습니다” | `추가 정보` → `실행` |
| Defender가 파일을 격리 | `Windows 보안` → `바이러스 및 위협 방지` → `보호 기록` → `복원`, 필요하면 폴더 제외 추가 |
| 회사/학교 PC에서 차단 | 정책으로 막힌 경우 우회 불가. 관리자에게 예외 요청 |

격리 전에 체크섬을 먼저 확인한다:

```powershell
Get-FileHash .\valtrack-v*-win64.zip -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

서명을 붙이려면 인증서를 산 뒤 빌드 후 한 줄을 추가한다:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a dist\valtrack.exe
```

## 라이선스

GPL-3.0-only. Copyright (C) 2026 CrashedSystem. 전문은 `LICENSE` 참고.
Riot Games와 무관한 비공식 도구다.

소스를 수정해 배포하려면 같은 라이선스로 소스를 공개해야 한다. 소스는
[이 저장소](.)에 있다.
