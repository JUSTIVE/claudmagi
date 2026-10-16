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
use crate::render::scene::{self, Screen, Shape};
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

/// Flat-top axial coordinates to a centre point (#132).
///
/// Flat top and bottom, points left and right. `q` is the column and steps
/// by three quarters of the width; `row` steps by the full height, and a
/// column carries half a row of its own so the columns interlock.
fn axial(q: i32, row: i32, r: f32) -> Pt {
    Pt::new(1.5 * r * q as f32, r * SQRT3 * (row as f32 + q as f32 / 2.0))
}

/// Half a cell's width: centre to the point on either side.
fn half_w(r: f32) -> f32 {
    r
}

/// Half a cell's height: centre to the flat above or below.
fn half_h(r: f32) -> f32 {
    r * SQRT3 / 2.0
}

const SQRT3: f32 = 1.732_050_8;


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

/// The six corners of a flat-top hex, pulled in so two neighbours leave
/// `gap` between their edges. Corner 0 is the point due east, and they go
/// clockwise from there.
pub fn corners(center: Pt, r: f32, gap: f32) -> Vec<Pt> {
    // Each of the two gives up half the gap, and insetting a regular
    // hexagon's edges by `d` pulls its circumradius in by `2d / sqrt(3)`.
    let r = (r - gap / SQRT3).max(1.0);
    (0..6)
        .map(|i| {
            let a = std::f32::consts::PI / 180.0 * (60.0 * i as f32);
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
/// Scanned outwards from the middle of the board, so the first answer is the
/// free spot nearest the middle (#138). `taken` is everything already seated;
/// a candidate is refused when any of its seats lands on one of those or
/// beside one, which is what keeps a rim of empty cells between clusters
/// (#121).
fn seat_somewhere(
    members: &[usize],
    taken: &std::collections::HashSet<(i32, i32)>,
    width: f32,
    height: f32,
    r: f32,
) -> Option<Vec<((i32, i32), usize)>> {
    let half = half_w(r);
    // The flower's shape does not depend on where it is put — `seat_cluster`
    // works in offsets from its centre — so it is worked out once and then
    // carried across the scan. Rebuilding it at every candidate was the whole
    // cost of this: a dozen clusters of seven took 5ms a frame.
    let shape = seat_cluster(0, 0, members);
    // Where the flower's weight actually falls, which is not the cell it is
    // anchored on: a cluster grows downwards out of its first seat (#109), so
    // measuring from the anchor would hang it above the middle by half its
    // own height. `axial` is linear in the coordinates, so the middle of the
    // seats is exactly the middle of their centres.
    let mid = shape.iter().fold(Pt::new(0.0, 0.0), |acc, ((dq, drow), _)| acc + axial(*dq, *drow, r))
        * (1.0 / shape.len().max(1) as f32);
    let board = Pt::new(width / 2.0, height / 2.0);
    // What is already seated, as a weight. A cluster is put where it brings
    // the middle of everything on the board nearest the middle of the screen,
    // not merely where it is nearest the middle itself: the second cluster
    // has to stand aside for the first, and scoring each one alone would then
    // let every later one drift off the same way, building the arrangement
    // out of one corner of the middle (#138).
    let (seated, count) = taken.iter().fold((Pt::new(0.0, 0.0), 0.0f32), |(sum, n), (q, row)| {
        (sum + axial(*q, *row, r), n + 1.0)
    });
    let weight = shape.len() as f32;
    // How far below its anchor the flower's lowest edge reaches, for telling
    // a spot on the screen from one under the fold.
    let low = shape
        .iter()
        .map(|((dq, drow), _)| axial(*dq, *drow, r).y + half_h(r))
        .fold(f32::MIN, f32::max);
    // Far enough to hold any flower the board can throw at it, plus the rows
    // a tall board can show; the scan stops at the first fit long before.
    let rows = (members.len() as i32 + 2) * 4 + 64;
    let cols = (width / (1.5 * r)).ceil() as i32 + 2;

    // Candidates nearest the middle first, and with a flat top that has to be
    // worked out rather than read off the indices: a column carries half a
    // row of its own, so a constant `row` runs diagonally down the board.
    // Sorting the anchors by where the flower would actually land is what
    // keeps "the first that fits" meaning the nearest one (#132, #138).
    let mut anchors: Vec<(f32, f32, f32, f32, (i32, i32))> =
        Vec::with_capacity((rows * cols) as usize);
    for q in 0..=cols {
        let shift = q as f32 / 2.0;
        for row in (-shift.ceil() as i32)..=(rows - shift.floor() as i32) {
            let anchor = axial(q, row, r);
            let at = anchor + mid;
            // Anything on the screen beats anything under the fold, however
            // well the balance would come out: the grid has a top edge and
            // nothing may hang over it (#111), but it has no bottom edge at
            // all — the board simply scrolls — so without this a cluster
            // would settle off the end of a screen whose own rows were still
            // empty.
            let under = if anchor.y + low > height { 1.0 } else { 0.0 };
            // How near the middle the whole arrangement would come out, and
            // how near this flower itself would sit.
            let whole = (seated + at * weight) * (1.0 / (count + weight));
            let (bx, by) = (whole.x - board.x, whole.y - board.y);
            let (dx, dy) = (at.x - board.x, at.y - board.y);
            let (balance, tight) = (bx * bx + by * by, dx * dx + dy * dy);
            // Two clusters cannot both have the middle, so the balance alone
            // leaves pairs of equally good answers: the tighter packing wins
            // them, and then the higher and the lefter, so a board that
            // cannot be balanced at least lands the same way every time.
            anchors.push((under, balance, tight, at.y, (q, row)));
        }
    }
    anchors.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
            .then(a.4.cmp(&b.4))
    });

    for (_, _, _, _, (q, row)) in anchors {
        let fits = shape.iter().all(|((dq, drow), _)| {
            let cell = (dq + q, drow + row);
            let at = axial(cell.0, cell.1, r);
            at.x - half >= 0.0
                && at.x + half <= width
                && at.y - half_h(r) >= 0.0
                && !taken.contains(&cell)
                // The rim: a seat beside somebody else's is what makes
                // two clusters read as one (#121).
                && !DIRS.iter().any(|(nq, nrow)| taken.contains(&(cell.0 + nq, cell.1 + nrow)))
        });
        if fits {
            return Some(shape.iter().map(|((dq, drow), m)| ((dq + q, drow + row), *m)).collect());
        }
    }
    None
}

/// The six cells sharing a wall with a cell.
const DIRS: [(i32, i32); 6] = [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)];

/// The neighbour across each wall, in the order `corners` walks them (#128).
///
/// `corners` starts at the point due east and goes clockwise, so wall `i`
/// runs from corner `i` to corner `i + 1` and faces out at `60i + 30`
/// degrees. With a flat top those line up with `DIRS` as they stand (#132);
/// with the pointy top this had before, they were a step out.
const FACE_DIRS: [(i32, i32); 6] = DIRS;

/// A seated cell's wall, against the hairline an empty one gets.
const WALL: f32 = BORDER * 1.6;

/// How far inside a wall the second line runs, where two cells of one
/// cluster meet (#128).
///
/// The space is twice the lines either side of it (#129, #130, #131).
/// Derived rather than chosen: an absolute gap picked to look right on a
/// wide board reaches too far into a cell on a narrow one, where the cells
/// shrink with the window but the lines do not.
const SEAM: f32 = WALL * 3.0;

