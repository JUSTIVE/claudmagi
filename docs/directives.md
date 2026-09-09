# Directives Log

지시는 구현 전에 여기에 먼저 기록한다. 흐름: 기록 → 스펙 반영 → 구현 → 상태 갱신.
10개마다(#10, #20, …) 완료된 행을 "현재 유효한 결정" 요약으로 통합하고 표에서 삭제한다.

## 현재 유효한 결정

(#1–#10 통합, 2026-09-10. 다음 통합: #20)

**제품**
- `claudmagi`: 현재 살아 있는 Claude Code 세션을 보여주는 macOS 앱. gpui(Rust)로 만든다. (#1, #2)
- 세션 하나 = PCB 트레이스 위의 칩 하나. 디자인은 `docs/reference/circuit-board-reference.mp4`를 따른다 — 주황 배경, 45° 계단형 검은 선, 녹색 라벨의 검은 칩. (#3, #4)
- 사용자 입력이 필요하거나 턴이 끝나면 그 칩의 선이 끊어진다(케이블 파형 → 홈 파인 소켓 → 분리된 칩, 앰버 색). (#5)
- 칩을 누르면 해당 세션의 Warp 탭으로 이동한다 (`WARP_FOCUS_URL`). (#6)
- 프레임리스 창: 타이틀바·신호등 없음, 보드 드래그로 이동, 가장자리로 크기 조절, `Esc`/`⌘Q` 종료. (#8)
- 큰 화면(4K 전체화면 등)에서도 비지 않게: 창 크기에 따라 최대 2.6배 확대, 레인은 높이를 채울 만큼, 대각선 스트라이프는 폭에 맞춰 반복, 상단 보조 레인 8개. (#9)

**구조**
- 데이터(`model.rs`, `sources.rs`)와 렌더링(`render/`)과 UI(`ui/`)를 분리한다. gpui는 `ui/`와 `render/paint.rs`만 안다. (#10)
- 세션 소스는 `SessionSource` 트레이트로 추상화: 실제(`ClaudeSource`) / 가상(`FakeSource`). (#10)
- UI 안에 플로팅 테스트 도구(`T` 키 / 상태바 `TEST`): 소스 전환, 가상 세션 생성·삭제·상태 변경·자동 변동. (#10)
- 시각 검증은 `--svg --demo --size WxH` + `tools/svg2png.swift`로 한다(화면 캡처 권한 불필요).

**작업 방식**
- 저장소는 `~/git/personal/claudmagi`, git으로 관리. (#7)
- 지시는 이 문서에 먼저 기록하고, 10개마다 통합한다.

## 미통합 지시

| # | 날짜 | 지시 | 상태 | 반영 위치 |
|---|------|------|------|-----------|
| 11 | 2026-09-10 | 마우스를 올렸을 때 하이라이트(칩 외곽선)를 흰색이 아니라 검은색으로 | ✅ | `src/theme.rs` (`outline`) |
| 12 | 2026-09-10 | 서브에이전트(세션 안에서 Agent 도구로 띄운 에이전트)도 보드에 보이게 | ✅ | `src/sources.rs` (subagents dir), `src/model.rs` (`SubState`), `src/render/scene.rs` |
| 13 | 2026-09-10 | 한 줄(레인)당 에이전트 하나. 서브에이전트는 부모 세션 칩의 오른쪽, 같은 선 위에 매달리게 | ✅ | `src/render/scene.rs` (`chip_draws` chain, left-horizontal anchor) |
| 14 | 2026-09-10 | 선이 모두 균일해서 밋밋함. 원본처럼 두 줄씩 짝지어 짝 사이 간격을 더 띄운다 | ✅ | `src/render/scene.rs` (`lane_offset`, GAP 13 + PAIR_GAP 9) |
| 15 | 2026-09-10 | 제목 배지에 "CLAUDMAGI" 대신 머신 사용자 이름을 표시 | ✅ | `src/sources.rs` (`machine_user`), `Frame.title` |
| 16 | 2026-09-10 | 배경을 주황 대신 흰색으로. 선 위를 흐르는 작은 요철(패킷)은 주황색으로 | ✅ | `src/theme.rs` (`bg` white, `packet` orange) |
| 17 | 2026-09-10 | "설치한 앱이 목록에 보이지 않아" → "이 앱을 빌드해서 설치해줘": claudmagi를 macOS 앱 번들로 빌드해 /Applications에 설치한다 (Launchpad·Dock에 보이도록) | ✅ | `tools/bundle.sh`, `tools/round_icon.swift`, `--icon` |

상태: ✅ 완료 / 🔨 진행 중 / 📋 대기
