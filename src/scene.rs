//! Renderer-independent description of a board frame.
//!
//! `build_shapes` turns lanes + chips into a flat list of `Shape`s. The gpui
//! view paints them with paths; `to_svg` writes the same list as SVG so the
//! layout can be checked headlessly (`claudmagi --svg out.svg`).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use crate::font;
use crate::geom::{Polyline, Pt, smoothstep};
use crate::sessions::Phase;
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
pub const BOTTOM_PAD: f32 = 70.0;
pub const PULL: f32 = 16.0;
pub const SOCK_W: f32 = 24.0;
pub const SOCK_H: f32 = 14.0;
pub const NOTCH_W: f32 = 6.0;
pub const NOTCH_H: f32 = 6.0;
pub const WAVE_LEN: f32 = 46.0;
pub const WAVE_AMP: f32 = 6.0;
pub const BUNDLE: usize = 12;
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
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgba },
    Stroke { pieces: Vec<Vec<Pt>>, width: f32, color: Rgba },
    RoundedRect { center: Pt, w: f32, h: f32, r: f32, angle: f32, color: Rgba, stroke: Option<f32> },
    Text { text: String, scale: f32, stroke: f32, angle: f32, center: Pt, color: Rgba },
}

pub fn lane_count(chips: usize) -> usize {
    MIN_LANES.max(chips + 14)
}

pub fn content_height(chips: usize) -> f32 {
    TOP_PAD + lane_count(chips) as f32 * GAP + DIAG + BOTTOM_PAD
}

/// Where lane `i` starts its diagonal (`ax`) and its top-left y.
pub fn lane_params(i: usize, width: f32) -> (f32, f32) {
    let y0 = TOP_PAD + i as f32 * GAP;
    let bundle = i / BUNDLE;
    let j = (i % BUNDLE) as f32;
    let anchor = if bundle % 2 == 0 { width * 0.46 } else { width * 0.30 };
    let ax = (anchor - j * GAP).max(72.0);
    (ax, y0)
}

pub fn lane_path(i: usize, width: f32) -> Polyline {
    let (ax, y0) = lane_params(i, width);
    let corners = [
        Pt::new(-24.0, y0),
        Pt::new(ax, y0),
        Pt::new(ax + DIAG, y0 + DIAG),
        Pt::new(width + 24.0, y0 + DIAG),
    ];
    Polyline::rounded(&corners, CORNER_R)
}

pub fn build_lanes(count: usize, width: f32) -> Vec<Polyline> {
    (0..count).map(|i| lane_path(i, width)).collect()
}

pub fn chip_width(label: &str) -> f32 {
    CHIP_MIN_W.max(font::measure(label, TEXT_SCALE) + 2.0 * CHIP_PAD)
}

/// Arc length of the chip centre on its lane.
pub fn chip_anchor(lane: &Polyline, lane_index: usize, on_diag: bool, width: f32, chip_w: f32) -> f32 {
    let (ax, y0) = lane_params(lane_index, width);
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

/// Produces the draw list for a frame. `origin` is the on-screen position of
/// the content's (0, 0) *after* scrolling has been applied.
pub fn build_shapes(f: &Frame, origin: Pt) -> Vec<Shape> {
    let ox = origin.x;
    let oy = origin.y - f.scroll_y;
    let to_screen = move |p: Pt| Pt::new(p.x + ox, p.y + oy);
    let mut out = Vec::new();

    out.push(Shape::Rect { x: origin.x, y: origin.y, w: f.width, h: f.height, color: PALETTE.bg });

    let chip_by_lane: HashMap<usize, &ChipDraw> = f.chips.iter().map(|c| (c.lane, c)).collect();

    // Traces + packets.
    for (li, lane) in f.lanes.iter().enumerate() {
        let (Some(first), Some(last)) = (lane.pts.first(), lane.pts.last()) else { continue };
        let y_min = first.y.min(last.y) + oy - 40.0;
        let y_max = first.y.max(last.y) + oy + 40.0;
        if y_max < origin.y || y_min > origin.y + f.height {
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

    // Title badge.
    let title = "CLAUDMAGI";
    let tw = font::measure(title, TITLE_SCALE);
    let center = Pt::new(origin.x + 26.0 + tw / 2.0, origin.y + 26.0);
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

fn svg_color(c: Rgba) -> String {
    format!(
        "rgba({},{},{},{:.3})",
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        c.a
    )
}

/// Serialises shapes as a standalone SVG document.
pub fn to_svg(shapes: &[Shape], width: f32, height: f32) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#
    );
    for shape in shapes {
        match shape {
            Shape::Rect { x, y, w, h, color } => {
                let _ = writeln!(s, r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{}"/>"#, svg_color(*color));
            }
            Shape::Stroke { pieces, width, color } => {
                for piece in pieces {
                    if piece.len() < 2 {
                        continue;
                    }
                    let pts: Vec<String> = piece.iter().map(|p| format!("{:.2},{:.2}", p.x, p.y)).collect();
                    let _ = writeln!(
                        s,
                        r#"<polyline points="{}" fill="none" stroke="{}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                        pts.join(" "),
                        svg_color(*color)
                    );
                }
            }
            Shape::RoundedRect { center, w, h, r, angle, color, stroke } => {
                let (fill, stroke_attr) = match stroke {
                    Some(sw) => ("none".to_string(), format!(r#"stroke="{}" stroke-width="{sw}""#, svg_color(*color))),
                    None => (svg_color(*color), String::new()),
                };
                let _ = writeln!(
                    s,
                    r#"<rect x="{:.2}" y="{:.2}" width="{w:.2}" height="{h:.2}" rx="{r}" fill="{fill}" {stroke_attr} transform="rotate({angle:.2} {:.2} {:.2})"/>"#,
                    center.x - w / 2.0,
                    center.y - h / 2.0,
                    center.x,
                    center.y
                );
            }
            Shape::Text { text, scale, stroke, angle, center, color } => {
                let width = font::measure(text, *scale);
                let x0 = center.x - width / 2.0;
                let y0 = center.y - font::height(*scale) / 2.0;
                let _ = writeln!(
                    s,
                    r#"<g fill="none" stroke="{}" stroke-width="{stroke}" stroke-linecap="round" stroke-linejoin="round" transform="rotate({angle:.2} {:.2} {:.2})">"#,
                    svg_color(*color),
                    center.x,
                    center.y
                );
                for (i, ch) in text.chars().enumerate() {
                    let gx = x0 + i as f32 * font::ADVANCE * scale;
                    for stroke_pts in font::glyph(ch) {
                        let pts: Vec<String> = stroke_pts
                            .iter()
                            .map(|(ux, uy)| format!("{:.2},{:.2}", gx + ux * scale, y0 + uy * scale))
                            .collect();
                        if pts.len() == 1 {
                            let _ = writeln!(s, r#"<polyline points="{} {}"/>"#, pts[0], pts[0]);
                        } else {
                            let _ = writeln!(s, r#"<polyline points="{}"/>"#, pts.join(" "));
                        }
                    }
                }
                let _ = writeln!(s, "</g>");
            }
        }
    }
    s.push_str("</svg>\n");
    s
}