/// The inner line of a doubled wall: the same wall, a `SEAM` further in.
///
/// Taken off a whole inner hexagon rather than offset by hand, so it comes
/// out shorter than the wall it doubles and stops clear of the walls either
/// side instead of crossing them.
pub fn seam(center: Pt, r: f32, face: usize) -> Vec<Pt> {
    let inner = corners(center, r, GAP + SEAM * 2.0);
    vec![inner[face % 6], inner[(face + 1) % 6]]
}

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
    let at = axial(q, row, r);
    at.x - half_w(r) >= -0.5 && at.x + half_w(r) <= width + 0.5 && at.y - half_h(r) >= -0.5
}

fn has_free_neighbour(at: (i32, i32), taken: &[(i32, i32)]) -> bool {
    ring(1).into_iter().any(|(dq, dr)| !taken.contains(&(at.0 + dq, at.1 + dr)))
}

/// Lays the sessions out as clusters on a grid that fills `width`.
///
/// `height` is what the board can actually show, which is where the middle
/// of the screen is reckoned from (#138) as well as how far the empty grid
/// is drawn.
///
/// Sessions are grouped the way the board groups them (#41): panes of one
/// tab land in one flower. Flowers are placed outwards from the middle of
/// the screen, each leaving a ring of empty cells around it.
pub fn lay_out(model: &BoardModel, width: f32, height: f32) -> Comb {
    // `COLS` columns across the width. A flat-top column is three quarters
    // of a cell wide, with the last one's far point sticking out past that.
    let r = (width / (COLS * 1.5 + 0.5)).clamp(R_MIN, R_MAX);

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
    // So each flower takes the cell nearest the middle of the screen where
    // the whole of it fits on the board and nothing of it touches a flower
    // already placed (#138). The first one lands dead centre, where the eye
    // already is, and the rest gather round it rather than everything
    // huddling in the top left corner of a board that is mostly empty.
    //
    // Nearest-free rather than laying all of them out and then centring the
    // result: a seat only has to be found once, so a cluster keeps its cell
    // while its neighbours come and go. Centring the whole arrangement would
    // move every cell on the board each time one session ended.
    let mut seats: Vec<((i32, i32), usize)> = Vec::new();
    let mut taken: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    // Subagents take a cell of their own, next door to the session that
    // spawned them (#88) — and they take it with their flower rather than
    // after every flower is down (#145). Seated last, the nearest free cell
    // could be one in the rim between two clusters, where a colony is as
    // near one body as the other and reads as nobody's.
    let mut sub_seats: Vec<((i32, i32), usize)> = Vec::new();
    for (_, members) in &groups {
        // Everybody else's cells, taken before this flower puts down its
        // own: a colony may touch its own cluster and may not touch another.
        let strangers = taken.clone();
        let Some(placed) = seat_somewhere(members, &taken, width, height, r) else { continue };
        taken.extend(placed.iter().map(|(at, _)| *at));
        for (at, chip) in &placed {
            let has_subs = model.chips.get(*chip).is_some_and(|c| c.subs.iter().any(|s| !s.anim.gone));
            if !has_subs {
                continue;
            }
            // The nearest free cell, not merely an adjacent one: the middle
            // of a full flower has six neighbours and a session in every one
            // of them, and its subagents were being dropped on the floor
            // (#108).
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
                .find(|n| {
                    !taken.contains(n)
                        && on_screen(*n, r, width)
                        && !DIRS.iter().any(|(dq, drow)| strangers.contains(&(n.0 + dq, n.1 + drow)))

                });
            if let Some(n) = free {
                sub_seats.push((n, *chip));
                // Held like any other seat from here on, so the flower that
                // goes down next keeps its rim from the colony as well.
                taken.insert(n);
            }
        }
        seats.extend(placed);
    }
    // How far down the seated cells reach, in pixels: with a flat top a
    // column carries half a row of its own, so the row index alone no longer
    // says which cell is lowest.
    let seated_bottom = seats
        .iter()
        .chain(sub_seats.iter())
        .map(|((q, row), _)| axial(*q, *row, r).y)
        .fold(0.0f32, f32::max);

    // Every cell of the grid, occupied or not: the empties are half the point.
    //
    // Columns rather than rows: a flat-top cell's `x` depends only on `q`, so
    // a column is a straight line down the board and the row range it needs
    // is the one that covers the board's height at that column.
    let mut cells = Vec::new();
    let bottom = height.max(seated_bottom) + r * SQRT3 * PAD_ROWS;
    let mut q = 0;
    loop {
        let x = 1.5 * r * q as f32;
        if x - half_w(r) > width {
            break;
        }
        // `y = r * sqrt(3) * (row + q / 2)`, so the rows that cover a strip of
        // the board shift up by half a step for every column across.
        let shift = q as f32 / 2.0;
        let lo = (-(PAD_ROWS) - shift).floor() as i32;
        let hi = (bottom / (r * SQRT3) - shift).ceil() as i32;
        for row in lo..=hi {
            let center = axial(q, row, r);
            let chip = seats.iter().find(|(at, _)| *at == (q, row)).map(|(_, i)| *i);
            let subs = sub_seats.iter().find(|(at, _)| *at == (q, row)).map(|(_, i)| *i);
            cells.push(Cell { center, q, row, chip, subs, wave: 0.0 });
        }
        q += 1;
    }
    // How far each cell is from the nearest session, in cells. The clusters
    // are where the view starts and everything else follows outwards (#92).
    let hubs: Vec<Pt> = cells.iter().filter(|c| c.chip.is_some()).map(|c| c.center).collect();
    let pitch = r * SQRT3;
    for cell in &mut cells {
        cell.wave = hubs
            .iter()
            .map(|h| ((cell.center.x - h.x).powi(2) + (cell.center.y - h.y).powi(2)).sqrt() / pitch)
            .fold(f32::MAX, f32::min);
        if !cell.wave.is_finite() {
            cell.wave = 0.0;
        }
    }

    let height = cells.iter().map(|c| c.center.y).fold(0.0f32, f32::max) + half_h(r);
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
    /// What the work produced: the pull request, and the pages the session
    /// published alongside it (#137). One line, because both are things to
    /// open and a cell has no room for a fourth row.
    Output,
}

/// The band the session's row is bedded on: the cell's whole interior at
/// that height, points and all (#91, #133).
///
/// A rectangle would stop short of the two points a flat-top cell has at
/// either side and leave them empty. So the band takes the cell's own shape
/// where the cell is the nearer of the two: a six-sided slab, flat along the
/// top and bottom and coming to a point at each end exactly where the cell
/// does. It fills side to side without leaving the hexagon, which a
/// rectangle wide enough to reach the points could not do.
pub fn session_bed(center: Pt, r: f32) -> Vec<Pt> {
    let h = r * BED_H;
    // Inside the wall by its own width, so the band never paints over it.
    let inner = (r - (GAP + WALL * 2.0) / SQRT3).max(1.0);
    let (half_h, point) = (h / 2.0, inner);
    // The sides fall away by `1 / sqrt(3)` for every step off the middle.
    let shoulder = (inner - half_h / SQRT3).max(0.0);
    vec![
        Pt::new(center.x + point, center.y),
        Pt::new(center.x + shoulder, center.y + half_h),
        Pt::new(center.x - shoulder, center.y + half_h),
        Pt::new(center.x - point, center.y),
        Pt::new(center.x - shoulder, center.y - half_h),
        Pt::new(center.x + shoulder, center.y - half_h),
    ]
}


