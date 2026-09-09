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

- Frameless window: drag anywhere on the board to move it, drag edges to resize.
- Scroll to pan when there are more lanes than fit.
- `Esc` or `⌘Q` quits.

### Headless helpers

```sh
cargo run -- --list                 # print discovered sessions
cargo run -- --svg out.svg          # render the current board as SVG
cargo run -- --svg out.svg --demo   # same, with fake sessions in every state
cargo run -- --focus PERSONAL-71    # jump to a session's Warp tab from the CLI
```

`qlmanage -t -s 1960 -o . out.svg` turns the SVG into a PNG for a quick look.

## How it works

| Piece | Source |
|-------|--------|
| Session list | `~/.claude/sessions/<pid>.json`, written by Claude Code itself (`status`, `waitingFor`, `name`, `cwd`, …). Dead pids are skipped. |
| Phase | `waitingFor` present → needs input · `status == busy` → working · otherwise idle (`tempo/state == blocked` also counts as needs input). |
| Warp tab | Warp exports `WARP_FOCUS_URL=warp://session/<uuid>` into each shell. claudmagi reads it from the Claude process environment (`sysctl KERN_PROCARGS2`) and `open`s it. Sessions not started in Warp fall back to activating Warp. |
| Rendering | `scene.rs` builds a renderer-independent shape list; `board.rs` paints it with gpui paths, `--svg` serialises the same list. Labels use a small built-in stroke font so they can rotate with the diagonal traces. |

## Layout

See [docs/design.md](docs/design.md) for the visual spec derived from the
reference clip in `docs/reference/`, and [docs/directives.md](docs/directives.md)
for the running log of product decisions.
