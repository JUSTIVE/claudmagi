# claudmagi

A circuit-board view of your live Claude Code sessions, built with [gpui](https://www.gpui.rs/).

Every running `claude` process becomes a chip riding on a trace, and the subagents it spawns hang off it on the same trace, alternating right and left so they stay on screen. While Claude is
working the chip stays plugged in and packets flow along the line. The moment a
session needs you (permission prompt, question, dialog) or finishes its turn, the
trace goes slack and the chip pulls out of its socket. Click a chip to jump to the
Warp tab that hosts that session.

## Install

```sh
tools/bundle.sh          # release build → claudmagi.app → /Applications
open -a claudmagi
```

The script renders the icon from the board itself (`--icon`), ad-hoc signs the
bundle, and falls back to `~/Applications` if `/Applications` is not writable.

## Run from source

```sh
cargo run --release
```

- Frameless window: drag anywhere on the board to move it, drag edges to resize. `CLAUDMAGI_WINDOW=WxH` opens at a given size. The board draws sessions + 6 lanes (plus a few lead lanes at the top); bigger windows get more trace stripes, not more lanes, and the chips never scale with the window.
- `⌘+` / `⌘-` / `⌘0` (or ⌘ + wheel) zoom the whole board between 50% and 300%.
- `⌘,` (or the `SETTINGS` button in the status bar) opens the settings panel: background (white / orange / dark) and UI size. Settings persist in `~/Library/Application Support/claudmagi/settings.json`.
- Sessions are grouped by where they run with a wide gap between groups: panes of the same Warp tab stay together, otherwise each Warp pane, other terminals, and the Claude desktop app form their own groups. The pane → tab map is read from Warp's own state database (`warp.sqlite` under `~/Library/Group Containers/2BBY89MBSN.dev.warp`, via the system `sqlite3`, read-only, every 5s), so it needs no Warp setting; Warp Control (`warpctrl pane list`) is the fallback when that database is missing. Groups are separated by three empty rows with no traces in them. Inside a group a session keeps its lane for life: when one ends, the others stay put and the next new session takes the freed lane. When a session joins a group in the middle of the board, nothing jumps: its new trace draws in from the left and the traces below slide down with their chips aboard. Chips never sit in the left 20% of the board; a spot that would is swapped for the next one to the right, and a chip whose spot changes (say, after a resize) fades out and back in.
- The status bar shows your Claude plan usage (the `/usage` numbers: 5-hour and 7-day utilization with the time to reset), read once a minute from Anthropic's usage endpoint with the login Claude Code keeps in the keychain. Nothing is shown when there is no Claude Code login. `claudmagi --usage` prints the same numbers.
- Scroll to pan when there are more lanes than fit.
- `Esc` or `⌘Q` quits.

### Headless helpers

```sh
cargo run -- --list                 # print discovered sessions
cargo run -- --svg out.svg          # render the current board as SVG
cargo run -- --svg out.svg --demo   # same, with fake sessions in every state (--count N, --theme dark|orange, --zoom 1.5)
cargo run -- --focus PERSONAL-71    # jump to a session's Warp tab from the CLI
```

`swift tools/svg2png.swift out.svg out.png [scale]` rasterises the SVG at its exact size; add `--size 1920x1080` to the `--svg` run to preview other window sizes.

## Architecture

Data and rendering are separate layers; only `ui/` touches gpui.

| Layer | Module | Role |
|-------|--------|------|
| Data | `model.rs` | `SessionInfo` (facts), `Phase`, `BoardModel` (animated chip state: `apply` a snapshot, `tick` time). Pure Rust, unit-tested. |
| Data | `sources.rs` | `SessionSource` trait. `ClaudeSource` reads `~/.claude/sessions/<pid>.json`, the process env, and each session's `projects/<slug>/<session>/subagents/` transcripts; `FakeSource` is the in-memory sandbox the test tools edit. |
| Render | `render/scene.rs` | `Layout` (zoom, lanes, stripes), `chip_draws` (model → lane positions), `build_shapes` (→ flat `Shape` list). |
| Render | `render/paint.rs`, `render/svg.rs` | The same shape list painted with gpui paths (tessellations cached by geometry), or serialised to SVG for headless checks. |
| UI | `ui/board.rs` | The frameless window view: polls the active source, hit-tests chips, forwards clicks to `warp.rs`. |
| UI | `ui/panel.rs`, `ui/devtools.rs`, `ui/settings.rs` | Shared floating-panel chrome, the test tools (below), and the settings panel. |
| Data | `settings.rs` | Theme + zoom, persisted as JSON. |
| Support | `font.rs`, `geom.rs`, `theme.rs`, `mac.rs`, `warp.rs` | Embedded D-DIN outlines, polylines, palette, AppKit shims, Warp focus. |

Phase rules: `waitingFor` present → needs input · `status == busy` → working ·
otherwise idle (`tempo`/`state == blocked` also counts as needs input). Warp
jumps use `WARP_FOCUS_URL=warp://session/<uuid>` read from the Claude process
environment via `sysctl KERN_PROCARGS2`.

## Test tools

Press `T` (or click `TEST` in the status bar) to open the test panel. Panels dock in the bottom-right corner above the status bar, side by side with the settings panel, so they never overlap:

- **SOURCE** switches the board between live sessions and a sandbox.
- **SANDBOX SESSIONS** creates synthetic sessions in any phase, fills eight at
  once, clears them, or turns on *auto churn*: every 1.4 s a subagent is
  spawned, finishes or disappears, a session arrives, leaves or changes phase.
- **SESSIONS** lists what the board shows; sandbox rows have `W/N/I` phase
  buttons, `+SUB` (spawn a synthetic subagent) and `×`; subagent rows have
  `RUN/DONE` and `×`; live rows have `→ WARP`. Clicking a sandbox name cycles
  its phase. `Esc` closes it.

## Fonts

The board and the gpui UI are both set in **D-DIN**, compiled into the binary
from `assets/fonts/D-DIN.ttf` so there is no system-font dependency.

> D-DIN is Copyright © 2017 Datto Inc. (<https://www.datto.com/fonts/d-din>),
> with Reserved Font Name "D-DIN", drawn by Charles Nix at Monotype after the
> DIN 1451 standard. It is licensed under the SIL Open Font License, Version
> 1.1, which permits bundling and embedding it in software. The full licence is
> in [`assets/fonts/OFL.txt`](assets/fonts/OFL.txt) and ships inside the app
> bundle at `Contents/Resources/OFL.txt`.

## Layout

See [docs/design.md](docs/design.md) for the visual spec derived from the
reference clip in `docs/reference/`, and [docs/directives.md](docs/directives.md)
for the running log of product decisions.
