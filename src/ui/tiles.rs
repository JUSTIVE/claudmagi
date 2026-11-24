//! The board as a Metro start screen (#157), built from gpui-metro's own
//! `tile_grid` and live `tile` (#176).
//!
//! The panels around the board speak Windows Phone Metro (#156); this is the
//! board in the same language, and since gpui-metro has the start screen
//! itself, it is drawn by gpui-metro rather than by the board's canvas:
//! the turnstile, the live tiles' peek (frames only while a tile slides, at
//! the display's rate), the tilt under the pointer and the packing are all
//! its own.
//!
//! Tiles are grouped by the *work*, not by the window (#162): everything on
//! one Linear issue sits in that issue's group — failing an issue, one pull
//! request, and failing that, one directory — whichever Warp tab its
//! sessions run in. A group reads as the work does: the issue, the sessions
//! as live tiles (with their subagents as small tiles after them), and what
//! the work produced, a tile per pull request or page.

use gpui::{AnyElement, ClickEvent, Context, Rgba, SharedString, div, prelude::*, svg};
use gpui_metro::{Theme, Tile, TileSize, page_header, scaled, tile, tile_grid, u};

use crate::logos::Mark;
use crate::model::{BoardModel, ChipState, Phase, SessionInfo, Target};
use crate::render::comb;
use crate::theme::{self, Palette};
use crate::ui::board::Board;

/// A small tile's side and the gutter after it, in design px: a session is
/// a medium tile, two of them and the gutter a side.
const UNIT: f32 = 64.0;
const GAP: f32 = 8.0;
/// A group is one medium tile wide, or two when any group has more than
/// two tiles' worth in it (`columns`).
const NARROW: usize = 2;
const WIDE: usize = 4;
/// Kept clear around the screen, and under the traffic lights.
const MARGIN: f32 = 24.0;
const TOP: f32 = 22.0;
/// Between one row of groups and the next.
const ROW_GAP: f32 = 28.0;
/// A live tile's cycle, gpui-metro's own default.
const PERIOD: f32 = 12.0;
/// The mark in a tile's free corner (#187), in design px.
const MARK: f32 = 11.0;
/// The most letters a group's title runs to before it is cut, per small
/// tile of the group's width: the title is set in one line over the group.
const TITLE_CHARS_PER_UNIT: usize = 6;

/// What a session is working on, as the group it goes under (#162): its
/// Linear issue, or failing that its pull request, or failing that the
/// directory it works in — and there, the seat it sits in (#191).
///
/// A ticket or a pull request is a session saying what it is on, and two
/// sessions saying the same thing belong together whatever window they run
/// in. A shared directory says nothing of the kind: one repository holds
/// every piece of work anybody does in it. So where that is all there is,
/// the place breaks the tie — another tab, or another app entirely, is its
/// own heading rather than another tile under a folder's name.
pub fn work_unit(info: &SessionInfo) -> String {
    if let Some(t) = &info.ticket {
        return format!("issue:{}", t.key);
    }
    if let Some(p) = info.prs.last() {
        return format!("pr:{}", p.url());
    }
    if let Some(r) = info.prs_unanswered.last() {
        return format!("pr:{}", r.url());
    }
    format!("dir:{}@{}", info.cwd, info.group)
}

/// A group's title: the names its Claude sessions go by (#165) — what
/// `/rename` set, as written, or the session's directory where it has none —
/// each once, in board order.
pub fn section_name<'a>(sessions: impl Iterator<Item = &'a SessionInfo>) -> String {
    let mut names: Vec<String> = Vec::new();
    for info in sessions {
        let name = info.name.trim();
        let name = if name.is_empty() { info.label().to_lowercase() } else { name.to_string() };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join("  ·  ")
}

/// One tile of the start screen, by what it shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// The Linear issue of the group, carried by this session.
    Issue(usize),
    /// A session, as a medium live tile.
    Session(usize),
    /// One of a session's subagents, as a small tile: the session and the
    /// subagent's place among its live ones. The fourth of more than four
    /// counts the rest.
    Agent(usize, usize),
    /// What a session produced: one of its pull requests.
    Pr(usize, usize),
    /// A pull request number GitHub would not answer for (#112).
    Unanswered(usize, usize),
    /// The newest page a session published, when it has no pull request.
    Page(usize),
}

