//! Renderer-independent description of a board frame.
//!
//! `chip_draws` projects the model onto lanes, and `build_shapes` turns lanes
//! + chips into a flat list of `Shape`s. `render::paint` draws that list with
//! gpui; `render::svg` serialises it so layouts can be checked headlessly.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use crate::font;
use crate::geom::{Polyline, Pt, smoothstep};
use crate::model::{BoardModel, Phase, RowKind, Rows, TRAIL_ROWS, Target};
use crate::pr;
use crate::theme::{self, Palette, Rgba};

/// Spacing inside a pair of traces and the extra gap between pairs (#14).
pub const GAP: f32 = 22.0;
pub const PAIR_GAP: f32 = 24.0;
pub const LINE_W: f32 = 1.6;
/// Vertical drop of a diagonal. The first diagonal of lane `i` is
/// `DIAG + i * DIAG_GROWTH` (clamped to `DIAG_MIN` and to what fits before
/// the next stripe), so the traces fan out from top to bottom (#45); every
/// later stripe uses plain `DIAG`.
pub const DIAG: f32 = 128.0;
pub const DIAG_GROWTH: f32 = 12.0;
pub const DIAG_MIN: f32 = 24.0;
pub const CORNER_R: f32 = 8.0;
pub const TOP_PAD: f32 = 54.0;
pub const BOTTOM_PAD: f32 = 34.0;
/// Empty lanes drawn below the last session (#45): the board is exactly
/// `LEAD_LANES + sessions + TRAIL_LANES` lanes, never padded to the window.
pub const TRAIL_LANES: usize = TRAIL_ROWS;
pub const TITLE_SCALE: f32 = 2.6;
/// How much of its height a settled subagent gives up (#53): 16 → 12.
pub const SETTLE_SHRINK: f32 = 0.25;

/// A session's PR sits where its lane reaches this far from the right edge:
/// the trace runs into the connector and stops, so the work visibly leaves
/// the board (#56).
pub const PR_INSET: f32 = 62.0;
pub const PR_H: f32 = 17.0;
pub const PR_PAD: f32 = 8.0;
pub const PR_TEXT: f32 = 1.05;
/// Card-edge pins on the incoming side of the connector body.
pub const PR_PINS: usize = 4;
const PIN_W: f32 = 2.4;
const PIN_GAP: f32 = 4.6;

/// Path distance between a chip's trailing edge and the next subagent chip.
pub const SUB_GAP: f32 = 44.0;
/// The two session chips of a lane pair are staggered along their diagonal
/// by this much (total), so they read as offset rather than side by side.
pub const PAIR_STAGGER: f32 = 36.0;

pub const DESIGN_W: f32 = 980.0;
pub const DESIGN_H: f32 = 620.0;
/// User zoom (⌘+ / ⌘-) is the only scaling: the board never grows with the
/// window (#19); bigger windows simply show more lanes and stripes.
pub const USER_ZOOM_MIN: f32 = 0.5;
pub const USER_ZOOM_MAX: f32 = 3.0;
pub const USER_ZOOM_STEP: f32 = 1.15;
/// Extra lanes above lane 0 whose diagonals fill the top-right corner.
pub const LEAD_LANES: usize = 8;
/// Where the first diagonal stripe meets the top edge, as a fraction of width.
pub const ANCHOR: f32 = 0.62;
/// Horizontal distance between diagonal stripes, as a fraction of width.
pub const STRIPE_PITCH: f32 = 0.55;

/// Geometry of one chip family (sessions are bigger than subagents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChipStyle {
    pub h: f32,
    pub r: f32,
    pub pad: f32,
    pub min_w: f32,
    pub text_scale: f32,
    pub text_stroke: f32,
    /// How far the chip slides away from its socket when unplugged.
    pub pull: f32,
    pub wave_len: f32,
    pub wave_amp: f32,
    pub sock_w: f32,
    pub sock_h: f32,
    pub notch: f32,
}

impl ChipStyle {
    pub fn width(&self, label: &str) -> f32 {
        self.min_w.max(font::measure(label, self.text_scale) + 2.0 * self.pad)
    }
}

pub const SESSION_STYLE: ChipStyle = ChipStyle {
    h: 22.0,
    r: 4.0,
    pad: 13.0,
    min_w: 56.0,
    text_scale: 1.5,
    text_stroke: 1.45,
    pull: 16.0,
    wave_len: 46.0,
    wave_amp: 6.0,
    sock_w: 24.0,
    sock_h: 14.0,
    notch: 6.0,
};

pub const SUB_STYLE: ChipStyle = ChipStyle {
    h: 16.0,
    r: 3.0,
    pad: 9.0,
    min_w: 40.0,
    text_scale: 1.1,
    text_stroke: 1.15,
    pull: 11.0,
    wave_len: 28.0,
    wave_amp: 4.0,
    sock_w: 16.0,
    sock_h: 10.0,
    notch: 4.5,
};

#[derive(Clone, Debug)]
pub struct ChipDraw {
    pub target: Target,
    pub lane: usize,
    /// Arc length of the chip centre when plugged in.
    pub s_c: f32,
    pub width: f32,
    pub label: String,
    pub p: f32,
    /// A finished subagent sinks in place instead of unplugging (#53): it
    /// keeps the trace running through it, takes the idle colours and loses a
    /// quarter of its height. Sessions always leave this at 0.
    pub settled: f32,
    pub hover: f32,
    pub alpha: f32,
    pub phase: Phase,
    /// Content-space centre (before scrolling), including the unplug slide.
    pub center: Pt,
    pub tangent: Pt,
    pub style: ChipStyle,
}

/// One trace as drawn this frame. Its geometry is that of lane index
/// `pos`, which is fractional while the rows slide into a new arrangement
/// (#49); a brand-new lane draws in from the left (`reveal`), and the gap
/// rows between groups are laid out but never drawn (`visible`).
#[derive(Clone, Debug)]
pub struct Lane {
    pub path: Polyline,
    pub pos: f32,
    pub reveal: f32,
    pub visible: bool,
}

impl Lane {
    fn fixed(path: Polyline, pos: f32) -> Self {
        Self { path, pos, reveal: 1.0, visible: true }
    }
}

/// One PR connector, already projected onto its lane.
#[derive(Clone, Debug)]
pub struct PrDraw {
    pub lane: usize,
    /// Arc length where the trace enters the connector.
    pub s: f32,
    pub center: Pt,
    pub tangent: Pt,
    pub width: f32,
    pub label: String,
    pub look: pr::Look,
    pub alpha: f32,
}

#[derive(Clone)]
pub struct Frame {
    pub lanes: Rc<Vec<Lane>>,
    pub chips: Vec<ChipDraw>,
    /// PR connectors, one per session that resolved to a pull request (#56).
    pub prs: Vec<PrDraw>,
    pub scroll_y: f32,
    pub t: f32,
    pub layout: Layout,
    /// Badge text in the corner (the machine user, #15). Empty = no badge.
    pub title: String,
    /// Horizontal pan in design units (icon rendering).
    pub pan_x: f32,
    pub palette: Palette,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgba },
    Stroke { pieces: Vec<Vec<Pt>>, width: f32, color: Rgba },
    RoundedRect { center: Pt, w: f32, h: f32, r: f32, angle: f32, color: Rgba, stroke: Option<f32> },
    Text { text: String, scale: f32, stroke: f32, angle: f32, center: Pt, color: Rgba },
}

