//! The honeycomb view (#86): the same sessions as the board, laid out as a
//! vertically scrolling hex grid instead of chips on traces.
//!
//! The grid is the whole surface. Every cell is drawn, and the ones nobody
//! sits in are drawn grey, so the shape of what is running reads against the
//! shape of what is not. Cells carry a thin border and nothing else, and each
//! one is inset by half a border so the gap between two neighbours comes out
//! exactly one border wide.
//!
//! Panes of one tab are a cluster: a flower of cells around a centre,
//! placed far enough from the next flower that a ring of empty cells always
//! separates them. Inside a cell the three things a session has are stacked in
//! a fixed order — the Linear issue on top, the session itself in the middle,
//! its pull request at the bottom — so the eye can read down a column of cells
//! and compare like with like. Subagents hang underneath as a row of small
//! hexes, a sub-grid of the cell that spawned them.

use crate::font;
use crate::geom::Pt;
use crate::logos;
use crate::model::{BoardModel, Phase, SessionInfo};
use crate::render::scene::{self, Shape};
use crate::theme::{self, Palette, Rgba};

/// Every hex is a hairline.
pub const BORDER: f32 = 1.4;
/// Edge to edge between two neighbours. A gap the width of the border, which
/// is what this started as, is invisible at the size a cell is actually drawn:
/// the grid read as one continuous lattice rather than as cells standing
/// apart (#87).
pub const GAP: f32 = 7.0;
/// The mini hexes of a subagent sub-grid sit closer, being smaller.
const SUB_GAP: f32 = 2.4;

/// A mini hex's label against its own radius, and the room kept clear of its
/// wall. Smaller than a cell's own rows: these are names nobody reads across
/// the room, and the smaller they are set the more of each one survives the
/// cut (#98).
const SUB_TEXT: f32 = 0.032;
const SUB_PAD: f32 = 4.0;

/// What one mini hex says, cut to what it can hold.
pub fn sub_box(mr: f32, label: &str) -> (String, f32) {
    let scale = mr * SUB_TEXT;
    let room = room_at(mr, 0.0, font::height(scale), SUB_GAP) - SUB_PAD * 2.0;
    (fit(label, scale, room), scale)
}
/// Roughly how many cells span the width. Fewer means bigger cells with room
/// for longer names; more means the whole board at a glance.
const COLS: f32 = 7.0;
const R_MIN: f32 = 46.0;
const R_MAX: f32 = 104.0;
/// Cells kept above the first flower and below the last.
const PAD_ROWS: f32 = 1.0;
/// Contrast every colour in a cell has to clear against the board behind it.
const READABLE: f32 = 3.0;

/// A single hex of the grid.
#[derive(Clone, Debug)]
pub struct Cell {
    pub center: Pt,
    /// Its axial coordinates. The grid is built from these, so carrying them
    /// is free, and anything that wants to ask whether two cells touch can
    /// ask it of the grid rather than of the pixels (#121).
    pub q: i32,
    pub row: i32,
    /// Index into `model.chips`, or `None` for a cell nobody sits in.
    pub chip: Option<usize>,
    /// Set when this cell holds the subagents of a session next door, which
    /// is where they live rather than spilling out of their parent's hex
    /// (#88).
    pub subs: Option<usize>,
    /// How far this cell is from the nearest cluster, in cells. The view
    /// opens by spreading out from the clusters, and this is each cell's
    /// place in that wave (#92).
    pub wave: f32,
}

/// The laid-out grid.
#[derive(Clone, Debug)]
pub struct Comb {
    pub cells: Vec<Cell>,
    /// Circumradius of a cell: centre to corner.
    pub r: f32,
    /// How far down the grid reaches, for the scroll.
    pub height: f32,
}

/// Pointy-top axial coordinates to a centre point.
fn axial(q: i32, row: i32, r: f32) -> Pt {
    let sqrt3 = 3.0f32.sqrt();
    Pt::new(r * sqrt3 * (q as f32 + row as f32 / 2.0), r * 1.5 * row as f32)
}


/// How far out a subagent cell may be looked for. One ring is what it should
/// be; more is for a cell hemmed in by its own cluster (#108).
const SUB_SEARCH: i32 = 4;

/// Whether a point is inside a hex. The half-plane test over the hexagon's own
/// corners, which is what a sub-grid has to satisfy to stay in its cell (#88).
pub fn inside_hex(p: Pt, center: Pt, r: f32, gap: f32) -> bool {
    let pts = corners(center, r, gap);
    pts.iter().enumerate().all(|(i, a)| {
        let b = pts[(i + 1) % pts.len()];
        let (e, v) = (b - *a, p - *a);
        e.x * v.y - e.y * v.x >= -1e-3
    })
}

/// The six corners of a pointy-top hex, pulled in so two neighbours leave
/// `gap` between their edges.
pub fn corners(center: Pt, r: f32, gap: f32) -> Vec<Pt> {
    // Each of the two gives up half the gap, and insetting a regular
    // hexagon's edges by `d` pulls its circumradius in by `2d / sqrt(3)`.
    let r = (r - gap / 3.0f32.sqrt()).max(1.0);
    (0..6)
        .map(|i| {
            let a = std::f32::consts::PI / 180.0 * (60.0 * i as f32 - 90.0);
            Pt::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect()
}

/// The ring of hexes `k` steps out from the origin, starting due east and
/// walking anticlockwise. Ring 0 is the centre itself.
fn ring(k: i32) -> Vec<(i32, i32)> {
    if k == 0 {
        return vec![(0, 0)];
    }
    const DIRS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
    let mut out = Vec::new();
    let (mut q, mut r) = (DIRS[4].0 * k, DIRS[4].1 * k);
    for dir in DIRS {
        for _ in 0..k {
            out.push((q, r));
            q += dir.0;
            r += dir.1;
        }
    }
    out
}

/// Cells of a flower big enough to seat `n`, nearest the centre first.
fn flower(n: usize) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut k = 0;
    while out.len() < n.max(1) {
        out.extend(ring(k));
        k += 1;
    }
    out.truncate(n.max(1));
    out
}

/// How many rings a flower of `n` needs.
fn rings(n: usize) -> i32 {
    let mut k = 0;
    let mut seats = 1;
    while seats < n.max(1) {
        k += 1;
        seats += 6 * k as usize;
    }
    k
}

/// A whole flower's worth of seats, somewhere it fits.
///
/// Scanned in reading order from the top left, so the first answer is also
/// the tightest packing. `taken` is everything already seated; a candidate is
/// refused when any of its seats lands on one of those or beside one, which
/// is what keeps a rim of empty cells between clusters (#121).
fn seat_somewhere(
    members: &[usize],
    taken: &std::collections::HashSet<(i32, i32)>,
    width: f32,
    r: f32,
) -> Option<Vec<((i32, i32), usize)>> {
    let sqrt3 = 3.0f32.sqrt();
    let half = r * sqrt3 / 2.0;
    // The flower's shape does not depend on where it is put — `seat_cluster`
    // works in offsets from its centre — so it is worked out once and then
    // carried across the scan. Rebuilding it at every candidate was the whole
    // cost of this: a dozen clusters of seven took 5ms a frame.
    let shape = seat_cluster(0, 0, members);
    // Far enough to hold any flower the board can throw at it, plus the rows
    // a tall board can show; the scan stops at the first fit long before.
    let rows = (members.len() as i32 + 2) * 4 + 64;
    let cols = (width / (r * sqrt3)).ceil() as i32 + 2;
    for row in 0..rows {
        // A row is offset by half a cell for every row down, so the column
        // that sits at the left edge moves with it.
        let first = -(row as f32 / 2.0).floor() as i32 - 1;
        for q in first..=first + cols {
            let fits = shape.iter().all(|((dq, drow), _)| {
                let cell = (dq + q, drow + row);
                let at = axial(cell.0, cell.1, r);
                at.x - half >= 0.0
                    && at.x + half <= width
                    && at.y - r >= 0.0
                    && !taken.contains(&cell)
                    // The rim: a seat beside somebody else's is what makes
                    // two clusters read as one (#121).
                    && !DIRS.iter().any(|(nq, nrow)| taken.contains(&(cell.0 + nq, cell.1 + nrow)))
            });
            if fits {
                return Some(shape.iter().map(|((dq, drow), m)| ((dq + q, drow + row), *m)).collect());
            }
        }
    }
    None
}

/// The six cells sharing a wall with a cell.
const DIRS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];

