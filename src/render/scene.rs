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
use crate::model::{BoardModel, Phase};
use crate::theme::{self, PALETTE, Rgba};

pub const GAP: f32 = 16.0;
pub const LINE_W: f32 = 1.6;
pub const CHIP_H: f32 = 22.0;
pub const CHIP_R: f32 = 4.0;
pub const CHIP_PAD: f32 = 13.0;
pub const CHIP_MIN_W: f32 = 56.0;
pub const TEXT_SCALE: f32 = 1.5;
pub const TEXT_STROKE: f32 = 1.45;
pub const DIAG: f32 = 8.0 * GAP;
pub const CORNER_R: f32 = 8.0;
pub const TOP_PAD: f32 = 54.0;
pub const BOTTOM_PAD: f32 = 34.0;
pub const PULL: f32 = 16.0;
pub const SOCK_W: f32 = 24.0;
pub const SOCK_H: f32 = 14.0;
pub const NOTCH_W: f32 = 6.0;
pub const NOTCH_H: f32 = 6.0;
pub const WAVE_LEN: f32 = 46.0;
pub const WAVE_AMP: f32 = 6.0;
pub const MIN_LANES: usize = 24;
pub const TITLE_SCALE: f32 = 2.6;

#[derive(Clone, Debug)]
pub struct ChipDraw {
    pub lane: usize,
    pub s_c: f32,
    pub width: f32,
    pub label: String,
    pub p: f32,
    pub hover: f32,
    pub alpha: f32,
    pub phase: Phase,
    /// Content-space centre (before scrolling).
    pub center: Pt,
    pub tangent: Pt,
}

#[derive(Clone)]
pub struct Frame {
    pub lanes: Rc<Vec<Polyline>>,
    pub chips: Vec<ChipDraw>,
    pub scroll_y: f32,
    pub t: f32,
    pub layout: Layout,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgba },
    Stroke { pieces: Vec<Vec<Pt>>, width: f32, color: Rgba },
    RoundedRect { center: Pt, w: f32, h: f32, r: f32, angle: f32, color: Rgba, stroke: Option<f32> },
    Text { text: String, scale: f32, stroke: f32, angle: f32, center: Pt, color: Rgba },
}

pub const DESIGN_W: f32 = 980.0;
pub const DESIGN_H: f32 = 620.0;
pub const MAX_ZOOM: f32 = 2.6;
/// Extra lanes above lane 0 whose diagonals fill the top-right corner.
pub const LEAD_LANES: usize = 8;
/// Where the first diagonal stripe meets the top edge, as a fraction of width.
pub const ANCHOR: f32 = 0.62;
/// Horizontal distance between diagonal stripes, as a fraction of width.
pub const STRIPE_PITCH: f32 = 0.55;

/// Board geometry derived from the window size. All layout happens in
/// "design units"; `zoom` maps them to window pixels so a fullscreen 4K
/// board keeps the density of the reference clip instead of a sparse corner.
///
/// Every lane is one global staircase: lane `i` bends `GAP` further left
/// than lane `i - 1`, so the bends form 45° stripes running from top-right
/// to bottom-left. Wide windows get extra stripes (`bands`) `STRIPE_PITCH`
/// apart; this construction never lets two lanes cross.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub zoom: f32,
    pub width: f32,
    pub height: f32,
    pub bands: usize,
    pub lanes: usize,
}

impl Layout {
    pub fn new(win_w: f32, win_h: f32, chips: usize) -> Self {
        let zoom = ((win_w * win_h) / (DESIGN_W * DESIGN_H)).sqrt().clamp(1.0, MAX_ZOOM);
        let width = win_w / zoom;
        let height = win_h / zoom;
        // Stripes are needed until one starts far enough right to still be
        // on screen at the bottom-right corner.
        let span = width + height - width * ANCHOR;
        let bands = (span / (width * STRIPE_PITCH)).ceil().max(1.0) as usize;
        let fill = ((height - TOP_PAD - DIAG - BOTTOM_PAD) / GAP).ceil().max(0.0) as usize + 1;
        let lanes = LEAD_LANES + MIN_LANES.max(chips + 14).max(fill);
        Self { zoom, width, height, bands, lanes }
    }

