# claudmagi

A circuit-board view of your live Claude Code sessions, built with [gpui](https://www.gpui.rs/)
and [base-gpui](https://github.com/LukeTandjung/base-gpui) for the headless parts.

Every running `claude` process becomes a chip riding on a trace, with Claude's mark inside it ahead of the name, and the subagents it spawns hang off it on the same trace, alternating right and left so they stay on screen. A background job the session parked rides that trace too, labelled with its `/jobs` handle: it is a process the session spawned in the session's own Warp pane, so it is not a chip of its own (#68). While Claude is
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

Then grant the app **Full Disk Access** once, in System Settings → Privacy &
Security. Warp keeps the pane → tab map in its group container
(`~/Library/Group Containers/2BBY89MBSN.dev.warp/…/warp.sqlite`), and macOS
puts that whole directory behind that one permission. Without it the board
still runs, but every pane becomes its own group instead of the panes of a tab
sitting together, and the status bar says `NO WARP TABS`. Running from a
terminal never shows this: a CLI inherits the grant its terminal app already
holds, which is why `--list` can print tabs while the installed app cannot
(#67).

The grant is recorded against the app's code requirement, so **grant it to a
bundle signed with a certificate**: `tools/bundle.sh` picks the first Developer
ID, else the first Apple Development certificate in your keychain
(`CLAUDMAGI_SIGN_ID` overrides it), and that requirement survives every
rebuild. With no certificate the bundle falls back to ad-hoc, whose
requirement is the binary's own hash, and every rebuild voids the grant.
Switching identity changes the requirement once, so after the first
certificate build the old entry no longer matches: **remove claudmagi from the
Full Disk Access list with `−` and add it again with `+`** — toggling the
switch off and on keeps the stale requirement and stays denied (#69). The
`NO WARP TABS` tag in the status bar is the button that opens that pane
(#70).

## Run from source

```sh
cargo run --release
```

- Frameless window: drag anywhere on the board to move it, drag edges to resize. `CLAUDMAGI_WINDOW=WxH` opens at a given size. The board draws sessions + 6 lanes (plus a few lead lanes at the top); bigger windows get more trace stripes, not more lanes, and the chips never scale with the window.
- `⌘+` / `⌘-` / `⌘0` (or ⌘ + wheel) zoom the whole board between 50% and 300%.
- `⌘K` searches everything on the board at once: session names, Linear issue keys, pull request numbers, and the paths and titles behind them. Picking a row does what clicking that node does — jump to the Warp tab, open the PR, open the issue in Linear.app. Letters may be scattered across a candidate, but an unbroken run of them is worth far more than the same letters spread out, so `pjm-20` puts the tickets that carry it whole on top. A hit brings its whole lane with it — find a pull request by its number and its issue and its session come along under it, stepped in behind the hit — since the number is often all anyone remembers and the work around it is what was wanted. Each row is drawn as the node itself, in the body the board would give it and carrying the mark of whose thing it is — Claude, GitHub, Linear — at one chip size for every kind, so a merged pull request, a chip waiting on you and an issue nobody has started read the same in the list as they do on the board. `↑↓` moves, `⏎` opens, `esc` or a click on the board closes.
- **Two views**, switched in the settings panel. `CIRCUIT` is the board above. `COMB` lays the same sessions out as a vertically scrolling honeycomb: every cell is drawn, empty ones in grey, and the panes of one Warp tab cluster into a flower with a ring of empty cells around it. A cell carries its Linear issue, its session and its pull request, in that order top to bottom, with subagents as a sub-grid of small hexes along its floor. Cells are outlines only, and they stand apart with a gap between them. `claudmagi --svg out.svg --view comb` renders it without a window.
- `⌘L` opens a list down the right-hand side: every session the board is drawing, in board order, with its issue, its pull requests and its subagents underneath. Every line clicks through to the same place its node on the board does (#103).
- `⌘,` (or the `SETTINGS` button in the status bar) opens the settings panel: view, background (white / orange / dark) and UI size. Settings persist in `~/Library/Application Support/claudmagi/settings.json`.
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

The session registry at `~/.claude/sessions/<pid>.json` is Claude Code's own
and says nothing about terminals: a session writes one wherever it runs, the
desktop app included — that app is a reader of the same registry, as this
board is. What a terminal adds is a *seat*: which window of which app the
session sits in, so sessions sharing one can be drawn together, and a way back
to it when a chip is clicked. That is the `host/` layer, and the hosts work
nothing alike. Warp keeps its tabs in a database behind Full Disk Access;
Orca puts the tab straight into the environment of everything it spawns, so it
needs no permission at all; the desktop app leaves no mark and gets one group
and the app itself as the way back.

| Layer | Module | Role |
|-------|--------|------|
| Data | `model.rs` | `SessionInfo` (facts), `Phase`, `BoardModel` (animated chip state: `apply` a snapshot, `tick` time). Pure Rust, unit-tested. |
| Data | `sources.rs` | `SessionSource` trait. `ClaudeSource` reads `~/.claude/sessions/<pid>.json`, the process env, and each session's `projects/<slug>/<session>/subagents/` transcripts; `FakeSource` is the in-memory sandbox the test tools edit. |
| Data | `host/` | `Host` trait: which app a session is seated in, which window of it, and how to bring that window to the front. `warp.rs`, `orca.rs`, `desktop.rs`. |
| Render | `render/scene.rs` | `Layout` (zoom, lanes, stripes), `chip_draws` (model → lane positions), `build_shapes` (→ flat `Shape` list). |
| Render | `render/paint.rs`, `render/svg.rs` | The same shape list painted with gpui paths (tessellations cached by geometry), or serialised to SVG for headless checks. |
| UI | `ui/board.rs` | The frameless window view: polls the active source, hit-tests chips, forwards clicks to `host/`. |
| UI | `ui/panel.rs`, `ui/devtools.rs`, `ui/settings.rs` | Shared floating-panel chrome, the test tools (below), and the settings panel. |
| Data | `settings.rs` | Theme + zoom, persisted as JSON. |
| Data | `pr.rs`, `ticket.rs` | What a session links out to: the GitHub PR (transcript → `gh`) and the Linear issue (name, PR title tag, or transcript links). |
| Support | `font.rs`, `geom.rs`, `theme.rs`, `mac.rs` | Embedded D-DIN outlines, polylines, palette, AppKit shims + the launch `PATH` fix. |

Pull requests: a session's PR is read out of its own transcript — the last
`github.com/<owner>/<repo>/pull/<n>` inside a message body whose repo matches
the checkout's `origin`. `cwd → branch` cannot do this job when several
sessions share one checkout. One ticket often has several PRs, so a session's are chained along the right
edge with the newest outermost and the oldest folded into a `+n` badge when the
chain runs long. State comes from `gh pr view` on a small pool of worker threads — never
inside the session poll — and the trace runs into the leftmost connector of
that chain, where the body's colour
is the whole signal — hollow draft, green outline open, orange failing, purple
merged, faint closed — with a yellow border laid over any of them while CI is
still running, and a tick beside the number where review has signed off.

Review is not one of those colours and never was one of them: it is orthogonal
to every one, so it is drawn as a tick on top instead, the way a running check
is a border on top. A pull request can be approved and failing, approved and
still a draft, approved and landed, and each reads as itself. Merging counts as
a sign-off — nothing lands without one — so a merged pull request keeps the
tick (#116). Hovering a connector puts the pull
request's own title beside it, above where there is room and below where there
is not, and never off the edge of the window (#77, #115).

The Linear issue docks at the left edge, where the lane begins: the work comes
from a ticket and leaves through a PR. Both ends share a grammar — outlined
while a thing is still in flight, filled once it has resolved, black for done. Its key is the session's own name when
something corroborates it — a `linear.app` link, a `[ABC-123]` tag in the PR
title, or a team prefix the board has seen in a real link — otherwise the PR
title's tag. Status comes from `orca linear issue <KEY> --json`, which needs the
Orca app to be running; with Orca closed the node falls back to showing just
the key. Clicking a PR node opens it in the browser; a Linear node opens in the desktop app through `linear://`, falling back to the web.
`claudmagi --prs` prints what resolved.

A lane is drawn bold while a session is working on it, and turns green when
all three agree that the work is over: the issue done, the pull request
merged, the session idle. Weight says what is happening now, colour says what
is finished, and the two are independent.

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
  **RATE** runs that between a quarter speed and four times, since watching a
  lane slide into place wants one end of it and a crowd arriving wants the
  other. **GROW** adds a session to a group already on the board, one pane at
  a time: the sandbox otherwise starts a new group every third session and a
  tab full of panes never turns up.
- **SESSIONS** (sandbox only; the live list is `⌘L`) has `W/N/I` phase
  buttons, `+SUB` (spawn a synthetic subagent) and `×`; subagent rows have
  `RUN/DONE` and `×`. Clicking a sandbox name cycles its phase. `Esc` closes
  it.

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