/// Where a cluster's sessions sit around `(cq, crow)`.
///
/// The flower, walked outwards, but a seat is passed over when taking it
/// would leave somebody with nowhere to put their subagents: every session
/// keeps a neighbouring cell free for them (#109). Six around one fills the
/// ring and walls the middle in, so the seventh goes a step further out
/// rather than closing the last door.
fn seat_cluster(cq: i32, crow: i32, members: &[usize]) -> Vec<((i32, i32), usize)> {
    let mut out: Vec<((i32, i32), usize)> = Vec::new();
    let mut seats: Vec<(i32, i32)> = Vec::new();
    let mut k = 0;
    let mut pool: Vec<(i32, i32)> = ring(0);
    for member in members {
        loop {
            if pool.is_empty() {
                k += 1;
                pool = ring(k);
                pool.sort_by_key(|(q, row)| (-row, *q));
            }
            let seat = (cq + pool[0].0, crow + pool[0].1);
            pool.remove(0);
            let mut with = seats.clone();
            with.push(seat);
            // Everyone, the newcomer included, must still have somewhere to
            // put a subagent.
            if with.iter().all(|s| has_free_neighbour(*s, &with)) {
                seats.push(seat);
                out.push((seat, *member));
                break;
            }
            // Nowhere left to try: seat it anyway rather than lose it.
            if k > 4 {
                seats.push(seat);
                out.push((seat, *member));
                break;
            }
        }
    }
    out
}

/// Whether a cell is somewhere the grid will actually draw it: inside the
/// width, and not above the first row (#111).
fn on_screen((q, row): (i32, i32), r: f32, width: f32) -> bool {
    let sqrt3 = 3.0f32.sqrt();
    let at = axial(q, row, r);
    at.x - r * sqrt3 / 2.0 >= -0.5 && at.x + r * sqrt3 / 2.0 <= width + 0.5 && at.y - r >= -0.5
}

fn has_free_neighbour(at: (i32, i32), taken: &[(i32, i32)]) -> bool {
    ring(1).into_iter().any(|(dq, dr)| !taken.contains(&(at.0 + dq, at.1 + dr)))
}

/// Lays the sessions out as clusters on a grid that fills `width`.
///
/// Sessions are grouped the way the board groups them (#41): panes of one
/// tab land in one flower. Flowers are placed left to right and then down,
/// each leaving a ring of empty cells around it.
pub fn lay_out(model: &BoardModel, width: f32, height: f32) -> Comb {
    let sqrt3 = 3.0f32.sqrt();
    let r = (width / (COLS * sqrt3)).clamp(R_MIN, R_MAX);

    // Sessions by group, in board order.
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

    // Where each session sits.
    //
    // The grid is asked directly rather than stepped through in pixels. It
    // used to be the other way round: a flower was placed at a running `x`,
    // snapped to the nearest cell, then shoved back by whole columns when the
    // snap put it over an edge. Every one of those corrections moves seats
    // after the step that was meant to space them, and `seat_cluster` moves
    // them again on its own (#109). On a narrow board the steps are small and
    // the corrections are not, and two groups ended up sharing a wall, which
    // reads as one larger cluster rather than two (#121).
    //
    // So each flower takes the first cell, in reading order, where the whole
    // of it fits on the board and nothing of it touches a flower already
    // placed. Taking the first one packs them left to right and then down on
    // its own, which is the arrangement the pixel walk was after.
    let mut seats: Vec<((i32, i32), usize)> = Vec::new();
    let mut taken: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    for (_, members) in &groups {
        let Some(placed) = seat_somewhere(members, &taken, width, r) else { continue };
        taken.extend(placed.iter().map(|(at, _)| *at));
        seats.extend(placed);
    }
    // Subagents take a cell of their own, next door to the session that
    // spawned them (#88).
    let mut sub_seats: Vec<((i32, i32), usize)> = Vec::new();
    for (at, chip) in &seats {
        let has_subs = model.chips.get(*chip).is_some_and(|c| c.subs.iter().any(|s| !s.anim.gone));
        if !has_subs {
            continue;
        }
        let taken = |n: &(i32, i32)| {
            seats.iter().any(|(s, _)| s == n) || sub_seats.iter().any(|(s, _)| s == n)
        };
        // The nearest free cell, not merely an adjacent one: the middle of a
        // full flower has six neighbours and a session in every one of them,
        // and its subagents were being dropped on the floor (#108).
        let free = (1..=SUB_SEARCH)
            .flat_map(|k| {
                let mut out = ring(k);
                // Downwards first, where the eye goes after the cell itself.
                out.sort_by_key(|(q, row)| (-row, *q));
                out
            })
            .map(|(dq, dr)| (at.0 + dq, at.1 + dr))
            // And on screen: a cell off the side or above the first row is
            // not drawn at all, which loses the subagents as surely as
            // finding nowhere did (#111). Downwards is unbounded, the view
            // scrolling that way.
            .find(|n| !taken(n) && on_screen(*n, r, width));
        if let Some(n) = free {
            sub_seats.push((n, *chip));
        }
    }
    let last_row = seats
        .iter()
        .chain(sub_seats.iter())
        .map(|((_, row), _)| *row)
        .max()
        .unwrap_or(0)
        + 2;

    // Every cell of the grid, occupied or not: the empties are half the point.
    let mut cells = Vec::new();
    let rows = ((height / (r * 1.5)).ceil() as i32).max(last_row + 2);
    for row in -(PAD_ROWS as i32)..=rows {
        let mut q = -(row / 2) - 1;
        loop {
            let center = axial(q, row, r);
            if center.x - r * sqrt3 / 2.0 > width {
                break;
            }
            if center.x + r * sqrt3 / 2.0 >= 0.0 {
                let chip = seats.iter().find(|(at, _)| *at == (q, row)).map(|(_, i)| *i);
                let subs = sub_seats.iter().find(|(at, _)| *at == (q, row)).map(|(_, i)| *i);
                cells.push(Cell { center, q, row, chip, subs, wave: 0.0 });
            }
            q += 1;
        }
    }
    // How far each cell is from the nearest session, in cells. The clusters
    // are where the view starts and everything else follows outwards (#92).
    let hubs: Vec<Pt> = cells.iter().filter(|c| c.chip.is_some()).map(|c| c.center).collect();
    let pitch = r * sqrt3;
    for cell in &mut cells {
        cell.wave = hubs
            .iter()
            .map(|h| ((cell.center.x - h.x).powi(2) + (cell.center.y - h.y).powi(2)).sqrt() / pitch)
            .fold(f32::MAX, f32::min);
        if !cell.wave.is_finite() {
            cell.wave = 0.0;
        }
    }

    let height = cells.iter().map(|c| c.center.y).fold(0.0f32, f32::max) + r;
    Comb { cells, r, height }
}

