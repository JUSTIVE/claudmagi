# claudmagi

A board of your live Claude Code sessions, built on [Native Shell](https://nsh.dev)
(AppKit views, their motion on Core Animation) and dressed in Windows Phone Metro by
[nsh-metro](https://github.com/JUSTIVE/nsh-metro): flat square sheets that slide in from
the edge, an accent colour, light lower-case titles. It was a gpui app until #195; the
last gpui build is `5a67699`.

Every running `claude` process is a live tile on a Metro start screen, with its Linear
issue, its subagents and the pull requests it opened beside it.
A session that needs you (permission prompt, question, dialog) is the one in the accent.
Click a session to jump to the Warp tab that hosts it. The circuit board this started as,
and the metaballs after it, went in #183; the app icon still remembers the circuit.

## Install

nsh-metro comes from [its GitHub repo](https://github.com/JUSTIVE/nsh-metro),
pinned to a commit; Native Shell (`nsh-core`, `nsh-platform-macos`) from crates.io.
Native Shell wants Rust 1.97 or later.

```sh
tools/bundle.sh          # release build → claudmagi.app → /Applications
open -a claudmagi
```

The script packs `assets/icon.svg` (the circuit board, kept as art since that view went) into the icon, ad-hoc signs the
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

- Frameless window: drag the top strip, the status bar or any bare part of the board to move it, drag edges to resize. `CLAUDMAGI_WINDOW=WxH` opens at a given size.
- `⌘+` / `⌘-` / `⌘0` (or ⌘ + wheel) zoom the whole board between 50% and 300%.
- `⌘K` searches everything on the board at once: session names, Linear issue keys, pull request numbers, and the paths and titles behind them. Picking a row does what clicking that node does — jump to the Warp tab, open the PR, open the issue in Linear.app. Letters may be scattered across a candidate, but an unbroken run of them is worth far more than the same letters spread out, so `pjm-20` puts the tickets that carry it whole on top. A hit brings its whole lane with it — find a pull request by its number and its issue and its session come along under it, stepped in behind the hit — since the number is often all anyone remembers and the work around it is what was wanted. Each row is drawn as the node itself, in the body the board would give it and carrying the mark of whose thing it is — Claude, GitHub, Linear — at one chip size for every kind, so a merged pull request, a chip waiting on you and an issue nobody has started read the same in the list as they do on the board. `↑↓` moves, `⏎` opens, `esc` or a click on the board closes.
- **The start screen** is the board (#157). It groups by the work, not the window (#162): everything on one Linear issue sits in one group (failing an issue, one pull request; failing that, one directory), whichever Warp tab it runs in, headed by the names of its Claude sessions (#165). A group reads left to right: the issue as a tile of its own, the sessions as live tiles (name and state; subagents split the block beside them into four small tiles), and what they produced, each pull request a tile painted in its state. Working tiles wear the board's chip colour (black on the white board); a tile waiting on you is the one in the accent, with a `!`; an idle one goes grey (#161). Each group is nsh-metro's `tile_grid` of live `tile`s (#176, #195): tiles turn in on its turnstile, every session tile slides to a back that says what it is up to, a tile that arrives swings open in its group while the rest slide over (a new group joins without the screen turning in again), and a press swings the whole screen away, the pressed tile last, before it lands. The honeycomb view went with gpui (#195); `5a67699` still has it.
- `⌘L` opens a list down the right-hand side: every session the board is drawing, in board order, with its issue, its pull requests and its subagents underneath. Every line clicks through to the same place its node on the board does (#103).
- `⌘,` (or the `SETTINGS` button in the status bar) opens the settings panel: view, background (white / orange / dark), Metro accent (the twenty Windows Phone accents, orange by default) and UI size (a slider plus presets). The panels take the dark Metro ground on the dark board and the light one otherwise; the status bar keeps the board's own ink so it reads on all three, with the accent filling whatever is switched on (ink on the orange board, where an orange accent would vanish). Settings persist in `~/Library/Application Support/claudmagi/settings.json`.
- The status bar shows your Claude plan usage (the `/usage` numbers: 5-hour and 7-day utilization with the time to reset), read once a minute from Anthropic's usage endpoint with the login Claude Code keeps in the keychain. Nothing is shown when there is no Claude Code login. `claudmagi --usage` prints the same numbers.
- `Esc` or `⌘Q` quits.

### Headless helpers

```sh
cargo run -- --list                 # print discovered sessions
cargo run -- --focus PERSONAL-71    # jump to a session's Warp tab from the CLI
```

## Architecture

Data and the window are separate layers; only `ui/` touches Native Shell.

The session registry at `~/.claude/sessions/<pid>.json` is Claude Code's own
and says nothing about terminals: a session writes one wherever it runs. It is
written by the CLI entrypoint, though, so it holds only sessions running on
this machine. A cloud session — started from the desktop app, the phone or a
schedule, and run in Anthropic's own container — leaves nothing here at all:
no process, no record, not even a transcript. Those are read over the wire
from `/v1/code/sessions`, the endpoint the desktop app lists them with, using
the token `usage.rs` already takes out of the login keychain. What comes back
is richer than the local registry: whether the container is still connected,
what the last turn made of itself, which branch it is on, and whether anyone
has looked (#120). What a terminal adds is a *seat*: which window of which app the
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
| Data | `host/` | `Host` trait: which app a session is seated in, which window of it, and how to bring that window to the front. `warp.rs`, `orca.rs`, `cloud.rs`, `desktop.rs`. |
| Data | `cloud.rs` | Sessions running in Anthropic's containers, read from `/v1/code/sessions` with the login keychain's token. |
| UI | `ui/state.rs` | The board's state behind one handle (`Ui`): every change bumps a revision signal, and only the bindings that read the board run again (#195). Opening links and jumping to a session's terminal run off the engine thread. |
| UI | `ui/app.rs` | The window (no system title bar, drawn up to the top edge), the status bar, the keys, and the clocks that poll the sessions and the plan usage. |
| UI | `ui/panel.rs`, `ui/devtools.rs`, `ui/settings.rs` | Shared floating-panel chrome, the test tools (below), and the settings panel. |
| UI | `ui/tiles.rs` | The Metro start screen (#157): sessions grouped by the work, a nsh-metro `tile_grid` per group; a tile is known by what it is of, and its faces are bound to the board by that key (#195). |
| UI | `ui/metro.rs` | Settings → nsh-metro theme (mode from the board, accent by name, the face set to D-DIN), and the panel-scale Metro pieces (`chip`, `section`, `hint`, `phase_mark`). |
| UI | `ink.rs`, `zoom.rs` | The colours a tile is painted in against the board, and the user's zoom. |
| Data | `settings.rs` | Theme + zoom, persisted as JSON. |
| Data | `pr.rs`, `ticket.rs` | What a session links out to: the GitHub PR (transcript → `gh`) and the Linear issue (name, PR title tag, or transcript links). |
| Support | `font.rs`, `theme.rs`, `mac.rs` | The embedded D-DIN, the palette, AppKit shims + the launch `PATH` fix. |

Pull requests: a session's PR is read out of its own transcript — the last
`github.com/<owner>/<repo>/pull/<n>` inside a message body whose repo matches
the checkout's `origin`. `cwd → branch` cannot do this job when several
sessions share one checkout. State comes from `gh pr view` on a small pool of worker threads — never inside
the session poll — and it is what a pull request's tile or floor is painted in:
draft, open, failing, merged, closed, with CI still running and review signed
off said on top.

Review is not one of those colours and never was one of them: it is orthogonal
to every one, so it is drawn as a tick on top instead, the way a running check
is a border on top. A pull request can be approved and failing, approved and
still a draft, approved and landed, and each reads as itself. Merging counts as
a sign-off — nothing lands without one — so a merged pull request keeps the
tick (#116).

The Linear issue is the work's other end: it comes from a ticket and leaves
through a PR. Its key is the session's own name when
something corroborates it — a `linear.app` link, a `[ABC-123]` tag in the PR
title, or a team prefix the board has seen in a real link — otherwise the PR
title's tag. Status comes from `orca linear issue <KEY> --json`, which needs the
Orca app to be running; with Orca closed it falls back to showing just
the key. Clicking a pull request opens it in the browser; an issue opens in the desktop app through `linear://`, falling back to the web.
`claudmagi --prs` prints what resolved.

Phase rules: `waitingFor` present → needs input · `status == busy` → working ·
otherwise idle (`tempo`/`state == blocked` also counts as needs input). Warp
jumps use `WARP_FOCUS_URL=warp://session/<uuid>` read from the Claude process
environment via `sysctl KERN_PROCARGS2`.

## Test tools

Press `T` (or click `TEST` in the status bar) to open the test panel. Panels dock down the right edge, from the top of the window to the status bar, and only one is open at a time: the test tools, the settings and the ⌘L list each close the others (#163). A panel taller than the window scrolls under its header (#158, #159):

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

The board's UI is set in **D-DIN**, compiled into the binary
from `assets/fonts/D-DIN.ttf` so there is no system-font dependency.

D-DIN covers 232 codepoints, which is Latin-1 and a little punctuation, so
anything outside that — Hangul, most obviously — used to come out of the board
as the `?` the glyph lookup falls back to. The faces are a chain now: each
character takes the first face that has it, and only what D-DIN lacks ever
reaches macOS's own. They are mapped rather than read, since the Korean one is
55MB and a board of ASCII never touches a page of it; the whole chain costs
about a third of a megabyte resident. Every face is drawn to the same cap
height, so a mixed label is one line at one size (#119).

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
