//! The board as a Metro start screen (#157).
//!
//! The panels around the board speak Windows Phone Metro (#156); this is the
//! board in the same language. Tiles are grouped by the *work*, not by the
//! window (#162): everything on one Linear issue sits under that issue's
//! header — failing an issue, one pull request, and failing that, one
//! directory — whichever Warp tab its sessions happen to run in. A group
//! reads left to right as the work does: the issue, the sessions on it as
//! live tiles (with the subagents a session has out splitting the block
//! beside it into four small tiles), and what the work produced.
//!
//! The issue and the pull request are tiles of their own rather than rows
//! of a session's tile: a tile has room for one thing said large, and an
//! issue shared by three sessions is one issue, not three rows.
//!
//! It is a seated view like the comb: it hands the board the same `Comb`
//! seating, only on a square grid (`Grid::Tiles`), so the scroll, the press
//! on each row, the drop-down and the glide to a new seat (#150) all work
//! as they do there.
//!
//! The motion is Metro's too, flattened to 2D the way gpui-metro flattens
//! it: tiles come in with a turnstile — a short travel from the right on a
//! hard ease-out, staggered left to right — and a working tile now and then
//! *peeks*: its back slides up from the bottom to say where it is running,
//! then drops away again.

use gpui::Rgba;

use crate::font;
use crate::geom::Pt;
use crate::model::{BoardModel, Phase, SessionInfo};
use crate::render::comb::{self, Cell, Comb, Extras, Grid, Slot, reveal};
use crate::render::scene::{Screen, Shape};
use crate::theme::{self, Palette};

/// A small tile's side, and the gap after it. A session is a medium tile,
/// two small ones and the gap between them a side (gpui-metro's
/// `TILE_UNIT`/`TILE_GAP`, at board scale).
pub const TILE: f32 = 64.0;
pub const GAP: f32 = 8.0;
const PITCH: f32 = TILE + GAP;
/// Most medium tiles in a row, however wide the window: a start screen is a
/// column you read down, not a wall.
const MAX_ACROSS: usize = 6;
/// Kept clear at either side of the grid.
const MARGIN: f32 = 32.0;
/// The page title's band above the first group.
const TITLE_H: f32 = 92.0;
/// Inside a tile, from its edges to its letters.
const PAD: f32 = 9.0;

/// How far a tile travels in on its turnstile, and what it takes to arrive.
const TURN: f32 = 44.0;

/// A live tile's cycle, front to back to front, and how long each slide
/// takes: gpui-metro's own numbers (`Tile::period`, `PEEK_SLIDE_SECS`).
const PEEK_PERIOD: f32 = 12.0;
const PEEK_SLIDE: f32 = 0.9;

/// Cap heights, in board pixels, of the tile's type ramp.
const CAP_TITLE: f32 = 30.0;
const CAP_HEADER: f32 = 13.0;
const CAP_NAME: f32 = 13.0;
const CAP_SMALL: f32 = 7.5;
const CAP_COUNT: f32 = 17.0;

fn scale_for(cap: f32) -> f32 {
    cap / font::height(1.0)
}

/// A session's medium tile, a side.
pub fn side() -> f32 {
    2.0 * TILE + GAP
}

/// What a session is working on, as the group it goes under (#162): its
/// Linear issue, or failing that its pull request, or failing that the
/// directory it works in. The key the group is told apart by, and its header.
pub fn work_unit(info: &SessionInfo) -> (String, String) {
    if let Some(t) = &info.ticket {
        let head = if t.title.is_empty() { t.label() } else { format!("{}  {}", t.label(), t.title) };
        return (format!("issue:{}", t.key), head);
    }
    if let Some(p) = info.prs.last() {
        let head = if p.title.is_empty() { p.label() } else { format!("{}  {}", p.label(), p.title) };
        return (format!("pr:{}", p.url()), head);
    }
    if let Some(r) = info.prs_unanswered.last() {
        return (format!("pr:{}", r.url()), r.label());
    }
    (format!("dir:{}", info.cwd), info.short_cwd())
}

/// One seat of the grid: a session, its subagents, or one of its rows as a
/// tile of its own.
struct Block {
    chip: Option<usize>,
    subs: Option<usize>,
    slot: Option<Slot>,
}