/// The session under a point, in the grid's own coordinates.
pub fn hit(comb: &Comb, p: Pt) -> Option<usize> {
    comb.cells
        .iter()
        .filter(|c| c.chip.is_some())
        .find(|c| {
            let d = p - c.center;
            // Close enough: the inscribed circle, which never reaches into a
            // neighbour.
            d.x * d.x + d.y * d.y <= (comb.r * 0.87) * (comb.r * 0.87)
        })
        .and_then(|c| c.chip)
}

/// The three things a cell holds, top to bottom. Each is its own target
/// (#89).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Ticket,
    Session,
    Pr,
}

/// The band the session's row is bedded on: the full width of the cell's
/// interior at that height (#91).
///
/// The middle row sits at the hexagon's widest point, where its sides run
/// straight up and down, so a band no taller than the cell's radius can span
/// the whole interior and still touch nothing but those two sides.
pub fn session_bed(center: Pt, r: f32) -> (Pt, f32, f32) {
    let sqrt3 = 3.0f32.sqrt();
    let inner = r - GAP / sqrt3;
    (center, sqrt3 * inner - BORDER * 2.0, r * BED_H)
}

/// How tall that band is, against the cell's radius. Kept under a half, which
/// is where the hexagon starts tapering towards its points.
const BED_H: f32 = 0.3;

/// Between a row's mark and its label.
const MARK_GAP: f32 = 4.0;
/// Kept clear inside the cell wall, so a label never touches its own border.
const ROW_PAD: f32 = 5.0;

/// A row of a cell, worked out: where it sits, how big it is drawn, and how
/// much of its label fits without leaving the hexagon (#94).
pub struct RowBox {
    pub dy: f32,
    pub scale: f32,
    pub mark: f32,
    /// How wide the origin mark is drawn, or zero where the row carries none
    /// (#122). Like the tick, it comes out of the label's room.
    pub origin: f32,
    pub label: String,
    /// How wide the approval tick is drawn, or zero where there is none
    /// (#116). It comes out of the label's room like the mark does.
    pub check: f32,
    /// What the row had to play with, mark and label together.
    pub room: f32,
}

pub fn row_box(r: f32, row: usize, label: &str, extras: Extras) -> RowBox {
    let dy = ROW_DY[row] * r;
    let scale = if row == 1 { r * 0.021 } else { r * 0.016 };
    let mark = if row == 1 { r * 0.17 } else { r * 0.13 };
    let room = row_room(r, dy, font::height(scale).max(mark)) - ROW_PAD * 2.0;
    let check = if extras.check { font::height(scale) * CHECK_H } else { 0.0 };
    let origin = if extras.origin { mark * crate::render::scene::ORIGIN_SIZE } else { 0.0 };
    let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
    let taken = mark + MARK_GAP + span(origin) + span(check);
    RowBox { dy, scale, mark, origin, label: fit(label, scale, room - taken), check, room }
}

/// What a row carries besides its own mark and label.
#[derive(Clone, Copy, Default)]
pub struct Extras {
    /// A second mark saying where the session is running (#122).
    pub origin: bool,
    /// Review has signed off on this row's pull request (#116).
    pub check: bool,
}

/// The approval tick, against the cap height of the row it sits in. A hair
/// under, so it reads as a mark beside the number rather than as a letter of
/// it. (#116)
const CHECK_H: f32 = 0.95;

/// How wide the cell's interior is at a row, taking the row's own height into
/// account: a hexagon narrows towards its points, so what matters is the
/// width at the row's far edge rather than at its middle.
fn row_room(r: f32, dy: f32, height: f32) -> f32 {
    room_at(r, dy, height, GAP)
}

/// Width available inside a hex of radius `r` at height `dy`, for something
/// `height` tall. Shared with the mini hexes of a sub-grid, which are drawn to
/// a gap of their own (#94, #95).
fn room_at(r: f32, dy: f32, height: f32, gap: f32) -> f32 {
    let sqrt3 = 3.0f32.sqrt();
    let inner = r - gap / sqrt3;
    let y = dy.abs() + height / 2.0;
    let half = if y <= inner / 2.0 { sqrt3 / 2.0 * inner } else { sqrt3 * (inner - y).max(0.0) };
    2.0 * half
}

/// `label` cut to `room`, with an ellipsis where it was cut. Nothing that
/// cannot show at least one letter shows at all.
fn fit(label: &str, scale: f32, room: f32) -> String {
    if room <= 0.0 {
        return String::new();
    }
    if font::measure(label, scale) <= room {
        return label.to_string();
    }
    let mut kept = String::new();
    for ch in label.chars() {
        let mut next = kept.clone();
        next.push(ch);
        next.push('…');
        if font::measure(&next, scale) > room {
            break;
        }
        kept.push(ch);
    }
    if kept.is_empty() { String::new() } else { kept + "…" }
}

/// Where in a cell each row sits, as a fraction of the cell's radius.
const ROW_DY: [f32; 3] = [-0.42, 0.0, 0.42];

/// The row of a cell under a point, and whose cell it is. The caller knows
/// whether that row has anything in it.
pub fn hit_slot(comb: &Comb, p: Pt) -> Option<(usize, Slot)> {
    let cell = comb
        .cells
        .iter()
        .find(|c| c.chip.is_some() && inside_hex(p, c.center, comb.r, GAP))?;
    let dy = (p.y - cell.center.y) / comb.r;
    let slot = if dy < (ROW_DY[0] + ROW_DY[1]) / 2.0 {
        Slot::Ticket
    } else if dy > (ROW_DY[1] + ROW_DY[2]) / 2.0 {
        Slot::Pr
    } else {
        Slot::Session
    };
    Some((cell.chip?, slot))
}

/// What goes on one of a cell's three lines.
struct SlotInk {
    mark: logos::Mark,
    label: String,
    color: Rgba,
    /// Review has signed off on this pull request (#116). Only the GitHub
    /// row ever sets it.
    check: bool,
    /// Where the session is running (#122). Only the middle row sets it:
    /// the issue and the pull request are not anywhere in particular.
    origin: Option<logos::Mark>,
}

/// How long each ring of cells waits before it starts appearing, and how long
/// one takes to arrive once it does (#92).
const WAVE_STEP: f32 = 0.07;
const WAVE_FADE: f32 = 0.5;

/// When the last cell of a grid this wide can still be arriving (#126). The
/// furthest cell waits `wave` steps and then fades, and `wave` is bounded by
/// the grid, so this is generous rather than exact — the clock only has to
/// not stop early.
pub fn wave_over(comb: &Comb, t: f32) -> bool {
    let far = comb.cells.iter().map(|c| c.wave).fold(0.0f32, f32::max);
    t > far * WAVE_STEP + WAVE_FADE
}

/// How faint an empty cell is before the wave reaches it. Not nothing: the
/// grid is the furniture of this view and is there whether anything is
/// running or not (#102). What the wave does is bring it up.
const GRID_FLOOR: f32 = 0.55;

/// How far into its arrival a cell is at `t` seconds: 0 before its turn, 1
/// once it is all the way in. An empty cell never goes below the floor, so
/// the grid stands even on a board with nothing on it.
pub fn reveal(cell: &Cell, t: f32) -> f32 {
    let arrived = ((t - cell.wave * WAVE_STEP) / WAVE_FADE).clamp(0.0, 1.0);
    if cell.chip.is_none() && cell.subs.is_none() { arrived.max(GRID_FLOOR) } else { arrived }
}