/// The start screen's groups in board order, each with its title and its
/// tiles: the issue once, the sessions each followed by their subagents,
/// then everything they produced, each thing once.
pub fn groups(model: &BoardModel) -> Vec<(String, Vec<Kind>)> {
    let mut units: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, c) in model.chips.iter().enumerate() {
        if c.anim.gone {
            continue;
        }
        let key = work_unit(&c.info);
        match units.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(i),
            None => units.push((key, vec![i])),
        }
    }
    units
        .into_iter()
        .map(|(_, members)| {
            let title = section_name(members.iter().map(|&i| &model.chips[i].info));
            let mut tiles = Vec::new();
            if let Some(&i) = members.iter().find(|&&i| model.chips[i].info.ticket.is_some()) {
                tiles.push(Kind::Issue(i));
            }
            for &i in &members {
                tiles.push(Kind::Session(i));
                let live = model.chips[i].subs.iter().filter(|s| !s.anim.gone).count();
                tiles.extend((0..live.min(4)).map(|k| Kind::Agent(i, k)));
            }
            let mut shown: Vec<String> = Vec::new();
            let mut once = |key: String| {
                let fresh = !shown.contains(&key);
                if fresh {
                    shown.push(key);
                }
                fresh
            };
            for &i in &members {
                let info = &model.chips[i].info;
                for (k, p) in info.prs.iter().enumerate() {
                    if once(p.url()) {
                        tiles.push(Kind::Pr(i, k));
                    }
                }
                for (k, r) in info.prs_unanswered.iter().enumerate() {
                    if once(r.url()) {
                        tiles.push(Kind::Unanswered(i, k));
                    }
                }
                if info.prs.is_empty() && info.prs_unanswered.is_empty() {
                    if let Some(a) = info.artifacts.last() {
                        if once(a.url()) {
                            tiles.push(Kind::Page(i));
                        }
                    }
                }
            }
            (title, tiles)
        })
        .collect()
}

/// How many small tiles wide every group is: narrow — one medium tile, so
/// groups of one or two things stand close — unless some group holds more
/// than two medium tiles' worth, which would make it a tall column (#173).
pub fn columns(groups: &[(String, Vec<Kind>)]) -> usize {
    let worth = |kinds: &Vec<Kind>| {
        let small = kinds.iter().filter(|k| matches!(k, Kind::Agent(..))).count();
        kinds.len() - small + small.div_ceil(4)
    };
    if groups.iter().any(|(_, k)| worth(k) > 2) { WIDE } else { NARROW }
}

/// The most groups — pieces of work — a row holds, however wide the
/// window (#178).
const MAX_PER_ROW: usize = 4;

/// How many groups `columns` small tiles wide stand side by side in `room`
/// design px: as many as fit, never more than `MAX_PER_ROW`, and always one.
pub fn groups_per_row(room: f32, columns: usize) -> usize {
    let (group, gap) = (group_width(columns), gpui_metro::tile::GROUP_GAP);
    (((room + gap) / (group + gap)).floor() as usize).clamp(1, MAX_PER_ROW)
}

/// One group's width in design px, `columns` small tiles across.
fn group_width(columns: usize) -> f32 {
    columns as f32 * UNIT + columns.saturating_sub(1) as f32 * GAP
}

/// How far in from the margin the start screen sits so its widest row is
/// centred in `room` (#179): `groups` in all, `per_row` to a row. Nothing
/// when the row fills the room.
pub fn centring(room: f32, columns: usize, groups: usize, per_row: usize) -> f32 {
    let k = per_row.min(groups).max(1) as f32;
    let row = k * group_width(columns) + (k - 1.0) * gpui_metro::tile::GROUP_GAP;
    ((room - row) / 2.0).max(0.0)
}

/// `text` cut to `n` characters, with an ellipsis if it was longer.
fn clip(text: &str, n: usize) -> String {
    if text.chars().count() <= n { text.to_string() } else { format!("{}…", text.chars().take(n - 1).collect::<String>()) }
}

