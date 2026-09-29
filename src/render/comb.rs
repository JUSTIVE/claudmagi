//! The honeycomb view (#86): the same sessions as the board, laid out as a
//! vertically scrolling hex grid instead of chips on traces.
//!
//! The grid is the whole surface. Every cell is drawn, and the ones nobody
//! sits in are drawn grey, so the shape of what is running reads against the
//! shape of what is not. Cells carry a thin border and nothing else, and each
//! one is inset by half a border so the gap between two neighbours comes out
//! exactly one border wide.
//!
//! Panes of one Warp tab are a cluster: a flower of cells around a centre,
//! placed far enough from the next flower that a ring of empty cells always
//! separates them. Inside a cell the three things a session has are stacked in
//! a fixed order — the Linear issue on top, the session itself in the middle,
//! its pull request at the bottom — so the eye can read down a column of cells
//! and compare like with like. Subagents hang underneath as a row of small
//! hexes, a sub-grid of the cell that spawned them.

use crate::font;
use crate::geom::Pt;
use crate::logos;
use crate::model::{BoardModel, Phase};
use crate::render::scene::Shape;
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
/// The most of a cell one mini hex may take. Enough that a name fits inside
/// it (#95), still plainly smaller than the cell around it.
const SUB_MAX: f32 = 0.42;
/// A mini hex's label against its own radius, and the room kept clear of its
/// wall. Smaller than a cell's own rows: these are names nobody reads across
/// the room, and the smaller they are set the more of each one survives the
/// cut (#98).
const SUB_TEXT: f32 = 0.042;
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

/// The cell a point falls in. Rounding in cube space is what keeps the answer
/// on the grid rather than between two of its rows.
fn cell_at(p: Pt, r: f32) -> (i32, i32) {
    let sqrt3 = 3.0f32.sqrt();
    let (fq, fr) = ((sqrt3 / 3.0 * p.x - p.y / 3.0) / r, (2.0 / 3.0 * p.y) / r);
    let fs = -fq - fr;
    let (mut q, mut rr, s) = (fq.round(), fr.round(), fs.round());
    let (dq, dr, ds) = ((q - fq).abs(), (rr - fr).abs(), (s - fs).abs());
    if dq > dr && dq > ds {
        q = -rr - s;
    } else if dr > ds {
        rr = -q - s;
    }
    (q as i32, rr as i32)
}

/// The six neighbours of a cell, in the order a subagent cell is looked for:
/// under the parent first, where the eye goes next.
const NEIGHBOURS: [(i32, i32); 6] = [(0, 1), (-1, 1), (1, 0), (-1, 0), (1, -1), (0, -1)];

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

/// Lays the sessions out as clusters on a grid that fills `width`.
///
/// Sessions are grouped the way the board groups them (#41): panes of one Warp
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

    // Where each session sits. Flowers are placed by where they land on
    // screen rather than by axial arithmetic: a row's own half-cell offset
    // would otherwise creep into the horizontal spacing and push later bands
    // off the right edge.
    let mut seats: Vec<((i32, i32), usize)> = Vec::new();
    // A flower is placed by its own extent, not by its centre: a cluster of
    // three seats one member a full cell to the left of the middle, and
    // stepping by centres alone hangs that one off the edge.
    let gap = r * sqrt3;
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut band = 0.0f32;
    for (_, members) in &groups {
        let k = rings(members.len()) as f32;
        let (half_w, half_h) = ((k + 0.5) * r * sqrt3, k * r * 1.5 + r);
        if x + 2.0 * half_w > width && x > 0.0 {
            x = 0.0;
            y += band;
            band = 0.0;
        }
        let (cq, crow) = cell_at(Pt::new(x + half_w, y + half_h), r);
        let mut placed: Vec<((i32, i32), usize)> =
            flower(members.len()).into_iter().zip(members).map(|(s, m)| ((cq + s.0, crow + s.1), *m)).collect();
        // Snapping the centre to the grid can round the whole flower half a
        // cell off, so measure what actually landed and shove it back inside
        // by whole columns.
        let edge = |seats: &[((i32, i32), usize)], f: fn(f32, f32) -> f32, side: f32| {
            seats.iter().map(|((q, row), _)| axial(*q, *row, r).x + side).fold(-side / side.abs() * f32::MAX, f)
        };
        let left = edge(&placed, f32::min, -r * sqrt3 / 2.0);
        let right = edge(&placed, f32::max, r * sqrt3 / 2.0);
        let shift = if left < 0.0 {
            (-left / (r * sqrt3)).ceil() as i32
        } else if right > width {
            -((right - width) / (r * sqrt3)).ceil() as i32
        } else {
            0
        };
        for ((q, _), _) in placed.iter_mut() {
            *q += shift;
        }
        // Same again for the top, where a snapped flower can reach above the
        // first row and lose its heads.
        let top = placed.iter().map(|((q, row), _)| axial(*q, *row, r).y - r).fold(f32::MAX, f32::min);
        if top < 0.0 {
            let down = (-top / (r * 1.5)).ceil() as i32;
            for ((_, row), _) in placed.iter_mut() {
                *row += down;
            }
        }
        seats.extend(placed);
        x += 2.0 * half_w + gap;
        band = band.max(2.0 * half_h);
    }
    // Subagents take a cell of their own, next door to the session that
    // spawned them (#88).
    let mut sub_seats: Vec<((i32, i32), usize)> = Vec::new();
    for (at, chip) in &seats {
        let has_subs = model.chips.get(*chip).is_some_and(|c| c.subs.iter().any(|s| !s.anim.gone));
        if !has_subs {
            continue;
        }
        let free = NEIGHBOURS.iter().map(|(dq, dr)| (at.0 + dq, at.1 + dr)).find(|n| {
            !seats.iter().any(|(s, _)| s == n) && !sub_seats.iter().any(|(s, _)| s == n)
        });
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
                cells.push(Cell { center, chip, subs, wave: 0.0 });
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
    pub label: String,
    /// What the row had to play with, mark and label together.
    pub room: f32,
}