impl Shape {
    fn scale(&mut self, z: f32) {
        match self {
            Shape::Rect { x, y, w, h, .. } => {
                *x *= z;
                *y *= z;
                *w *= z;
                *h *= z;
            }
            Shape::Stroke { pieces, width, .. } => {
                for piece in pieces {
                    for p in piece {
                        *p = *p * z;
                    }
                }
                *width *= z;
            }
            Shape::RoundedRect { center, w, h, r, .. } => {
                *center = *center * z;
                *w *= z;
                *h *= z;
                *r *= z;
            }
            Shape::Text { center, scale, stroke, .. } => {
                *center = *center * z;
                *scale *= z;
                *stroke *= z;
            }
        }
    }
}

/// Vertical offset of lane `i` (may be negative for lead lanes): traces come
/// in pairs, `GAP` apart, with `PAIR_GAP` extra between pairs.
pub fn lane_offset(i: i32) -> f32 {
    let pair = i.div_euclid(2) as f32;
    let odd = i.rem_euclid(2) as f32;
    pair * (2.0 * GAP + PAIR_GAP) + odd * GAP
}

/// Vertical drop of lane `i`'s first diagonal (#45). Grows by `DIAG_GROWTH`
/// per lane, but never so long that the diagonal would reach past the
/// start of the next stripe on a board `width` wide.
pub fn first_diag(i: i32, width: f32) -> f32 {
    let cap = ((width * STRIPE_PITCH + DIAG) / 2.0 - 2.0 * CORNER_R).max(DIAG_MIN);
    (DIAG + i as f32 * DIAG_GROWTH).clamp(DIAG_MIN, cap)
}

/// Vertical offset of lane `i` once past its first diagonal, measured as if
/// that diagonal were `DIAG` long: the lane spacing after the fan.
pub fn fanned_offset(i: i32, width: f32) -> f32 {
    lane_offset(i) + first_diag(i, width) - DIAG
}

/// Board geometry derived from the window size. All layout happens in
/// "design units"; `zoom` (user-controlled) maps them to window pixels.
/// A bigger window gets more lanes and stripes, not bigger chips.
///
/// Every lane is one global staircase: lane `i` bends exactly as far left
/// as it sits lower than lane `i - 1`, so the bends form 45° stripes running
/// from top-right to bottom-left. Wide windows get extra stripes (`bands`)
/// `STRIPE_PITCH` apart; this construction never lets two lanes cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub zoom: f32,
    pub width: f32,
    pub height: f32,
    pub bands: usize,
    pub lanes: usize,
}

impl Layout {
    pub fn new(win_w: f32, win_h: f32, chips: usize, user_zoom: f32) -> Self {
        let zoom = user_zoom.clamp(USER_ZOOM_MIN, USER_ZOOM_MAX);
        let width = win_w / zoom;
        let height = win_h / zoom;
        let span = width + height - width * ANCHOR;
        let area_bands = (span / (width * STRIPE_PITCH)).ceil().max(1.0) as usize;
        // Enough stripes that even the lowest session lane has a diagonal on
        // screen (its bends move left by its fanned offset, #27/#29).
        let last_offset = if chips == 0 { 0.0 } else { fanned_offset((chips - 1) as i32, width) };
        let need = (last_offset + width * LEFT_BOUND - width * ANCHOR) / (width * STRIPE_PITCH);
        let chip_bands = need.ceil().max(0.0) as usize + 1;
        let bands = area_bands.max(chip_bands);
        let lanes = LEAD_LANES + chips + TRAIL_LANES;
        Self { zoom, width, height, bands, lanes }
    }

    pub fn content_height(&self) -> f32 {
        TOP_PAD + fanned_offset((self.lanes - LEAD_LANES) as i32, self.width) + DIAG + BOTTOM_PAD
    }

    /// Lane index (into `build_lanes`) for session slot `k`: every lane
    /// carries a session (#31); `GAP` is wide enough that chips on the two
    /// lanes of a pair never overlap.
    pub fn chip_lane(k: usize) -> usize {
        LEAD_LANES + k
    }

    /// Every lane at rest and visible (geometry checks, tests).
    pub fn build_lanes(&self) -> Vec<Lane> {
        (0..self.lanes).map(|i| Lane::fixed(lane_path(i, self), i as f32)).collect()
    }

    /// The lanes as the model's rows want them drawn at `now`: session and
    /// trailing rows at their (possibly sliding) positions, gap rows hidden,
    /// new rows drawing in (#49). Lead lanes never move.
    pub fn build_lanes_for(&self, rows: &Rows, now: Instant) -> Vec<Lane> {
        (0..self.lanes)
            .map(|i| {
                if i < LEAD_LANES {
                    return Lane::fixed(lane_path(i, self), i as f32);
                }
                let r = i - LEAD_LANES;
                let pos = LEAD_LANES as f32 + rows.pos(r, now);
                Lane {
                    path: lane_path_at(pos, self),
                    pos,
                    reveal: rows.reveal(r, now),
                    visible: rows.kind(r) != RowKind::Gap,
                }
            })
            .collect()
    }
}

/// One diagonal of a lane: it starts at `(ax, y)` and drops `d` while
/// moving `d` to the right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stripe {
    pub ax: f32,
    pub y: f32,
    pub d: f32,
}

impl Stripe {
    /// Where the diagonal ends (the bend back to horizontal).
    pub fn end(&self) -> Pt {
        Pt::new(self.ax + self.d, self.y + self.d)
    }

    pub fn mid(&self) -> Pt {
        Pt::new(self.ax + self.d / 2.0, self.y + self.d / 2.0)
    }
}

/// For lane `idx`, one `Stripe` per band: where that stripe's diagonal
/// starts and how far it drops. `ax` may be off screen on either side. The
/// first `LEAD_LANES` lanes sit above the top padding. The first diagonal
/// grows down the board (#45); from the second stripe on, every lane bends
/// exactly as far left as it sits lower, so stripes stay 45° and never cross.
pub fn lane_params(idx: usize, layout: &Layout) -> Vec<Stripe> {
    let i = idx as i32 - LEAD_LANES as i32;
    let off = lane_offset(i);
    let fanned = fanned_offset(i, layout.width);
    (0..layout.bands)
        .map(|b| {
            if b == 0 {
                Stripe { ax: layout.width * ANCHOR - off, y: TOP_PAD + off, d: first_diag(i, layout.width) }
            } else {
                let ax = layout.width * (ANCHOR + b as f32 * STRIPE_PITCH) - fanned;
                let y = TOP_PAD + fanned + b as f32 * DIAG;
                Stripe { ax, y, d: DIAG }
            }
        })
        .collect()
}