/// Where in its cycle a session's tile starts, from its id: every tile
/// keeps its own clock, so a screen of them never turns in step.
fn phase_of(info: &SessionInfo) -> f32 {
    let mut h: u32 = 2_166_136_261;
    for b in info.session_id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    (h % 1000) as f32 / 1000.0
}

/// How long ago `started` was, short: `40m`, `3h 05m`, `2d`. `started` is
/// whatever the session file said, seconds or milliseconds.
fn running_for(started: u64) -> Option<String> {
    let started = if started > 100_000_000_000 { started / 1000 } else { started };
    // Before 2001 is not a start time anybody wrote down; the sandbox's
    // sessions carry a placeholder.
    if started < 1_000_000_000 {
        return None;
    }
    let secs = crate::usage::now_secs().checked_sub(started)?;
    let (m, h, d) = (secs / 60, secs / 3600, secs / 86_400);
    Some(if d > 0 {
        format!("{d}d")
    } else if h > 0 {
        format!("{h}h {:02}m", m % 60)
    } else {
        format!("{m}m")
    })
}

/// The back of a session's tile: what it is up to, a line large and a few
/// small. Waiting, what for; working, what its subagents are on, or where
/// it works on its own; idle, where it is and how long it has been up.
fn back_lines(info: &SessionInfo, chip: &ChipState) -> (String, Vec<String>) {
    let place = info.short_cwd();
    match info.phase() {
        Phase::NeedsUser => {
            let mut small: Vec<String> = info.waiting_for.iter().map(|w| w.to_lowercase()).collect();
            small.push(place);
            ("needs you".into(), small)
        }
        Phase::Working => {
            let busy: Vec<String> = chip
                .subs
                .iter()
                .filter(|x| !x.anim.gone && x.info.running)
                .map(|x| {
                    let what = x.info.description.trim();
                    if what.is_empty() { x.info.label().to_lowercase() } else { what.to_lowercase() }
                })
                .collect();
            if busy.is_empty() {
                (place, vec!["on its own".into()])
            } else {
                (format!("{} at work", busy.len()), busy.into_iter().take(3).collect())
            }
        }
        Phase::Idle => (place, running_for(info.started_at).map(|up| vec![format!("up {up}")]).unwrap_or_default()),
    }
}