/// Every shape of the honeycomb view, in design units. `t` is seconds since
/// the view opened, which is what the cells arrive on.
pub fn build_shapes(model: &BoardModel, comb: &Comb, pal: Palette, scroll_y: f32, origin: Pt, t: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    let r = comb.r;
    out.push(Shape::Rect { x: 0.0, y: 0.0, w: 1.0e5, h: 1.0e5, color: pal.bg });

    for cell in &comb.cells {
        let center = cell.center - Pt::new(0.0, scroll_y) + origin;
        if center.y < -r * 2.0 || center.y > 1.0e4 {
            continue;
        }
        let arrived = reveal(cell, t);
        // The grid is complete from the first frame: a cell whose contents
        // have not arrived still stands as an empty one, so the wave fills
        // holes in rather than punching them (#102).
        if arrived < 1.0 {
            out.push(Shape::Stroke {
                pieces: vec![closed(corners(center, r, GAP))],
                width: BORDER,
                color: theme::with_alpha(pal.ink, 0.18 * GRID_FLOOR),
            });
        }
        if arrived <= 0.001 {
            continue;
        }
        let fade = |c: Rgba| theme::with_alpha(c, arrived);
        if let Some(parent) = cell.subs {
            let Some(chip) = model.chips.get(parent) else { continue };
            let agents: Vec<(String, bool)> = chip
                .subs
                .iter()
                .filter(|s| !s.anim.gone)
                .map(|s| (s.info.label(), s.info.running))
                .collect();
            if agents.is_empty() {
                continue;
            }
            // No cell drawn around them: the subagents are the only thing
            // there, and a hex around a hex reads as a container nobody asked
            // for (#97). The cell still governs where they may go.

            for ((at, mr), (label, going)) in sub_grid(center, r, agents.len()).into_iter().zip(&agents) {
                // Solid, not an outline: a subagent is a thing doing work,
                // and the cell around it is the empty part (#96). It wears
                // the board's chip colour, which is what a working chip
                // wears out there — black on the white board, white on the
                // dark one — and a finished one is the same colour given
                // back to the board (#110).
                let color = if *going { pal.chip } else { theme::lerp(pal.bg, pal.chip, 0.45) };
                out.push(Shape::Poly { points: corners(at, mr, SUB_GAP), color: fade(color) });
                let (text, scale) = sub_box(mr, label);
                if !text.is_empty() {
                    out.push(Shape::Text {
                        text,
                        scale,
                        stroke: 1.0,
                        angle: 0.0,
                        center: at,
                        color: fade(pal.bg),
                    });
                }
            }
            continue;
        }
        let Some(i) = cell.chip else {
            // An empty cell says so quietly: the grid stays visible, the eye
            // goes to the ones with something in them.
            out.push(Shape::Stroke {
                pieces: vec![closed(corners(center, r, GAP))],
                width: BORDER,
                color: fade(theme::with_alpha(pal.ink, 0.18)),
            });
            continue;
        };
        let Some(chip) = model.chips.get(i) else { continue };
        let info = &chip.info;

        // Everything here is drawn straight onto the board, so every colour
        // is put through the floor first (#86).
        let read = |c: Rgba| theme::readable(c, pal.bg, pal.ink, READABLE);
        // The border speaks for the whole cell: it goes to the alarm colour
        // when anything in there is waiting on a person, which is the session
        // asking outright or a pull request whose checks went red. An issue
        // merely sitting in Todo is not an interruption and does not count.
        // (#90)
        let ticket = info.ticket.as_ref();
        let pr = info.prs.last();
        // GitHub is the problem when it will not answer for a pull request
        // the transcript named: the board knows the number and nothing else
        // (#112).
        let unanswered = info.prs_unanswered.last();
        // A row blinks when that row is the one waiting.
        let row_beat = |row: usize| {
            let waiting = match row {
                0 => ticket.is_some_and(ticket_needs_action),
                2 => pr.is_some_and(pr_needs_action) || unanswered.is_some(),
                _ => false,
            };
            if waiting { theme::pulse_at(t) } else { 1.0 }
        };
        let needs = info.phase() == Phase::NeedsUser || pr.is_some_and(pr_needs_action);
        // Colour and blink answer different questions (#100). The border's
        // colour speaks for the whole cell: anything in here wants a person.
        // The blink says who is waiting, and only the session's own asking
        // moves the cell itself — a red check or an unstarted issue blinks on
        // its own row, where the thing that needs doing actually is.
        let beat = if info.phase() == Phase::NeedsUser { theme::pulse_at(t) } else { 1.0 };
        // Opaque, because the session's row is filled with this same colour
        // and a label has to read on top of it (#89).
        let edge = if needs {
            theme::with_alpha(read(pal.alarm), beat)
        } else {
            match info.phase() {
                Phase::Working => pal.ink,
                Phase::Idle | Phase::NeedsUser => theme::lerp(pal.bg, pal.ink, 0.55),
            }
        };
        out.push(Shape::Stroke {
            pieces: vec![closed(corners(center, r, GAP))],
            width: BORDER * 1.6,
            color: fade(edge),
        });

        // Linear on top, the session in the middle, its pull request at the
        // bottom. Always in that order, so a column of cells can be read down.
        let slots = [
            ticket.map(|t| SlotInk {
                mark: logos::Mark::Linear,
                label: t.key.clone(),
                color: read(ticket_color(t, pal)),
                check: false,
                origin: None,
            }),
            Some(SlotInk {
                mark: logos::Mark::Claude,
                label: info.label(),
                color: pal.bg,
                check: false,
                // Where this session is running, beside the mark that says
                // what it is (#122).
                origin: info.seat.as_ref().and_then(|s| logos::Mark::host(s.host)),
            }),
            // The answered one if there is one, otherwise the number GitHub
            // would not speak for, drawn quietly and blinking.
            pr.map(|p| SlotInk {
                mark: logos::Mark::GitHub,
                label: p.label(),
                color: read(pr_color(p, pal)),
                check: p.signed_off(),
                origin: None,
            })
            .or_else(|| {
                unanswered.map(|id| SlotInk {
                    mark: logos::Mark::GitHub,
                    label: format!("#{}", id.number),
                    color: theme::with_alpha(pal.ink, 0.5),
                    check: false,
                    origin: None,
                })
            }),
        ];
        for (row, slot) in slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let extras = Extras { origin: slot.origin.is_some(), check: slot.check };
            let RowBox { dy, scale, mark, origin, label, check, .. } = row_box(r, row, &slot.label, extras);
            // The session's label lies on the bed, so it blinks with it;
            // the other two answer for themselves.
            let slot_beat = if row == 1 { beat } else { row_beat(row) };
            let text_w = font::measure(&label, scale);
            let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
            let (tick, badge) = (span(check), span(origin));
            let left = center.x - (mark + MARK_GAP + badge + text_w + tick) / 2.0;
            // The session wears the cell's own colour as a bed, so the middle
            // row reads as the chip it is on the other view (#89).
            if row == 1 {
                let (at, w, h) = session_bed(center, r);
                out.push(Shape::RoundedRect {
                    center: at,
                    w,
                    h,
                    r: 1.0,
                    angle: 0.0,
                    color: fade(edge),
                    stroke: None,
                });
            }
            out.push(Shape::Mark {
                mark: slot.mark,
                size: mark,
                angle: 0.0,
                center: Pt::new(left + mark / 2.0, center.y + dy),
                color: fade(theme::with_alpha(slot.color, slot_beat)),
            });
            // The origin rides between the mark and the label, the way it
            // does inside a chip on the other view (#122).
            if let (Some(badge_mark), true) = (slot.origin, origin > 0.0) {
                out.push(Shape::Mark {
                    mark: badge_mark,
                    size: origin,
                    angle: 0.0,
                    center: Pt::new(left + mark + MARK_GAP + origin / 2.0, center.y + dy),
                    color: fade(theme::with_alpha(slot.color, slot_beat)),
                });
            }
            out.push(Shape::Text {
                text: label.clone(),
                scale,
                stroke: 1.0,
                angle: 0.0,
                center: Pt::new(left + mark + MARK_GAP + badge + text_w / 2.0, center.y + dy),
                color: fade(theme::with_alpha(slot.color, slot_beat)),
            });
            // Review's tick, after the number it signed off on (#116). The
            // row has no fill to say it with, so it is said in a mark.
            if check > 0.0 {
                let at = Pt::new(left + mark + MARK_GAP + badge + text_w + MARK_GAP + check / 2.0, center.y + dy);
                out.push(Shape::Stroke {
                    pieces: scene::check_mark(at, check),
                    width: (check * 0.2).max(0.7),
                    color: fade(theme::with_alpha(slot.color, slot_beat)),
                });
            }
        }

    }
    out
}