/// How tall that band is, against the cell's radius. Kept under a half, which
/// is where the hexagon starts tapering towards its points.
const BED_H: f32 = 0.3;

/// The floor the pull request's row is painted on: the cell from the top of
/// that row down to its own bottom edge (#139).
///
/// The session's band is a slab through the middle because a band is all it
/// can be — the cell goes on above and below it. The bottom row has nothing
/// under it, so it takes the whole of what is left: the two sides come in as
/// the hexagon does and the shape ends on the cell's own flat bottom, which
/// is what makes it read as the cell being filled from below rather than as
/// a second stripe.
pub fn output_bed(center: Pt, r: f32) -> Vec<Pt> {
    // Inside the wall by its own width, the same as the session's band, so
    // the border stays the border.
    let inner = (r - (GAP + WALL * 2.0) / SQRT3).max(1.0);
    // Where the row's own band begins, which is the line `hit_slot` divides
    // the cell on: what is painted is exactly what the press belongs to.
    let top = (ROW_DY[1] + ROW_DY[2]) / 2.0 * r;
    let bottom = inner * SQRT3 / 2.0;
    // The sides fall away by `1 / sqrt(3)` for every step off the middle.
    let half = |y: f32| (inner - y / SQRT3).max(0.0);
    vec![
        Pt::new(center.x + half(top), center.y + top),
        Pt::new(center.x + half(bottom), center.y + bottom),
        Pt::new(center.x - half(bottom), center.y + bottom),
        Pt::new(center.x - half(top), center.y + top),
    ]
}

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
    /// How wide the pages mark is drawn after the label, or zero where the
    /// row has none to tell about (#137).
    pub made: f32,
    /// How many pages, where there is more than one to count. The mark alone
    /// says "there is a page"; this says how many.
    pub tally: String,
}

pub fn row_box(r: f32, row: usize, label: &str, extras: Extras) -> RowBox {
    let dy = ROW_DY[row] * r;
    let scale = if row == 1 { r * 0.021 } else { r * 0.016 };
    let tall = font::height(scale).max(if row == 1 { r * 0.17 } else { r * 0.13 });
    row_box_in(row_room(r, dy, tall), r, row, label, extras)
}

/// The same row, given the room it has instead of working it out from a
/// hexagon (#140).
///
/// The blobs are round and merge into one another, so what a row has to play
/// with is a chord of a circle rather than a slice of a cell — but what it
/// does with the room, down to which marks come out of the label's share, is
/// the same thing said in the same order.
pub fn row_box_in(room: f32, r: f32, row: usize, label: &str, extras: Extras) -> RowBox {
    let dy = ROW_DY[row] * r;
    let scale = if row == 1 { r * 0.021 } else { r * 0.016 };
    let mark = if row == 1 { r * 0.17 } else { r * 0.13 };
    let room = room - ROW_PAD * 2.0;
    let check = if extras.check { font::height(scale) * CHECK_H } else { 0.0 };
    let origin = if extras.origin { mark * crate::render::scene::ORIGIN_SIZE } else { 0.0 };
    // A second mark, after the label rather than before it: the row speaks
    // for its pull request and the pages come after, the way they do in the
    // list the row drops (#137).
    let made = if extras.made > 0 { mark } else { 0.0 };
    let tally = if extras.made > 1 { extras.made.to_string() } else { String::new() };
    let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
    let taken = mark + MARK_GAP + span(origin) + span(check) + span(made) + span(font::measure(&tally, scale));
    RowBox { dy, scale, mark, origin, label: fit(label, scale, room - taken), check, room, made, tally }
}

/// What a row carries besides its own mark and label.
#[derive(Clone, Copy, Debug, Default)]
pub struct Extras {
    /// A second mark saying where the session is running (#122).
    pub origin: bool,
    /// Review has signed off on this row's pull request (#116).
    pub check: bool,
    /// How many pages this session published, for the row that is already
    /// speaking for something else (#137). Zero where the row has none to
    /// tell about, or where the pages *are* what the row is showing.
    pub made: usize,
}

/// The approval tick, against the cap height of the row it sits in. A hair
/// under, so it reads as a mark beside the number rather than as a letter of
/// it. (#116)
const CHECK_H: f32 = 0.95;

/// How wide the cell's interior is at a row, taking the row's own height into
/// account: a flat-top hexagon is widest along its own middle and narrows
/// every step away from it, so what matters is the width at the row's far
/// edge rather than at its middle.
fn row_room(r: f32, dy: f32, height: f32) -> f32 {
    room_at(r, dy, height, GAP)
}