/// A medium tile's face: a corner mark, a large line, small lines under it,
/// and the tile's title at the bottom-left where Metro keeps it.
#[allow(clippy::too_many_arguments)]
fn face(
    ink: Rgba,
    corner: Option<String>,
    big: String,
    small: Vec<String>,
    title: String,
    foot: Option<String>,
    mark: Option<Mark>,
) -> AnyElement {
    let soft = gpui::Hsla::from(ink).opacity(0.72);
    div()
        .size_full()
        .relative()
        .p(u(9.))
        .flex()
        .flex_col()
        .gap(u(3.))
        .text_color(theme::hsla(ink))
        .children(corner.map(|c| div().absolute().top(u(6.)).right(u(9.)).text_size(u(20.)).child(c)))
        .child(div().text_size(u(15.)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(big))
        .children(small.into_iter().map(|line| {
            div().text_size(u(10.)).text_color(soft).whitespace_nowrap().overflow_hidden().text_ellipsis().child(line)
        }))
        .child(
            div()
                .absolute()
                .left(u(9.))
                .right(u(MARK + 12.))
                .bottom(u(7.))
                .flex()
                .flex_col()
                .child(div().text_size(u(13.)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(title))
                .children(foot.map(|f| div().text_size(u(10.)).text_color(soft).child(f))),
        )
        // What the tile is a tile *of*, in the one corner nothing else uses
        // (#187). The badge has the top right, the label the top left, the
        // name and its state the bottom left; this sits opposite the name,
        // in the ink at the same weight the state line is drawn in, so it
        // reads as a mark on the tile rather than another thing on it.
        .children(mark.map(|m| {
            svg()
                .absolute()
                .right(u(8.))
                .bottom(u(7.))
                .size(u(MARK))
                .flex_none()
                .path(m.path())
                .text_color(soft)
        }))
        .into_any_element()
}

const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// The colours of the board the tiles stand on.
#[derive(Clone, Copy)]
struct Inks {
    pal: Palette,
    accent: Rgba,
}

impl Inks {
    /// A tile's fill and the ink on it. A board that prints has one ink
    /// (#146): every tile is that ink, lettered in the paper.
    fn on(self, fill: Rgba) -> (Rgba, Rgba) {
        if self.pal.screened {
            return (self.pal.ink, self.pal.bg);
        }
        // White on the accent, which is what Metro sets on it whatever the
        // accent is; everything else is worked out against its own fill.
        let ink = if fill == self.accent { WHITE } else { comb::legible(WHITE, fill, self.pal) };
        (fill, ink)
    }

    /// Working tiles wear the board's chip colour; one waiting on you is the
    /// one in the accent; an idle one goes grey (#161).
    ///
    /// The accent is the session's own voice (#188). A pull request with red
    /// checks wants a person too, and the comb's cell had to say so — there
    /// the pull request is a row *inside* the cell, and the cell's colour is
    /// the only thing that carries across the board. On a start screen it is
    /// a tile of its own, already alarm-coloured and lettered `fail`, so the
    /// session saying it again only put the loudest colour on a tile whose
    /// own word is `idle`, while the status bar counted nobody waiting.
    fn session(self, info: &SessionInfo) -> Rgba {
        match info.phase() {
            Phase::NeedsUser => self.accent,
            Phase::Working => self.pal.chip,
            Phase::Idle => self.quiet(),
        }
    }

    fn quiet(self) -> Rgba {
        theme::lerp(self.pal.bg, self.pal.ink, 0.32)
    }
}

impl Board {
    /// The tiles view: a page header and the start screen under it, scrolling
    /// both ways in the space above the status bar.
    pub(crate) fn render_tiles(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pal = self.palette();
        let accent = if self.settings.theme == crate::settings::BoardTheme::Orange {
            pal.ink
        } else {
            Rgba::from(Theme::palette(cx).accent)
        };
        let inks = Inks { pal, accent };
        let board = cx.entity().downgrade();
        let replay = self.tiles_replay;

        let groups = groups(&self.model);
        let columns = columns(&groups);
        // gpui-metro sets a start screen's groups side by side in one long
        // row, the way Windows 8 scrolled sideways. A window is not a
        // screen: the groups that fit across it make a row, and the rest
        // go on in rows below, each a start screen of its own (#177).
        let room = self.window.0 / self.layout.zoom.max(0.1) - 2.0 * MARGIN;
        let per_row = groups_per_row(room, columns);
        // On a wide window the screen stands in the middle, the title over
        // its left edge, rather than hugging the left with the rest empty.
        let inset = centring(room, columns, groups.len(), per_row);
        let rows: Vec<AnyElement> = groups
            .chunks(per_row)
            .enumerate()
            .map(|(r, row)| {
                let mut grid = tile_grid()
                    .id(("start", r))
                    .metrics(UNIT, GAP)
                    .columns(columns)
                    .hinge(MARGIN + inset)
                    .turnstile(true)
                    .replay(replay);
                for (title, kinds) in row {
                    let tiles: Vec<Tile> = kinds.iter().filter_map(|k| self.tile_for(k, inks, &board)).collect();
                    // One short of the room: the last group of a row has nothing to
                    // its right for a long title to run into, and it wraps instead.
                    grid = grid.group(clip(title, columns * TITLE_CHARS_PER_UNIT - 1), tiles);
                }
                grid.into_any_element()
            })
            .collect();
        let body = if groups.is_empty() {
            div().pl(u(MARGIN)).text_color(Theme::palette(cx).subtle).child("no claude code sessions").into_any_element()
        } else {
            div().pl(u(MARGIN)).flex().flex_col().gap(u(ROW_GAP)).children(rows).into_any_element()
        };
        div()
            .id("tiles")
            .size_full()
            .bg(theme::hsla(pal.bg))
            .overflow_scroll()
            .child(
                scaled(self.layout.zoom).child(
                    div()
                        .pt(u(TOP))
                        .pb(u(MARGIN))
                        .pr(u(MARGIN))
                        .pl(u(inset))
                        .flex()
                        .flex_col()
                        .child(page_header("start"))
                        .child(body),
                ),
            )
            .into_any_element()
    }

    /// The tile for one of the start screen's things, pressing as what it
    /// shows: a session (or its subagent) jumps to its terminal, an issue
    /// opens in Linear, a pull request or a page in the browser. The grid
    /// swings away first and the press lands after; the next frame brings
    /// the screen back in.
    fn tile_for(&self, kind: &Kind, inks: Inks, board: &gpui::WeakEntity<Board>) -> Option<Tile> {
        let press = |act: Press| {
            let board = board.clone();
            move |_: &ClickEvent, _: &mut gpui::Window, cx: &mut gpui::App| {
                let _ = board.update(cx, |this, cx| {
                    match &act {
                        Press::Session(i) => this.activate(Target::Session(*i), cx),
                        Press::Link { label, first, then, what } => {
                            this.open_link(label.clone(), first.clone(), then.clone(), what, cx)
                        }
                    }
                    this.tiles_replay += 1;
                    this.touch(cx);
                });
            }
        };
        match *kind {
            Kind::Session(i) => {
                let chip = self.model.chips.get(i)?;
                let info = &chip.info;
                let (fill, ink) = inks.on(inks.session(info));
                let running = chip.subs.iter().filter(|x| !x.anim.gone && x.info.running).count();
                let corner = if info.phase() == Phase::NeedsUser {
                    Some("!".to_string())
                } else {
                    (running > 0).then(|| running.to_string())
                };
                let name = info.label();
                let state = info.phase().label().to_lowercase();
                let (big, small) = back_lines(info, chip);
                let back_name = name.clone();
                // Where it runs, in the same mark the cell wore (#123): the
                // Claude mark would be on every session tile and say nothing.
                let mark = info.seat.as_ref().and_then(|s| Mark::host(s.host));
                // Everything the two faces show: with it the tile keeps its
                // last drawing while it slides, instead of every slide of
                // every tile redrawing the whole screen (#182).
                let shows = (
                    &name,
                    &state,
                    &corner,
                    &big,
                    &small,
                    mark,
                    [fill, ink].map(|c| [c.r, c.g, c.b, c.a].map(f32::to_bits)),
                );
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    shows.hash(&mut h);
                    h.finish()
                };
                Some(
                    tile(SharedString::from(format!("s-{}", info.session_id)), TileSize::Medium)
                        .metrics(UNIT, GAP)
                        .color(theme::hsla(fill))
                        .content_key(key)
                        .front(move |_, _| face(ink, corner, String::new(), Vec::new(), name, Some(state), mark))
                        .back(move |_, _| face(ink, None, big, small, back_name, None, mark))
                        .period(PERIOD)
                        .phase(phase_of(info))
                        .on_click(press(Press::Session(i))),
                )
            }
            Kind::Agent(i, k) => {
                let chip = self.model.chips.get(i)?;
                let agents: Vec<_> = chip.subs.iter().filter(|x| !x.anim.gone).collect();
                let n = agents.len();
                let agent = agents.get(k)?;
                let over = k == 3 && n > 4;
                let base = if inks.pal.screened { inks.pal.ink } else { inks.pal.chip };
                // A finished one is the colour given back to the board.
                let fill = if over || agent.info.running { base } else { theme::lerp(inks.pal.bg, base, 0.45) };
                let ink = if inks.pal.screened { inks.pal.bg } else { comb::legible(WHITE, fill, inks.pal) };
                let label = if over { format!("+{}", n - 3) } else { agent.info.label().to_lowercase() };
                Some(
                    tile(SharedString::from(format!("a-{}-{k}", chip.info.session_id)), TileSize::Small)
                        .metrics(UNIT, GAP)
                        .color(theme::hsla(fill))
                        .front(move |_, _| {
                            div()
                                .size_full()
                                .relative()
                                .text_color(theme::hsla(ink))
                                .child(if over {
                                    div().absolute().left(u(7.)).top(u(18.)).text_size(u(20.)).child(label)
                                } else {
                                    div()
                                        .absolute()
                                        .left(u(6.))
                                        .right(u(4.))
                                        .bottom(u(5.))
                                        .text_size(u(9.))
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(label)
                                })
                        })
                        .on_click(press(Press::Session(i))),
                )
            }
            Kind::Issue(i) => {
                let info = &self.model.chips.get(i)?.info;
                let t = info.ticket.as_ref()?;
                let (fill, ink) = inks.on(comb::floor_color(comb::ticket_color(t, inks.pal), inks.pal));
                let state = if t.state_name.is_empty() {
                    t.status.map(|st| st.short().to_lowercase()).unwrap_or_default()
                } else {
                    t.state_name.to_lowercase()
                };
                let (label, title) = (t.label(), t.title.clone());
                let act = Press::Link { label: t.label(), first: t.app_url(), then: t.url(), what: "Linear issue" };
                Some(
                    tile(SharedString::from(format!("i-{}", t.key)), TileSize::Medium)
                        .metrics(UNIT, GAP)
                        .color(theme::hsla(fill))
                        .front(move |_, _| face(ink, None, label, wrap_lines(&title), String::new(), Some(state), Some(Mark::Linear)))
                        .on_click(press(act)),
                )
            }
            Kind::Pr(i, k) => {
                let info = &self.model.chips.get(i)?.info;
                let p = info.prs.get(k)?;
                let (fill, ink) = inks.on(comb::floor_color(comb::pr_color(p, inks.pal), inks.pal));
                // A review still pending is said in a band across the top
                // (#153): the fill already says the pull request's state.
                let waiting = p.awaiting_review();
                let (label, title, state) = (p.label(), p.title.clone(), p.look().short().to_lowercase());
                let act = Press::Link { label: p.label(), first: None, then: Some(p.url()), what: "pull request" };
                Some(
                    tile(SharedString::from(format!("p-{}", p.url())), TileSize::Medium)
                        .metrics(UNIT, GAP)
                        .color(theme::hsla(fill))
                        .front(move |_, _| {
                            div()
                                .size_full()
                                .relative()
                                .child(face(ink, None, label, wrap_lines(&title), String::new(), Some(state), Some(Mark::GitHub)))
                                .when(waiting, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top_0()
                                            .left_0()
                                            .right_0()
                                            .h(u(5.))
                                            .bg(gpui::Hsla::from(ink).opacity(0.5)),
                                    )
                                })
                        })
                        .on_click(press(act)),
                )
            }
            Kind::Unanswered(i, k) => {
                let info = &self.model.chips.get(i)?.info;
                let r = info.prs_unanswered.get(k)?;
                let (fill, ink) = inks.on(inks.quiet());
                let label = format!("#{}", r.number);
                let act = Press::Link { label: label.clone(), first: None, then: Some(r.url()), what: "pull request" };
                Some(
                    tile(SharedString::from(format!("u-{}", r.url())), TileSize::Medium)
                        .metrics(UNIT, GAP)
                        .color(theme::hsla(fill))
                        .front(move |_, _| face(ink, None, label, Vec::new(), String::new(), Some("github did not answer".into()), Some(Mark::GitHub)))
                        .on_click(press(act)),
                )
            }
            Kind::Page(i) => {
                let info = &self.model.chips.get(i)?.info;
                let a = info.artifacts.last()?;
                let (fill, ink) = inks.on(inks.quiet());
                let (label, title) = (a.label(), a.title.clone());
                let act = Press::Link { label: a.label(), first: None, then: Some(a.url()), what: "page" };
                Some(
                    tile(SharedString::from(format!("g-{}", a.url())), TileSize::Medium)
                        .metrics(UNIT, GAP)
                        .color(theme::hsla(fill))
                        .front(move |_, _| face(ink, None, label, wrap_lines(&title), String::new(), Some("published".into()), Some(Mark::Artifact)))
                        .on_click(press(act)),
                )
            }
        }
    }
}

