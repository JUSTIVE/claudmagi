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
use crate::model::{BoardModel, Phase, Target};
use crate::theme::{self, Palette, Rgba};

/// Spacing inside a pair of traces and the extra gap between pairs (#14).
pub const GAP: f32 = 22.0;
pub const PAIR_GAP: f32 = 24.0;
pub const LINE_W: f32 = 1.6;
pub const DIAG: f32 = 128.0;
pub const CORNER_R: f32 = 8.0;
pub const TOP_PAD: f32 = 54.0;
pub const BOTTOM_PAD: f32 = 34.0;
pub const MIN_LANES: usize = 24;
pub const TITLE_SCALE: f32 = 2.6;
/// Path distance between a chip's trailing edge and the next subagent chip.
pub const SUB_GAP: f32 = 44.0;

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
    pub hover: f32,
    pub alpha: f32,
    pub phase: Phase,
    /// Content-space centre (before scrolling), including the unplug slide.
    pub center: Pt,
    pub tangent: Pt,
    pub style: ChipStyle,
}

#[derive(Clone)]
pub struct Frame {
    pub lanes: Rc<Vec<Polyline>>,
    pub chips: Vec<ChipDraw>,
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
        // screen (its bend moves left by its offset, #27/#29).
        let last_offset = if chips == 0 { 0.0 } else { lane_offset((chips - 1) as i32) };
        let need = (last_offset + width * LEFT_BOUND - width * ANCHOR) / (width * STRIPE_PITCH);
        let chip_bands = need.ceil().max(0.0) as usize + 1;
        let bands = area_bands.max(chip_bands);
        let pitch = (2.0 * GAP + PAIR_GAP) / 2.0;
        let fill = ((height - TOP_PAD - DIAG - BOTTOM_PAD) / pitch).ceil().max(0.0) as usize + 1;
        let lanes = LEAD_LANES + MIN_LANES.max(chips + 14).max(fill);
        Self { zoom, width, height, bands, lanes }
    }

    pub fn content_height(&self) -> f32 {
        TOP_PAD + lane_offset((self.lanes - LEAD_LANES) as i32) + DIAG + BOTTOM_PAD
    }

    /// Lane index (into `build_lanes`) for session slot `k`: every lane
    /// carries a session (#31); `GAP` is wide enough that chips on the two
    /// lanes of a pair never overlap.
    pub fn chip_lane(k: usize) -> usize {
        LEAD_LANES + k
    }

    pub fn build_lanes(&self) -> Vec<Polyline> {
        (0..self.lanes).map(|i| lane_path(i, self)).collect()
    }
}

/// For lane `idx`, one `(ax, y0)` per stripe: where that stripe's diagonal
/// starts. `ax` may be off screen on either side. The first `LEAD_LANES`
/// lanes sit above the top padding.
pub fn lane_params(idx: usize, layout: &Layout) -> Vec<(f32, f32)> {
    let off = lane_offset(idx as i32 - LEAD_LANES as i32);
    (0..layout.bands)
        .map(|b| {
            let ax = layout.width * (ANCHOR + b as f32 * STRIPE_PITCH) - off;
            let y = TOP_PAD + off + b as f32 * DIAG;
            (ax, y)
        })
        .collect()
}