/// Width available inside a hex of radius `r` at height `dy`, for something
/// `height` tall. Shared with the mini hexes of a sub-grid, which are drawn to
/// a gap of their own (#94, #95).
fn room_at(r: f32, dy: f32, height: f32, gap: f32) -> f32 {
    let inner = r - gap / SQRT3;
    let y = dy.abs() + height / 2.0;
    // Flat top: the sides run from the point at `inner` down to `inner / 2`
    // at the flat, so the half-width falls away by `1 / sqrt(3)` per step up
    // or down. There is no band of constant width as there was with a pointy
    // top — the widest the cell ever is, is the one line through its middle.
    let half = (inner - y / SQRT3).max(0.0);
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
pub const ROW_DY: [f32; 3] = [-0.42, 0.0, 0.42];

/// A colour made to read against whatever it is being drawn on (#140).
///
/// The comb draws straight onto the board, so that is what its colours have
/// to clear (#86); the blobs put their rows on a body of their own. Which way
/// to mix is the background's own question: against a dark body a colour has
/// to go lighter and against the white board darker, and the palette's two
/// ends are the only colours guaranteed to be there to go towards.
pub fn legible(c: Rgba, on: Rgba, pal: Palette) -> Rgba {
    let toward = if theme::contrast(pal.bg, on) > theme::contrast(pal.ink, on) { pal.bg } else { pal.ink };
    // Laid on first: contrast is a question about two colours, and a
    // see-through one does not answer it until it is standing on something
    // (#139). Ink at half on the white board reads as grey and has to be
    // measured as grey; on a dark body it reads as nothing at all.
    theme::readable(theme::over(c, on), on, toward, READABLE)
}

/// A pull request's colour as something to paint with rather than write in
/// (#139): laid on the board first, since a see-through colour is not the
/// colour it was given as, and then made to hold a label drawn in the
/// board's own colour.
pub fn floor_color(c: Rgba, pal: Palette) -> Rgba {
    legible(theme::over(c, pal.bg), pal.bg, pal)
}

/// Whether anything in this cell is waiting on a person (#90, #100).
fn wants_a_person(info: &SessionInfo) -> bool {
    info.phase() == Phase::NeedsUser || info.prs.last().is_some_and(pr_needs_action)
}

/// The same statement as `body`'s colour, for a board that prints rather
/// than paints (#146, #149): the screen the whole cell is laid down with.
///
/// Written here beside the colour because they are one statement in two
/// alphabets, and a board that had them in two places would sooner or later
/// say different things in each. Solid for work going on, warning stripes
/// for anything waiting on a person, and a fine half-tone for a session
/// sitting idle.
pub fn body_screen(info: &SessionInfo) -> Screen {
    if wants_a_person(info) {
        return Screen::Slash { width: 4.0, gap: 4.0 };
    }
    match info.phase() {
        Phase::Working => Screen::Solid,
        Phase::Idle | Phase::NeedsUser => Screen::Check { size: 2.0 },
    }
}

/// What the whole cell speaks in, and the beat the session's own row keeps.
///
/// Colour and blink answer different questions (#100). The colour speaks for
/// the whole cell: anything in here wants a person. The blink says who is
/// waiting, and only the session's own asking moves the cell itself — a red
/// check or an unstarted issue blinks on its own row, where the thing that
/// needs doing actually is. (#90)
pub fn body(info: &SessionInfo, pal: Palette, t: f32) -> (Rgba, f32) {
    let needs = wants_a_person(info);
    let beat = if info.phase() == Phase::NeedsUser { theme::pulse_at(t) } else { 1.0 };
    // Opaque, because the session's row is drawn on this same colour and a
    // label has to read on top of it (#89).
    let color = if needs {
        theme::with_alpha(legible(pal.alarm, pal.bg, pal), beat)
    } else {
        match info.phase() {
            Phase::Working => pal.ink,
            Phase::Idle | Phase::NeedsUser => theme::lerp(pal.bg, pal.ink, 0.55),
        }
    };
    (color, beat)
}

/// Whether a row is the one waiting, as a beat to draw it on.
pub fn row_beat(info: &SessionInfo, row: usize, t: f32) -> f32 {
    let waiting = match row {
        0 => info.ticket.as_ref().is_some_and(ticket_needs_action),
        2 => info.prs.last().is_some_and(pr_needs_action) || !info.prs_unanswered.is_empty(),
        _ => false,
    };
    if waiting { theme::pulse_at(t) } else { 1.0 }
}

/// The three lines a session shows, top to bottom, in the colours they read
/// in on `on` — the board for the comb, a blob's own body for the other view
/// (#140).
///
/// Linear on top, the session in the middle, what the work produced at the
/// bottom. Always in that order, so a column of them can be read down.
pub fn slots(info: &SessionInfo, pal: Palette, on: Rgba) -> [Option<SlotInk>; 3] {
    let read = |c: Rgba| legible(c, on, pal);
    [
        info.ticket.as_ref().map(|t| SlotInk {
            mark: logos::Mark::Linear,
            label: t.key.clone(),
            color: read(ticket_color(t, pal)),
            check: false,
            origin: None,
            made: 0,
            fill: None,
            screen: Screen::Solid,
        }),
        Some(SlotInk {
            mark: logos::Mark::Claude,
            label: info.label(),
            color: pal.bg,
            check: false,
            // Where this session is running, beside the mark that says what
            // it is (#122).
            origin: info.seat.as_ref().and_then(|s| logos::Mark::host(s.host)),
            made: 0,
            // The session's bed is the cell's own colour and is drawn for the
            // whole cell, not for this row alone (#89).
            fill: None,
            screen: Screen::Solid,
        }),
        // The answered one if there is one, otherwise the number GitHub would
        // not speak for, drawn quietly and blinking. With no pull request at
        // all the line is the newest page's, under its own name: a cell with
        // one thing to show shows it rather than leaving the line empty and
        // the page unreachable (#137).
        info.prs
            .last()
            .map(|p| SlotInk {
                mark: logos::Mark::GitHub,
                label: p.label(),
                // The state is the floor now, so the letters stand on it in
                // the board's own colour, the way the session's do (#139).
                color: pal.bg,
                check: p.signed_off(),
                origin: None,
                made: info.artifacts.len(),
                fill: Some(floor_color(pr_color(p, pal), pal)),
                screen: pr_screen(p),
            })
            .or_else(|| {
                info.prs_unanswered.last().map(|id| SlotInk {
                    mark: logos::Mark::GitHub,
                    label: format!("#{}", id.number),
                    color: theme::with_alpha(read(pal.ink), 0.5),
                    check: false,
                    origin: None,
                    made: info.artifacts.len(),
                    // Nothing to paint it with: a number GitHub would not
                    // answer for has no state, and a floor would be the cell
                    // claiming one. It stays the quiet, blinking line it was
                    // (#112).
                    fill: None,
                    screen: Screen::Solid,
                })
            })
            .or_else(|| {
                info.artifacts.last().map(|a| SlotInk {
                    mark: logos::Mark::Artifact,
                    label: a.label(),
                    color: read(theme::lerp(pal.bg, pal.ink, 0.75)),
                    check: false,
                    origin: None,
                    // The mark at the front is already the pages' own, so a
                    // second one after the name would be saying it twice.
                    made: 0,
                    // What was asked for is the pull request's state as a
                    // floor (#139); a page has no state to paint.
                    fill: None,
                    screen: Screen::Solid,
                })
            }),
    ]
}

/// One row's marks and letters, around the point it is centred on.
///
/// Both views lay the same line out the same way — mark, where it is running,
/// the label, review's tick, the pages — and differ only in the room they
/// have to do it in (#140).
/// How much of the row the drawing actually covers: the mark, the label and
/// everything that rides after it. What wants it is a board that has to put
/// something behind the letters rather than beside them (#146).
pub fn row_width(rb: &RowBox) -> f32 {
    let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
    let text_w = font::measure(&rb.label, rb.scale);
    let tally_w = font::measure(&rb.tally, rb.scale);
    rb.mark + MARK_GAP + span(rb.origin) + text_w + span(rb.check) + span(rb.made) + span(tally_w)
}

pub fn row_shapes(at: Pt, rb: &RowBox, slot: &SlotInk, color: Rgba) -> Vec<Shape> {
    let RowBox { scale, mark, origin, label, check, made, tally, .. } = rb;
    let (scale, mark, origin, check, made) = (*scale, *mark, *origin, *check, *made);
    let text_w = font::measure(label, scale);
    let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
    let (tick, badge) = (span(check), span(origin));
    let tally_w = font::measure(tally, scale);
    let left = at.x - row_width(rb) / 2.0;
    let mut out = vec![Shape::Mark {
        mark: slot.mark,
        size: mark,
        angle: 0.0,
        center: Pt::new(left + mark / 2.0, at.y),
        color,
    }];
    // The origin rides between the mark and the label, the way it does
    // inside a chip on the other view (#122).
    if let (Some(badge_mark), true) = (slot.origin, origin > 0.0) {
        out.push(Shape::Mark {
            mark: badge_mark,
            size: origin,
            angle: 0.0,
            center: Pt::new(left + mark + MARK_GAP + origin / 2.0, at.y),
            color,
        });
    }
    out.push(Shape::Text {
        text: label.clone(),
        scale,
        stroke: 1.0,
        angle: 0.0,
        center: Pt::new(left + mark + MARK_GAP + badge + text_w / 2.0, at.y),
        color,
    });
    // Review's tick, after the number it signed off on (#116). The row has no
    // fill to say it with, so it is said in a mark.
    if check > 0.0 {
        let tick_at = Pt::new(left + mark + MARK_GAP + badge + text_w + MARK_GAP + check / 2.0, at.y);
        out.push(Shape::Stroke {
            pieces: scene::check_mark(tick_at, check),
            width: (check * 0.2).max(0.7),
            color,
        });
    }
    // The pages this session published, after everything the row was already
    // saying (#137). A mark on its own means there is one; the count follows
    // when there is more than one. Drawn in the row's own colour, because it
    // belongs to the same line.
    if made > 0.0 {
        let pages_at = left + mark + MARK_GAP + badge + text_w + tick + MARK_GAP;
        out.push(Shape::Mark {
            mark: logos::Mark::Artifact,
            size: made,
            angle: 0.0,
            center: Pt::new(pages_at + made / 2.0, at.y),
            color,
        });
        if tally_w > 0.0 {
            out.push(Shape::Text {
                text: tally.clone(),
                scale,
                stroke: 1.0,
                angle: 0.0,
                center: Pt::new(pages_at + made + MARK_GAP + tally_w / 2.0, at.y),
                color,
            });
        }
    }
    out
}

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
        Slot::Output
    } else {
        Slot::Session
    };
    Some((cell.chip?, slot))
}

