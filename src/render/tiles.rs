//! The board as a Metro start screen (#157).
//!
//! The panels around the board speak Windows Phone Metro (#156); this is the
//! board in the same language. A session is a live tile: a flat square of
//! the accent, its Linear issue across the top, its name in the middle and
//! what the work produced along the bottom — the same three rows, in the
//! same order, as a honeycomb cell (#89), so a column of tiles reads down
//! the way a column of cells does. The sessions of one window sit under a
//! lower-case group header, the way the start screen groups its tiles, and
//! the subagents a session has out split the block next to it into four
//! small tiles.
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
use crate::render::comb::{self, Cell, Comb, Extras, Grid, TILE_BANDS, reveal};
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

/// A working tile peeks once a period, for `PEEK_HOLD` seconds, taking
/// `PEEK_MOVE` to slide up and as long to drop back (#157). Long enough
/// apart that the front, where the rows are pressed, is what is there.
const PEEK_PERIOD: f32 = 13.0;
const PEEK_MOVE: f32 = 0.55;
const PEEK_HOLD: f32 = 2.6;

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

/// Lays the sessions out as a start screen: group by group in board order,
/// a header row and then the group's blocks, left to right and wrapping.
pub fn lay_out(model: &BoardModel, width: f32, _height: f32) -> Comb {
    let across = (((width - 2.0 * MARGIN + GAP) / (2.0 * PITCH)).floor() as usize).clamp(1, MAX_ACROSS);
    let grid_w = across as f32 * 2.0 * PITCH - GAP;
    let origin = Pt::new(((width - grid_w) / 2.0).max(MARGIN.min(width / 8.0)), TITLE_H);

    // Sessions by group, in board order — the comb's grouping, so the two
    // views agree on who belongs with whom (#41).
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, c) in model.chips.iter().enumerate() {
        if c.anim.gone {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == c.info.group) {
            Some((_, members)) => members.push(i),
            None => groups.push((c.info.group.clone(), vec![i])),
        }
    }

    let mut cells = Vec::new();
    let mut headers = Vec::new();
    let mut row = 0i32;
    for (_, members) in &groups {
        let label = model.chips[members[0]].info.group_label().to_lowercase();
        headers.push((origin + Pt::new(0.0, row as f32 * PITCH + TILE * 0.55), label));
        row += 1;
        // A session's block, then its subagents' block right after it.
        let mut blocks: Vec<(Option<usize>, Option<usize>)> = Vec::new();
        for &i in members {
            blocks.push((Some(i), None));
            if model.chips[i].subs.iter().any(|s| !s.anim.gone) {
                blocks.push((None, Some(i)));
            }
        }
        for (k, (chip, subs)) in blocks.iter().enumerate() {
            let q = (k % across) as i32 * 2;
            let r = row + (k / across) as i32 * 2;
            cells.push(Cell { center: Pt::new(0.0, 0.0), q, row: r, chip: *chip, subs: *subs, wave: 0.0 });
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

/// How far up a working tile's back is at `t`, from 0 (all front) to 1
/// (peeked). Every tile keeps its own clock, so a screen of them does not
/// flip in step.
pub fn peek(info: &SessionInfo, t: f32) -> f32 {
    if info.phase() != Phase::Working {
        return 0.0;
    }
    let mut h: u32 = 2_166_136_261;
    for b in info.session_id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    // The first peek waits a whole period, so the view opens on fronts.
    if t < PEEK_PERIOD {
        return 0.0;
    }
    let offset = (h % 1000) as f32 / 1000.0 * PEEK_PERIOD;
    // The peek is the last stretch of each tile's period.
    let at = (t + offset).rem_euclid(PEEK_PERIOD) - (PEEK_PERIOD - (2.0 * PEEK_MOVE + PEEK_HOLD));
    if at < 0.0 {
        return 0.0;
    }
    let out = if at < PEEK_MOVE {
        at / PEEK_MOVE
    } else if at < PEEK_MOVE + PEEK_HOLD {
        1.0
    } else {
        1.0 - (at - PEEK_MOVE - PEEK_HOLD) / PEEK_MOVE
    };
    ease_out(out.clamp(0.0, 1.0))
}

/// Whether any tile is mid-peek, which is when the frame clock has to run
/// at full rate. A tile that is all front or all back is still.
pub fn peeking(model: &BoardModel, t: f32) -> bool {
    model.live().any(|c| {
        let k = peek(&c.info, t);
        k > 0.0 && k < 1.0
    })
}

/// Metro's hard exponential ease-out (gpui-metro `ease_out_expo`).
fn ease_out(x: f32) -> f32 {
    if x >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * x) }
}