pub fn lane_path(i: usize, layout: &Layout) -> Polyline {
    let params = lane_params(i, layout);
    let mut corners = Vec::with_capacity(2 + params.len() * 2);
    let (ax0, y0) = params[0];
    corners.push(Pt::new((-24.0f32).min(ax0 - 1.0), y0));
    for (ax, y) in &params {
        corners.push(Pt::new(*ax, *y));
        corners.push(Pt::new(ax + DIAG, y + DIAG));
    }
    let (ax_last, y_last) = *params.last().unwrap();
    corners.push(Pt::new((layout.width * 2.0 + 24.0).max(ax_last + DIAG + 1.0), y_last + DIAG));
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
pub fn session_anchor(lane: &Polyline, lane_index: usize, layout: &Layout, chip_w: f32) -> (f32, u32) {
    let params = lane_params(lane_index, layout);
    let cands: Vec<(Pt, u32)> = params
        .iter()
        .enumerate()
        .map(|(b, (ax, y))| (Pt::new(ax + DIAG / 2.0, y + DIAG / 2.0), b as u32))
        .collect();
    let pick = cands
        .iter()
        .find(|(p, _)| inside(layout, p.x, chip_w))
        .or_else(|| cands.iter().find(|(p, _)| p.x - chip_w / 2.0 >= layout.width * LEFT_BOUND))
        .unwrap_or_else(|| cands.last().unwrap());
    (lane.nearest_s(pick.0), pick.1)
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
fn place_all(model: &BoardModel, layout: &Layout, lanes: &[Polyline]) -> Vec<Placement> {
    let mut out = Vec::new();
    for (k, c) in model.chips.iter().enumerate() {
        let lane_idx = Layout::chip_lane(c.slot);
        let Some(lane) = lanes.get(lane_idx) else { continue };
        let label = c.info.label();
        let style = SESSION_STYLE;
        let width = style.width(&label);
        let (s_c, stripe) = session_anchor(lane, lane_idx, layout, width);
        // Keys carry the lane so a group shift also fades (#28, #41).
        let key = (lane_idx as u32) << 8 | stripe;
        out.push(Placement { target: Target::Session(k), lane: lane_idx, key, s_c, width, label, style });

        // Horizontal runs either side of the parent's diagonal (stripe `key`),
        // trimmed by CURVE_MARGIN so nothing sits on a bend (#36).
        let params = lane_params(lane_idx, layout);
        let b = stripe as usize;
        let (ax, y) = params[b];
        let s_bend_in = lane.nearest_s(Pt::new(ax, y));
        let s_bend_out = lane.nearest_s(Pt::new(ax + DIAG, y + DIAG));
        let right_limit = match params.get(b + 1) {
            Some((nax, ny)) => lane.nearest_s(Pt::new(*nax, *ny)) - CURVE_MARGIN,
            None => lane.length(),
        };
        let left_limit = match b.checked_sub(1).and_then(|pb| params.get(pb)) {
            Some((pax, py)) => lane.nearest_s(Pt::new(pax + DIAG, py + DIAG)) + CURVE_MARGIN,
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
            let key = (lane_idx as u32) << 8 | (100 + j as u32 * 2 + on_right as u32);
            out.push(Placement { target: Target::Sub(k, j), lane: lane_idx, key, s_c, width, label, style });
        }
    }
    out
}

/// `(target, placement key, arc length)` for every chip — feed these to
/// `Anim::track_placement` before drawing so relocations fade (#28).
pub fn placements(model: &BoardModel, layout: &Layout, lanes: &[Polyline]) -> Vec<(Target, u32, f32)> {
    place_all(model, layout, lanes).into_iter().map(|p| (p.target, p.key, p.s_c)).collect()
}

/// Projects every session chip and its subagent chain onto their lane,
/// applying unplug, hover, appear/fade and relocation animations.
pub fn chip_draws(model: &BoardModel, layout: &Layout, lanes: &[Polyline], now: Instant) -> Vec<ChipDraw> {
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
        let p = smoothstep(anim.disconnect);
        let (center, tangent) = lane.point_at(s_c + p * pl.style.pull);
        out.push(ChipDraw {
            target: pl.target,
            lane: pl.lane,
            s_c,
            width: pl.width,
            label: pl.label,
            p,
            hover: anim.hover_t,
            alpha: anim.alpha(now).min(parent_alpha) * reloc_alpha,
            phase,
            center,
            tangent,
            style: pl.style,
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
    for (li, lane) in f.lanes.iter().enumerate() {
        let (Some(first), Some(last)) = (lane.pts.first(), lane.pts.last()) else { continue };
        let y_min = first.y.min(last.y) + oy - 40.0;
        let y_max = first.y.max(last.y) + oy + 40.0;
        if y_max < origin.y || y_min > origin.y + height {
            continue;
        }
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
        let (pos, tan) = lane.point_at(socket_center(c));
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
        let fill_c = theme::with_alpha(theme::lerp(pal.chip, fill_off, c.p), c.alpha);
        let text_c = theme::with_alpha(theme::lerp(pal.text_on, text_off, c.p), c.alpha);

        if c.hover > 0.01 {
            let grow = 5.0 + 1.5 * c.hover;
            out.push(Shape::RoundedRect {
                center,
                w: c.width + grow,
                h: st.h + grow,
                r: st.r + 2.5,
                angle,
                color: theme::with_alpha(pal.outline, c.hover * c.alpha),
                stroke: Some(1.5),
            });
        }
        out.push(Shape::RoundedRect {
            center,
            w: c.width,
            h: st.h,
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
    fn layout_fills_big_windows_with_more_lanes_and_stripes_not_zoom() {
        let small = Layout::new(DESIGN_W, DESIGN_H, 3, 1.0);
        let big = Layout::new(3840.0, 2160.0, 3, 1.0);
        assert_eq!(small.zoom, 1.0);
        assert_eq!(big.zoom, 1.0, "the board never scales with the window (#19)");
        assert!(big.lanes > small.lanes);
        assert!(big.bands >= small.bands);
        assert!(big.content_height() >= 2160.0 - BOTTOM_PAD);
    }

    #[test]
    fn user_zoom_scales_the_board_and_refills_lanes() {
        let base = Layout::new(DESIGN_W, DESIGN_H, 3, 1.0);
        let out = Layout::new(DESIGN_W, DESIGN_H, 3, 0.5);
        let inn = Layout::new(DESIGN_W, DESIGN_H, 3, 2.0);
        assert!((out.zoom - 0.5).abs() < 1e-6 && (inn.zoom - 2.0).abs() < 1e-6);
        assert!(out.lanes >= base.lanes, "zoomed out never needs fewer lanes");
        let tall = Layout::new(DESIGN_W, 2400.0, 3, 0.5);
        assert!(tall.lanes > base.lanes, "zoomed out on a tall window needs more lanes");
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
                let dy = pb.1 - pa.1;
                assert!(dy > 0.0);
                assert!((pa.0 - pb.0 - dy).abs() < 1e-3, "bend moves left exactly as far as the lane drops");
            }
        }
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
        assert_eq!(draws[2].p, 1.0, "finished subagent is unplugged");
        assert!((draws[0].tangent.angle_deg() - 45.0).abs() < 1.0, "first session rides the diagonal");
        assert!((draws[3].tangent.angle_deg() - 45.0).abs() < 1.0, "so does the second (#29)");
        for d in &draws[1..3] {
            assert!(d.tangent.angle_deg().abs() < 0.5, "{} sits on a horizontal (#36)", d.label);
            let lane = &lanes[d.lane];
            // The rounded corner starts CORNER_R before the apex, so probe a
            // little inside the margin: the trace must still be flat there.
            for s in [d.s_c - d.width / 2.0 - 16.0, d.s_c + d.width / 2.0 + 16.0] {
                assert!(lane.point_at(s).1.angle_deg().abs() < 0.5, "{} keeps clear of the bends", d.label);
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
        let (pieces, cut) = splice_lane(&lanes[draws[0].lane], &refs);
        assert!(pieces.iter().filter(|p| p.len() >= 2).count() >= 5);
        assert_eq!(cut, lanes[draws[0].lane].length(), "nothing unplugged: packets flow to the end");
    }
}