/// Where one of a cell's rows sits, and the box it has to itself, in the
/// grid's own coordinates (#134).
///
/// The same band `hit_slot` answers for, as a rectangle. Taken at the band's
/// far edge, which is where the cell is narrowest: a flat-top hexagon is
/// widest along its middle line and falls away by `1 / sqrt(3)` every step
/// from it, so a box measured at its own middle would poke out of the sides
/// at its corners.
pub fn slot_box(comb: &Comb, chip: usize, slot: Slot) -> Option<(Pt, f32, f32)> {
    let cell = comb.cells.iter().find(|c| c.chip == Some(chip))?;
    let row = match slot {
        Slot::Ticket => 0,
        Slot::Session => 1,
        Slot::Output => 2,
    };
    let dy = ROW_DY[row] * comb.r;
    // Every band is the same height: the rows are evenly spaced, so each one
    // reaches halfway to its neighbour on either side.
    let h = (ROW_DY[2] - ROW_DY[1]) * comb.r;
    let w = row_room(comb.r, dy, h);
    (w > 0.0).then(|| (Pt::new(cell.center.x, cell.center.y + dy), w, h))
}

/// What goes on one of a cell's three lines.
#[derive(Clone, Debug)]
pub struct SlotInk {
    pub mark: logos::Mark,
    pub label: String,
    pub color: Rgba,
    /// Review has signed off on this pull request (#116). Only the GitHub
    /// row ever sets it.
    pub check: bool,
    /// Where the session is running (#122). Only the middle row sets it:
    /// the issue and the pull request are not anywhere in particular.
    pub origin: Option<logos::Mark>,
    /// Pages this session published, told after the row's own label (#137).
    pub made: usize,
    /// The colour the row is painted on, where the row says its state by
    /// being filled rather than by the colour of its own letters (#139).
    /// Opaque, because the label is drawn in the board's colour on top of
    /// it.
    pub fill: Option<Rgba>,
    /// The same statement for a board that prints rather than paints (#146,
    /// #149): the weave the floor is laid down with.
    pub screen: Screen,
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

    // Which cluster sits in each seated cell, so a wall can be asked whether
    // the cell across it is a neighbour or a cluster-mate (#128).
    let mates: std::collections::HashMap<(i32, i32), &str> = comb
        .cells
        .iter()
        .filter_map(|c| Some(((c.q, c.row), model.chips.get(c.chip?)?.info.group.as_str())))
        .collect();

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
                out.push(Shape::poly(corners(at, mr, SUB_GAP), fade(color)));
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
        // The cell's own colour, which its border and the session's bed are
        // drawn in, and the beat the session's row keeps (#90, #100). The
        // comb draws straight onto the board, so every colour in here is the
        // one that reads against the board (#86).
        let (edge, beat) = body(info, pal, t);
        out.push(Shape::Stroke {
            pieces: vec![closed(corners(center, r, GAP))],
            width: WALL,
            color: fade(edge),
        });
        for (row, slot) in slots(info, pal, pal.bg).iter().enumerate() {
            let Some(slot) = slot else { continue };
            let extras = Extras { origin: slot.origin.is_some(), check: slot.check, made: slot.made };
            let rb = row_box(r, row, &slot.label, extras);
            // The session's label lies on the bed, so it blinks with it;
            // the other two answer for themselves. A row that says its state
            // by being painted does not blink at all: the cell's own border
            // is already going red and pulsing for a pull request that needs
            // somebody (#90), and a floor fading in and out under letters
            // drawn in the board's colour would take the letters with it.
            let slot_beat = if row == 1 {
                beat
            } else if slot.fill.is_some() {
                1.0
            } else {
                row_beat(info, row, t)
            };
            // The session wears the cell's own colour as a bed, so the middle
            // row reads as the chip it is on the other view (#89).
            if row == 1 {
                out.push(Shape::poly(session_bed(center, r), fade(edge)));
            }
            // The pull request's state as the cell's floor (#139).
            if let Some(fill) = slot.fill {
                out.push(Shape::poly(output_bed(center, r), fade(fill)));
            }
            let at = Pt::new(center.x, center.y + rb.dy);
            out.extend(row_shapes(at, &rb, slot, fade(theme::with_alpha(slot.color, slot_beat))));
        }

