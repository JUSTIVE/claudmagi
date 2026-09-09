# claudmagi

A circuit-board view of your live Claude Code sessions, built with [gpui](https://www.gpui.rs/).

Every running `claude` process becomes a chip riding on a trace. While Claude is
working the chip stays plugged in and packets flow along the line. The moment a
session needs you (permission prompt, question, dialog) or finishes its turn, the
trace goes slack and the chip pulls out of its socket. Click a chip to jump to the
Warp tab that hosts that session.

## Run

```sh
cargo run --release
```

- Frameless window: drag anywhere on the board to move it, drag edges to resize. Bigger windows zoom the board (up to 2.6×) and add more trace stripes so a fullscreen 4K display stays as dense as the reference clip.
- Scroll to pan when there are more lanes than fit.
- `Esc` or `⌘Q` quits.

### Headless helpers

```sh
cargo run -- --list                 # print discovered sessions
cargo run -- --svg out.svg          # render the current board as SVG
cargo run -- --svg out.svg --demo   # same, with fake sessions in every state
cargo run -- --focus PERSONAL-71    # jump to a session's Warp tab from the CLI
```

`swift tools/svg2png.swift out.svg out.png [scale]` rasterises the SVG at its exact size; add `--size 1920x1080` to the `--svg` run to preview other window sizes.

## Architecture

Data and rendering are separate layers; only `ui/` touches gpui.

| Layer | Module | Role |
|-------|--------|------|
| Data | `model.rs` | `SessionInfo` (facts), `Phase`, `BoardModel` (animated chip state: `apply` a snapshot, `tick` time). Pure Rust, unit-tested. |
| Data | `sources.rs` | `SessionSource` trait. `ClaudeSource` reads `~/.claude/sessions/<pid>.json` + the process env; `FakeSource` is the in-memory sandbox the test tools edit. |
| Render | `render/scene.rs` | `Layout` (zoom, lanes, stripes), `chip_draws` (model → lane positions), `build_shapes` (→ flat `Shape` list). |
| Render | `render/paint.rs`, `render/svg.rs` | The same shape list painted with gpui paths, or serialised to SVG for headless checks. |
| UI | `ui/board.rs` | The frameless window view: polls the active source, hit-tests chips, forwards clicks to `warp.rs`. |
| UI | `ui/devtools.rs` | Floating test tools (below). |
| Support | `font.rs`, `geom.rs`, `theme.rs`, `mac.rs`, `warp.rs` | Stroke font, polylines, palette, AppKit shims, Warp focus. |

Phase rules: `waitingFor` present → needs input · `status == busy` → working ·
otherwise idle (`tempo`/`state == blocked` also counts as needs input). Warp
jumps use `WARP_FOCUS_URL=warp://session/<uuid>` read from the Claude process
environment via `sysctl KERN_PROCARGS2`.

## Test tools

Press `T` (or click `TEST` in the status bar) to open a floating panel:

- **SOURCE** switches the board between live sessions and a sandbox.
- **SANDBOX SESSIONS** creates synthetic sessions in any phase, fills eight at
  once, clears them, or turns on *auto churn* (random phase changes, arrivals
  and departures every 1.4 s) to watch the plug/unplug and fade animations.
- **SESSIONS** lists what the board shows; sandbox rows have `W/N/I` phase
  buttons and `×`, live rows have `→ WARP`. Clicking a sandbox name cycles its
  phase. Drag the header to move the panel; `Esc` closes it.

## Layout

See [docs/design.md](docs/design.md) for the visual spec derived from the
reference clip in `docs/reference/`, and [docs/directives.md](docs/directives.md)
for the running log of product decisions.