pub fn row_box(r: f32, row: usize, label: &str) -> RowBox {
    let dy = ROW_DY[row] * r;
    let scale = if row == 1 { r * 0.021 } else { r * 0.016 };
    let mark = if row == 1 { r * 0.17 } else { r * 0.13 };
    let room = row_room(r, dy, font::height(scale).max(mark)) - ROW_PAD * 2.0;
    RowBox { dy, scale, mark, label: fit(label, scale, room - mark - MARK_GAP), room }
}

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
}

/// How long each ring of cells waits before it starts appearing, and how long
/// one takes to arrive once it does (#92).
const WAVE_STEP: f32 = 0.07;
const WAVE_FADE: f32 = 0.5;

/// How far into its arrival a cell is at `t` seconds: 0 before its turn, 1
/// once it is all the way in.
pub fn reveal(cell: &Cell, t: f32) -> f32 {
    ((t - cell.wave * WAVE_STEP) / WAVE_FADE).clamp(0.0, 1.0)
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
            let read = |c: Rgba| theme::readable(c, pal.bg, pal.ink, READABLE);
            // No cell drawn around them: the subagents are the only thing
            // there, and a hex around a hex reads as a container nobody asked
            // for (#97). The cell still governs where they may go.

            for ((at, mr), (label, going)) in sub_grid(center, r, agents.len()).into_iter().zip(&agents) {
                // Solid, not an outline: a subagent is a thing doing work,
                // and the cell around it is the empty part (#96).
                let color = if *going { read(pal.text_on) } else { theme::lerp(pal.bg, pal.ink, 0.45) };
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
        let needs = info.phase() == Phase::NeedsUser
            || info.prs.iter().any(|p| p.look() == crate::pr::Look::Failing);
        // Opaque, because the session's row is filled with this same colour
        // and a label has to read on top of it (#89).
        let edge = if needs {
            read(pal.alarm)
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
        let ticket = info.ticket.as_ref();
        let pr = info.prs.last();
        let slots = [
            ticket.map(|t| SlotInk { mark: logos::Mark::Linear, label: t.key.clone(), color: read(ticket_color(t, pal)) }),
            Some(SlotInk { mark: logos::Mark::Claude, label: info.label(), color: pal.bg }),
            pr.map(|p| SlotInk { mark: logos::Mark::GitHub, label: p.label(), color: read(pr_color(p, pal)) }),
        ];
        for (row, slot) in slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let RowBox { dy, scale, mark, label, .. } = row_box(r, row, &slot.label);
            let text_w = font::measure(&label, scale);
            let left = center.x - (mark + MARK_GAP + text_w) / 2.0;
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
                color: fade(slot.color),
            });
            out.push(Shape::Text {
                text: label.clone(),
                scale,
                stroke: 1.0,
                angle: 0.0,
                center: Pt::new(left + mark + MARK_GAP + text_w / 2.0, center.y + dy),
                color: fade(slot.color),
            });
        }

    }
    out
}