        // A wall shared with a cell of the same cluster is drawn twice
        // (#128). Adjacency is already what says two sessions belong together
        // (#121); doubling the wall is that said in the drawing rather than
        // left to be inferred from a gap.
        //
        // Last, after the rows: both beds reach the walls they are under, and
        // a seam painted over is a cluster the eye has to infer again (#139).
        for (face, (dq, drow)) in FACE_DIRS.into_iter().enumerate() {
            let next = (cell.q + dq, cell.row + drow);
            if mates.get(&next).is_some_and(|g| *g == info.group.as_str()) {
                out.push(Shape::Stroke {
                    pieces: vec![seam(center, r, face)],
                    width: WALL,
                    color: fade(edge),
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

/// How a pull request's floor is screened, where the board has one ink and
/// cannot say it in colour (#146, #149).
///
/// Told apart by grain rather than by darkness: the six floor colours differ
/// by hue and `floor_color` has already pinned every one of them to the same
/// contrast against the board, so as tones they are identical. Each is a
/// different weave instead, and none of them is the cell's own (`body_screen`
/// above) — the floor has to be a floor and not the rest of the body.
fn pr_screen(p: &crate::pr::Pr) -> Screen {
    use crate::pr::Look;
    if p.running() {
        // In flight: a thin hatch with air in it.
        return Screen::Slash { width: 1.0, gap: 3.0 };
    }
    match p.look() {
        // Packed tight, and the cell is already striped for it (#90).
        Look::Failing => Screen::Slash { width: 2.0, gap: 1.0 },
        // Landed: the blocks of a tiled floor.
        Look::Merged => Screen::Check { size: 4.0 },
        Look::Open => Screen::Slash { width: 2.0, gap: 4.0 },
        // Barely there, and barely there at all.
        Look::Draft => Screen::Slash { width: 1.0, gap: 1.0 },
        Look::Closed => Screen::Slash { width: 1.0, gap: 6.0 },
    }
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

    /// What a board with a few sessions on it looks like: they gather in the
    /// middle of the screen, where the eye already is, rather than in the top
    /// left corner of a grid that is mostly empty (#138).
    #[test]
    fn the_clusters_gather_in_the_middle_of_the_screen() {
        let shapes: [&[(&str, usize)]; 5] = [
            &[("a", 1)],
            &[("a", 3)],
            &[("a", 2), ("b", 2)],
            &[("a", 1), ("b", 1), ("c", 1)],
            &[("a", 2), ("b", 3), ("c", 2), ("d", 1), ("e", 1)],
        ];
        for (w, h) in [(980.0f32, 620.0f32), (520.0, 900.0), (1600.0, 480.0)] {
            for shape in shapes {
                let model = board(shape);
                let comb = lay_out(&model, w, h);
                let seats: Vec<Pt> =
                    comb.cells.iter().filter(|c| c.chip.is_some()).map(|c| c.center).collect();
                let n = seats.len() as f32;
                let mid = seats.iter().fold(Pt::new(0.0, 0.0), |a, b| a + *b) * (1.0 / n);
                let limit = match shape.len() {
                    // One cluster has the middle, and misses it only by the
                    // lattice: the farthest any point can be from the nearest
                    // cell of a grid is a cell's own radius.
                    1 => comb.r,
                    // Two cannot share it, and the rim holds them a clear two
                    // cells apart (#121), so the pair straddles it instead.
                    2 => comb.r * 2.0,
                    // Three or more can stand round it, and each is seated
                    // where it brings the whole arrangement's middle nearest
                    // the screen's, so being a cell out is not an excuse the
                    // board has any more.
                    _ => comb.r * 0.9,
                };
                let (dx, dy) = (mid.x - w / 2.0, mid.y - h / 2.0);
                assert!(dx.abs() <= limit, "{w}x{h} {shape:?}: {dx} off the middle across");
                assert!(dy.abs() <= limit, "{w}x{h} {shape:?}: {dy} off the middle down");
            }
        }
    }

    /// A cluster too tall for the board drops rather than hanging off the
    /// top: the grid has a first row and nothing is drawn above it (#111).
    #[test]
    fn a_cluster_too_tall_to_centre_sits_under_the_top_instead() {
        let model = board(&[("a", 12)]);
        let comb = lay_out(&model, 980.0, 320.0);
        let top = comb
            .cells
            .iter()
            .filter(|c| c.chip.is_some())
            .map(|c| c.center.y - half_h(comb.r))
            .fold(f32::MAX, f32::min);
        assert!(top >= -0.5, "a cell starts at {top}, above the grid");
        assert!(top <= comb.r * SQRT3, "and no lower than it has to: {top}");
    }

    /// The grid has a top edge and nothing may hang over it (#111), but it
    /// has no bottom edge at all — the board scrolls — so a cluster that the
    /// balance pulls downwards would settle under the fold with the screen
    /// still half empty. Being seen comes first (#138).
    #[test]
    fn a_cluster_takes_a_spot_on_the_screen_over_a_better_one_under_the_fold() {
        let (w, h) = (1280.0f32, 360.0f32);
        let model = board(&[("a", 2), ("b", 1), ("c", 1)]);
        let comb = lay_out(&model, w, h);
        for cell in comb.cells.iter().filter(|c| c.chip.is_some()) {
            let low = cell.center.y + half_h(comb.r);
            assert!(low <= h + 0.5, "a cell reaches {low} of a {h} screen that had room on it");
        }
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

    /// A colony answers to one session, so it must never sit where it is as
    /// near somebody else's body as its own (#145). Seated after every
    /// flower was down, the nearest free cell was very often exactly that:
    /// the one-cell rim that is there to keep two clusters apart (#121).
    #[test]
    fn a_subagent_cell_never_sits_beside_another_cluster() {
        use crate::model::SubagentInfo;
        let mut model = BoardModel::new();
        let mut list = Vec::new();
        for g in 0..4u32 {
            for k in 0..3u32 {
                let mut s = SessionInfo::synthetic(g * 10 + k + 1, &format!("S-{g}-{k}"), Phase::Working);
                s.group = format!("warp-tab:1-{g}");
                // One in each cluster has agents out, so the colonies land
                // among the flowers rather than off on their own.
                if k == 0 {
                    s.subagents = vec![SubagentInfo::synthetic(1, "Explore", "", true)];
                }
                list.push(s);
            }
        }
        model.apply(list, Instant::now());
        model.settle();
        for width in [620.0f32, 980.0, 1280.0] {
            let comb = lay_out(&model, width, 620.0);
            let group = |cell: &Cell| {
                cell.chip.or(cell.subs).and_then(|i| model.chips.get(i)).map(|c| c.info.group.clone())
            };
            let colonies = comb.cells.iter().filter(|c| c.subs.is_some());
            assert_eq!(colonies.clone().count(), 4, "{width}px: a cluster lost its agents");
            for cell in colonies {
                let mine = group(cell);
                for (dq, drow) in DIRS {
                    let Some(n) = comb.cells.iter().find(|c| c.q == cell.q + dq && c.row == cell.row + drow)
                    else {
                        continue;
                    };
                    let theirs = group(n);
                    assert!(
                        theirs.is_none() || theirs == mine,
                        "{width}px: a colony of {mine:?} sits against {theirs:?}"
                    );
                }
                // And it is still next door to its own, which is the whole
                // of what says whose it is.
                let parent = comb.cells.iter().find(|c| c.chip == cell.subs).expect("the session");
                let d = ((cell.center.x - parent.center.x).powi(2) + (cell.center.y - parent.center.y).powi(2)).sqrt();
                assert!(d < comb.r * 2.0, "{width}px: a colony sits {d} from its session");
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
        assert_eq!(hit_slot(&comb, at + Pt::new(0.0, comb.r * 0.42)), Some((0, Slot::Output)));
        assert_eq!(hit_slot(&comb, at + Pt::new(0.0, comb.r * 3.0)), None, "outside the cell is nothing");
    }

    /// A row's box is the row: what it covers answers for that row, and all
    /// four of its corners are inside the cell (#134).
    #[test]
    fn a_rows_box_covers_its_own_row_and_nothing_outside_the_cell() {
        let model = board(&[("warp-tab:1-1", 1)]);
        let comb = lay_out(&model, 980.0, 620.0);
        let center = comb.cells.iter().find(|c| c.chip == Some(0)).unwrap().center;
        for slot in [Slot::Ticket, Slot::Session, Slot::Output] {
            let (at, w, h) = slot_box(&comb, 0, slot).expect("every row has a box");
            assert_eq!(hit_slot(&comb, at), Some((0, slot)), "{slot:?}: the box is over another row");
            for sx in [-1.0, 1.0f32] {
                for sy in [-1.0, 1.0f32] {
                    let corner = at + Pt::new(sx * w / 2.0, sy * h / 2.0);
                    assert!(
                        inside_hex(corner, center, comb.r, GAP),
                        "{slot:?}: the corner at ({sx}, {sy}) is outside the cell"
                    );
                    // A hair inside: the far edge is the line two rows share,
                    // and which side of it a point falls on is `hit_slot`'s
                    // business rather than the box's.
                    let inside = at + Pt::new(sx * w / 2.0, sy * h * 0.49);
                    assert_eq!(
                        hit_slot(&comb, inside).map(|(_, s)| s),
                        Some(slot),
                        "{slot:?}: the corner at ({sx}, {sy}) belongs to another row"
                    );
                }
            }
        }
        assert!(slot_box(&comb, 999, Slot::Output).is_none(), "a cell nobody sits in has no box");
    }

    /// The session's bed runs wall to wall inside the cell and touches
    /// nothing else (#91).
    #[test]
    fn the_session_bed_fills_the_cell_from_side_to_side() {
        for r in [46.0f32, 70.0, 104.0] {
            let center = Pt::new(300.0, 300.0);
            let bed = session_bed(center, r);
            assert_eq!(bed.len(), 6, "the band takes the cell's own shape at its ends (#133)");
            for p in &bed {
                assert!(inside_hex(*p, center, r, GAP), "r={r}: the bed pokes out of its cell");
            }

            // Side to side: its ends reach the cell's points, give or take
            // the wall it has to stay off. A rectangle wide enough to do
            // that would have left the hexagon at its corners, which is the
            // whole reason this has six sides (#133).
            let h = r * BED_H;
            let reach = bed.iter().map(|p| p.x - center.x).fold(f32::MIN, f32::max);
            let point = corners(center, r, GAP).iter().map(|p| p.x - center.x).fold(f32::MIN, f32::max);
            assert!(point - reach < WALL * 2.0, "r={r}: the bed stops {} short of the point", point - reach);
            let rect_corner = Pt::new(center.x + reach, center.y + h / 2.0);
            assert!(
                !inside_hex(rect_corner, center, r, GAP),
                "r={r}: a rectangle that wide would still have fitted, so the shaping buys nothing"
            );

            // And it is a band, not a wedge: flat along the top and bottom.
            let flats: Vec<f32> = bed.iter().map(|p| p.y - center.y).collect();
            assert!(flats.iter().filter(|y| (**y - h / 2.0).abs() < 0.01).count() == 2);
            assert!(flats.iter().filter(|y| (**y + h / 2.0).abs() < 0.01).count() == 2);
        }
    }

    /// What is painted is what is pressed (#139): the floor starts on the
    /// line `hit_slot` divides the cell on and runs to the cell's own bottom,
    /// which is what makes it read as the cell filled from below rather than
    /// as a second stripe.
    #[test]
    fn the_pull_request_floor_takes_the_bottom_of_the_cell() {
        for r in [46.0f32, 70.0, 104.0] {
            let center = Pt::new(300.0, 300.0);
            let bed = output_bed(center, r);
            assert_eq!(bed.len(), 4, "a trapezoid: the cell from one line down to its floor");
            for p in &bed {
                assert!(inside_hex(*p, center, r, GAP), "r={r}: the floor pokes out of its cell");
            }
            let dy: Vec<f32> = bed.iter().map(|p| p.y - center.y).collect();
            let (top, low) = (dy.iter().cloned().fold(f32::MAX, f32::min), dy.iter().cloned().fold(f32::MIN, f32::max));
            let press = (ROW_DY[1] + ROW_DY[2]) / 2.0 * r;
            assert!((top - press).abs() < 0.01, "r={r}: the floor starts at {top}, the press at {press}");
            // Down to the wall, with only the wall's own width left under it.
            let wall = corners(center, r, GAP).iter().map(|p| p.y - center.y).fold(f32::MIN, f32::max);
            assert!(wall - low < WALL * 2.0, "r={r}: the floor stops {} short of the bottom", wall - low);
            // And as wide as the cell is down there: the bottom edge of the
            // hexagon, not a rectangle's corner hanging over the sides.
            let wide = bed.iter().filter(|p| (p.y - center.y - low).abs() < 0.01).count();
            assert_eq!(wide, 2, "r={r}: the floor ends flat, the way the cell does");
        }
    }

    /// The pull request says its state by what it is painted on rather than
    /// by the colour of its own letters (#139), so the letters are the
    /// board's colour and the floor has to hold them: a see-through colour
    /// laid on the board is not the colour it was given as.
    #[test]
    fn a_pull_request_is_painted_on_the_floor_of_its_cell() {
        use crate::pr::{Look, Pr};
        let scene = |prs: Vec<Pr>, pal: Palette| {
            let mut model = board(&[("warp-tab:1-1", 1)]);
            model.chips[0].info.prs = prs;
            let comb = lay_out(&model, 980.0, 620.0);
            // Well past the arrival wave, so the cell is fully drawn (#92).
            build_shapes(&model, &comb, pal, 0.0, Pt::new(0.0, 0.0), 3.0)
        };
        // The only four-sided fill in the scene: a cell's wall is a stroked
        // hexagon, and the session's band and a subagent have six sides.
        let floor = |shapes: &[Shape]| -> Option<Rgba> {
            shapes.iter().find_map(|s| match s {
                Shape::Poly { points, color, .. } if points.len() == 4 => Some(*color),
                _ => None,
            })
        };
        for pal in [theme::PALETTE, theme::ORANGE, theme::DARK] {
            for look in Look::ALL {
                let shapes = scene(vec![Pr::synthetic(7, look)], pal);
                let paint = floor(&shapes).unwrap_or_else(|| panic!("{look:?}: the row has no floor"));
                assert_eq!(paint.a, 1.0, "{look:?}: the board shows through the floor");
                assert!(
                    theme::contrast(paint, pal.bg) >= READABLE - 0.01,
                    "{look:?}: a label in the board's own colour would not read on this floor"
                );
                let label = shapes.iter().find_map(|s| match s {
                    Shape::Text { text, color, .. } if text == "#7" => Some(*color),
                    _ => None,
                });
                assert_eq!(label, Some(pal.bg), "{look:?}: the number stands on the floor");
            }
            // Nothing to paint it with: a cell whose bottom line is a page
            // (#137) or a number GitHub would not answer for (#112) has no
            // state to say, and says none.
            assert!(floor(&scene(vec![], pal)).is_none(), "an empty row painted a floor anyway");
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
        // Every combination of the three things that come out of a label's
        // room: review's tick (#116), the origin badge (#122), and the pages
        // mark with its count (#137) — the mark alone and the mark with a
        // number beside it are different widths.
        let cases: Vec<Extras> = [false, true]
            .into_iter()
            .flat_map(|check| [false, true].into_iter().map(move |origin| (check, origin)))
            .flat_map(|(check, origin)| {
                [0usize, 1, 4].into_iter().map(move |made| Extras { origin, check, made })
            })
            .collect();
        for r in [R_MIN, 70.0, R_MAX] {
            let center = Pt::new(500.0, 500.0);
            for row in 0..3 {
                for label in labels {
                    for extras in cases.iter().copied() {
                        let b = row_box(r, row, label, extras);
                        if b.label.is_empty() {
                            continue;
                        }
                        let text_w = font::measure(&b.label, b.scale);
                        let span = |w: f32| if w > 0.0 { w + MARK_GAP } else { 0.0 };
                        let tally_w = font::measure(&b.tally, b.scale);
                        let w = b.mark
                            + MARK_GAP
                            + span(b.origin)
                            + text_w
                            + span(b.check)
                            + span(b.made)
                            + span(tally_w);
                        let h = font::height(b.scale).max(b.mark).max(b.check).max(b.origin);
                        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                            let corner = Pt::new(center.x + sx * w / 2.0, center.y + b.dy + sy * h / 2.0);
                            assert!(
                                inside_hex(corner, center, r, GAP),
                                "r={r} row={row} {extras:?} {label:?}: {:?} reaches outside its cell",
                                b.label
                            );
                        }
                        assert_eq!(b.check > 0.0, extras.check, "the tick is there when it was asked for");
                        assert_eq!(b.origin > 0.0, extras.origin, "and so is the origin mark");
                        assert_eq!(b.made > 0.0, extras.made > 0, "and the pages mark when there are pages");
                        assert_eq!(
                            b.tally.is_empty(),
                            extras.made < 2,
                            "one page is the mark alone; the count is for the rest"
                        );
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

    /// A wall two cluster-mates share is drawn twice, and a wall facing
    /// anything else is drawn once (#128).
    #[test]
    fn a_shared_wall_is_a_double_line() {
        // The seam is the only two-point stroke in the scene: a cell wall is
        // a closed hexagon and the approval tick has three.
        let seams = |model: &BoardModel, width: f32| {
            let comb = lay_out(model, width, 900.0);
            build_shapes(model, &comb, theme::PALETTE, 0.0, Pt::new(0.0, 0.0), 1.0e3)
                .into_iter()
                .filter(|s| matches!(s, Shape::Stroke { pieces, .. } if pieces.len() == 1 && pieces[0].len() == 2))
                .count()
        };

        // One session has nobody to share a wall with.
        assert_eq!(seams(&board(&[("a", 1)]), 980.0), 0);
        // Two in one cluster share exactly one wall, which both of them draw.
        assert_eq!(seams(&board(&[("a", 2)]), 980.0), 2);
        // Three make a triangle: three shared walls, drawn from both sides.
        assert_eq!(seams(&board(&[("a", 3)]), 980.0), 6);
        // And separate clusters share nothing, however many there are:
        // nothing of one ever touches another (#121).
        assert_eq!(seams(&board(&[("a", 1), ("b", 1), ("c", 1), ("d", 1)]), 980.0), 0);
        assert_eq!(seams(&board(&[("a", 2), ("b", 2)]), 420.0), 4, "narrow boards too");

        // And a seam is drawn after what it crosses (#139). Both beds reach
        // the walls they lie under, so a seam drawn before them is a seam
        // painted over, and the cluster goes back to being inferred from the
        // gap it was given a drawing to replace.
        let mut model = board(&[("a", 2)]);
        for chip in &mut model.chips {
            chip.info.prs = vec![crate::pr::Pr::synthetic(7, crate::pr::Look::Merged)];
        }
        let comb = lay_out(&model, 980.0, 900.0);
        let shapes = build_shapes(&model, &comb, theme::PALETTE, 0.0, Pt::new(0.0, 0.0), 1.0e3);
        let last = |f: &dyn Fn(&Shape) -> bool| shapes.iter().rposition(f).expect("drawn");
        let seam = last(&|s: &Shape| {
            matches!(s, Shape::Stroke { pieces, .. } if pieces.len() == 1 && pieces[0].len() == 2)
        });
        let bed = last(&|s: &Shape| matches!(s, Shape::Poly { points, .. } if points.len() == 4));
        assert!(seam > bed, "a seam at {seam} is under the bed at {bed}");
    }

    /// Wall `i` of the drawing is the wall facing `FACE_DIRS[i]` (#128).
    ///
    /// The one thing here that can be wrong without looking wrong in a count:
    /// rotate the mapping and exactly as many seams are drawn, on the wrong
    /// walls. So it is pinned against the grid itself — the seam for a wall
    /// has to lean towards the cell across that wall, further than towards
    /// any of the other five.
    #[test]
    fn each_seam_faces_the_cell_it_is_shared_with() {
        let r = 70.0;
        let center = Pt::new(500.0, 500.0);
        for (face, (dq, drow)) in FACE_DIRS.into_iter().enumerate() {
            let line = seam(center, r, face);
            let mid = Pt::new((line[0].x + line[1].x) / 2.0, (line[0].y + line[1].y) / 2.0);
            let out = Pt::new(mid.x - center.x, mid.y - center.y);
            // How far the seam leans towards the cell one step that way.
            let toward = |q: i32, row: i32| {
                let n = axial(q, row, r);
                (out.x * n.x + out.y * n.y) / (n.x * n.x + n.y * n.y).sqrt().max(1.0)
            };
            let mine = toward(dq, drow);
            assert!(mine > 0.0, "face {face} leans away from its own neighbour");
            for (q, row) in DIRS {
                if (q, row) == (dq, drow) {
                    continue;
                }
                assert!(
                    mine > toward(q, row) + 1.0,
                    "face {face} leans towards {:?} as much as towards {:?}",
                    (q, row),
                    (dq, drow)
                );
            }
        }
    }

    /// The second line sits inside the wall it doubles, and stops short of
    /// the walls either side rather than crossing them (#128).
    #[test]
    fn the_second_line_stays_inside_its_cell() {
        for r in [R_MIN, 70.0, R_MAX] {
            let center = Pt::new(500.0, 500.0);
            let wall = corners(center, r, GAP);
            for face in 0..6 {
                let line = seam(center, r, face);
                assert_eq!(line.len(), 2);
                for p in &line {
                    assert!(inside_hex(*p, center, r, GAP), "r={r} face={face}: the seam left its cell");
                }
                // Shorter than the wall it doubles, at both ends.
                let span = |a: Pt, b: Pt| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                let outer = span(wall[face], wall[(face + 1) % 6]);
                assert!(span(line[0], line[1]) < outer, "r={r} face={face}: the seam is not inset");

                // The space between the two lines is a fixed share of their
                // own width (#129, #130). An absolute gap picked to look
                // right on a wide board reaches too far into a cell on a
                // narrow one, so it is derived from the line, not chosen.
                let mid = Pt::new((line[0].x + line[1].x) / 2.0, (line[0].y + line[1].y) / 2.0);
                let wall_mid =
                    Pt::new((wall[face].x + wall[(face + 1) % 6].x) / 2.0, (wall[face].y + wall[(face + 1) % 6].y) / 2.0);
                let space = span(mid, wall_mid) - WALL;
                assert!(
                    (space / WALL - 2.0).abs() < 0.01,
                    "r={r} face={face}: the space is {space} against a line of {WALL}"
                );
            }
        }
    }

    #[test]
    fn two_neighbours_leave_a_gap_between_them() {
        let r = 60.0;
        let a = Pt::new(0.0, 0.0);
        // The one neighbour a flat-top cell meets squarely is the one below
        // it, so the gap can be read straight off their two edges.
        let below = axial(0, 1, r);
        let bottom = corners(a, r, GAP).into_iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let top = corners(below, r, GAP).into_iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!((top - bottom - GAP).abs() < 0.01, "the gap should be {GAP}, got {}", top - bottom);

        // And every other neighbour is the same distance away, which is what
        // makes the gap a property of the grid rather than of one direction.
        let span = |p: Pt| (p.x * p.x + p.y * p.y).sqrt();
        let pitch = span(axial(0, 1, r));
        for (q, row) in DIRS {
            let d = span(axial(q, row, r));
            assert!((d - pitch).abs() < 0.01, "{:?} sits {d} away, not {pitch}", (q, row));
        }
        assert!(GAP > BORDER * 3.0, "a gap the width of a hairline is no gap at all");
    }
}
