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
    let last_row = seats.iter().map(|((_, row), _)| *row).max().unwrap_or(0) + 2;

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
                cells.push(Cell { center, chip });
            }
            q += 1;
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

/// What goes on one of a cell's three lines.
struct Slot {
    mark: logos::Mark,
    label: String,
    color: Rgba,
}

/// Every shape of the honeycomb view, in design units.
pub fn build_shapes(model: &BoardModel, comb: &Comb, pal: Palette, scroll_y: f32, origin: Pt) -> Vec<Shape> {
    let mut out = Vec::new();
    let r = comb.r;
    out.push(Shape::Rect { x: 0.0, y: 0.0, w: 1.0e5, h: 1.0e5, color: pal.bg });

    for cell in &comb.cells {
        let center = cell.center - Pt::new(0.0, scroll_y) + origin;
        if center.y < -r * 2.0 || center.y > 1.0e4 {
            continue;
        }
        let Some(i) = cell.chip else {
            // An empty cell says so quietly: the grid stays visible, the eye
            // goes to the ones with something in them.
            out.push(Shape::Stroke {
                pieces: vec![closed(corners(center, r, GAP))],
                width: BORDER,
                color: theme::with_alpha(pal.ink, 0.18),
            });
            continue;
        };
        let Some(chip) = model.chips.get(i) else { continue };
        let info = &chip.info;

        // Everything here is drawn straight onto the board, so every colour
        // is put through the floor first (#86).
        let read = |c: Rgba| theme::readable(c, pal.bg, pal.ink, READABLE);
        let (edge, accent) = match info.phase() {
            Phase::Working => (pal.ink, read(pal.text_on)),
            Phase::NeedsUser => (read(pal.alarm), read(pal.alarm)),
            Phase::Idle => (theme::with_alpha(pal.ink, 0.55), read(pal.text_idle)),
        };
        out.push(Shape::Stroke { pieces: vec![closed(corners(center, r, GAP))], width: BORDER * 1.6, color: edge });

        // Linear on top, the session in the middle, its pull request at the
        // bottom. Always in that order, so a column of cells can be read down.
        let ticket = info.ticket.as_ref();
        let pr = info.prs.last();
        let slots = [
            ticket.map(|t| Slot { mark: logos::Mark::Linear, label: t.key.clone(), color: read(ticket_color(t, pal)) }),
            Some(Slot { mark: logos::Mark::Claude, label: info.label(), color: accent }),
            pr.map(|p| Slot { mark: logos::Mark::GitHub, label: p.label(), color: read(pr_color(p, pal)) }),
        ];
        for (row, slot) in slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let dy = (row as f32 - 1.0) * r * 0.42;
            let scale = if row == 1 { r * 0.021 } else { r * 0.016 };
            let mark = if row == 1 { r * 0.17 } else { r * 0.13 };
            let text_w = font::measure(&slot.label, scale);
            let left = center.x - (mark + 4.0 + text_w) / 2.0;
            out.push(Shape::Mark {
                mark: slot.mark,
                size: mark,
                angle: 0.0,
                center: Pt::new(left + mark / 2.0, center.y + dy),
                color: slot.color,
            });
            out.push(Shape::Text {
                text: slot.label.clone(),
                scale,
                stroke: 1.0,
                angle: 0.0,
                center: Pt::new(left + mark + 4.0 + text_w / 2.0, center.y + dy),
                color: slot.color,
            });
        }

        // The subagents, as a sub-grid of small hexes along the cell's floor.
        let subs: Vec<bool> = chip.subs.iter().filter(|s| !s.anim.gone).map(|s| s.info.running).collect();
        if !subs.is_empty() {
            let sr = r * 0.13;
            let step = sr * 3.0f32.sqrt();
            let n = subs.len().min(5);
            let x0 = center.x - step * (n as f32 - 1.0) / 2.0;
            for (k, running) in subs.iter().take(n).enumerate() {
                let at = Pt::new(x0 + step * k as f32, center.y + r * 0.72);
                let color = if *running { read(pal.text_on) } else { theme::with_alpha(pal.ink, 0.45) };
                out.push(Shape::Stroke { pieces: vec![closed(corners(at, sr, SUB_GAP))], width: BORDER, color });
            }
        }
    }
    out
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