/// The sub-grid of a cell: where each of `n` mini hexes goes, and how big
/// they are. A flower like the clusters themselves, sized so the whole of it
/// stays inside the cell however many there are (#88).
pub fn sub_grid(center: Pt, r: f32, n: usize) -> Vec<(Pt, f32)> {
    let sqrt3 = 3.0f32.sqrt();
    let seats = flower(n);
    // Sized to the seats actually taken, not to the ring they came from: two
    // subagents sit on a ring of six, and paying for the four empty ones
    // shrinks them until their names are three dots (#95).
    let pts: Vec<Pt> = seats.iter().map(|(q, row)| axial(*q, *row, 1.0)).collect();
    let (x0, x1) = bounds(pts.iter().map(|p| p.x));
    let (y0, y1) = bounds(pts.iter().map(|p| p.y));
    let (ex, ey) = ((x1 - x0) / 2.0 + sqrt3 / 2.0, (y1 - y0) / 2.0 + 1.0);
    let inner = r - GAP / sqrt3;
    // Centred on what they take up, so the group sits in the middle.
    let mid = Pt::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let place = |mr: f32| -> Vec<(Pt, f32)> {
        pts.iter().map(|p| (center + (*p - mid) * mr, mr)).collect()
    };
    // The estimate treats both shapes as boxes, which they are not: a hexagon
    // narrows towards its points, so a flower that fits the box can still
    // poke through a wall. Shrink until it genuinely fits rather than trusting
    // the arithmetic.
    let mut mr = (sqrt3 / 2.0 * inner / ex).min(inner / ey).min(r * SUB_MAX);
    for _ in 0..24 {
        let out = place(mr);
        let fits = out
            .iter()
            .all(|(at, mr)| corners(*at, *mr, SUB_GAP).into_iter().all(|c| inside_hex(c, center, r, GAP)));
        if fits {
            return out;
        }
        mr *= 0.95;
    }
    place(mr)
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

fn ticket_color(t: &crate::ticket::Ticket, pal: Palette) -> Rgba {
    use crate::ticket::Status;
    match t.status {
        Some(Status::Started) => pal.text_idle,
        Some(Status::Done) => pal.done,
        Some(Status::Cancelled) | None => theme::with_alpha(pal.ink, 0.45),
        Some(_) => theme::with_alpha(pal.ink, 0.7),
    }
}

fn pr_color(p: &crate::pr::Pr, pal: Palette) -> Rgba {
    use crate::pr::Look;
    if p.running() {
        return pal.busy;
    }
    match p.look() {
        Look::Merged => pal.merged,
        Look::Approved | Look::Open => pal.text_on,
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
    fn subagents_take_the_cell_next_door_and_stay_inside_it() {
        for n in [1usize, 3, 7, 12, 19] {
            let model = with_subagents(n);
            let comb = lay_out(&model, 980.0, 620.0);
            let parent = comb.cells.iter().find(|c| c.chip == Some(0)).expect("the session").center;
            let home = comb.cells.iter().find(|c| c.subs == Some(0)).expect("a cell for the subagents");
            assert!(home.chip.is_none(), "{n}: the subagents took a cell somebody was sitting in");

            let d = ((home.center.x - parent.x).powi(2) + (home.center.y - parent.y).powi(2)).sqrt();
            let step = comb.r * 3.0f32.sqrt();
            assert!((d - step).abs() < 1.0, "{n}: the cell is not next door ({d} against {step})");

            for (at, mr) in sub_grid(home.center, comb.r, n) {
                assert!(mr <= comb.r * SUB_MAX + 0.01, "{n}: a mini hex this big stops reading as a sub-grid");
                for corner in corners(at, mr, SUB_GAP) {
                    assert!(
                        inside_hex(corner, home.center, comb.r, GAP),
                        "{n}: a mini hex reaches outside its cell"
                    );
                }
            }
        }
    }

    /// Each row of a cell is its own target (#89).
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

        assert_eq!(reveal(seated, 0.0), 0.0, "nothing is on screen before the clock starts");
        assert!(reveal(seated, WAVE_FADE) >= 1.0, "a session is fully in after one fade");
        assert_eq!(reveal(far, far.wave * WAVE_STEP * 0.5), 0.0, "a far cell waits its turn");
        let mid = far.wave * WAVE_STEP + WAVE_FADE / 2.0;
        assert!((0.0..1.0).contains(&reveal(far, mid)), "they come in partway through");
        // The shape of the thing, whatever the timings are set to: nothing
        // ever arrives ahead of a cell nearer the clusters.
        for t in [0.05f32, 0.2, 0.4, 0.8] {
            for cell in &comb.cells {
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
    /// are checked, mark and label together, on the narrowest cells.
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
                    let b = row_box(r, row, label);
                    if b.label.is_empty() {
                        continue;
                    }
                    let text_w = font::measure(&b.label, b.scale);
                    let w = b.mark + MARK_GAP + text_w;
                    let h = font::height(b.scale).max(b.mark);
                    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                        let corner =
                            Pt::new(center.x + sx * w / 2.0, center.y + b.dy + sy * h / 2.0);
                        assert!(
                            inside_hex(corner, center, r, GAP),
                            "r={r} row={row} {label:?}: {:?} reaches outside its cell",
                            b.label
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_label_that_had_to_be_cut_says_so() {
        let b = row_box(70.0, 1, "a-very-long-session-name-nobody-would-ever-type");
        assert!(b.label.ends_with('…'), "a cut label wears an ellipsis, got {:?}", b.label);
        assert!(b.label.chars().count() > 2, "and keeps enough to be worth reading");
        let short = row_box(70.0, 1, "S-1");
        assert_eq!(short.label, "S-1", "a label that fits is left alone");
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