/// Lays the sessions out as a start screen: work unit by work unit in board
/// order, a header row and then the unit's tiles, left to right and
/// wrapping — the issue, the sessions, what they produced.
pub fn lay_out(model: &BoardModel, width: f32, _height: f32) -> Comb {
    let across = (((width - 2.0 * MARGIN + GAP) / (2.0 * PITCH)).floor() as usize).clamp(1, MAX_ACROSS);
    let grid_w = across as f32 * 2.0 * PITCH - GAP;
    let origin = Pt::new(((width - grid_w) / 2.0).max(MARGIN.min(width / 8.0)), TITLE_H);

    // Sessions by what they are working on, in board order (#162). The
    // window they run in does not come into it.
    let mut units: Vec<(String, String, Vec<usize>)> = Vec::new();
    for (i, c) in model.chips.iter().enumerate() {
        if c.anim.gone {
            continue;
        }
        let (key, head) = work_unit(&c.info);
        match units.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, _, members)) => members.push(i),
            None => units.push((key, head, vec![i])),
        }
    }

    let mut cells = Vec::new();
    let mut headers = Vec::new();
    let mut row = 0i32;
    for (_, head, members) in &units {
        headers.push((origin + Pt::new(0.0, row as f32 * PITCH + TILE * 0.55), head.clone()));
        row += 1;
        let mut blocks: Vec<Block> = Vec::new();
        // The issue once, whoever of them carries it.
        if let Some(&i) = members.iter().find(|&&i| model.chips[i].info.ticket.is_some()) {
            blocks.push(Block { chip: Some(i), subs: None, slot: Some(Slot::Ticket) });
        }
        // The sessions, each with its subagents right after it.
        for &i in members {
            blocks.push(Block { chip: Some(i), subs: None, slot: None });
            if model.chips[i].subs.iter().any(|s| !s.anim.gone) {
                blocks.push(Block { chip: None, subs: Some(i), slot: None });
            }
        }
        // What they produced, once per thing: two sessions on one pull
        // request show it once.
        let mut shown: Vec<String> = Vec::new();
        for &i in members {
            let info = &model.chips[i].info;
            let Some(key) = output_key(info) else { continue };
            if !shown.contains(&key) {
                shown.push(key);
                blocks.push(Block { chip: Some(i), subs: None, slot: Some(Slot::Output) });
            }
        }
        for (k, b) in blocks.iter().enumerate() {
            let q = (k % across) as i32 * 2;
            let r = row + (k / across) as i32 * 2;
            cells.push(Cell { center: Pt::new(0.0, 0.0), q, row: r, chip: b.chip, subs: b.subs, wave: 0.0, slot: b.slot });
        }
        row += blocks.len().div_ceil(across) as i32 * 2;
    }

    let mut comb = Comb {
        cells,
        r: side() / 2.0,
        height: origin.y + row as f32 * PITCH + MARGIN,
        grid: Grid::Tiles { pitch: PITCH, gap: GAP, origin, headers },
    };
    for i in 0..comb.cells.len() {
        let (q, r) = (comb.cells[i].q, comb.cells[i].row);
        comb.cells[i].center = comb.seat_center(q, r);
        // The turnstile runs across a row from the left, and each row a
        // beat behind the one above it.
        comb.cells[i].wave = q as f32 / 2.0 + r as f32 * 0.25;
    }
    comb
}

/// What a session's output tile is known by, or `None` when it has nothing
/// to show: the newest pull request, the number GitHub would not answer
/// for, or the newest page it published — the same choice the comb's bottom
/// row makes (#137).
fn output_key(info: &SessionInfo) -> Option<String> {
    if let Some(p) = info.prs.last() {
        return Some(p.url());
    }
    if let Some(r) = info.prs_unanswered.last() {
        return Some(r.url());
    }
    info.artifacts.last().map(|a| a.url())
}

/// How far a tile has turned to its back at `t`, from 0 (front) to 1
/// (back) — gpui-metro's live tile, cycle for cycle (#157): it rests on each
/// face for half the period and takes `PEEK_SLIDE` to slide between them.
/// Every tile keeps its own clock, so a screen of them never turns in step.
pub fn peek(info: &SessionInfo, t: f32) -> f32 {
    let mut h: u32 = 2_166_136_261;
    for b in info.session_id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    // Half a period or more of front first, so the view opens on fronts.
    let offset = (h % 1000) as f32 / 1000.0 * 0.5 * PEEK_PERIOD;
    let cycle = ((t - offset).max(0.0) / PEEK_PERIOD).fract();
    let slide = PEEK_SLIDE / PEEK_PERIOD;
    let k = if cycle < 0.5 - slide {
        0.0
    } else if cycle < 0.5 {
        ease_in_out((cycle - (0.5 - slide)) / slide)
    } else if cycle < 1.0 - slide {
        1.0
    } else {
        1.0 - ease_in_out((cycle - (1.0 - slide)) / slide)
    };
    if t < offset { 0.0 } else { k }
}

/// Whether any tile is mid-slide, which is when the frame clock has to run
/// at full rate. A tile resting on either face is still.
pub fn peeking(model: &BoardModel, t: f32) -> bool {
    model.live().any(|c| {
        let k = peek(&c.info, t);
        k > 0.0 && k < 1.0
    })
}