/// The sub-grid of a cell: where each of `n` mini hexes goes, and how big
/// they are. A flower like the clusters themselves, sized so the whole of it
/// stays inside the cell however many there are (#88).
pub fn sub_grid(center: Pt, r: f32, n: usize) -> Vec<(Pt, f32)> {
    let mr = sub_radius(r, n);
    flower(n).into_iter().map(|(q, row)| (center + axial(q, row, mr), mr)).collect()
}

/// A sub-cell of the grid a cell divides into.
///
/// A cell is a honeycomb of seven: a middle and a ring of six, and a
/// subagent sits in one of them. The seven do not change size with how many
/// are taken — two subagents get two of the seven, not two big ones — and
/// past seven the cell divides into nineteen instead. (#105)
///
/// A ring of `k` reaches `1.5k + 1` mini radii up and `(k + 0.5) * sqrt(3)`
/// across, against a cell's `R` and `sqrt(3)/2 * R`; the width is the tighter
/// of the two and gives `R / (2k + 1)`.
fn sub_radius(r: f32, n: usize) -> f32 {
    let inner = r - GAP / 3.0f32.sqrt();
    let k = rings(n).max(1) as f32;
    inner / (2.0 * k + 1.0)
}

fn bounds(vs: impl Iterator<Item = f32>) -> (f32, f32) {
    vs.fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

fn closed(mut pts: Vec<Pt>) -> Vec<Pt> {
    if let Some(first) = pts.first().copied() {
        pts.push(first);
    }
    pts
}

/// Whether a pull request is waiting on somebody: its checks went red, and
/// nothing else on a PR is an interruption (#100).
fn pr_needs_action(p: &crate::pr::Pr) -> bool {
    p.look() == crate::pr::Look::Failing
}

/// Whether an issue is waiting on somebody. A session is running against it
/// and the issue has not been moved out of the queue, which is a thing to go
/// and do. A status nobody has answered for (Orca closed) is not: that is the
/// board not knowing rather than the issue asking. (#100)
fn ticket_needs_action(t: &crate::ticket::Ticket) -> bool {
    use crate::ticket::Status;
    matches!(t.status, Some(Status::Backlog) | Some(Status::Todo))
}

fn ticket_color(t: &crate::ticket::Ticket, pal: Palette) -> Rgba {
    use crate::ticket::Status;
    match t.status {
        Some(Status::Started) => pal.text_idle,
        Some(Status::Done) => pal.done,
        Some(Status::Cancelled) | None => theme::with_alpha(pal.ink, 0.45),
        Some(_) => theme::with_alpha(pal.ink, 0.7),
    }
}

/// This session puts something on screen that blinks (#126).
///
/// Every `pulse_at` in the drawing above is behind one of these three, so
/// the frame clock can ask once rather than guessing. Kept beside them so
/// the two cannot drift apart without the test below noticing.
pub fn blinks(info: &SessionInfo) -> bool {
    info.phase() == Phase::NeedsUser
        || info.ticket.as_ref().is_some_and(ticket_needs_action)
        || info.prs.last().is_some_and(pr_needs_action)
        || !info.prs_unanswered.is_empty()
}

fn pr_color(p: &crate::pr::Pr, pal: Palette) -> Rgba {
    use crate::pr::Look;
    if p.running() {
        return pal.busy;
    }
    match p.look() {
        Look::Merged => pal.merged,
        Look::Open => pal.text_on,
        Look::Failing => pal.alarm,
        Look::Draft => theme::with_alpha(pal.ink, 0.7),
        Look::Closed => theme::with_alpha(pal.ink, 0.35),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SessionInfo;
    use std::time::Instant;

    fn board(groups: &[(&str, usize)]) -> BoardModel {
        let mut model = BoardModel::new();
        let mut list = Vec::new();
        let mut seq = 0;
        for (group, n) in groups {
            for _ in 0..*n {
                seq += 1;
                let mut s = SessionInfo::synthetic(seq, &format!("S-{seq}"), Phase::Working);
                s.group = (*group).to_string();
                list.push(s);
            }
        }
        model.apply(list, Instant::now());
        model.settle();
        model
    }

    #[test]
    fn a_ring_has_six_cells_per_step_out() {
        assert_eq!(ring(0).len(), 1);
        assert_eq!(ring(1).len(), 6);
        assert_eq!(ring(2).len(), 12);
        let mut all = ring(1);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 6, "a ring never repeats a cell");
    }

    #[test]
    fn a_flower_seats_everyone_exactly_once() {
        for n in [1usize, 2, 5, 7, 8, 19, 20] {
            let seats = flower(n);
            assert_eq!(seats.len(), n, "{n} sessions need {n} seats");
            let mut sorted = seats.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), n, "{n}: two sessions would share a cell");
        }
    }

    /// The point of the view: panes of one tab read as one clump, and two
    /// clumps never touch (#86).
    #[test]
    fn panes_of_a_tab_cluster_and_clusters_keep_their_distance() {
        let model = board(&[("warp-tab:1-1", 3), ("warp-tab:1-2", 2)]);
        let comb = lay_out(&model, 980.0, 620.0);
        let seat = |i: usize| comb.cells.iter().find(|c| c.chip == Some(i)).expect("a seat").center;

        let near = comb.r * 2.0;
        for (a, b) in [(0, 1), (1, 2), (3, 4)] {
            let (pa, pb) = (seat(a), seat(b));
            let d = ((pa.x - pb.x).powi(2) + (pa.y - pb.y).powi(2)).sqrt();
            assert!(d <= near * 1.05, "sessions {a} and {b} share a tab and should be neighbours ({d})");
        }
        let (pa, pb) = (seat(0), seat(4));
        let d = ((pa.x - pb.x).powi(2) + (pa.y - pb.y).powi(2)).sqrt();
        assert!(d > near, "two tabs should not run into each other ({d})");
    }

    #[test]
    fn every_session_gets_a_cell_and_the_rest_are_empty() {
        let model = board(&[("warp-tab:1-1", 4)]);
        let comb = lay_out(&model, 980.0, 620.0);
        let taken: Vec<usize> = comb.cells.iter().filter_map(|c| c.chip).collect();
        assert_eq!(taken.len(), 4, "each session sits exactly once");
        assert!(comb.cells.iter().filter(|c| c.chip.is_none()).count() > 4, "the grid keeps its empties");
    }

    #[test]
    fn a_click_lands_on_the_cell_it_is_inside_and_nowhere_else() {
        let model = board(&[("warp-tab:1-1", 2)]);
        let comb = lay_out(&model, 980.0, 620.0);
        for want in [0usize, 1] {
            let at = comb.cells.iter().find(|c| c.chip == Some(want)).unwrap().center;
            assert_eq!(hit(&comb, at), Some(want), "the middle of a cell is that cell");
            assert_eq!(hit(&comb, at + Pt::new(0.0, comb.r * 3.0)), None, "well outside is nothing");
        }
    }

    /// A cell hanging off the side is a cell nobody can read, and the first
    /// placement did exactly that: a row's own half-cell offset crept into
    /// the horizontal spacing (#86).
    #[test]
    fn no_occupied_cell_hangs_off_the_side() {
        for width in [520.0f32, 760.0, 980.0, 1600.0] {
            for n in [1usize, 3, 7, 12] {
                let model = board(&[("warp-tab:1-1", n)]);
                let comb = lay_out(&model, width, 620.0);
                for cell in comb.cells.iter().filter(|c| c.chip.is_some()) {
                    let xs: Vec<f32> = corners(cell.center, comb.r, GAP).iter().map(|p| p.x).collect();
                    let (lo, hi) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
                    assert!(lo >= -0.5, "{width}px, {n} sessions: a cell starts at {lo}");
                    assert!(hi <= width + 0.5, "{width}px, {n} sessions: a cell reaches {hi} of {width}");
                }
            }
        }
    }

    #[test]
    fn no_occupied_cell_reaches_above_the_first_row() {
        for n in [1usize, 3, 7, 12] {
            let model = board(&[("warp-tab:1-1", n)]);
            let comb = lay_out(&model, 980.0, 620.0);
            for cell in comb.cells.iter().filter(|c| c.chip.is_some()) {
                let top = corners(cell.center, comb.r, GAP).iter().map(|p| p.y).fold(f32::MAX, f32::min);
                assert!(top >= -0.5, "{n} sessions: a cell starts at {top}, above the grid");
            }
        }
    }

    /// A cell is an outline with the board showing through, so every colour
    /// in it has to read against that board rather than against a chip (#86).
    #[test]
    fn every_colour_in_a_cell_reads_against_its_board() {
        for pal in [theme::PALETTE, theme::ORANGE, theme::DARK] {
            for c in [pal.text_on, pal.text_idle, pal.alarm, pal.merged, pal.done, pal.busy, pal.ink] {
                let got = theme::readable(c, pal.bg, pal.ink, READABLE);
                assert!(
                    theme::contrast(got, pal.bg) >= READABLE,
                    "a cell's colour came out at {:.1}:1 on its board",
                    theme::contrast(got, pal.bg)
                );
            }
        }
    }

    fn with_subagents(n: usize) -> BoardModel {
        use crate::model::SubagentInfo;
        let mut model = BoardModel::new();
        let mut s = SessionInfo::synthetic(1, "S-1", Phase::Working);
        s.group = "warp-tab:1-1".into();
        s.subagents = (0..n as u32).map(|i| SubagentInfo::synthetic(i + 1, "Explore", "", true)).collect();
        model.apply(vec![s], Instant::now());
        model.settle();
        model
    }

    /// Subagents moved out of their parent's hex and into the one next door
    /// (#88), and the sub-grid has to stay inside that one.
    #[test]
    fn a_cell_divides_into_seven_for_its_subagents() {
        for n in [1usize, 3, 7, 12, 19] {
            let model = with_subagents(n);
            let comb = lay_out(&model, 980.0, 620.0);
            let parent = comb.cells.iter().find(|c| c.chip == Some(0)).expect("the session").center;
            let home = comb.cells.iter().find(|c| c.subs == Some(0)).expect("a cell for the subagents");
            assert!(home.chip.is_none(), "{n}: the subagents took a cell somebody was sitting in");

            let d = ((home.center.x - parent.x).powi(2) + (home.center.y - parent.y).powi(2)).sqrt();
            let step = comb.r * 3.0f32.sqrt();
            assert!((d - step).abs() < 1.0, "{n}: the cell is not next door ({d} against {step})");

            // A cell divides into seven sub-cells, and a subagent sits in
            // one of them: the seven keep their size however few are taken,
            // and the whole lot stays inside the cell (#105).
            let grid = sub_grid(home.center, comb.r, n);
            assert_eq!(grid.len(), n);
            let seven = sub_radius(comb.r, 7);
            for (at, mr) in &grid {
                if n <= 7 {
                    assert!((mr - seven).abs() < 1e-3, "{n}: a sub-cell of seven changed size");
                }
                for corner in corners(*at, *mr, SUB_GAP) {
                    assert!(
                        inside_hex(corner, home.center, comb.r, GAP),
                        "{n}: a sub-cell reaches outside the cell it divides"
                    );
                }
            }
            assert!(
                grid.iter().any(|(at, _)| (at.x - home.center.x).abs() < 0.01 && (at.y - home.center.y).abs() < 0.01),
                "{n}: the middle sub-cell is taken first"
            );
        }
    }

    /// Each row of a cell is its own target (#89).
    /// Seating leaves everyone a neighbour to put subagents in, so no
    /// session is ever walled in by its own cluster (#108, #109).
    #[test]
    fn a_session_hemmed_in_by_its_own_cluster_still_gets_its_subagents() {
        use crate::model::SubagentInfo;
        let mut model = BoardModel::new();
        let list: Vec<SessionInfo> = (0..7u32)
            .map(|i| {
                let mut s = SessionInfo::synthetic(i + 1, &format!("S-{i}"), Phase::Working);
                s.group = "warp-tab:1-1".into();
                // Everyone has one, the middle included.
                s.subagents = vec![SubagentInfo::synthetic(i + 1, "Explore", "", true)];
                s
            })
            .collect();
        model.apply(list, Instant::now());
        model.settle();
        let comb = lay_out(&model, 980.0, 620.0);

        for i in 0..7 {
            let home = comb.cells.iter().find(|c| c.subs == Some(i));
            assert!(home.is_some(), "session {i} lost its subagents");
            assert!(home.unwrap().chip.is_none(), "session {i}: they took a seat somebody is in");
        }
        let seats: Vec<(f32, f32)> =
            comb.cells.iter().filter(|c| c.subs.is_some()).map(|c| (c.center.x, c.center.y)).collect();
        for (i, a) in seats.iter().enumerate() {
            for b in &seats[i + 1..] {
                assert_ne!(a, b, "two sessions were given the same cell");
            }
        }
    }

    /// The rule the seating follows, on clusters of every size (#109).
    #[test]
    fn every_session_in_a_cluster_keeps_a_free_neighbour() {
        for n in 1..=12usize {
            let members: Vec<usize> = (0..n).collect();
            let seats = seat_cluster(0, 0, &members);
            assert_eq!(seats.len(), n);
            let taken: Vec<(i32, i32)> = seats.iter().map(|(at, _)| *at).collect();
            let mut sorted = taken.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), n, "{n}: two sessions were seated in one cell");
            for at in &taken {
                assert!(has_free_neighbour(*at, &taken), "{n}: a session was walled in by its own cluster");
            }
        }
    }

    /// A cell off the side is never drawn, so subagents sent to one are lost
    /// exactly as if nowhere had been found (#111).
    #[test]
    fn a_subagent_cell_is_always_somewhere_the_grid_draws() {
        use crate::model::SubagentInfo;
        for width in [420.0f32, 620.0, 980.0, 1600.0] {
            for n in [1usize, 3, 7, 9] {
                let mut model = BoardModel::new();
                let list: Vec<SessionInfo> = (0..n as u32)
                    .map(|i| {
                        let mut s = SessionInfo::synthetic(i + 1, &format!("S-{i}"), Phase::Working);
                        s.group = "warp-tab:1-1".into();
                        s.subagents = vec![SubagentInfo::synthetic(i + 1, "Explore", "", true)];
                        s
                    })
                    .collect();
                model.apply(list, Instant::now());
                model.settle();
                let comb = lay_out(&model, width, 620.0);
                let where_ = format!("{width}px, {n} sessions");

                for i in 0..n {
                    let home = comb.cells.iter().find(|c| c.subs == Some(i));
                    assert!(home.is_some(), "{where_}: session {i} lost its subagents");
                    let home = home.unwrap();
                    let xs: Vec<f32> = corners(home.center, comb.r, GAP).iter().map(|p| p.x).collect();
                    let top = corners(home.center, comb.r, GAP).iter().map(|p| p.y).fold(f32::MAX, f32::min);
                    assert!(xs.iter().cloned().fold(f32::MAX, f32::min) >= -1.0, "{where_}: off the left");
                    assert!(
                        xs.iter().cloned().fold(f32::MIN, f32::max) <= width + 1.0,
                        "{where_}: off the right"
                    );
                    assert!(top >= -1.0, "{where_}: above the first row");
                }
            }
        }
    }

    #[test]
    fn a_cell_answers_for_the_row_that_was_clicked() {
        let model = board(&[("warp-tab:1-1", 1)]);
        let comb = lay_out(&model, 980.0, 620.0);
        let at = comb.cells.iter().find(|c| c.chip == Some(0)).unwrap().center;
        assert_eq!(hit_slot(&comb, at + Pt::new(0.0, -comb.r * 0.42)), Some((0, Slot::Ticket)));
        assert_eq!(hit_slot(&comb, at), Some((0, Slot::Session)));
        assert_eq!(hit_slot(&comb, at + Pt::new(0.0, comb.r * 0.42)), Some((0, Slot::Pr)));
        assert_eq!(hit_slot(&comb, at + Pt::new(0.0, comb.r * 3.0)), None, "outside the cell is nothing");
    }

    /// The session's bed runs wall to wall inside the cell and touches
    /// nothing else (#91).
    #[test]
    fn the_session_bed_fills_the_cell_from_side_to_side() {
        for r in [46.0f32, 70.0, 104.0] {
            let center = Pt::new(300.0, 300.0);
            let (at, w, h) = session_bed(center, r);
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let corner = Pt::new(at.x + sx * w / 2.0, at.y + sy * h / 2.0);
                assert!(inside_hex(corner, center, r, GAP), "r={r}: the bed pokes out of its cell");
            }
            // Wall to wall: anything wider would be outside.
            let wider = Pt::new(at.x + w / 2.0 + BORDER * 2.5, at.y);
            assert!(!inside_hex(wider, center, r, GAP), "r={r}: the bed is leaving room at the sides");
        }
    }

    /// The view opens out of its clusters: a session is there from the first
    /// frame, and the empty field arrives in rings behind it (#92).
    #[test]
    fn the_cells_arrive_spreading_out_of_the_clusters() {
        let model = board(&[("warp-tab:1-1", 3)]);
        let comb = lay_out(&model, 980.0, 620.0);
        let seated = comb.cells.iter().find(|c| c.chip.is_some()).unwrap();
        assert_eq!(seated.wave, 0.0, "a session is where the wave starts");

        let far = comb.cells.iter().max_by(|a, b| a.wave.total_cmp(&b.wave)).unwrap();
        assert!(far.wave > 2.0, "the far corner of the grid is several cells out");

        assert_eq!(reveal(seated, 0.0), 0.0, "a session is not on screen before its turn");
        let empty = comb.cells.iter().find(|c| c.chip.is_none() && c.subs.is_none()).unwrap();
        assert!(reveal(empty, 0.0) >= GRID_FLOOR, "the grid itself is there from the first frame (#102)");
        assert!(reveal(seated, WAVE_FADE) >= 1.0, "a session is fully in after one fade");
        // A far cell waits its turn; being empty it waits at the floor
        // rather than at nothing, since the grid stands throughout (#102).
        assert_eq!(reveal(far, far.wave * WAVE_STEP * 0.5), GRID_FLOOR);
        let mid = far.wave * WAVE_STEP + WAVE_FADE * 0.95;
        assert!((GRID_FLOOR..1.0).contains(&reveal(far, mid)), "and comes up from it");
        // The shape of the thing, whatever the timings are set to: nothing
        // ever arrives ahead of a cell nearer the clusters.
        for t in [0.05f32, 0.2, 0.4, 0.8] {
            for cell in &comb.cells {
                if cell.chip.is_none() && cell.subs.is_none() {
                    continue;
                }
                if cell.wave > far.wave / 2.0 {
                    assert!(
                        reveal(cell, t) <= reveal(seated, t),
                        "an outer cell overtook the cluster it should be following"
                    );
                }
            }
        }
        for cell in &comb.cells {
            assert_eq!(reveal(cell, 1.0e3), 1.0, "everything settles");
        }
    }

    /// Nothing a cell says may leave it (#94). The corners of what is drawn
    /// are checked, mark, label and approval tick together, on the narrowest
    /// cells (#116).
    #[test]
    fn a_cell_never_lets_its_text_out() {
        let labels = [
            "S-1",
            "FEATURE-REPORT",
            "PJM-2033",
            "#8792",
            "a-very-long-session-name-nobody-would-ever-type",
            "서비스-긴-이름의-한글-라벨",
        ];
        for r in [R_MIN, 70.0, R_MAX] {
            let center = Pt::new(500.0, 500.0);
            for row in 0..3 {
                for label in labels {
                    for check in [false, true] {
                        for origin in [false, true] {
                            let extras = Extras { origin, check };
                            let b = row_box(r, row, label, extras);
                            if b.label.is_empty() {
                                continue;
                            }
                            let text_w = font::measure(&b.label, b.scale);
                            let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
                            let w = b.mark + MARK_GAP + span(b.origin) + text_w + span(b.check);
                            let h = font::height(b.scale).max(b.mark).max(b.check).max(b.origin);
                            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                                let corner =
                                    Pt::new(center.x + sx * w / 2.0, center.y + b.dy + sy * h / 2.0);
                                assert!(
                                    inside_hex(corner, center, r, GAP),
                                    "r={r} row={row} check={check} origin={origin} {label:?}: \
                                     {:?} reaches outside its cell",
                                    b.label
                                );
                            }
                            assert_eq!(b.check > 0.0, check, "the tick is there when it was asked for");
                            assert_eq!(b.origin > 0.0, origin, "and so is the origin mark");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_label_that_had_to_be_cut_says_so() {
        let b = row_box(70.0, 1, "a-very-long-session-name-nobody-would-ever-type", Extras::default());
        assert!(b.label.ends_with('…'), "a cut label wears an ellipsis, got {:?}", b.label);
        assert!(b.label.chars().count() > 2, "and keeps enough to be worth reading");
        let short = row_box(70.0, 1, "S-1", Extras::default());
        assert_eq!(short.label, "S-1", "a label that fits is left alone");
        // The tick comes out of the label's room, the way the mark does, so
        // a row that gains one gives up letters rather than the cell's edge
        // (#94, #116).
        let long = "a-very-long-session-name-nobody-would-ever-type";
        let ticked = row_box(70.0, 2, long, Extras { check: true, ..Extras::default() });
        let plain = row_box(70.0, 2, long, Extras::default());
        assert!(
            ticked.label.chars().count() < plain.label.chars().count(),
            "the tick costs the label letters: {:?} against {:?}",
            ticked.label,
            plain.label
        );
        // And so does the origin mark, for the same reason (#122).
        let badged = row_box(70.0, 1, long, Extras { origin: true, ..Extras::default() });
        let bare = row_box(70.0, 1, long, Extras::default());
        assert!(
            badged.label.chars().count() < bare.label.chars().count(),
            "the origin mark costs the label letters: {:?} against {:?}",
            badged.label,
            bare.label
        );
    }

    /// Colour and blink answer different questions: the border's colour is
    /// the cell's whole state, the blink is who is waiting (#100).
    #[test]
    fn only_the_part_that_is_waiting_blinks() {
        use crate::pr::{Look, Pr};
        use crate::ticket::{Status, Ticket};

        assert!(pr_needs_action(&Pr::synthetic(1, Look::Failing)));
        for calm in [Look::Draft, Look::Open, Look::Merged, Look::Closed] {
            assert!(!pr_needs_action(&Pr::synthetic(1, calm)), "{calm:?} is not an interruption");
        }

        assert!(ticket_needs_action(&Ticket::synthetic(1, Some(Status::Todo))));
        assert!(ticket_needs_action(&Ticket::synthetic(1, Some(Status::Backlog))));
        for calm in [Status::Started, Status::Done, Status::Cancelled] {
            assert!(!ticket_needs_action(&Ticket::synthetic(1, Some(calm))), "{calm:?} wants nothing");
        }
        assert!(
            !ticket_needs_action(&Ticket::synthetic(1, None)),
            "a status nobody answered for is the board not knowing, not the issue asking"
        );
    }

    /// A cell has no fill to say approval with, so it says it in a tick after
    /// the number — and only where review actually signed off (#116).
    #[test]
    fn the_comb_ticks_a_pull_request_review_signed_off_on() {
        use crate::pr::{Look, Pr};
        // The tick is the only three-point stroke in the scene: a cell wall
        // is a closed hexagon and a subagent is a filled polygon.
        let ticks = |pr: Pr| {
            let mut model = board(&[("warp-tab:1-1", 1)]);
            model.chips[0].info.prs = vec![pr];
            let comb = lay_out(&model, 980.0, 620.0);
            // Well past the arrival wave, so the cell is fully drawn (#92).
                build_shapes(&model, &comb, theme::PALETTE, 0.0, Pt::new(0.0, 0.0), 3.0)
                .into_iter()
                .filter(|s| matches!(s, Shape::Stroke { pieces, .. } if pieces.len() == 1 && pieces[0].len() == 3))
                .count()
        };
        // Approval rides on every look, which is the whole point of taking it
        // out of the colour (#116): the colour says the state and the checks,
        // the tick says review is done with it.
        for look in Look::ALL {
            let signed = ticks(Pr { approved: true, ..Pr::synthetic(1, look) });
            assert_eq!(signed, 1, "{look:?}: an approved pull request wears a tick");
        }
        for bare in [Look::Draft, Look::Open, Look::Failing, Look::Closed] {
            assert_eq!(ticks(Pr::synthetic(1, bare)), 0, "{bare:?} has nothing to tick");
        }
        assert_eq!(ticks(Pr::synthetic(1, Look::Merged)), 1, "nothing lands without a sign-off");
    }

    /// A cluster is a flower with a rim of empty cells, which is the only
    /// thing that says where one ends and the next begins. Two sessions from
    /// different groups touching reads as one bigger cluster (#121).
    #[test]
    fn clusters_never_touch_each_other() {
        const DIRS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];
        let shapes: [&[(&str, usize)]; 5] = [
            &[("a", 1), ("b", 1), ("c", 1), ("d", 1), ("e", 1), ("f", 1), ("g", 1), ("h", 1)],
            &[("a", 3), ("b", 3), ("c", 3), ("d", 3)],
            &[("a", 7), ("b", 2), ("c", 7), ("d", 1)],
            &[("a", 1), ("b", 12), ("c", 1), ("d", 5)],
            &[("a", 2), ("b", 2), ("c", 2), ("d", 2), ("e", 2), ("f", 2)],
        ];
        for shape in shapes {
            for width in [380.0f32, 420.0, 520.0, 640.0, 820.0, 980.0, 1280.0, 1600.0] {
                let model = board(shape);
                let comb = lay_out(&model, width, 900.0);
                // Which group each seated cell belongs to.
                let mut owner: Vec<((i32, i32), String)> = Vec::new();
                for cell in comb.cells.iter().filter(|c| c.chip.is_some()) {
                    let chip = cell.chip.unwrap();
                    let group = model.chips[chip].info.group.clone();
                    owner.push(((cell.q, cell.row), group));
                }
                for (at, group) in &owner {
                    assert!(
                        owner.iter().filter(|(p, _)| p == at).count() == 1,
                        "w={width} {shape:?}: two sessions were seated in {at:?}"
                    );
                    for (dq, drow) in DIRS {
                        let n = (at.0 + dq, at.1 + drow);
                        if let Some((_, other)) = owner.iter().find(|(p, _)| *p == n) {
                            assert_eq!(
                                other, group,
                                "w={width} {shape:?}: {group} at {at:?} is touching {other} at {n:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The frame clock asks `blinks()` whether anything is moving, and a
    /// board that stops repainting while something still blinks is the one
    /// failure that would not look like a bug in the clock (#126).
    #[test]
    fn nothing_blinks_without_blinks_saying_so() {
        use crate::pr::{Look, Pr};
        use crate::ticket::{Status, Ticket};
        use crate::model::SessionInfo;

        // Every drawing above reaches `pulse_at` through one of these.
        let waiting = SessionInfo::synthetic(1, "S", Phase::NeedsUser);
        assert!(blinks(&waiting), "a session asking outright");
        let mut red = SessionInfo::synthetic(2, "S", Phase::Working);
        red.prs = vec![Pr::synthetic(1, Look::Failing)];
        assert!(blinks(&red), "a failing check");
        let mut todo = SessionInfo::synthetic(3, "S", Phase::Working);
        todo.ticket = Some(Ticket::synthetic(1, Some(Status::Todo)));
        assert!(blinks(&todo), "an issue still in the queue");
        let mut mute = SessionInfo::synthetic(4, "S", Phase::Working);
        mute.prs_unanswered = vec![crate::pr::PrRef { repo: "o/r".into(), number: 1 }];
        assert!(blinks(&mute), "a pull request GitHub would not answer for");

        // And a board with none of them is genuinely still.
        let mut calm = SessionInfo::synthetic(5, "S", Phase::Working);
        calm.prs = vec![Pr::synthetic(1, Look::Merged)];
        calm.ticket = Some(Ticket::synthetic(1, Some(Status::Started)));
        assert!(!blinks(&calm));
        assert!(!blinks(&SessionInfo::synthetic(6, "S", Phase::Idle)));
    }

    #[test]
    fn two_neighbours_leave_a_gap_between_them() {
        let r = 60.0;
        let (a, b) = (Pt::new(0.0, 0.0), axial(1, 0, r));
        let right = corners(a, r, GAP).into_iter().map(|p| p.x).fold(f32::MIN, f32::max);
        let left = corners(b, r, GAP).into_iter().map(|p| p.x).fold(f32::MAX, f32::min);
        assert!((left - right - GAP).abs() < 0.01, "the gap should be {GAP}, got {}", left - right);
        assert!(GAP > BORDER * 3.0, "a gap the width of a hairline is no gap at all");
    }
}