/// `lane_params` at a fractional lane position: a lane mid-slide (#49)
/// takes the geometry between its two integer neighbours.
pub fn lane_params_at(pos: f32, layout: &Layout) -> Vec<Stripe> {
    let i0 = pos.floor().max(0.0);
    let t = pos - i0;
    let a = lane_params(i0 as usize, layout);
    if t <= 1e-4 {
        return a;
    }
    let b = lane_params(i0 as usize + 1, layout);
    a.iter()
        .zip(&b)
        .map(|(p, q)| Stripe { ax: p.ax + (q.ax - p.ax) * t, y: p.y + (q.y - p.y) * t, d: p.d + (q.d - p.d) * t })
        .collect()
}

pub fn lane_path(i: usize, layout: &Layout) -> Polyline {
    lane_path_at(i as f32, layout)
}

pub fn lane_path_at(pos: f32, layout: &Layout) -> Polyline {
    let params = lane_params_at(pos, layout);
    let mut corners = Vec::with_capacity(2 + params.len() * 2);
    let first = params[0];
    corners.push(Pt::new((-24.0f32).min(first.ax - 1.0), first.y));
    for st in &params {
        corners.push(Pt::new(st.ax, st.y));
        corners.push(st.end());
    }
    let last = params.last().unwrap().end();
    corners.push(Pt::new((layout.width * 2.0 + 24.0).max(last.x + 1.0), last.y));
    Polyline::rounded(&corners, CORNER_R)
}

/// Chips never sit inside the left `LEFT_BOUND` of the board (#27, 20% since #30) nor past
/// the right edge; a spot that would is swapped for the next one to the right.
pub const LEFT_BOUND: f32 = 0.20;
const EDGE_MARGIN: f32 = 12.0;
/// Subagents sit on the horizontals either side of their session's diagonal,
/// at least this far from the curves (#36).
const CURVE_MARGIN: f32 = 30.0 + CORNER_R;

fn inside(layout: &Layout, x: f32, w: f32) -> bool {
    x - w / 2.0 >= layout.width * LEFT_BOUND && x + w / 2.0 <= layout.width - EDGE_MARGIN
}

/// Subagents ignore the left `LEFT_BOUND` (#37); they only stay on screen.
fn inside_sub(layout: &Layout, x: f32, w: f32) -> bool {
    x - w / 2.0 >= EDGE_MARGIN && x + w / 2.0 <= layout.width - EDGE_MARGIN
}

/// Where a session chip goes on its lane: `(arc length, placement key)`.
/// Session chips only ever ride a diagonal (#29); candidates are the
/// diagonals of each stripe, left to right, and the first one that fits on
/// screen wins. The key identifies the spot so a change can be animated (#28).
/// Within a lane pair the upper lane sits `PAIR_STAGGER / 2` further down
/// the diagonal and the lower lane the same distance back, so the two chips
/// are offset along the trace instead of lining up shoulder to shoulder.
pub fn session_anchor(lane: &Lane, layout: &Layout, chip_w: f32) -> (f32, u32) {
    let params = lane_params_at(lane.pos, layout);
    let stagger = session_stagger_at(lane.pos);
    // Along a 45° diagonal, an arc-length shift `d` moves (d/√2, d/√2).
    let d = stagger / std::f32::consts::SQRT_2;
    let cands: Vec<(Pt, u32)> =
        params.iter().enumerate().map(|(b, st)| (st.mid() + Pt::new(d, d), b as u32)).collect();
    let pick = cands
        .iter()
        .find(|(p, _)| inside(layout, p.x, chip_w))
        .or_else(|| cands.iter().find(|(p, _)| p.x - chip_w / 2.0 >= layout.width * LEFT_BOUND))
        .unwrap_or_else(|| cands.last().unwrap());
    (lane.path.nearest_s(pick.0), pick.1)
}

/// Signed arc-length shift of a session chip from its diagonal's midpoint:
/// the upper lane of a pair (even slot) goes forward, the lower lane back.
/// For a lane mid-slide (#49) it blends linearly between the two parities,
/// so the chip glides rather than hopping when its lane changes from an
/// upper to a lower one.
pub fn session_stagger_at(pos: f32) -> f32 {
    let slot = pos - LEAD_LANES as f32;
    let k = slot.floor();
    let frac = slot - k;
    let sign = if (k as i32).rem_euclid(2) == 0 { 1.0 - 2.0 * frac } else { 2.0 * frac - 1.0 };
    PAIR_STAGGER / 2.0 * sign
}

/// A chip's intended spot before any animation is applied.
struct Placement {
    target: Target,
    lane: usize,
    key: u32,
    s_c: f32,
    width: f32,
    label: String,
    style: ChipStyle,
}

/// Lays every session and subagent out on its lane. Subagents alternate
/// right / left of the session chip and avoid the side where they would run
/// off screen (#22, #27).
fn place_all(model: &BoardModel, layout: &Layout, lanes: &[Lane]) -> Vec<Placement> {
    let mut out = Vec::new();
    for (k, c) in model.chips.iter().enumerate() {
        let lane_idx = Layout::chip_lane(c.slot);
        let Some(lane_ref) = lanes.get(lane_idx) else { continue };
        let label = c.info.label();
        let style = SESSION_STYLE;
        let width = style.width(&label);
        let (s_c, stripe) = session_anchor(lane_ref, layout, width);
        // Keys name the spot on the lane, not the lane: when blocks shift,
        // chips slide along with their lane instead of fading (#49).
        let key = stripe;
        out.push(Placement { target: Target::Session(k), lane: lane_idx, key, s_c, width, label, style });

        // Horizontal runs either side of the parent's diagonal (stripe `key`),
        // trimmed by CURVE_MARGIN so nothing sits on a bend (#36).
        let params = lane_params_at(lane_ref.pos, layout);
        let lane = &lane_ref.path;
        let b = stripe as usize;
        let st = params[b];
        let s_bend_in = lane.nearest_s(Pt::new(st.ax, st.y));
        let s_bend_out = lane.nearest_s(st.end());
        let right_limit = match params.get(b + 1) {
            Some(next) => lane.nearest_s(Pt::new(next.ax, next.y)) - CURVE_MARGIN,
            None => lane.length(),
        };
        let left_limit = match b.checked_sub(1).and_then(|pb| params.get(pb)) {
            Some(prev) => lane.nearest_s(prev.end()) + CURVE_MARGIN,
            None => 0.0,
        };
        let mut right_cursor = s_bend_out + CURVE_MARGIN;
        let mut left_cursor = s_bend_in - CURVE_MARGIN;
        let fits = |s: f32, w: f32| inside_sub(layout, lane.point_at(s).0.x, w);
        for (j, sub) in c.subs.iter().enumerate() {
            let style = SUB_STYLE;
            let label = sub.info.label();
            let width = style.width(&label);
            let right_s = right_cursor + width / 2.0;
            let left_s = left_cursor - width / 2.0;
            let right_ok = right_s + width / 2.0 <= right_limit && fits(right_s, width);
            let left_ok = left_s - width / 2.0 >= left_limit && fits(left_s, width);
            let prefer_right = j % 2 == 0;
            let (s_c, on_right) = match (right_ok, left_ok) {
                (true, true) => (if prefer_right { right_s } else { left_s }, prefer_right),
                (true, false) => (right_s, true),
                (false, true) => (left_s, false),
                (false, false) => (right_s, true),
            };
            if on_right {
                right_cursor = s_c + width / 2.0 + SUB_GAP;
            } else {
                left_cursor = s_c - width / 2.0 - SUB_GAP;
            }
            // Key: side + ordinal on that side, so flipping sides animates.
            let key = 100 + j as u32 * 2 + on_right as u32;
            out.push(Placement { target: Target::Sub(k, j), lane: lane_idx, key, s_c, width, label, style });
        }
    }
    out
}