/// gpui-metro's `ease_in_out_cubic`, which its tiles slide on.
fn ease_in_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

/// Metro's hard exponential ease-out (gpui-metro `ease_out_expo`).
fn ease_out(x: f32) -> f32 {
    if x >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * x) }
}

/// What a session's tile is painted with: the accent while it waits on a
/// person, the board's own chip colour while it works (black on the white
/// board, the way a working chip is out on the circuit), and the board's
/// grey while it idles (#161).
///
/// The accent is the loudest thing on the screen, so it goes to the tile
/// that wants you: a screen of busy sessions is a screen of quiet black
/// tiles, and the one that needs an answer is the one in colour.
pub fn tile_fill(info: &SessionInfo, pal: Palette, accent: Rgba) -> Rgba {
    if comb::wants_a_person(info) {
        return accent;
    }
    match info.phase() {
        Phase::Working => pal.chip,
        Phase::Idle | Phase::NeedsUser => theme::lerp(pal.bg, pal.ink, 0.32),
    }
}

fn square(x: f32, y: f32, w: f32, h: f32) -> Vec<Pt> {
    vec![Pt::new(x, y), Pt::new(x + w, y), Pt::new(x + w, y + h), Pt::new(x, y + h)]
}

/// Letters set from their left edge, which is how Metro sets them.
fn text_left(text: String, cap: f32, left: f32, y: f32, color: Rgba) -> Shape {
    let scale = scale_for(cap);
    let w = font::measure(&text, scale);
    Shape::Text { text, scale, stroke: 1.0, angle: 0.0, center: Pt::new(left + w / 2.0, y), color }
}

const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// Every shape of the tile view, in design units. `t` is seconds since the
/// view opened; `accent` is the Metro accent the tiles are painted with.
pub fn build_shapes(
    model: &BoardModel,
    comb: &Comb,
    pal: Palette,
    scroll_y: f32,
    origin: Pt,
    t: f32,
    accent: Rgba,
) -> Vec<Shape> {
    let mut out = vec![Shape::Rect { x: 0.0, y: 0.0, w: 1.0e5, h: 1.0e5, color: pal.bg }];
    let Grid::Tiles { origin: grid, headers, .. } = &comb.grid else { return out };
    let grid = *grid;
    let lift = |p: Pt| p - Pt::new(0.0, scroll_y) + origin;
    let quiet = theme::lerp(pal.bg, pal.ink, 0.6);

    // The page title, Metro's huge light lower-case line.
    let title_at = lift(Pt::new(grid.x - 2.0, TITLE_H * 0.5));
    out.push(text_left("start".into(), CAP_TITLE, title_at.x, title_at.y, pal.ink));
    if comb.cells.is_empty() {
        let at = lift(grid + Pt::new(0.0, TILE * 0.4));
        out.push(text_left("no claude code sessions".into(), CAP_HEADER, at.x, at.y, quiet));
    }
    for (at, label) in headers {
        let at = lift(*at);
        out.push(text_left(label.clone(), CAP_HEADER, at.x, at.y, quiet));
    }

    let s = side();
    for cell in &comb.cells {
        let center = lift(cell.center);
        if center.y < -s || center.y > 1.0e4 {
            continue;
        }
        let arrived = reveal(cell, t);
        if arrived <= 0.001 {
            continue;
        }
        // The turnstile: in from the right on a hard ease-out, fading up.
        let e = ease_out(arrived);
        let left = center.x - s / 2.0 + (1.0 - e) * TURN;
        let top = center.y - s / 2.0;
        let fade = |c: Rgba| theme::with_alpha(c, arrived);

        if let Some(parent) = cell.subs {
            if let Some(chip) = model.chips.get(parent) {
                out.extend(colony(chip, left, top, pal, fade));
            }
            continue;
        }
        let Some(chip) = cell.chip.and_then(|i| model.chips.get(i)) else { continue };
        out.extend(match cell.slot {
            Some(Slot::Ticket) => issue_tile(&chip.info, left, top, pal, t, &fade),
            Some(Slot::Output) => output_tile(&chip.info, left, top, pal, t, &fade),
            _ => session_tile(&chip.info, chip, left, top, pal, accent, t, &fade),
        });
    }
    out
}