/// What a session's tile is painted with: the accent while it works, the
/// board's own chip colour while it waits on a person, and the board's grey
/// while it idles.
///
/// Waiting is not the alarm colour: the accent is the user's to pick and the
/// default one is orange, a step from the alarm's red, so a screen of
/// working tiles and one waiting tile came out the same colour. The chip
/// colour is the opposite of the board, whatever the accent, and the alarm
/// goes on as a bar across its top instead.
pub fn tile_fill(info: &SessionInfo, pal: Palette, accent: Rgba) -> Rgba {
    if comb::wants_a_person(info) {
        return pal.chip;
    }
    match info.phase() {
        Phase::Working => accent,
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
                out.extend(colony(chip, left, top, pal, accent, fade));
            }
            continue;
        }
        let Some(chip) = cell.chip.and_then(|i| model.chips.get(i)) else { continue };
        out.extend(session_tile(&chip.info, chip, left, top, pal, accent, t, &fade));
    }
    out
}

/// One session as a medium live tile.
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
    let mut out = Vec::new();
    let s = side();
    let fill = tile_fill(info, pal, accent);
    // A board that prints has one ink (#146): every tile is that ink, and
    // the state is said by the weave of a band across its top instead.
    let (fill, band) = if pal.screened { (pal.ink, Some(comb::body_screen(info))) } else { (fill, None) };
    out.push(Shape::poly(square(left, top, s, s), fade(fill)));
    if let Some(screen) = band.filter(|b| *b != Screen::Solid) {
        out.push(Shape::Poly { points: square(left, top, s, 5.0), color: fade(pal.bg), screen });
    }
    // White on the accent, which is what Metro sets on it whatever the
    // accent is; everything else is worked out against its own fill.
    let ink = if fill == accent && !pal.screened { WHITE } else { comb::legible(WHITE, fill, pal) };
    let soft = theme::with_alpha(ink, 0.72);
    // A session waiting on a person says so in the alarm colour, across the
    // top of the tile and in the corner, keeping time with the wait (#90).
    let waits = comb::wants_a_person(info) && !pal.screened;
    let alarm = comb::legible(pal.alarm, fill, pal);
    if waits {
        let beat = if info.phase() == Phase::NeedsUser { theme::pulse_at(t) } else { 1.0 };
        out.push(Shape::poly(square(left, top, s, 5.0), fade(theme::with_alpha(alarm, beat))));
    }
    let lift = peek(info, t);
    // Where the back has come up to; the front's rows below it are behind it.
    let back_top = top + s - lift * s * (1.0 - TILE_BANDS[0]);
    let shown = |y: f32, cap: f32| lift <= 0.0 || y + cap / 2.0 < back_top;

    let slots = comb::slots(info, pal, fill);
    let band_mid = |from: f32, to: f32| top + (from + to) / 2.0 * s;

    // Top: the issue, and the count Metro puts in a tile's corner — the
    // subagents running — or a `!` keeping time with a session that waits.
    let running = chip.subs.iter().filter(|x| !x.anim.gone && x.info.running).count();
    let corner = if info.phase() == Phase::NeedsUser {
        (theme::pulse_at(t) > 0.9).then(|| "!".to_string())
    } else {
        (running > 0).then(|| running.to_string())
    };
    let corner_w = corner.as_ref().map_or(0.0, |c| font::measure(c, scale_for(CAP_COUNT)) + 6.0);
    let y0 = band_mid(0.0, TILE_BANDS[0]);
    if let Some(c) = corner {
        let w = font::measure(&c, scale_for(CAP_COUNT));
        let color = if waits { alarm } else { ink };
        out.push(text_left(c, CAP_COUNT, left + s - PAD - w, y0, fade(color)));
    }
    if let Some(slot) = &slots[0] {
        out.extend(row(slot, 0, left + PAD, y0, s - 2.0 * PAD - corner_w, CAP_SMALL, fade(soft)));
    }

    // Middle: the name, and under it what the session is doing.
    let y1 = band_mid(TILE_BANDS[0], TILE_BANDS[1]) - CAP_NAME * 0.45;
    if let Some(slot) = &slots[1]
        && shown(y1, CAP_NAME)
    {
        out.extend(row(slot, 1, left + PAD, y1, s - 2.0 * PAD, CAP_NAME, fade(ink)));
    }
    let doing = info.phase().label().to_lowercase();
    let y1b = y1 + CAP_NAME * 1.25;
    if shown(y1b, CAP_SMALL) {
        out.push(text_left(doing, CAP_SMALL, left + PAD, y1b, fade(soft)));
    }

    // Bottom: what the work produced, on its state as a floor (#139).
    if let Some(slot) = &slots[2] {
        let y2 = band_mid(TILE_BANDS[1], 1.0);
        let floor_top = top + TILE_BANDS[1] * s + 4.0;
        if let Some(floor) = slot.fill {
            let pts = square(left, floor_top, s, top + s - floor_top);
            out.push(Shape::Poly { points: pts.clone(), color: fade(floor), screen: slot.screen });
            if slot.waiting {
                out.push(Shape::Poly { points: pts, color: fade(comb::pending_rule(pal)), screen: comb::PENDING });
            }
        }
        if shown(y2, CAP_SMALL) {
            let on = slot.fill.unwrap_or(fill);
            let color = if slot.fill.is_some() { comb::legible(slot.color, on, pal) } else { slot.color };
            let beat = if slot.fill.is_some() { 1.0 } else { comb::row_beat(info, 2, t) };
            out.extend(row(slot, 2, left + PAD, y2, s - 2.0 * PAD, CAP_SMALL, fade(theme::with_alpha(color, beat))));
        }
    }

    // The back: where the session is running, and how many are helping.
    if lift > 0.0 {
        let back = theme::lerp(fill, pal.ink, 0.28);
        let back_ink = comb::legible(WHITE, back, pal);
        out.push(Shape::poly(square(left, back_top, s, top + s - back_top), fade(back)));
        let lines = [
            (comb::fit(&info.short_cwd(), scale_for(CAP_NAME), s - 2.0 * PAD), CAP_NAME, 1.0),
            (
                match chip.subs.iter().filter(|x| !x.anim.gone).count() {
                    0 => "on its own".to_string(),
                    1 => "1 subagent".to_string(),
                    n => format!("{n} subagents"),
                },
                CAP_SMALL,
                0.72,
            ),
        ];
        let mut y = back_top + PAD + CAP_NAME / 2.0;
        for (text, cap, a) in lines {
            if y + cap / 2.0 <= top + s - 2.0 {
                out.push(text_left(text, cap, left + PAD, y, fade(theme::with_alpha(back_ink, a))));
            }
            y += cap + 8.0;
        }
    }
    out
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
    accent: Rgba,
    fade: impl Fn(Rgba) -> Rgba,
) -> Vec<Shape> {
    let mut out = Vec::new();
    let agents: Vec<(String, bool)> =
        chip.subs.iter().filter(|x| !x.anim.gone).map(|x| (x.info.label().to_lowercase(), x.info.running)).collect();
    let n = agents.len();
    let base = if pal.screened { pal.ink } else { accent };
    for (k, (name, running)) in agents.iter().take(4).enumerate() {
        let (q, r) = ((k % 2) as f32, (k / 2) as f32);
        let (x, y) = (left + q * PITCH, top + r * PITCH);
        let over = k == 3 && n > 4;
        let going = over || *running;
        // A finished one is the accent given back to the board.
        let fill = if going { base } else { theme::lerp(pal.bg, base, 0.45) };
        out.push(Shape::poly(square(x, y, TILE, TILE), fade(fill)));
        let ink = if going && !pal.screened { WHITE } else { comb::legible(WHITE, fill, pal) };
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
                s.group = format!("sandbox:{g}");
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

    #[test]
    fn a_press_lands_on_the_tile_and_its_row() {
        let model = board(&[("a", 1)]);
        let comb = lay_out(&model, 700.0, 600.0);
        let c = comb.cells[0].center;
        let s = side();
        assert_eq!(comb::hit(&comb, c), Some(0));
        assert_eq!(comb::hit(&comb, c + Pt::new(s, 0.0)), None);
        let top = c.y - s / 2.0;
        assert_eq!(comb::hit_slot(&comb, Pt::new(c.x, top + 5.0)), Some((0, comb::Slot::Ticket)));
        assert_eq!(comb::hit_slot(&comb, c), Some((0, comb::Slot::Session)));
        assert_eq!(comb::hit_slot(&comb, Pt::new(c.x, top + s - 5.0)), Some((0, comb::Slot::Output)));
        let (at, w, h) = comb::slot_box(&comb, 0, comb::Slot::Output).unwrap();
        assert!((w - s).abs() < 1e-3 && (at.y + h / 2.0 - (top + s)).abs() < 1e-3);
    }

    #[test]
    fn a_working_tile_peeks_now_and_then_and_an_idle_one_never_does() {
        let working = SessionInfo::synthetic(1, "W", Phase::Working);
        let idle = SessionInfo::synthetic(2, "I", Phase::Idle);
        let samples: Vec<f32> = (0..2000).map(|k| peek(&working, k as f32 * 0.02)).collect();
        assert!(samples.iter().take((PEEK_PERIOD / 0.02) as usize - 1).all(|k| *k == 0.0), "opens on its front");
        assert!(samples.iter().any(|k| *k > 0.99), "peeks all the way");
        let up = samples.iter().filter(|k| **k > 0.5).count() as f32 / samples.len() as f32;
        assert!(up < 0.35, "mostly front: back for {up}");
        assert!((0..2000).all(|k| peek(&idle, k as f32 * 0.02) == 0.0));
    }
}