/// `(target, placement key, arc length)` for every chip — feed these to
/// `Anim::track_placement` before drawing so relocations fade (#28).
pub fn placements(model: &BoardModel, layout: &Layout, lanes: &[Lane]) -> Vec<(Target, u32, f32)> {
    place_all(model, layout, lanes).into_iter().map(|p| (p.target, p.key, p.s_c)).collect()
}

/// Projects every session chip and its subagent chain onto their lane,
/// applying unplug, hover, appear/fade and relocation animations.
pub fn chip_draws(model: &BoardModel, layout: &Layout, lanes: &[Lane], now: Instant) -> Vec<ChipDraw> {
    let mut out = Vec::new();
    for pl in place_all(model, layout, lanes) {
        let Some(lane) = lanes.get(pl.lane) else { continue };
        let Some(anim) = model.anim(pl.target) else { continue };
        let parent_alpha = match pl.target {
            Target::Sub(i, _) => model.chips.get(i).map(|c| c.anim.alpha(now)).unwrap_or(1.0),
            Target::Session(_) => 1.0,
        };
        let phase = match pl.target {
            Target::Session(i) => model.chips[i].phase(),
            Target::Sub(i, j) => model.chips[i].subs[j].info.phase(),
        };
        let (s_c, reloc_alpha) = match anim.reloc {
            Some(r) => {
                let (old, a) = r.stage(now);
                (if old { r.from_s } else { pl.s_c }, a)
            }
            None => (pl.s_c, 1.0),
        };
        // Sessions unplug; subagents settle in place (#53), so the same
        // disconnect animation drives a different look for each.
        let t = smoothstep(anim.disconnect);
        let (p, settled) = match pl.target {
            Target::Sub(..) => (0.0, t),
            Target::Session(_) => (t, 0.0),
        };
        let (center, tangent) = lane.path.point_at(s_c + p * pl.style.pull);
        // On a lane still drawing in, the chip shows as the trace reaches it (#49).
        let reveal_gate = if lane.reveal < 1.0 {
            smoothstep((lane.reveal * lane.path.length() - s_c + 20.0) / 60.0)
        } else {
            1.0
        };
        out.push(ChipDraw {
            target: pl.target,
            lane: pl.lane,
            s_c,
            width: pl.width,
            label: pl.label,
            p,
            settled,
            hover: anim.hover_t,
            alpha: anim.alpha(now).min(parent_alpha) * reloc_alpha * reveal_gate,
            phase,
            center,
            tangent,
            style: pl.style,
        });
    }
    out
}

/// Walks back from `s` to the nearest point where the lane runs flat, so a
/// connector never lands mid-bend. `None` when the whole reach is diagonal.
fn horizontal_at_or_before(path: &Polyline, s: f32) -> Option<f32> {
    const STEP: f32 = 3.0;
    const REACH: f32 = 260.0;
    let mut walked = 0.0;
    while walked <= REACH {
        let t = s - walked;
        if t <= 0.0 {
            return None;
        }
        if path.point_at(t).1.y.abs() < 0.15 {
            return Some(t);
        }
        walked += STEP;
    }
    None
}

/// Projects each session's PR onto the point where its lane reaches the right
/// edge. The lane comes from the session's own chip, so a connector always
/// lands on the trace its chip rides. A lane that never gets that far, or is
/// still drawing in, has no connector (#56).
pub fn pr_draws(model: &BoardModel, layout: &Layout, lanes: &[Lane], chips: &[ChipDraw]) -> Vec<PrDraw> {
    let x = layout.width - PR_INSET;
    let mut out = Vec::new();
    for c in chips {
        let Target::Session(i) = c.target else { continue };
        let Some(pr) = model.chips.get(i).and_then(|ch| ch.info.pr.as_ref()) else { continue };
        let Some(lane) = lanes.get(c.lane) else { continue };
        if !lane.visible || c.alpha <= 0.01 {
            continue;
        }
        let Some(at_edge) = lane.path.s_at_x(x) else { continue };
        let label = pr.label();
        let width = (font::measure(&label, PR_TEXT) + 2.0 * PR_PAD).max(30.0);
        // A connector rotated onto a diagonal reads as debris, so back up to
        // the horizontal run before it and leave room for the body (#56).
        let s = match horizontal_at_or_before(&lane.path, at_edge) {
            Some(h) if h < at_edge - 0.5 => h - width / 2.0 - 6.0,
            _ => at_edge,
        };
        if s <= 0.0 || (lane.reveal < 1.0 && lane.reveal * lane.path.length() < s) {
            continue;
        }
        let (center, tangent) = lane.path.point_at(s);
        out.push(PrDraw {
            lane: c.lane,
            s,
            center,
            tangent,
            width,
            label,
            look: pr.look(),
            alpha: c.alpha,
        });
    }
    out
}

pub fn hash01(i: usize) -> f32 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x1234_5678);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 10_000) as f32 / 10_000.0
}

/// Where a chip's socket sits along the lane.
fn socket_center(c: &ChipDraw) -> f32 {
    c.s_c - c.width / 2.0 - 6.0
}

/// Cuts a lane around every chip on it (sorted by position): loose-cable
/// wave, socket, gap, chip, then the trace resumes under the chip. Returns
/// the pieces and the arc length where packets should stop flowing.
fn splice_lane(lane: &Polyline, chips: &[&ChipDraw]) -> (Vec<Vec<Pt>>, f32) {
    let mut pieces = Vec::new();
    let mut cursor = 0.0;
    let mut cut = lane.length();
    for c in chips {
        let st = &c.style;
        let sock_c = socket_center(c);
        let sock_left = sock_c - st.sock_w / 2.0;
        let sock_right = sock_c + st.sock_w / 2.0;
        let chip_left = c.s_c - c.width / 2.0 + c.p * st.pull;
        let end1 = chip_left.min(sock_right);
        let wave_end = sock_left;
        let wave_start = (wave_end - st.wave_len).max(cursor);

        pieces.push(lane.slice(cursor, wave_start));
        const N: usize = 26;
        let amp = st.wave_amp * c.p;
        let len = (wave_end - wave_start).max(0.0);
        let mut wave = Vec::with_capacity(N + 1);
        for k in 0..=N {
            let t = k as f32 / N as f32;
            let s = wave_start + t * len;
            let (pos, tan) = lane.point_at(s);
            let env = (1.0 - (std::f32::consts::TAU * t).cos()) / 2.0;
            let off = amp * (std::f32::consts::TAU * t).sin() * env;
            wave.push(pos + tan.perp() * off);
        }
        pieces.push(wave);
        pieces.push(lane.slice(wave_end, end1));
        cursor = c.s_c + c.p * st.pull;
        if c.p > 0.35 && cut >= lane.length() {
            cut = wave_start;
        }
    }
    pieces.push(lane.slice(cursor, lane.length()));
    (pieces, cut)
}