/// One session as a medium live tile: a front with the session's three
/// rows, and a back that says what it is up to, the two sliding past each
/// other inside the tile.
#[allow(clippy::too_many_arguments)]
fn session_tile(
    info: &SessionInfo,
    chip: &crate::model::ChipState,
    left: f32,
    top: f32,
    pal: Palette,
    accent: Rgba,
    t: f32,
    fade: &dyn Fn(Rgba) -> Rgba,
) -> Vec<Shape> {
    let s = side();
    let fill = tile_fill(info, pal, accent);
    // A board that prints has one ink (#146): every tile is that ink, and
    // the state is said by the weave of a band across its top instead.
    let (fill, band) = if pal.screened { (pal.ink, Some(comb::body_screen(info))) } else { (fill, None) };
    let mut out = vec![Shape::poly(square(left, top, s, s), fade(fill))];
    // White on the accent, which is what Metro sets on it whatever the
    // accent is; everything else is worked out against its own fill.
    let ink = if fill == accent && !pal.screened { WHITE } else { comb::legible(WHITE, fill, pal) };

    // Both faces, each laid out as if it were the only one, then slid past
    // each other and cut to the tile.
    let k = peek(info, t);
    let front = if k < 1.0 { front_face(info, chip, left, top, pal, fill, ink, t) } else { Vec::new() };
    let back = if k > 0.0 { back_face(info, chip, left, top, ink) } else { Vec::new() };
    let (lo, hi) = (top, top + s);
    out.extend(slid(front, -k * s, lo, hi).into_iter().map(|sh| faded(sh, fade)));
    out.extend(slid(back, (1.0 - k) * s, lo, hi).into_iter().map(|sh| faded(sh, fade)));

    // What stays put whichever face is up: the state, across the top.
    if let Some(screen) = band.filter(|b| *b != Screen::Solid) {
        out.push(Shape::Poly { points: square(left, top, s, 5.0), color: fade(pal.bg), screen });
    }
    out
}

/// The front: the count in the corner, and the session's name over what it
/// is doing, low on the tile where Metro sets a title. Its issue and what it
/// produced are tiles of their own beside it (#162).
#[allow(clippy::too_many_arguments)]
fn front_face(
    info: &SessionInfo,
    chip: &crate::model::ChipState,
    left: f32,
    top: f32,
    pal: Palette,
    fill: Rgba,
    ink: Rgba,
    t: f32,
) -> Vec<Shape> {
    let mut out = Vec::new();
    let s = side();
    let soft = theme::with_alpha(ink, 0.72);

    // The count Metro puts in a tile's corner: the subagents running, or a
    // `!` keeping time with a session that waits.
    let running = chip.subs.iter().filter(|x| !x.anim.gone && x.info.running).count();
    let corner = if info.phase() == Phase::NeedsUser {
        (theme::pulse_at(t) > 0.9).then(|| "!".to_string())
    } else {
        (running > 0).then(|| running.to_string())
    };
    if let Some(c) = corner {
        let w = font::measure(&c, scale_for(CAP_COUNT));
        out.push(text_left(c, CAP_COUNT, left + s - PAD - w, top + PAD + CAP_COUNT / 2.0, ink));
    }

    let name = comb::slots(info, pal, fill)[1].clone();
    let y = top + s - PAD - CAP_SMALL - 7.0 - CAP_NAME / 2.0;
    if let Some(slot) = &name {
        out.extend(row(slot, 1, left + PAD, y, s - 2.0 * PAD, CAP_NAME, ink));
    }
    out.push(text_left(info.phase().label().to_lowercase(), CAP_SMALL, left + PAD, top + s - PAD - CAP_SMALL / 2.0, soft));
    out
}

/// The Linear issue a unit of work is on, as a tile of its own (#162): its
/// key large, its title under it, and where it stands at the bottom. Painted
/// in the issue's state, the way the comb colours its top row.
fn issue_tile(info: &SessionInfo, left: f32, top: f32, pal: Palette, t: f32, fade: &dyn Fn(Rgba) -> Rgba) -> Vec<Shape> {
    let Some(ticket) = &info.ticket else { return Vec::new() };
    let s = side();
    let fill = if pal.screened { pal.ink } else { comb::floor_color(comb::ticket_color(ticket, pal), pal) };
    let ink = comb::legible(WHITE, fill, pal);
    let soft = theme::with_alpha(ink, 0.72);
    // An issue nobody has started on keeps time, as its row does (#90) —
    // on the line that says where it stands, so the rest stays readable.
    let beat = comb::row_beat(info, 0, t);
    let mut out = vec![Shape::poly(square(left, top, s, s), fade(fill))];
    out.push(Shape::Mark {
        mark: crate::logos::Mark::Linear,
        size: 15.0,
        angle: 0.0,
        center: Pt::new(left + PAD + 7.5, top + PAD + 7.5),
        color: fade(ink),
    });
    let key_y = top + PAD + 15.0 + 10.0 + CAP_NAME / 2.0;
    out.push(fade_text(text_left(ticket.label(), CAP_NAME, left + PAD, key_y, ink), fade));
    let mut y = key_y + CAP_NAME / 2.0 + 8.0 + CAP_SMALL / 2.0;
    for line in wrap(&ticket.title, CAP_SMALL, s - 2.0 * PAD, 3) {
        out.push(fade_text(text_left(line, CAP_SMALL, left + PAD, y, soft), fade));
        y += CAP_SMALL + 5.0;
    }
    let state = if ticket.state_name.is_empty() {
        ticket.status.map(|st| st.short().to_lowercase()).unwrap_or_default()
    } else {
        ticket.state_name.to_lowercase()
    };
    let state_ink = theme::with_alpha(ink, beat);
    out.push(fade_text(text_left(state, CAP_SMALL, left + PAD, top + s - PAD - CAP_SMALL / 2.0, state_ink), fade));
    out
}