    pub fn content_height(&self) -> f32 {
        TOP_PAD + (self.lanes - LEAD_LANES) as f32 * GAP + DIAG + BOTTOM_PAD
    }

    /// Lane index (into `build_lanes`) for the `k`-th chip.
    pub fn chip_lane(k: usize) -> usize {
        LEAD_LANES + 1 + k
    }

    pub fn build_lanes(&self) -> Vec<Polyline> {
        (0..self.lanes).map(|i| lane_path(i, self)).collect()
    }
}

/// For lane `idx`, one `(ax, y0)` per stripe: where that stripe's diagonal
/// starts. `ax` may be off screen on either side. The first `LEAD_LANES`
/// lanes sit above the top padding.
pub fn lane_params(idx: usize, layout: &Layout) -> Vec<(f32, f32)> {
    let i = idx as f32 - LEAD_LANES as f32;
    (0..layout.bands)
        .map(|b| {
            let ax = layout.width * (ANCHOR + b as f32 * STRIPE_PITCH) - i * GAP;
            let y = TOP_PAD + i * GAP + b as f32 * DIAG;
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
    corners.push(Pt::new((layout.width + 24.0).max(ax_last + DIAG + 1.0), y_last + DIAG));
    Polyline::rounded(&corners, CORNER_R)
}

pub fn chip_width(label: &str) -> f32 {
    CHIP_MIN_W.max(font::measure(label, TEXT_SCALE) + 2.0 * CHIP_PAD)
}

/// Arc length of the chip centre on its lane: the first stripe whose
/// diagonal is comfortably on screen.
pub fn chip_anchor(lane: &Polyline, lane_index: usize, on_diag: bool, layout: &Layout, chip_w: f32) -> f32 {
    let params = lane_params(lane_index, layout);
    let (ax, y0) = params
        .iter()
        .copied()
        .find(|(ax, _)| ax + DIAG / 2.0 > 40.0)
        .unwrap_or(*params.last().unwrap());
    let target = if on_diag {
        Pt::new(ax + DIAG / 2.0, y0 + DIAG / 2.0)
    } else {
        Pt::new(ax + DIAG + CORNER_R + 70.0 + chip_w / 2.0, y0 + DIAG)
    };
    lane.nearest_s(target)
}

pub fn hash01(i: usize) -> f32 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x1234_5678);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 10_000) as f32 / 10_000.0
}

struct Splice {
    pieces: Vec<Vec<Pt>>,
    wave_start: f32,
}