/// Produces the draw list for a frame in window pixels. `origin_px` is the
/// window position of the board element; scrolling and zoom are applied here.
pub fn build_shapes(f: &Frame, origin_px: Pt) -> Vec<Shape> {
    let zoom = f.layout.zoom;
    let origin = origin_px * (1.0 / zoom);
    let mut out = build_design_shapes(f, origin);
    if (zoom - 1.0).abs() > 1e-3 {
        for s in &mut out {
            s.scale(zoom);
        }
    }
    out
}

/// Same as `build_shapes` but in design units, with `origin` already in
/// design units.
fn build_design_shapes(f: &Frame, origin: Pt) -> Vec<Shape> {
    let pal = f.palette;
    let ox = origin.x + f.pan_x;
    let oy = origin.y - f.scroll_y;
    let to_screen = move |p: Pt| Pt::new(p.x + ox, p.y + oy);
    let mut out = Vec::new();
    let (width, height) = (f.layout.width, f.layout.height);

    // Cover twice the layout width, like the traces, so panned renders (icon) stay filled.
    out.push(Shape::Rect { x: origin.x, y: origin.y, w: width * 2.0, h: height, color: pal.bg });

    let mut by_lane: HashMap<usize, Vec<&ChipDraw>> = HashMap::new();
    for c in &f.chips {
        by_lane.entry(c.lane).or_default().push(c);
    }
    for chips in by_lane.values_mut() {
        chips.sort_by(|a, b| a.s_c.partial_cmp(&b.s_c).unwrap());
    }

    // Traces + packets.
    for (li, lane_ref) in f.lanes.iter().enumerate() {
        if !lane_ref.visible {
            continue;
        }
        // A new lane draws in from the left: only the revealed arc is stroked (#49).
        let drawn_in;
        let lane: &Polyline = if lane_ref.reveal < 1.0 {
            let shown = lane_ref.reveal * lane_ref.path.length();
            if shown < 2.0 {
                continue;
            }
            drawn_in = Polyline::new(lane_ref.path.slice(0.0, shown));
            &drawn_in
        } else {
            &lane_ref.path
        };
        let (Some(first), Some(last)) = (lane.pts.first(), lane.pts.last()) else { continue };
        let y_min = first.y.min(last.y) + oy - 40.0;
        let y_max = first.y.max(last.y) + oy + 40.0;
        if y_max < origin.y || y_min > origin.y + height {
            continue;
        }
        // A lane carrying a PR ends at its connector: the work leaves the
        // board there, so nothing is stroked past it (#56).
        let ends_at = f.prs.iter().find(|p| p.lane == li).map(|p| p.s - p.width / 2.0);
        let cut_short;
        let lane: &Polyline = match ends_at {
            Some(end) if end < lane.length() - 1.0 => {
                cut_short = Polyline::new(lane.slice(0.0, end));
                &cut_short
            }
            _ => lane,
        };
        let chips = by_lane.get(&li).map(|v| v.as_slice()).unwrap_or(&[]);
        let (pieces, cut) = if chips.is_empty() {
            (vec![lane.pts.clone()], lane.length())
        } else {
            splice_lane(lane, chips)
        };
        let pieces: Vec<Vec<Pt>> =
            pieces.into_iter().map(|piece| piece.into_iter().map(to_screen).collect()).collect();
        out.push(Shape::Stroke { pieces, width: LINE_W, color: pal.line });

        let working = chips
            .first()
            .is_some_and(|c| matches!(c.target, Target::Session(_)) && c.phase == Phase::Working && c.alpha > 0.5);
        let (spacing, speed, size, alpha) = if working {
            (104.0, 96.0, (7.0, 3.0), 1.0)
        } else {
            (230.0 + hash01(li * 7 + 1) * 120.0, 22.0 + hash01(li * 3 + 2) * 10.0, (5.0, 2.4), 0.9)
        };
        let len = lane.length();
        let mut s = (f.t * speed + hash01(li) * spacing) % spacing;
        while s < len {
            if s <= cut {
                let (pos, tan) = lane.point_at(s);
                out.push(Shape::RoundedRect {
                    center: to_screen(pos),
                    w: size.0,
                    h: size.1,
                    r: 1.2,
                    angle: tan.angle_deg(),
                    color: theme::with_alpha(pal.packet, alpha),
                    stroke: None,
                });
            }
            s += spacing;
        }
    }

    // Sockets.
    for c in &f.chips {
        let Some(lane) = f.lanes.get(c.lane) else { continue };
        let a = smoothstep(c.p) * c.alpha;
        if a <= 0.01 {
            continue;
        }
        let st = &c.style;
        let (pos, tan) = lane.path.point_at(socket_center(c));
        let center = to_screen(pos);
        let angle = tan.angle_deg();
        out.push(Shape::RoundedRect {
            center,
            w: st.sock_w,
            h: st.sock_h,
            r: 2.5,
            angle,
            color: theme::with_alpha(pal.line, a),
            stroke: None,
        });
        let notch_c = center + tan * (st.sock_w / 2.0 - st.notch / 2.0 + 0.6);
        out.push(Shape::RoundedRect {
            center: notch_c,
            w: st.notch,
            h: st.notch,
            r: 0.6,
            angle,
            color: theme::with_alpha(pal.bg, a),
            stroke: None,
        });
    }

    // Chips + labels.
    for c in &f.chips {
        if c.alpha <= 0.01 {
            continue;
        }
        let st = &c.style;
        let angle = c.tangent.angle_deg();
        let center = to_screen(c.center);
        let (fill_off, text_off) = match c.phase {
            Phase::NeedsUser | Phase::Working => (pal.chip_needs, pal.text_needs),
            Phase::Idle => (pal.chip_idle, pal.text_idle),
        };
        let off = c.p.max(c.settled);
        let fill_c = theme::with_alpha(theme::lerp(pal.chip, fill_off, off), c.alpha);
        let text_c = theme::with_alpha(theme::lerp(pal.text_on, text_off, off), c.alpha);
        let h = st.h * (1.0 - SETTLE_SHRINK * c.settled);

        if c.hover > 0.01 {
            let grow = 5.0 + 1.5 * c.hover;
            out.push(Shape::RoundedRect {
                center,
                w: c.width + grow,
                h: h + grow,
                r: st.r + 2.5,
                angle,
                color: theme::with_alpha(pal.outline, c.hover * c.alpha),
                stroke: Some(1.5),
            });
        }
        out.push(Shape::RoundedRect {
            center,
            w: c.width,
            h,
            r: st.r,
            angle,
            color: fill_c,
            stroke: None,
        });
        out.push(Shape::Text {
            text: c.label.clone(),
            scale: st.text_scale,
            stroke: st.text_stroke,
            angle,
            center,
            color: text_c,
        });
    }

    // PR connectors: a card-edge block the trace runs into, with pins on the
    // incoming side. Colour carries the state (#56).
    for c in &f.prs {
        let center = to_screen(c.center);
        let angle = c.tangent.angle_deg();
        let a = c.alpha;
        let ink = |k: f32| theme::with_alpha(pal.ink, k * a);
        let (fill, stroke, label, pin) = match c.look {
            pr::Look::Draft => (None, Some(ink(0.85)), ink(0.85), ink(0.55)),
            pr::Look::Open => (Some(theme::with_alpha(pal.chip, a)), None, theme::with_alpha(pal.text_on, a), ink(0.5)),
            pr::Look::Failing => (
                Some(theme::with_alpha(pal.chip, a)),
                None,
                theme::with_alpha(pal.text_on, a),
                theme::with_alpha(pal.alarm, a),
            ),
            pr::Look::Merged => (Some(ink(0.42)), None, theme::with_alpha(pal.bg, a), ink(0.42)),
            pr::Look::Closed => (None, Some(ink(0.32)), ink(0.4), ink(0.25)),
        };

        // Pins march back along the trace from the body's leading edge.
        for k in 0..PR_PINS {
            let back = c.width / 2.0 + 3.0 + k as f32 * PIN_GAP;
            out.push(Shape::RoundedRect {
                center: to_screen(c.center - c.tangent * back),
                w: PIN_W,
                h: PR_H - 5.0,
                r: 0.8,
                angle,
                color: pin,
                stroke: None,
            });
        }
        if let Some(color) = fill {
            out.push(Shape::RoundedRect { center, w: c.width, h: PR_H, r: 2.5, angle, color, stroke: None });
        }
        if let Some(color) = stroke {
            out.push(Shape::RoundedRect { center, w: c.width, h: PR_H, r: 2.5, angle, color, stroke: Some(1.3) });
        }
        out.push(Shape::Text {
            text: c.label.clone(),
            scale: PR_TEXT,
            stroke: 1.1,
            angle,
            center,
            color: label,
        });
    }

    // Title badge, on a patch of background so the lead lanes stay clear of it.
    // An empty title means "no badge" (used for the app icon).
    if f.title.is_empty() {
        return out;
    }
    let title = f.title.as_str();
    let tw = font::measure(title, TITLE_SCALE);
    let center = Pt::new(origin.x + 26.0 + tw / 2.0, origin.y + 26.0);
    out.push(Shape::RoundedRect {
        center,
        w: tw + 26.0 + 28.0,
        h: 30.0 + 18.0,
        r: 9.0,
        angle: 0.0,
        color: pal.bg,
        stroke: None,
    });
    out.push(Shape::RoundedRect {
        center,
        w: tw + 26.0,
        h: 30.0,
        r: 5.0,
        angle: 0.0,
        color: pal.chip,
        stroke: None,
    });
    out.push(Shape::Text {
        text: title.to_string(),
        scale: TITLE_SCALE,
        stroke: 2.4,
        angle: 0.0,
        center,
        color: pal.text_on,
    });

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SessionInfo, SubagentInfo};

    #[test]
    fn layout_fills_big_windows_with_more_stripes_not_lanes_or_zoom() {
        let small = Layout::new(DESIGN_W, DESIGN_H, 3, 1.0);
        let big = Layout::new(3840.0, 2160.0, 3, 1.0);
        assert_eq!(small.zoom, 1.0);
        assert_eq!(big.zoom, 1.0, "the board never scales with the window (#19)");
        assert_eq!(small.lanes, LEAD_LANES + 3 + TRAIL_LANES, "sessions + 6 lanes, no filler (#45)");
        assert_eq!(big.lanes, small.lanes, "a bigger window adds no lanes");
        assert!(big.bands >= small.bands, "stripes scale with the width instead");
        assert!(Layout::new(DESIGN_W, 2400.0, 3, 1.0).bands > small.bands, "a tall window adds stripes");
        assert_eq!(Layout::new(DESIGN_W, DESIGN_H, 10, 1.0).lanes, LEAD_LANES + 16);
    }

    #[test]
    fn user_zoom_scales_the_board_and_keeps_the_lane_count() {
        let base = Layout::new(DESIGN_W, DESIGN_H, 3, 1.0);
        let out = Layout::new(DESIGN_W, DESIGN_H, 3, 0.5);
        let inn = Layout::new(DESIGN_W, DESIGN_H, 3, 2.0);
        assert!((out.zoom - 0.5).abs() < 1e-6 && (inn.zoom - 2.0).abs() < 1e-6);
        assert_eq!(out.lanes, base.lanes);
        assert_eq!(inn.lanes, base.lanes);
        assert!(inn.width < base.width);
        let clamped = Layout::new(DESIGN_W, DESIGN_H, 3, 99.0);
        assert!((clamped.zoom - USER_ZOOM_MAX).abs() < 1e-6);
    }

    #[test]
    fn lanes_come_in_pairs_and_never_cross() {
        assert_eq!(lane_offset(0), 0.0);
        assert_eq!(lane_offset(1), GAP);
        assert_eq!(lane_offset(2), 2.0 * GAP + PAIR_GAP);
        assert_eq!(lane_offset(-1), -GAP - PAIR_GAP);
        let layout = Layout::new(1920.0, 1080.0, 8, 1.0);
        for i in 1..layout.lanes {
            let a = lane_params(i - 1, &layout);
            let b = lane_params(i, &layout);
            for (pa, pb) in a.iter().zip(b.iter()) {
                let dy = pb.y - pa.y;
                assert!(dy > 0.0);
                assert!((pa.ax - pb.ax - dy).abs() < 1e-3, "bend moves left exactly as far as the lane drops");
                // Parallel 45° diagonals: the lower lane's end sits below and
                // left of the upper lane's, so the lower lane is still on its
                // diagonal when the upper one has gone horizontal.
                assert!(pb.end().y > pa.end().y);
            }
        }
    }

    #[test]
    fn first_diagonal_grows_down_the_board_and_later_stripes_stay_regular() {
        let layout = Layout::new(1920.0, 1080.0, 8, 1.0);
        assert!(layout.bands >= 2, "wide window has a second stripe to compare");
        let lead = lane_params(0, &layout);
        let top = lane_params(LEAD_LANES, &layout);
        let low = lane_params(LEAD_LANES + 7, &layout);
        assert!((top[0].d - DIAG).abs() < 1e-6, "lane 0 keeps the base diagonal");
        assert!(lead[0].d < top[0].d, "lead lanes above it are shorter");
        assert!((low[0].d - (DIAG + 7.0 * DIAG_GROWTH)).abs() < 1e-6, "each lane adds DIAG_GROWTH");
        assert!((low[1].d - DIAG).abs() < 1e-6, "the second stripe is not fanned");
        // The fan widens the lane spacing on the right: after the first
        // diagonal the lanes are DIAG_GROWTH further apart than on the left.
        let a = lane_params(LEAD_LANES + 2, &layout);
        let b = lane_params(LEAD_LANES + 3, &layout);
        let left_gap = b[0].y - a[0].y;
        let right_gap = b[0].end().y - a[0].end().y;
        assert!((right_gap - left_gap - DIAG_GROWTH).abs() < 1e-6);
        // And the second stripe of a lane starts after its first diagonal ends.
        for i in 0..layout.lanes {
            let p = lane_params(i, &layout);
            assert!(p[1].ax > p[0].end().x, "lane {i}: stripes do not overlap");
            assert!((p[1].y - p[0].end().y).abs() < 1e-6, "lane {i}: horizontal joins the stripes");
        }
        // Narrow boards cap the fan so the first diagonal never runs into the
        // next stripe.
        let narrow = Layout::new(400.0, 800.0, 30, 2.0);
        for i in 0..narrow.lanes {
            let p = lane_params(i, &narrow);
            assert!(p[1].ax > p[0].end().x, "narrow lane {i}: capped diagonal fits");
        }
    }

    #[test]
    fn lanes_follow_the_rows_hide_gaps_and_slide_with_their_chips() {
        use crate::model::ROW_SECS;
        use std::time::Duration;
        let t0 = Instant::now();
        let mut model = BoardModel::new();
        let mut a = SessionInfo::synthetic(1, "a", Phase::Working);
        a.group = "g1".into();
        let mut b = SessionInfo::synthetic(2, "b", Phase::Working);
        b.group = "g2".into();
        model.apply(vec![a.clone(), b.clone()], t0);
        model.settle();
        let layout = Layout::new(DESIGN_W, DESIGN_H, model.slot_span(), 1.0);
        let lanes = layout.build_lanes_for(&model.rows, t0);
        assert_eq!(lanes.len(), layout.lanes);
        assert!(lanes[LEAD_LANES].visible, "a session row has a trace");
        assert!(!lanes[LEAD_LANES + 1].visible, "gap rows carry no trace (#49)");
        assert!(lanes[LEAD_LANES + 4].visible && lanes[LEAD_LANES + 5].visible, "session and trailing rows do");
        let key_before = placements(&model, &layout, &lanes).iter().find(|p| p.0 == Target::Session(1)).unwrap().1;

        // A session joins g1 above b.
        let mut c = SessionInfo::synthetic(3, "c", Phase::Working);
        c.group = "g1".into();
        let t1 = t0 + Duration::from_secs(5);
        model.apply(vec![a, b, c], t1);
        let layout = Layout::new(DESIGN_W, DESIGN_H, model.slot_span(), 1.0);
        let mid = t1 + Duration::from_secs_f32(ROW_SECS / 2.0);
        let lanes = layout.build_lanes_for(&model.rows, mid);
        let b_lane = &lanes[LEAD_LANES + 5];
        assert!((b_lane.pos - (LEAD_LANES as f32 + 4.5)).abs() < 1e-3, "b's lane is halfway down");
        let y4 = lane_params(LEAD_LANES + 4, &layout)[0].y;
        let y5 = lane_params(LEAD_LANES + 5, &layout)[0].y;
        assert!((lane_params_at(b_lane.pos, &layout)[0].y - (y4 + y5) / 2.0).abs() < 1e-3);
        assert!((lanes[LEAD_LANES + 1].reveal - 0.5).abs() < 1e-3, "the new lane is half drawn in");
        // b keeps its placement key, so it rides the lane instead of fading.
        let key_after = placements(&model, &layout, &lanes).iter().find(|p| p.0 == Target::Session(1)).unwrap().1;
        assert_eq!(key_before, key_after);
        // The stagger blends across the parity change rather than hopping.
        assert!((session_stagger_at(LEAD_LANES as f32 + 4.5)).abs() < 1e-3);
        assert_eq!(session_stagger_at(LEAD_LANES as f32 + 4.0), PAIR_STAGGER / 2.0);
        assert_eq!(session_stagger_at(LEAD_LANES as f32 + 5.0), -PAIR_STAGGER / 2.0);
    }

    #[test]
    fn paired_sessions_are_staggered_along_the_diagonal() {
        // Synthetic sessions group in threes, so six of them give slots
        // 0..3 and 6..9 (GROUP_GAP = 3): pairs (0,1) and (6,7).
        let layout = Layout::new(DESIGN_W, DESIGN_H, 6, 1.0);
        let lanes = layout.build_lanes();
        let infos: Vec<_> = (1..=6).map(|i| SessionInfo::synthetic(i, "s", Phase::Working)).collect();
        let mut model = BoardModel::new();
        model.apply(infos, Instant::now());
        model.settle();
        let slots: Vec<usize> = model.chips.iter().map(|c| c.slot).collect();
        assert_eq!(slots, vec![0, 1, 2, 6, 7, 8]);
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        assert_eq!(draws.len(), 6);
        let along = |d: &ChipDraw| (d.center.x + d.center.y) / std::f32::consts::SQRT_2;
        // Lanes 0/1 form a pair, so do 8/9: the upper chip sits further down
        // the diagonal than its partner by PAIR_STAGGER, less the half step
        // by which the lower lane's longer first diagonal moves its midpoint
        // forward (#45).
        let expected = PAIR_STAGGER - DIAG_GROWTH / std::f32::consts::SQRT_2;
        for pair in [(0usize, 1usize), (3, 4)] {
            let (upper, lower) = (&draws[pair.0], &draws[pair.1]);
            assert!((upper.tangent.angle_deg() - 45.0).abs() < 1.0);
            assert!((lower.tangent.angle_deg() - 45.0).abs() < 1.0);
            let got = along(upper) - along(lower);
            assert!((got - expected).abs() < 1.5, "pair {pair:?}: stagger {got} vs {expected}");
        }
        // Lane spacing is purely perpendicular to the diagonal, so consecutive
        // lanes zigzag: lane 1 (back) to lane 2 (forward) is the stagger
        // again, this time plus the fan's half step.
        let (a, b) = (&draws[1], &draws[2]);
        let expected = PAIR_STAGGER + DIAG_GROWTH / std::f32::consts::SQRT_2;
        assert!((along(b) - along(a) - expected).abs() < 1.5, "lanes alternate forward / back");
    }

    #[test]
    fn subagents_chain_to_the_right_of_their_session() {
        let layout = Layout::new(DESIGN_W, DESIGN_H, 2, 1.0);
        let lanes = layout.build_lanes();
        let mut a = SessionInfo::synthetic(1, "a", Phase::Working);
        a.subagents.push(SubagentInfo::synthetic(1, "Explore", "", true));
        a.subagents.push(SubagentInfo::synthetic(2, "Plan", "", false));
        let mut model = BoardModel::new();
        model.apply(vec![a, SessionInfo::synthetic(2, "b", Phase::Idle)], Instant::now());
        model.settle();
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        assert_eq!(draws.len(), 4);
        assert_eq!(draws[0].target, Target::Session(0));
        assert_eq!(draws[1].target, Target::Sub(0, 0));
        assert_eq!(draws[2].target, Target::Sub(0, 1));
        assert!(draws[1].s_c > draws[0].s_c + draws[0].width / 2.0, "first sub sits to the right");
        assert!(draws[2].s_c < draws[0].s_c - draws[0].width / 2.0, "second sub sits to the left");
        assert_eq!(draws[1].lane, draws[0].lane, "same lane as the parent");
        for d in &draws {
            let x = d.center.x;
            assert!(x - d.width / 2.0 >= 0.0 && x + d.width / 2.0 <= layout.width, "{} stays on screen", d.label);
        }
        assert_eq!(draws[2].settled, 1.0, "finished subagent settles in place");
        assert_eq!(draws[2].p, 0.0, "and never unplugs, so the trace runs through it (#53)");
        assert!((draws[0].tangent.angle_deg() - 45.0).abs() < 1.0, "first session rides the diagonal");
        assert!((draws[3].tangent.angle_deg() - 45.0).abs() < 1.0, "so does the second (#29)");
        for d in &draws[1..3] {
            assert!(d.tangent.angle_deg().abs() < 0.5, "{} sits on a horizontal (#36)", d.label);
            let lane = &lanes[d.lane];
            // The rounded corner starts CORNER_R before the apex, so probe a
            // little inside the margin: the trace must still be flat there.
            for s in [d.s_c - d.width / 2.0 - 16.0, d.s_c + d.width / 2.0 + 16.0] {
                assert!(lane.path.point_at(s).1.angle_deg().abs() < 0.5, "{} keeps clear of the bends", d.label);
            }
        }
        assert_eq!(draws[3].lane - draws[0].lane, 1, "sessions take consecutive lanes");
    }

    #[test]
    fn subagents_spill_to_the_other_side_instead_of_off_screen() {
        let layout = Layout::new(DESIGN_W, DESIGN_H, 1, 1.0);
        let lanes = layout.build_lanes();
        let mut a = SessionInfo::synthetic(1, "a", Phase::Working);
        for i in 0..8 {
            a.subagents.push(SubagentInfo::synthetic(i, "general-purpose", "", true));
        }
        let mut model = BoardModel::new();
        model.apply(vec![a], Instant::now());
        model.settle();
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        let on_screen = draws
            .iter()
            .filter(|d| d.center.x - d.width / 2.0 >= 0.0 && d.center.x + d.width / 2.0 <= layout.width)
            .count();
        // Right-only chaining would fit ~3; both sides fit noticeably more
        // even with the left 20% off limits.
        assert!(on_screen >= 5, "{on_screen} of {} chips fit the width", draws.len());
        let parent = draws[0].s_c;
        assert!(draws[1..].iter().any(|d| d.s_c > parent), "some subs sit to the right");
        assert!(draws[1..].iter().any(|d| d.s_c < parent), "some subs sit to the left");
    }

    #[test]
    fn chips_near_the_left_edge_move_to_the_right_of_the_bend() {
        // Many sessions push later lanes' bends far left; those chips must
        // land on a later stripe instead of inside the left 20%.
        let mut model = BoardModel::new();
        let list: Vec<_> = (0..40).map(|i| SessionInfo::synthetic(i, "SESSION-LONG-NAME", Phase::Working)).collect();
        model.apply(list, Instant::now());
        model.settle();
        let layout = Layout::new(DESIGN_W, DESIGN_H, model.slot_span(), 1.0);
        let lanes = layout.build_lanes();
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        for d in draws.iter().filter(|d| matches!(d.target, Target::Session(_))) {
            assert!(d.center.x - d.width / 2.0 >= layout.width * LEFT_BOUND - 1.0, "{} at x={}", d.label, d.center.x);
        }
        let keys: Vec<u32> = placements(&model, &layout, &lanes).iter().map(|p| p.1 & 0xff).collect();
        assert!(keys.iter().any(|k| *k >= 1), "some chips moved to the next stripe's diagonal");
        for d in &draws {
            assert!((d.tangent.angle_deg() - 45.0).abs() < 1.0, "{} rides a diagonal", d.label);
        }
    }

    #[test]
    fn relocation_fades_out_at_the_old_spot_first() {
        let layout = Layout::new(DESIGN_W, DESIGN_H, 1, 1.0);
        let lanes = layout.build_lanes();
        let mut model = BoardModel::new();
        model.apply(vec![SessionInfo::synthetic(1, "a", Phase::Working)], Instant::now());
        model.settle();
        let t0 = Instant::now();
        let (target, key, s) = placements(&model, &layout, &lanes)[0];
        model.anim_mut(target).unwrap().track_placement(key, s, t0);
        // Pretend the chip used to live 200px upstream and its spot changed.
        model.anim_mut(target).unwrap().track_placement(key + 2, s, t0);
        let anim = model.anim_mut(target).unwrap();
        anim.reloc.as_mut().unwrap().from_s = s - 200.0;
        let early = chip_draws(&model, &layout, &lanes, t0 + std::time::Duration::from_millis(10));
        assert!((early[0].s_c - (s - 200.0)).abs() < 1e-3, "drawn at the old spot while fading out");
        assert!(early[0].alpha < 1.0);
        let late = chip_draws(&model, &layout, &lanes, t0 + std::time::Duration::from_millis(400));
        assert!((late[0].s_c - s).abs() < 1e-3, "drawn at the new spot afterwards");
    }

    #[test]
    fn chips_of_different_pairs_never_overlap_and_pairs_stay_legible() {
        let mut model = BoardModel::new();
        let list: Vec<_> = (0..6).map(|i| SessionInfo::synthetic(i, "SESSION-LONG-NAME", Phase::Working)).collect();
        model.apply(list, Instant::now());
        model.settle();
        let layout = Layout::new(DESIGN_W, DESIGN_H, model.slot_span(), 1.0);
        let lanes = layout.build_lanes();
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        for (i, (a, b)) in draws.iter().zip(draws.iter().skip(1)).enumerate().take(2) {
            // Both chips are 45° rotated; their separation across the diagonals
            // is the perpendicular distance between centres.
            let d = b.center - a.center;
            let perp = (d.x - d.y).abs() / std::f32::consts::SQRT_2;
            if i % 2 == 0 {
                // Same pair: Ben chose a tight 24px gap (#35); most of each
                // chip must still show.
                assert!(perp >= SESSION_STYLE.h * 0.7, "{} and {} are {perp:.1}px apart", a.label, b.label);
            } else {
                assert!(perp >= SESSION_STYLE.h + 2.0, "{} and {} are {perp:.1}px apart", a.label, b.label);
            }
        }
    }

    #[test]
    fn splicing_a_lane_with_two_chips_keeps_the_trace_continuous() {
        let layout = Layout::new(DESIGN_W, DESIGN_H, 1, 1.0);
        let lanes = layout.build_lanes();
        let mut a = SessionInfo::synthetic(1, "a", Phase::Working);
        a.subagents.push(SubagentInfo::synthetic(1, "Explore", "", true));
        let mut model = BoardModel::new();
        model.apply(vec![a], Instant::now());
        model.settle();
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        let refs: Vec<&ChipDraw> = draws.iter().collect();
        let (pieces, cut) = splice_lane(&lanes[draws[0].lane].path, &refs);
        assert!(pieces.iter().filter(|p| p.len() >= 2).count() >= 5);
        assert_eq!(cut, lanes[draws[0].lane].path.length(), "nothing unplugged: packets flow to the end");
    }
}