/// What a session's work produced, as a tile of its own (#162): the pull
/// request painted in its state (#139) — ruled across while it waits on a
/// reviewer (#153) — with its number large and its title under it, or the
/// page it published. Several of them say so in the corner, and the press
/// drops the list the way the comb's bottom row does (#134).
fn output_tile(info: &SessionInfo, left: f32, top: f32, pal: Palette, t: f32, fade: &dyn Fn(Rgba) -> Rgba) -> Vec<Shape> {
    let Some(slot) = comb::slots(info, pal, pal.bg)[2].clone() else { return Vec::new() };
    let s = side();
    // Solid, so the letters on it read: the grain a state is screened with
    // (#149) and the rule of a review still pending (#153) are said in a
    // band across the top rather than over the whole tile. A board that
    // prints has one ink, and the band is all it has to say the state with.
    let fill = if pal.screened { pal.ink } else { slot.fill.unwrap_or(theme::lerp(pal.bg, pal.ink, 0.32)) };
    let mut out = vec![Shape::poly(square(left, top, s, s), fade(fill))];
    let band = square(left, top, s, 6.0);
    if pal.screened && slot.screen != Screen::Solid {
        out.push(Shape::Poly { points: band.clone(), color: fade(pal.bg), screen: slot.screen });
    }
    if slot.waiting {
        let rule = if pal.screened { pal.bg } else { comb::legible(WHITE, fill, pal) };
        out.push(Shape::Poly { points: band, color: fade(rule), screen: comb::PENDING });
    }
    let ink = comb::legible(WHITE, fill, pal);
    let soft = theme::with_alpha(ink, 0.72);
    // A number GitHub would not answer for keeps time (#112), on its state line.
    let beat = if slot.fill.is_some() { 1.0 } else { comb::row_beat(info, 2, t) };

    let more = crate::made::rows(info).len();
    if more > 1 {
        let c = format!("+{}", more - 1);
        let w = font::measure(&c, scale_for(CAP_COUNT));
        out.push(fade_text(text_left(c, CAP_COUNT, left + s - PAD - w, top + PAD + CAP_COUNT / 2.0, ink), fade));
    }
    let label_y = top + PAD + 15.0 + 10.0 + CAP_NAME / 2.0;
    out.extend(row(&slot, 2, left + PAD, label_y, s - 2.0 * PAD, CAP_NAME, ink).into_iter().map(|sh| faded(sh, fade)));
    let (title, state) = match info.prs.last() {
        Some(p) => (p.title.clone(), p.look().short().to_lowercase()),
        None if !info.prs_unanswered.is_empty() => (String::new(), "github did not answer".into()),
        None => (info.artifacts.last().map(|a| a.title.clone()).unwrap_or_default(), "published".into()),
    };
    let state_ink = theme::with_alpha(ink, beat);
    let mut y = label_y + CAP_NAME / 2.0 + 8.0 + CAP_SMALL / 2.0;
    for line in wrap(&title, CAP_SMALL, s - 2.0 * PAD, 3) {
        out.push(fade_text(text_left(line, CAP_SMALL, left + PAD, y, soft), fade));
        y += CAP_SMALL + 5.0;
    }
    out.push(fade_text(text_left(state, CAP_SMALL, left + PAD, top + s - PAD - CAP_SMALL / 2.0, state_ink), fade));
    out
}

fn fade_text(sh: Shape, fade: &dyn Fn(Rgba) -> Rgba) -> Shape {
    faded(sh, fade)
}

/// `text` broken into at most `lines` lines of `room`, at word boundaries,
/// the last one cut with an ellipsis if there is more.
fn wrap(text: &str, cap: f32, room: f32, lines: usize) -> Vec<String> {
    let scale = scale_for(cap);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut k = 0;
    while k < words.len() {
        let next = if line.is_empty() { words[k].to_string() } else { format!("{line} {}", words[k]) };
        if font::measure(&next, scale) <= room || line.is_empty() {
            line = next;
            k += 1;
        } else {
            out.push(std::mem::take(&mut line));
            if out.len() == lines - 1 {
                break;
            }
        }
    }
    if k < words.len() {
        line = if line.is_empty() { words[k..].join(" ") } else { format!("{line} {}", words[k..].join(" ")) };
    }
    if !line.is_empty() {
        out.push(comb::fit(&line, scale, room));
    }
    out.iter_mut().for_each(|l| *l = comb::fit(l, scale, room));
    out
}