/// Cuts a lane around its chip: loose-cable wave, socket, gap, chip.
fn splice_lane(lane: &Polyline, c: &ChipDraw) -> Splice {
    let p = c.p;
    let sock_c = c.s_c - c.width / 2.0 - 6.0;
    let sock_left = sock_c - SOCK_W / 2.0;
    let sock_right = sock_c + SOCK_W / 2.0;
    let chip_left = c.s_c - c.width / 2.0 + p * PULL;
    let end1 = chip_left.min(sock_right);
    let wave_end = sock_left;
    let wave_start = wave_end - WAVE_LEN;

    let mut pieces = vec![lane.slice(0.0, wave_start)];
    const N: usize = 26;
    let amp = WAVE_AMP * p;
    let mut wave = Vec::with_capacity(N + 1);
    for k in 0..=N {
        let t = k as f32 / N as f32;
        let s = wave_start + t * WAVE_LEN;
        let (pos, tan) = lane.point_at(s);
        let env = (1.0 - (std::f32::consts::TAU * t).cos()) / 2.0;
        let off = amp * (std::f32::consts::TAU * t).sin() * env;
        wave.push(pos + tan.perp() * off);
    }
    pieces.push(wave);
    pieces.push(lane.slice(wave_end, end1));
    pieces.push(lane.slice(c.s_c + p * PULL, lane.length()));
    Splice { pieces, wave_start }
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
    let ox = origin.x;
    let oy = origin.y - f.scroll_y;
    let to_screen = move |p: Pt| Pt::new(p.x + ox, p.y + oy);
    let mut out = Vec::new();
    let (width, height) = (f.layout.width, f.layout.height);

    out.push(Shape::Rect { x: origin.x, y: origin.y, w: width, h: height, color: PALETTE.bg });

    let chip_by_lane: HashMap<usize, &ChipDraw> = f.chips.iter().map(|c| (c.lane, c)).collect();

    // Traces + packets.
    for (li, lane) in f.lanes.iter().enumerate() {
        let (Some(first), Some(last)) = (lane.pts.first(), lane.pts.last()) else { continue };
        let y_min = first.y.min(last.y) + oy - 40.0;
        let y_max = first.y.max(last.y) + oy + 40.0;
        if y_max < origin.y || y_min > origin.y + height {
            continue;
        }
        let chip = chip_by_lane.get(&li).copied();
        let (pieces, cut) = match chip {
            Some(c) => {
                let s = splice_lane(lane, c);
                (s.pieces, if c.p > 0.35 { s.wave_start } else { lane.length() })
            }
            None => (vec![lane.pts.clone()], lane.length()),
        };
        let pieces: Vec<Vec<Pt>> =
            pieces.into_iter().map(|piece| piece.into_iter().map(to_screen).collect()).collect();
        out.push(Shape::Stroke { pieces, width: LINE_W, color: PALETTE.line });

        let working = chip.is_some_and(|c| c.phase == Phase::Working && c.alpha > 0.5);
        let (spacing, speed, size, alpha) = if working {
            (104.0, 96.0, (7.0, 3.0), 1.0)
        } else {
            (230.0 + hash01(li * 7 + 1) * 120.0, 22.0 + hash01(li * 3 + 2) * 10.0, (5.0, 2.4), 0.85)
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
                    color: theme::with_alpha(PALETTE.line, alpha),
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
        let sock_c = c.s_c - c.width / 2.0 - 6.0;
        let (pos, tan) = lane.point_at(sock_c);
        let center = to_screen(pos);
        let angle = tan.angle_deg();
        out.push(Shape::RoundedRect {
            center,
            w: SOCK_W,
            h: SOCK_H,
            r: 2.5,
            angle,
            color: theme::with_alpha(PALETTE.line, a),
            stroke: None,
        });
        let notch_c = center + tan * (SOCK_W / 2.0 - NOTCH_W / 2.0 + 0.6);
        out.push(Shape::RoundedRect {
            center: notch_c,
            w: NOTCH_W,
            h: NOTCH_H,
            r: 0.6,
            angle,
            color: theme::with_alpha(PALETTE.bg, a),
            stroke: None,
        });
    }

    // Chips + labels.
    for c in &f.chips {
        if c.alpha <= 0.01 {
            continue;
        }
        let angle = c.tangent.angle_deg();
        let center = to_screen(c.center);
        let (fill_off, text_off) = match c.phase {
            Phase::NeedsUser | Phase::Working => (PALETTE.chip_needs, PALETTE.text_needs),
            Phase::Idle => (PALETTE.chip_idle, PALETTE.text_idle),
        };
        let fill_c = theme::with_alpha(theme::lerp(PALETTE.chip, fill_off, c.p), c.alpha);
        let text_c = theme::with_alpha(theme::lerp(PALETTE.text_on, text_off, c.p), c.alpha);

        if c.hover > 0.01 {
            let grow = 5.0 + 1.5 * c.hover;
            out.push(Shape::RoundedRect {
                center,
                w: c.width + grow,
                h: CHIP_H + grow,
                r: CHIP_R + 2.5,
                angle,
                color: theme::with_alpha(PALETTE.outline, c.hover * c.alpha),
                stroke: Some(1.5),
            });
        }
        out.push(Shape::RoundedRect {
            center,
            w: c.width,
            h: CHIP_H,
            r: CHIP_R,
            angle,
            color: fill_c,
            stroke: None,
        });
        out.push(Shape::Text {
            text: c.label.clone(),
            scale: TEXT_SCALE,
            stroke: TEXT_STROKE,
            angle,
            center,
            color: text_c,
        });
    }

    // Title badge, on a patch of background so the lead lanes stay clear of it.
    let title = "CLAUDMAGI";
    let tw = font::measure(title, TITLE_SCALE);
    let center = Pt::new(origin.x + 26.0 + tw / 2.0, origin.y + 26.0);
    out.push(Shape::RoundedRect {
        center,
        w: tw + 26.0 + 28.0,
        h: 30.0 + 18.0,
        r: 9.0,
        angle: 0.0,
        color: PALETTE.bg,
        stroke: None,
    });
    out.push(Shape::RoundedRect {
        center,
        w: tw + 26.0,
        h: 30.0,
        r: 5.0,
        angle: 0.0,
        color: PALETTE.chip,
        stroke: None,
    });
    out.push(Shape::Text {
        text: title.to_string(),
        scale: TITLE_SCALE,
        stroke: 2.4,
        angle: 0.0,
        center,
        color: PALETTE.text_on,
    });

    out
}

/// Projects every chip of the model onto its lane.
pub fn chip_draws(model: &BoardModel, layout: &Layout, lanes: &[Polyline], now: Instant) -> Vec<ChipDraw> {
    model
        .chips
        .iter()
        .enumerate()
        .filter_map(|(k, c)| {
            let lane_idx = Layout::chip_lane(k);
            let lane = lanes.get(lane_idx)?;
            let label = c.info.label();
            let width = chip_width(&label);
            let on_diag = k % 2 == 0;
            let s_c = chip_anchor(lane, lane_idx, on_diag, layout, width);
            let p = smoothstep(c.disconnect);
            let (center, tangent) = lane.point_at(s_c + p * PULL);
            Some(ChipDraw {
                lane: lane_idx,
                s_c,
                width,
                label,
                p,
                hover: c.hover_t,
                alpha: c.alpha(now),
                phase: c.phase(),
                center,
                tangent,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fills_big_windows_with_more_lanes_and_stripes() {
        let small = Layout::new(DESIGN_W, DESIGN_H, 3);
        let big = Layout::new(3840.0, 2160.0, 3);
        assert_eq!(small.zoom, 1.0);
        assert!(big.zoom > 1.0 && big.zoom <= MAX_ZOOM);
        assert!(big.lanes > small.lanes);
        assert!(big.bands >= small.bands);
        assert!(big.content_height() * big.zoom >= 2160.0 - BOTTOM_PAD * big.zoom);
    }

    #[test]
    fn lanes_form_a_staircase_that_never_crosses() {
        let layout = Layout::new(1920.0, 1080.0, 8);
        for i in 1..layout.lanes {
            let a = lane_params(i - 1, &layout);
            let b = lane_params(i, &layout);
            for (pa, pb) in a.iter().zip(b.iter()) {
                assert!((pa.0 - pb.0 - GAP).abs() < 1e-3, "bend moves exactly one gap left per lane");
                assert!((pb.1 - pa.1 - GAP).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn chips_alternate_between_diagonal_and_horizontal_placement() {
        let layout = Layout::new(DESIGN_W, DESIGN_H, 2);
        let lanes = layout.build_lanes();
        let mut model = BoardModel::new();
        model.apply(
            vec![
                crate::model::SessionInfo::synthetic(1, "a", Phase::Working),
                crate::model::SessionInfo::synthetic(2, "b", Phase::Idle),
            ],
            Instant::now(),
        );
        model.settle();
        let draws = chip_draws(&model, &layout, &lanes, Instant::now());
        assert_eq!(draws.len(), 2);
        assert!((draws[0].tangent.angle_deg() - 45.0).abs() < 1.0, "first chip rides the diagonal");
        assert!(draws[1].tangent.angle_deg().abs() < 1.0, "second chip sits on the horizontal");
        assert_eq!(draws[0].p, 0.0);
        assert_eq!(draws[1].p, 1.0);
    }
}
