# Directives Log

지시는 구현 전에 여기에 먼저 기록한다. 흐름: 기록 → 스펙 반영 → 구현 → 상태 갱신.
10개마다(#10, #20, …) 완료된 행을 "현재 유효한 결정" 요약으로 통합하고 표에서 삭제한다.

## 현재 유효한 결정

(#1–#50 통합, 2026-09-10. 다음 통합: #60)

**제품**
- `claudmagi`: 현재 살아 있는 Claude Code 세션을 보여주는 macOS 앱. gpui(Rust)로 만든다. (#1, #2)
- 세션 하나 = 트레이스 한 줄 위의 칩 하나. 디자인은 `docs/reference/circuit-board-reference.mp4`를 따른다 — 45° 계단형 검은 선, 녹색 라벨의 검은 칩. (#3, #4, #13)
- 배경은 흰색, 선 위를 흐르는 작은 패킷은 참고 영상의 주황. 설정에서 배경을 흰색/주황/다크로 바꿀 수 있다. (#16, #20)
- 흰색·다크 테마에서 입력 필요 칩은 주황 바탕 + 검은 라벨, idle 칩은 어두운 적갈 + 앰버 라벨 — 기다리는 쪽이 눈에 띄어야 한다. 오렌지 테마는 주황이 배경에 묻히므로 둘 다 적갈 계열. 다크 테마에서 동작 중 칩은 흰 바탕 + 짙은 녹색 라벨. (#24, #32, #50)
- 선은 두 줄씩 짝: 짝 안 22, 짝 사이 46. 세션은 모든 레인에 순서대로, 항상 대각선 위. 칩 크기(세션 22, 서브 16)는 간격 때문에 줄이지 않는다 — 짝 안 칩이 살짝 겹치는 것은 참고 영상처럼 의도된 것. 짝의 두 칩은 나란히가 아니라 대각선 방향으로 36 어긋나게(위 레인 앞, 아래 레인 뒤). (#14, #29, #31, #33–#35, #39, #44)
- 첫 번째 꺾임의 빗변은 아래 레인일수록 길다(레인당 +12, 최소 24, 다음 스트라이프에 닿지 않게 상한) → 선이 왼쪽 위에서 오른쪽 아래로 부채꼴로 퍼진다. 두 번째 스트라이프부터는 모두 같은 길이. 레인 수는 창을 채우지 않고 세션 수 + 6(위쪽 보조 레인 8개는 별도). (#45)
- 사용자 입력이 필요하거나 턴이 끝나면 그 칩의 선이 끊어진다(케이블 파형 → 홈 파인 소켓 → 분리된 칩, 앰버 색). (#5)
- 서브에이전트(Agent 도구)는 부모 세션 칩과 같은 선 위에 더 작은 칩으로 매달린다. 실행 중이면 연결, 끝나면 분리됐다가 45초 뒤 사라진다. 부모 대각선 양옆의 수평 구간에만, 커브에서 38px 이상 떨어져, 좌우 번갈아 놓는다. 왼쪽 20% 회피는 적용하지 않고 화면 가장자리만 피한다. (#12, #13, #22, #36, #37)
- 세션 칩은 화면 왼쪽 20% 안쪽에 놓이지 않는다. 그 자리가 되면 다음 스트라이프의 대각선(오른쪽)을 쓴다. (#27, #30)
- 세션의 레인(슬롯)은 고정: 앞 세션이 사라져도 위로 당기지 않고, 새 세션은 가장 낮은 빈 슬롯을 받는다. (#23)
- 창 크기 변경 등으로 칩 자리가 바뀌면 옛 자리에서 페이드아웃, 새 자리에서 페이드인. (#28)
- 칩을 누르면 해당 세션의 Warp 탭으로 이동한다 (`WARP_FOCUS_URL`). 서브 칩은 부모 세션으로. (#6)
- 호버 하이라이트는 검은 외곽선. (#11)
- 제목 배지는 머신 사용자 이름(`$USER`). (#15)
- 프레임리스 창: 타이틀바·신호등 없음, 보드 드래그로 이동, 가장자리로 크기 조절, `Esc`/`⌘Q` 종료. (#8)
- 큰 화면에서도 비지 않게: 레인은 높이를 채울 만큼, 대각선 스트라이프는 폭에 맞춰 반복, 상단 보조 레인 8개. 창 크기에 따른 자동 확대는 하지 않는다. (#9, #19)
- UI 크기는 사용자가 조절: `⌘+`/`⌘-`/`⌘0`, `⌘`+휠, 설정 패널의 −/+와 프리셋. 0.5~3배, 파일에 저장. (#18)
- 설정 패널: 상태바 `SETTINGS` 버튼(TEST 옆) 또는 `⌘,`. 우선 배경화면과 UI 크기. (#20, #21)
- 설정·테스트 패널은 오른쪽 아래에 한 레이아웃(가로 나열)으로 고정해 겹치지 않는다. (#25)
- 앱은 `tools/bundle.sh`로 빌드해 `/Applications/claudmagi.app`으로 설치한다(아이콘 포함). (#17)
- 세션은 같은 Warp 탭(pane)끼리 붙이고, 다른 탭·프로그램에서 띄운 세션 사이만 빈 3행으로 띄운다. 갭 구간에는 선을 아예 그리지 않는다. (#41–#43, #49)
- 세션이 중간에 새로 들어오면 칩을 페이드로 옮기지 않는다: 새 선이 왼쪽에서부터 그려져 들어오고, 아래 선들이 칩을 얹은 채 아래로 밀린다. (#49)
- 보드 위 라벨·제목 배지부터 상태바·패널까지 모든 글자는 D-DIN. 시스템 폰트를 읽지 않고 `assets/fonts/D-DIN.ttf`를 바이너리에 내장한다 — SIL OFL 1.1이라 번들·임베딩이 허용되고, 고지는 저장소와 앱 번들 `Contents/Resources/OFL.txt`에 함께 간다. (#46, #47, #55)
- 끝난 서브에이전트는 세션처럼 뽑히지 않고 제자리에서 가라앉는다: 선은 그대로 관통, 바탕만 idle 색, 높이 16 → 12. (#53)

**구조**
- 데이터(`model.rs`, `sources.rs`, `settings.rs`)와 렌더링(`render/`)과 UI(`ui/`)를 분리한다. gpui는 `ui/`와 `render/paint.rs`만 안다. (#10)
- 세션 소스는 `SessionSource` 트레이트로 추상화: 실제(`ClaudeSource`) / 가상(`FakeSource`). (#10)
- UI 안에 테스트 도구(`T` 키 / 상태바 `TEST`): 소스 전환, 가상 세션·서브에이전트 생성·삭제·상태 변경. auto churn은 서브에이전트 생성·완료·삭제도 섞는다. (#10, #12, #26)
- Warp의 탭/pane 구조는 상태 DB(`~/Library/Group Containers/2BBY89MBSN.dev.warp/Library/Application Support/dev.warp.Warp-Stable/warp.sqlite`)에 있다: `terminal_panes.uuid`(BLOB, hex = `WARP_TERMINAL_SESSION_UUID`) → `pane_nodes.tab_id` → `tabs.window_id`. `~/Library/Application Support/dev.warp.Warp-Stable/warp.sqlite`는 빈 껍데기. 숨은 `warpctrl`(Warp Control CLI, JSON 출력)도 있지만 Warp 로컬 제어 서버가 꺼져 있으면 `no_instance`로 실패하고 문서화된 켜는 방법이 없다 → 폴백으로만 쓴다. (#43)
- 상태바 오른쪽에 Claude 플랜 사용량(`/usage`와 같은 숫자): 창마다 이름 + 프로그레스 바 + 퍼센트 + 리셋 카운트다운(#52). 토큰은 로그인 키체인 `Claude Code-credentials`(없으면 `~/.claude/.credentials.json`)에서 `security`로, 요청은 `curl`로 `api.anthropic.com/api/oauth/usage`에 1분마다. 로그인이 없으면 표시하지 않고 폴링도 멈춘다; 실패하면 마지막 값을 유지하고 15분 지나면 흐리게. 토큰 갱신은 하지 않는다(Claude Code가 한다). `--usage`로 터미널에서 확인. (#48)
- 시각 검증은 `--svg --demo --size WxH [--theme dark] [--zoom z] [--count n]` + `tools/svg2png.swift`로 한다(화면 캡처 권한 불필요).
- 성능: 경로 테셀레이션을 기하 해시로 캐시(정점 예산 32만), 30fps 타이머 갱신, 세션 폴링 1초. CPU 66%→8%, 큰 창에서도 메모리 ≈ 95MB. (#38, #40)

**작업 방식**
- 저장소는 `~/git/personal/claudmagi`, git으로 관리. (#7)
- 지시는 이 문서에 먼저 기록하고, 10개마다 통합한다.

## 미통합 지시

| # | 날짜 | 지시 | 상태 | 반영 위치 |
|---|------|------|------|-----------|
| 51 | 2026-09-10 | 라이트 테마에서 녹색 계열 텍스트 색을 오렌지로 (#54로 되돌림) | ↩︎ | `src/theme.rs` (`PALETTE.text_on` → `#F04A0E`, 패널 전용 `UI_ACCENT` 신설, `ORANGE.text_on`은 녹색 유지), `src/ui/panel.rs`, `src/ui/devtools.rs` |
| 52 | 2026-09-10 | 상태바의 usage를 프로그레스로 보여주기 | ✅ | `src/ui/board.rs` (`usage_label` → `usage_now`, `usage_meter`: 44×4 트랙 + 패킷 색 채움, stale이면 바까지 45%) |
| 53 | 2026-09-10 | done 인 subagent는 unplugged 말고 다른 디자인으로 | ✅ | `src/render/scene.rs` (`ChipDraw.settled`, `SETTLE_SHRINK` 0.25 — 서브는 `p`를 0으로 두어 파형·소켓·이탈 없이 제자리에서 idle 색 + 높이 16→12) |
| 54 | 2026-09-10 | 진행 중인 노드 텍스트를 다시 녹색으로 (#51 되돌림) | ✅ | `src/theme.rs` (`PALETTE.text_on` → `#3BE38A`, `UI_ACCENT`·`ORANGE.text_on` 제거), `src/ui/panel.rs`, `src/ui/devtools.rs` |
| 55 | 2026-09-10 | DIN 폰트로 교체 (적용 전 라이선스 확인) | ✅ | 시스템 DIN은 Bold만 있고 `fsType 0x0004`(Preview & Print only)라 제외. D-DIN(© 2017 Datto Inc., SIL OFL 1.1)을 `assets/fonts/`에 두고 `src/font.rs` (`FONT_TTF` = `include_bytes!`), `src/theme.rs` (`UI_FONT`), `src/ui/mod.rs` (`register_fonts`), `src/main.rs`, `tools/bundle.sh`(OFL 고지 동봉), `README.md` |
| 56 | 2026-09-10 | 세션과 연관된 GitHub PR을 보드에 렌더 (선 끝 엣지 커넥터) + 테스트 패널에 깃헙 노드 추가 | ✅ | `src/pr.rs`, `src/model.rs`, `src/sources.rs`, `src/render/scene.rs`, `src/geom.rs`, `src/theme.rs`, `src/ui/devtools.rs`, `src/main.rs` (`--prs`). `cwd`→브랜치는 세션이 전부 같은 체크아웃에 있어 못 쓰고, 트랜스크립트의 `message.content` + repo 일치로 해석 |
| 57 | 2026-09-10 | 커넥터 클릭하면 PR 열기 · PR 앞의 핀을 지우고 노드 색상으로 구분 · PR 노드는 오른쪽 끝에 붙이기 · 왼쪽 끝에 연관된 Linear 노드 | ✅ | `src/ticket.rs`(신규), `src/render/scene.rs` (`TicketDraw`, `PR_EDGE`/`TICKET_EDGE` 도킹, `splice_lane`에 시작 지점), `src/ui/board.rs` (`hit_pr`/`hit_ticket`/`open_link`), `src/pr.rs` (`url`, `open`), `src/sources.rs` (`attach_links` 2패스), `src/ui/devtools.rs`, `src/model.rs`, `src/theme.rs` |
| 58 | 2026-09-10 | test 패널에서 PR 상태를 직접 고를 수 있게 + PR 색 재지정(fail 오렌지 / open 테두리 그린 / approved 채움 그린 / merged 채움 검정, 라이트 기준이고 나머지 테마는 알아서) + Linear 상태 가져오기 | ✅ | `src/pr.rs` (`Look::Approved`, `reviewDecision`), `src/ticket.rs` (`Status`, `fetch` via `orca linear issue`, TTL 60초·실패 5분), `src/theme.rs` (`alarm`/`on_alarm`), `src/render/scene.rs`, `src/ui/devtools.rs` (링크 노드 전용 두 줄), `src/main.rs`. Orca 앱이 실행 중일 때만 Linear 상태가 온다 |
| 59 | 2026-09-10 | 처음 로드 후 Warp pane 정보 오기 전의 잘못된 레이아웃 렌더 막기 | ✅ | `src/sources.rs`: 실패한 조회를 `unwrap_or_default()`로 '탭 없음'으로 5초간 캐시하던 것이 원인. 마지막으로 성공한 맵을 유지하고 실패는 400ms 뒤 재시도(`WARP_TABS_RETRY`), 첫 스냅샷은 답이 올 때까지 최대 1.5초 기다린다(`warp_tabs_settled`, 프로세스당 한 번) |
| 60 | 2026-09-10 | Linear 노드는 웹 말고 앱으로 열기 | ✅ | `src/ticket.rs` (`app_url`: `linear://<slug>/issue/<KEY>`, 제목 슬러그는 버림), `src/ui/board.rs` (`open_link`가 딥링크 먼저·웹 폴백), `src/render/scene.rs` (`TicketDraw.app_url`) |
| 61 | 2026-09-10 | Linear 완료 + PR 머지 + Claude idle 이면 선을 볼드로 | ✅ | `src/render/scene.rs` (`DONE_LINE_W` 2.9, `done_lanes`, `Frame.done_lanes`), `src/ui/board.rs`, `src/main.rs`(데모에 끝난 레인 하나) |
| 62 | 2026-09-10 | CI가 돌고 있는 PR은 노란색 보더 | ✅ | `src/pr.rs` (`pending` 집계, `running()`; CheckRun `status` vs StatusContext `state`), `src/theme.rs` (`busy`), `src/render/scene.rs` (보더 오버레이 1.7px), `src/sources.rs`·`src/ui/devtools.rs` (`CI` 토글), `src/main.rs` |
| 63 | 2026-09-10 | 하나의 티켓에 PR이 여러 개일 수 있음 | ✅ | 수집은 세션 단위, 배치는 가로 연결. `src/model.rs` (`pr` → `prs: Vec`), `src/pr.rs` (`scan_transcript` → 전부, `MAX_PER_SESSION` 4), `src/render/scene.rs` (`PR_CHAIN_GAP`/`PR_CHAIN_SPAN`, 넘치면 `+n` 배지; `done_lanes`는 '머지 하나 이상 + 진행 중 없음'), `src/ui/devtools.rs` (`+` 버튼), `src/sources.rs` (`add_pr`) |
| 64 | 2026-09-10 | PR이 로드가 안 됨 | ✅ | 렌더는 정상이었고 원인은 조회가 1초 세션 폴링 안에 있던 것. `gh` 한 건 ≈1.2초 × 스냅샷당 2건이라 다 뜨는 데 20초+. `src/pr.rs`·`src/ticket.rs`: 배급 제거하고 워커 스레드로 분리(`Shared{wanted,seen,inflight}`, `want()`가 보드가 보는 것만 선언, PR 3워커·Linear 1워커), `snapshot()`은 캐시만 읽는다. `--prs`/`--svg`는 `eager`로 인라인 조회 |
| 65 | 2026-09-10 | YOSHI-DARK는 PR이 있는데 없다고 나옴 | ✅ | 트랜스크립트 경로를 `subagents_dir`에서 유도하던 게 원인 — 그 디렉터리는 서브에이전트를 띄워야 생기므로 안 띄운 세션은 PR·티켓이 통째로 안 잡혔다. `src/sources.rs`: `transcript()`를 독립 조회 + 캐시. 겸사겸사 `src/ticket.rs`: 링크만 된 키를 단독 채택하던 폴백 제거(CLAUDMAGI-F6이 읽기만 한 PJM-1953을 물었음) |
| 66 | 2026-09-17 | claudmagi에서 GitHub PR이 안 보임 | ✅ | Finder/`open`으로 띄운 번들은 launchd 환경을 물려받아 `PATH`가 `/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`뿐이다. `git`·`ps`·`open`·`orca`는 그 안에 있어서 Linear 노드는 멀쩡했고, `/opt/homebrew/bin`에 있는 `gh`만 못 찾아 PR이 전부 '커넥터 없음'으로 떨어졌다(터미널의 `--prs`는 정상이라 더 헷갈렸다). `src/mac.rs`: `widen_path()`가 빠진 셸 접두사(Homebrew·`/usr/local`·`~/.local/bin`·`~/.cargo/bin`)를 뒤에 덧붙인다 — 기존 순서는 그대로 두므로 셸에서 띄운 경우는 영향 없음. `src/main.rs`: 스레드가 생기기 전 `main` 첫 줄에서 호출 |
| 67 | 2026-09-17 | Warp 탭별 그룹이 적용 안 됨 | ✅ | `~/Library/Group Containers`는 Full Disk Access 영역이고 Dock에서 띄운 번들엔 그 권한이 없다. `sqlite3`가 `authorization denied`로 죽어 탭 맵이 비고 pane별 그룹으로 떨어졌다. 터미널은 터미널 앱의 권한을 물려받아 `--list`가 멀쩡히 탭을 찍으니 #66과 똑같이 재현이 안 됐다. 권한 자체는 시스템 설정에서 1회 부여(ad-hoc 서명이라 재빌드하면 다시 토글해야 할 수 있음). 코드는 조용히 망가지지 않게: `src/sources.rs` `TabsMiss`(`File::open`의 `PermissionDenied`로 거부를 구분 — `sqlite3` 실패는 손상된 파일과 구분이 안 된다), `SessionSource::note()`, 거부일 땐 첫 스냅샷의 1.5초 재시도 생략. `src/ui/board.rs` 상태바에 `NO WARP TABS` 태그, `src/main.rs` `--list`가 stderr로 |
| 68 | 2026-09-18 | 두개의 칩이 하나의 warp 탭을 가리키고 있어 | ✅ | 파킹된 백그라운드 잡(`~/.claude/sessions/<pid>.json`의 `kind: "bg"`)이 원인. 부모 세션이 띄운 프로세스라 `WARP_TERMINAL_SESSION_UUID`·`WARP_FOCUS_URL`을 그대로 물려받아 같은 pane을 가리키는 동명의 칩이 둘 생겼다(`PJM-1980` ×2). `src/sources.rs`: `RawSession`에 `kind`/`jobId`/`parkedJobId`, `fold_parked_jobs`가 잡의 `jobId` ↔ 부모의 `parkedJobId`로 짝을 찾아 잡을 서브에이전트 칩(`job_chip`, 라벨은 `/jobs` 핸들)으로 부모 트레이스에 매달고 잡의 PR·티켓은 부모 레인에 합친다. 부모가 사라진 잡은 자기 칩을 유지한다. `src/model.rs`: `SessionInfo.job`/`parked_job` |
| 69 | 2026-09-18 | 권한을 줬는데도 FDA가 없다고 하네 | ✅ | TCC는 권한을 준 시점의 code requirement로 검사하는데 ad-hoc 서명은 그 요구조건이 바이너리 cdhash 그 자체다. `tools/bundle.sh`를 돌릴 때마다 cdhash가 바뀌어(`30dba20c…` → `096e9ab3…`) tccd가 `SecStaticCodeCheckValidity ... status: -67050` → `kTCCServiceSystemPolicyAllFiles: Denied (Service Policy)` → 커널 `System Policy: claudmagi deny(1) file-read-data .../warp.sqlite`로 떨어졌다. 스위치 토글은 `auth_value`만 바꾸고 저장된 요구조건은 그대로라 #67에서 안내한 방법이 듣지 않는다 — 목록에서 지우고(`−`) 다시 추가해야(`+`) 현재 요구조건이 기록된다. `tools/bundle.sh`: 키체인의 Developer ID → Apple Development 순으로 인증서를 찾아 서명하고(`CLAUDMAGI_SIGN_ID`로 지정, 없으면 ad-hoc + 경고), 인증서 서명의 designated requirement는 `identifier + anchor apple generic + leaf CN`이라 재빌드해도 그대로다. `README.md` |
| 70 | 2026-09-18 | grant fda 버튼 누르면 설정 열리게 해줘 | ✅ | `src/ui/board.rs`: 상태바의 `NO WARP TABS` 태그를 누를 수 있게(`id`·`cursor_pointer`·hover·`on_click`) 바꾸고 `open_full_disk_access`가 `PRIVACY_ALL_FILES`(`x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`)를 연다. 여는 경로와 상태바 알림은 PR·Linear 노드와 같은 `open_link`를 탄다. 이 스킴은 시스템 설정에서도 그대로 먹고, 창 제목이 '전체 디스크 접근 권한'인 것으로 확인했다 |
| 71 | 2026-09-21 | cmdk로 검색할 수 있도록 하는 메뉴 만들자 · 리니어 티켓·세션 이름·PR 번호 아무거나 가까운 것들, exact match가 연속으로 이어지는 것에 매우 큰 가중치 | ✅ | `src/ui/palette.rs`(신규): `score()`는 subsequence DP로 모든 정렬을 보고 최고점을 고른다 — 연속 한 덩어리는 글자마다 `RUN`(60)×런 깊이를 받아 런 길이 n이 `RUN·n(n-1)/2`, 경계(`-` `_` `/` `.` `#` `~` 뒤)는 +18, 건너뛴 글자는 -2(최대 10), 앞쪽 등장이 유리. 후보는 세션 칩·Linear·PR이고 라벨이 1순위 필드, cwd·제목·repo는 65%. PR은 `#8792`와 맨 번호 `8792` 둘 다 건다. 행은 쿼리 변경과 세션 폴링 때만 다시 계산한다(30fps 재계산 금지). `src/ui/board.rs`: `palette` 상태, ⌘K 토글, 열려 있는 동안 모든 키를 팔레트가 먹고(`t`가 TEST를 열지 않게), 보드 클릭은 닫기, 선택 행은 `activate`/`open_link`로 클릭과 같은 경로. `src/ui/mod.rs`, `README.md`, `docs/design.md` |
| 72 | 2026-09-21 | cmdk도 테마 따라가게 해줘 (칩처럼, #73으로 되돌림) | ↩︎ | `src/ui/palette.rs`: 고정 `PALETTE`/`panel::dim`을 버리고 `Board::palette()`의 활성 팔레트에서 카드 색을 만든다 — 배경 `chip@97%`, 강조·테두리 `text_on`, 잉크는 `card_ink()`가 `luma(chip) > 0.5`로 고른다(흰·주황 보드는 칩이 검어 흰 잉크, dark 보드는 칩이 흰색이라 보드의 검은색). 선택·hover·구분선도 같은 잉크의 알파. 렌더 안에서 `self.palette()`(테마)와 `self.palette`(검색 상태)가 겹치므로 `theme` 지역 변수로 받는다. SETTINGS·TEST 패널은 지시 범위 밖이라 고정 어두운 카드 그대로 |
| 73 | 2026-09-21 | cmdk가 테마색을 안따라가는데? | ✅ | #72의 칩 기준이 구조적으로 불가능했다: `ORANGE`가 `chip`·`text_on`을 재정의하지 않고 `..PALETTE`로 물려받아 흰 테마와 값이 같다 → white ↔ orange에서 카드가 한 픽셀도 안 바뀐다(당시 설정은 `"theme": "white"`). `src/ui/palette.rs`: 상태바와 같은 레시피로 교체 — 배경 `lerp(bg, ink, 7%)@97%`, 글자·선·선택·hover는 `ink`의 알파, 강조는 `packet`, 테두리 `ink@35%`. 세 보드가 서로 다른 카드를 받고 잉크 대비가 4.5:1을 넘는지 테스트로 고정한다(WCAG 비율로 재는데, 주황 카드는 중간 밝기라 단순 밝기 차로는 통과를 못 한다). 주황 보드는 `packet`이 곧 잉크라 강조와 본문이 같은 색이고, 위계는 알파로만 준다 — 보드 위 패킷도 그 색이라 의도대로다 |
| 74 | 2026-09-21 | 같은 탭이 완전히 새로운 맥락의 일을 하면 연결된 리니어와 PR 을 치워줘 | ✅ | 맥락 전환의 신호로 세션 rename 을 쓴다: `nameSince > startedAt` 이면 그 시각이 경계고, 그보다 오래된 트랜스크립트 줄은 PR·Linear 수집에서 뻐다. `src/model.rs` `SessionInfo.context_since`, `src/sources.rs` `RawSession.name_since`, `src/pr.rs` `line_millis`·`in_context` + `scan_transcript(.., since)` + `resolve` 캐시 키에 since, `src/ticket.rs` `scan_transcript` 을 줄 단위로 바꾸고 같은 게이트. 시각이 없는 줄은 남기고, 이름을 안 바꾸 세션은 경계가 0 이라 종전과 동일하다. 실측: `PJM-2041` → `duo` 로 이름이 바뀜 탭에서 PR `#8945`·`#7295` 와 `PJM-2041` 포함 13개 키가 전부 떨어져 `--prs` 가 `DUO - -` 로 나온다 |
| 75 | 2026-09-21 | cmdk 에서 상태도 보여주도록 해 (텍스트 칸은 #76 에서 닷으로 대체) | ✅ | `src/ui/palette.rs`: 의미 없던 종류 단어(session/linear/pr) 칸을 상태 칸으로 바꿨다(종류는 글리프가 이미 말한다). `Entry.state`·`Entry.tone`, 세션은 phase, Linear 는 `Status::short()`, PR 은 `Look::short()` + CI 중이면 `⟳`. 색은 `Tone` 이 보드 색으로 매핑하되 `readable()` 이 카드 대비 3:1 을 넘길 때까지 잉크를 섞는다 — 칩용 색이라 흰 카드 위 `#3BE38A` 는 1.3:1 이다. 섞는 양은 이분탐색으로 최소화하고(색상 보존), 경로 중간에서 대비가 1 로 꺼지는 구간이 있어 단조가 아니므로 `hi` 쪽을 항상 통과값으로 잡아 잉크 쪽 구간으로 수렴시킨다. 주황 보드는 여유가 없어 상태어가 전부 잉크 근처로 수렴한다(보드 자신도 거기서는 색 대신 칩 모양으로 말한다). 테마×톤 전수가 3:1 을 넘는지 테스트로 고정 |
| 76 | 2026-09-21 | cmdk 목록의 상태가 텍스트로 보이지 않고 왼쪽의 닷에 칩의 상태 스타일이 반영되었으면 해 | ✅ | `src/ui/palette.rs`: 상태 텍스트 칸과 `Tone`·`readable()` 을 지우고 `Style{Chip(Phase)|Pr(Look)|Ticket(Status)}` 의 `dot()` 이 `scene` 과 같은 표에서 `(채움, 색)` 을 준다. 글리프는 채움이면 ▪◆●, 아니면 ▫◇○ 로 보드의 채움/테두리를 그대로 따른다. #75 의 잉크 혼합은 필요 없어졌다 — 테마가 이미 자기 보드에서 읽히도록 풀어둔 값이고 카드는 그 보드를 7% 띄운 면이라 보드에서 읽히면 카드에서도 읽힌다. 대비 테스트는 두지 않는다(흰 보드의 needs 칩 2.1:1, open PR 초록 1.3:1 은 보드 자신의 선택이다). 대신 채움 = 더 진행됨(#58) 규칙과 phase 3개가 테마마다 서로 다른 색을 받는지를 테스트로 고정한다 |
| 77 | 2026-09-22 | PR 칩에 호버하면 PR 이름 툴팁으로 · 툴팁은 뉴포트를 벗어나지 않는 선에서 PR 칩 근처에 앵커링 | ✅ | `src/render/scene.rs` `PrDraw.title`(`+n` 배지는 빈 문자열), `src/ui/board.rs` `pr_tooltip()` + 순수 함수 `tip_box()`. 자리는 `(x, y - scroll) * zoom`, 기본은 위·위가 좁으면 아래로 뒤집고, 사방 여백 8px 와 상태바를 넘지 않는다. 박스 너비를 먼저 정해야 클램프가 정확해진다 — `font::measure(title, 1.32)` 추정이 틀려도 박스는 그 너비라 뷰포트 보장은 산술로 성립한다(오차는 안쪽 여백·클리핑으로만 나타난다). 카드 배경은 ⌘K 와 같은 `palette::card_bg`. 테스트는 커넥터를 뷰포트 밖까지 쒸어도 네 변이 모두 안에 있는지, 좁은 창에서 너비를 먼저 포기하는지 |

상태: ✅ 완료 / 🔨 진행 중 / 📋 대기 / ↩︎ 되돌림