/// What a press on a tile does, once the grid has swung away.
enum Press {
    Session(usize),
    Link { label: String, first: Option<String>, then: Option<String>, what: &'static str },
}

/// A title in up to three short lines under a tile's large line.
fn wrap_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > 22 {
            out.push(std::mem::take(&mut line));
            if out.len() == 3 {
                break;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if out.len() < 3 && !line.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Three sessions in three different Warp tabs: the first two on one
    /// Linear issue (the first with a pull request), the third on nothing.
    fn work() -> BoardModel {
        use crate::sources::SessionSource;
        let src = crate::sources::FakeSource::new();
        for _ in 0..3 {
            src.add(Phase::Working);
        }
        let ids: Vec<String> = src.snapshot().into_iter().map(|s| s.session_id).collect();
        src.set_ticket(&ids[0], Some(Some(crate::ticket::Status::ALL[0])));
        src.add_pr(&ids[0], crate::pr::Look::ALL[0]);
        let mut list = src.snapshot();
        list.sort_by_key(|s| ids.iter().position(|id| *id == s.session_id));
        list[1].ticket = list[0].ticket.clone();
        list[1].prs.clear();
        list[1].artifacts.clear();
        list[2].ticket = None;
        list[2].prs.clear();
        list[2].artifacts.clear();
        for (k, s) in list.iter_mut().enumerate() {
            s.group = format!("warp-tab:{k}");
            s.subagents.clear();
            s.prs_unanswered.clear();
        }
        list[0].artifacts.clear();
        let mut model = BoardModel::new();
        model.apply(list, Instant::now());
        model.settle();
        model
    }

    #[test]
    fn sessions_on_one_issue_share_a_group_whatever_tab_they_run_in() {
        let model = work();
        let groups = groups(&model);
        assert_eq!(groups.len(), 2, "one group for the issue, one for the loose session");
        assert_eq!(
            groups[0].1,
            vec![Kind::Issue(0), Kind::Session(0), Kind::Session(1), Kind::Pr(0, 0)],
            "the issue once, both sessions, then what they produced"
        );
        assert_eq!(groups[1].1, vec![Kind::Session(2)]);
    }

    /// A folder is not a piece of work (#191). Sessions that share only a
    /// directory are sorted by where they sit — so a Herdr tab, a Warp tab
    /// and the next tab along each keep their own heading — while a ticket
    /// or a pull request still gathers them whatever window they run in
    /// (#162).
    #[test]
    fn a_shared_folder_alone_does_not_make_one_section() {
        let mut a = SessionInfo::synthetic(1, "A", Phase::Idle);
        let mut b = SessionInfo::synthetic(2, "B", Phase::Idle);
        for s in [&mut a, &mut b] {
            s.cwd = "/Users/x/git/crepe/yoshi".into();
            s.ticket = None;
            s.prs.clear();
            s.prs_unanswered.clear();
        }
        a.group = "warp-tab:1-1".into();
        b.group = "herdr-tab:93362db4".into();
        assert_ne!(work_unit(&a), work_unit(&b), "one folder, two places, two sections");

        b.group = "warp-tab:1-3".into();
        assert_ne!(work_unit(&a), work_unit(&b), "two tabs of one app are two places too");

        b.group = a.group.clone();
        assert_eq!(work_unit(&a), work_unit(&b), "the same folder in the same tab is one section");

        // What a session is on outranks where it sits, which is the whole
        // point of grouping by work (#162).
        b.group = "herdr-tab:93362db4".into();
        for s in [&mut a, &mut b] {
            s.ticket = Some(crate::ticket::Ticket::synthetic(7, Some(crate::ticket::Status::Started)));
        }
        assert_eq!(work_unit(&a), work_unit(&b), "one issue is one section, wherever it is worked on");
    }

    #[test]
    fn a_section_is_named_for_its_sessions() {
        let model = work();
        let mut a = model.chips[0].info.clone();
        a.name = "fix login".into();
        let mut b = model.chips[1].info.clone();
        b.name = String::new();
        let c = a.clone();
        let name = section_name([&a, &b, &c].into_iter());
        assert_eq!(name, format!("fix login  ·  {}", b.label().to_lowercase()), "renamed as written, unnamed by directory, each once");
    }

    #[test]
    fn subagents_follow_their_session_and_the_fourth_counts_the_rest() {
        let mut model = work();
        let info = model.chips[2].info.clone();
        let mut list: Vec<SessionInfo> = model.chips.iter().map(|c| c.info.clone()).collect();
        for k in 0..6 {
            list[2].subagents.push(crate::model::SubagentInfo::synthetic(k, "AGENT", "", true));
        }
        model.apply(list, Instant::now());
        model.settle();
        let groups = groups(&model);
        let loose = &groups.iter().find(|(_, k)| k.contains(&Kind::Session(2))).unwrap().1;
        assert_eq!(
            loose,
            &vec![Kind::Session(2), Kind::Agent(2, 0), Kind::Agent(2, 1), Kind::Agent(2, 2), Kind::Agent(2, 3)],
            "{}: four small tiles after the session, never more",
            info.label()
        );
    }

    #[test]
    fn groups_stay_one_tile_wide_until_one_needs_two() {
        let model = work();
        // The issue's group has four things in it: two tiles wide.
        assert_eq!(columns(&groups(&model)), WIDE);
        let one = vec![("a".to_string(), vec![Kind::Session(0), Kind::Pr(0, 0)])];
        assert_eq!(columns(&one), NARROW, "a session and its pull request fit one tile wide");
        let agents = vec![("a".to_string(), vec![Kind::Session(0), Kind::Agent(0, 0), Kind::Agent(0, 1)])];
        assert_eq!(columns(&agents), NARROW, "subagents take one medium tile's room between them");
    }

    #[test]
    fn groups_wrap_to_the_window_instead_of_running_off_it() {
        // One medium tile a group: 136 px and a 56 px gap each.
        assert_eq!(groups_per_row(136.0, NARROW), 1);
        assert_eq!(groups_per_row(136.0 * 3.0 + 56.0 * 2.0, NARROW), 3, "three fit exactly");
        assert_eq!(groups_per_row(136.0 * 3.0 + 56.0 * 2.0 - 1.0, NARROW), 2);
        assert_eq!(groups_per_row(20.0, WIDE), 1, "never none, however narrow");
        assert_eq!(groups_per_row(10_000.0, NARROW), MAX_PER_ROW, "four pieces of work a row at most");
    }

    #[test]
    fn a_wide_window_centres_the_widest_row() {
        // Three narrow groups (3 × 136 + 2 × 56 = 520) in 1000: 240 either side.
        assert_eq!(centring(1000.0, NARROW, 3, 4), 240.0);
        // Seven groups go four to a row: the four-wide row is what is centred.
        let four = 4.0 * 136.0 + 3.0 * 56.0;
        assert_eq!(centring(1000.0, NARROW, 7, 4), (1000.0 - four) / 2.0);
        assert_eq!(centring(300.0, NARROW, 3, 1), 82.0, "one to a row on a narrow window, still centred");
        assert_eq!(centring(100.0, WIDE, 1, 1), 0.0, "never pushed off the left");
    }

    /// The accent calls you over, so only the session may use it (#188). A
    /// pull request with red checks is its own tile on a start screen and
    /// says `fail` there; a session that is not asking anything stays grey
    /// however that tile reads, which is what the status bar counts.
    #[test]
    fn a_sleeping_session_is_not_coloured_for_its_pull_request() {
        let inks = Inks { pal: theme::PALETTE, accent: Rgba { r: 1.0, g: 0.44, b: 0.0, a: 1.0 } };
        let red = crate::pr::Pr::synthetic(1, crate::pr::Look::Failing);

        let mut idle = SessionInfo::synthetic(1, "S-1", Phase::Idle);
        idle.prs.push(red.clone());
        assert_eq!(inks.session(&idle), inks.quiet(), "a red pull request does not wake its session's tile");

        let mut busy = SessionInfo::synthetic(2, "S-2", Phase::Working);
        busy.prs.push(red);
        assert_eq!(inks.session(&busy), theme::PALETTE.chip, "nor does it stop one working");

        let asking = SessionInfo::synthetic(3, "S-3", Phase::NeedsUser);
        assert_eq!(inks.session(&asking), inks.accent, "the accent is for a session asking");
    }

    #[test]
    fn a_title_wraps_by_word_into_three_lines_at_most() {
        let lines = wrap_lines("make the start screen draw with the metro library instead of the board canvas please");
        assert!(lines.len() <= 3, "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 22 || !l.contains(' ')), "{lines:?}");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
    }
}