/// The back: what the session is up to, in a few short lines, over its name
/// at the bottom-left where Metro keeps a tile's title.
///
/// Waiting, it says what for. Working, it says what its subagents are on,
/// or where it is working when it is on its own. Idle, where it is and how
/// long it has been running.
fn back_face(info: &SessionInfo, chip: &crate::model::ChipState, left: f32, top: f32, ink: Rgba) -> Vec<Shape> {
    let s = side();
    let room = s - 2.0 * PAD;
    let soft = theme::with_alpha(ink, 0.72);
    let fit = |text: &str, cap: f32| comb::fit(text, scale_for(cap), room);
    let mut lines: Vec<(String, f32, Rgba)> = Vec::new();
    let place = info.short_cwd();
    match info.phase() {
        Phase::NeedsUser => {
            lines.push(("needs you".into(), CAP_NAME, ink));
            if let Some(why) = &info.waiting_for {
                lines.push((why.to_lowercase(), CAP_SMALL, soft));
            }
            lines.push((place, CAP_SMALL, soft));
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
                lines.push((place, CAP_NAME, ink));
                lines.push(("on its own".into(), CAP_SMALL, soft));
            } else {
                lines.push((format!("{} at work", busy.len()), CAP_NAME, ink));
                lines.extend(busy.into_iter().take(3).map(|b| (b, CAP_SMALL, soft)));
            }
        }
        Phase::Idle => {
            lines.push((place, CAP_NAME, ink));
            if let Some(up) = running_for(info.started_at) {
                lines.push((format!("up {up}"), CAP_SMALL, soft));
            }
        }
    }
    let mut out = Vec::new();
    let mut y = top + PAD + CAP_NAME / 2.0;
    for (text, cap, color) in lines {
        out.push(text_left(fit(&text, cap), cap, left + PAD, y, color));
        y += cap + if cap > CAP_SMALL { 9.0 } else { 6.0 };
    }
    // The title, bottom-left, the way Metro labels a tile.
    out.push(text_left(fit(&info.label(), CAP_SMALL), CAP_SMALL, left + PAD, top + s - PAD - CAP_SMALL / 2.0, ink));
    out
}

/// How long ago `started` was, short: `40m`, `3h 05m`, `2d`. `started` is
/// whatever the session file said, seconds or milliseconds.
fn running_for(started: u64) -> Option<String> {
    if started == 0 {
        return None;
    }
    let started = if started > 100_000_000_000 { started / 1000 } else { started };
    // Before 2001 is not a start time anybody wrote down; the sandbox's
    // sessions carry a placeholder, and "up 20780d" is nobody's uptime.
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

/// A face moved by `dy` and cut to the band `lo..hi`: what is wholly out of
/// the band goes, a fill is cut at its edges, and a line of letters or a
/// mark that straddles an edge goes as a whole — the slide is quick enough
/// that a word is either on the tile or past it.
fn slid(shapes: Vec<Shape>, dy: f32, lo: f32, hi: f32) -> Vec<Shape> {
    if dy == 0.0 {
        return shapes;
    }
    let inside = |y: f32, half: f32| y - half >= lo && y + half <= hi;
    shapes
        .into_iter()
        .filter_map(|mut sh| {
            match &mut sh {
                Shape::Text { center, scale, .. } => {
                    center.y += dy;
                    inside(center.y, font::height(*scale) / 2.0).then_some(())?;
                }
                Shape::Mark { center, size, .. } => {
                    center.y += dy;
                    inside(center.y, *size / 2.0).then_some(())?;
                }
                Shape::Poly { points, .. } => {
                    for p in points.iter_mut() {
                        p.y = (p.y + dy).clamp(lo, hi);
                    }
                    let (a, b) = points.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
                    (b - a > 0.5).then_some(())?;
                }
                Shape::Stroke { pieces, .. } => {
                    for p in pieces.iter_mut().flatten() {
                        p.y += dy;
                    }
                    pieces.iter().flatten().all(|p| p.y >= lo && p.y <= hi).then_some(())?;
                }
                Shape::Rect { y, h, .. } => {
                    let (a, b) = ((*y + dy).max(lo), (*y + dy + *h).min(hi));
                    (b > a).then_some(())?;
                    *y = a;
                    *h = b - a;
                }
                Shape::RoundedRect { center, h, .. } => {
                    center.y += dy;
                    inside(center.y, *h / 2.0).then_some(())?;
                }
            }
            Some(sh)
        })
        .collect()
}

/// A shape with the tile's arrival fade laid over its colour.
fn faded(mut sh: Shape, fade: &dyn Fn(Rgba) -> Rgba) -> Shape {
    match &mut sh {
        Shape::Text { color, .. }
        | Shape::Mark { color, .. }
        | Shape::Poly { color, .. }
        | Shape::Stroke { color, .. }
        | Shape::Rect { color, .. }
        | Shape::RoundedRect { color, .. } => *color = fade(*color),
    }
    sh
}

/// One of a tile's rows, set from its left edge: the comb's own row
/// drawing (#140), told how much room it has and how big to be.
fn row(slot: &comb::SlotInk, row: usize, left: f32, y: f32, room: f32, cap: f32, color: Rgba) -> Vec<Shape> {
    // The comb sizes a row's letters off the cell's radius; this is the
    // radius whose letters come out at `cap`.
    let per_r = if row == 1 { 0.021 } else { 0.016 };
    let r = scale_for(cap) / per_r;
    let extras = Extras { origin: slot.origin.is_some(), check: slot.check, made: slot.made };
    // `row_box_in` keeps a pad of its own either side; the tile has its own.
    let rb = comb::row_box_in(room + 10.0, r, row, &slot.label, extras);
    let w = comb::row_width(&rb);
    comb::row_shapes(Pt::new(left + w / 2.0, y), &rb, slot, color)
}

/// The block beside a session: its subagents as up to four small tiles,
/// the last one counting whoever did not fit.
fn colony(
    chip: &crate::model::ChipState,
    left: f32,
    top: f32,
    pal: Palette,
    fade: impl Fn(Rgba) -> Rgba,
) -> Vec<Shape> {
    let mut out = Vec::new();
    let agents: Vec<(String, bool)> =
        chip.subs.iter().filter(|x| !x.anim.gone).map(|x| (x.info.label().to_lowercase(), x.info.running)).collect();
    let n = agents.len();
    // Working subagents are working tiles: the chip colour, like their
    // session's (#161).
    let base = if pal.screened { pal.ink } else { pal.chip };
    for (k, (name, running)) in agents.iter().take(4).enumerate() {
        let (q, r) = ((k % 2) as f32, (k / 2) as f32);
        let (x, y) = (left + q * PITCH, top + r * PITCH);
        let over = k == 3 && n > 4;
        let going = over || *running;
        // A finished one is the accent given back to the board.
        let fill = if going { base } else { theme::lerp(pal.bg, base, 0.45) };
        out.push(Shape::poly(square(x, y, TILE, TILE), fade(fill)));
        let ink = comb::legible(WHITE, fill, pal);
        if over {
            let label = format!("+{}", n - 3);
            out.push(text_left(label, CAP_COUNT, x + 7.0, y + TILE / 2.0, fade(ink)));
        } else {
            let label = comb::fit(name, scale_for(CAP_SMALL), TILE - 12.0);
            out.push(text_left(label, CAP_SMALL, x + 6.0, y + TILE - 6.0 - CAP_SMALL / 2.0, fade(ink)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SessionInfo;
    use std::time::Instant;

    fn board(groups: &[(&str, usize)]) -> BoardModel {
        let mut list = Vec::new();
        let mut seq = 0;
        for (g, n) in groups {
            for _ in 0..*n {
                seq += 1;
                let mut s = SessionInfo::synthetic(seq, &format!("S-{seq}"), Phase::Working);
                // A unit of work is a directory when there is no issue.
                s.cwd = format!("/work/{g}");
                list.push(s);
            }
        }
        let mut model = BoardModel::new();
        model.apply(list, Instant::now());
        model.settle();
        model
    }

    #[test]
    fn a_group_wraps_under_its_own_header_and_tiles_never_overlap() {
        let model = board(&[("a", 5), ("b", 2)]);
        let comb = lay_out(&model, 700.0, 600.0);
        assert_eq!(comb.cells.len(), 7);
        let s = side();
        for (i, a) in comb.cells.iter().enumerate() {
            for b in &comb.cells[i + 1..] {
                let apart = (a.center.x - b.center.x).abs() >= s || (a.center.y - b.center.y).abs() >= s;
                assert!(apart, "tiles at {:?} and {:?} overlap", a.center, b.center);
            }
        }
        // Three across at this width: group a wraps onto a second row, and
        // group b starts below both rows and its own header.
        let a_bottom = comb.cells[..5].iter().map(|c| c.center.y).fold(0.0, f32::max);
        let b_top = comb.cells[5..].iter().map(|c| c.center.y).fold(f32::MAX, f32::min);
        assert!(b_top - a_bottom >= s + PITCH - 0.01, "group b must clear group a and a header row");
        assert!(comb.height >= b_top + s / 2.0);
    }

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
        let comb = lay_out(&model, 1100.0, 600.0);
        let Grid::Tiles { headers, .. } = &comb.grid else { panic!() };
        assert_eq!(headers.len(), 2, "one group for the issue, one for the loose session: {headers:?}");
        let first: Vec<(Option<usize>, Option<Slot>)> =
            comb.cells.iter().filter(|c| c.row < comb.cells.last().unwrap().row).map(|c| (c.chip, c.slot)).collect();
        assert_eq!(
            first,
            vec![(Some(0), Some(Slot::Ticket)), (Some(0), None), (Some(1), None), (Some(0), Some(Slot::Output))],
            "the issue once, both sessions, then what they produced"
        );
    }

    #[test]
    fn each_tile_presses_as_what_it_shows() {
        let model = work();
        let comb = lay_out(&model, 1100.0, 600.0);
        let at = |slot: Option<Slot>, chip: usize| {
            comb.cells.iter().find(|c| c.slot == slot && c.chip == Some(chip) && c.subs.is_none()).unwrap().center
        };
        let s = side();
        assert_eq!(comb::hit_slot(&comb, at(Some(Slot::Ticket), 0)), Some((0, Slot::Ticket)));
        assert_eq!(comb::hit_slot(&comb, at(Some(Slot::Output), 0)), Some((0, Slot::Output)));
        // A session tile is the session wherever it is pressed.
        let c = at(None, 1);
        for dy in [-0.45, 0.0, 0.45] {
            assert_eq!(comb::hit_slot(&comb, c + Pt::new(0.0, dy * s)), Some((1, Slot::Session)));
        }
        assert_eq!(comb::hit(&comb, at(Some(Slot::Ticket), 0)), Some(0), "the issue tile is pressable");
        let (box_at, w, _) = comb::slot_box(&comb, 0, Slot::Output).unwrap();
        assert_eq!(box_at, at(Some(Slot::Output), 0), "the drop-down hangs off the output tile");
        assert!((w - s).abs() < 1e-3);
    }

    #[test]
    fn a_title_wraps_by_word_and_never_overruns() {
        let lines = wrap("make the start screen group tiles by the work they belong to", CAP_SMALL, 110.0, 3);
        assert!(!lines.is_empty() && lines.len() <= 3, "{lines:?}");
        assert!(lines.iter().all(|l| font::measure(l, scale_for(CAP_SMALL)) <= 110.0 + 0.5), "{lines:?}");
        assert!(wrap("", CAP_SMALL, 110.0, 3).is_empty());
    }

    #[test]
    fn every_tile_is_live_resting_on_each_face_and_never_in_step() {
        let a = SessionInfo::synthetic(1, "A", Phase::Working);
        let b = SessionInfo::synthetic(2, "B", Phase::Idle);
        let step = 0.02;
        let ks = |info: &SessionInfo| (0..3000).map(|k| peek(info, k as f32 * step)).collect::<Vec<f32>>();
        let (ka, kb) = (ks(&a), ks(&b));
        assert_eq!(ka[0], 0.0, "opens on its front");
        for k in [&ka, &kb] {
            let back = k.iter().filter(|x| **x >= 1.0).count() as f32 / k.len() as f32;
            let front = k.iter().filter(|x| **x <= 0.0).count() as f32 / k.len() as f32;
            assert!(back > 0.3 && front > 0.3, "rests on both faces: back {back}, front {front}");
            let moving = k.iter().filter(|x| **x > 0.0 && **x < 1.0).count() as f32 * step;
            assert!(moving < 60.0 * 2.0 * PEEK_SLIDE / PEEK_PERIOD + 1.0, "slides are quick: {moving}s moving");
        }
        assert!(ka.iter().zip(&kb).any(|(x, y)| (x - y).abs() > 0.5), "two tiles turn on their own clocks");
    }

    #[test]
    fn a_slid_face_is_cut_to_the_tile() {
        let text = Shape::Text { text: "x".into(), scale: 2.0, stroke: 1.0, angle: 0.0, center: Pt::new(0.0, 50.0), color: WHITE };
        let fill = Shape::poly(square(0.0, 0.0, 100.0, 100.0), WHITE);
        let out = slid(vec![text, fill], -60.0, 0.0, 100.0);
        assert_eq!(out.len(), 1, "the line that left the top is gone");
        let Shape::Poly { points, .. } = &out[0] else { panic!() };
        assert!(points.iter().all(|p| p.y >= 0.0 && p.y <= 40.0), "the fill is cut at the edge");
    }
}
