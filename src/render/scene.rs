//! The shapes the honeycomb is drawn as, and the window it is drawn in.
//!
//! `render::comb` builds a frame as a flat list of `Shape`s; `render::paint`
//! draws that list with gpui and `render::svg` serialises it, so a layout can
//! be checked headlessly. This was the circuit board's renderer until that
//! view went (#183); what is left is what the honeycomb shares.

use crate::geom::Pt;
use crate::logos;
use crate::theme::Rgba;

/// How far one line of a mouse wheel scrolls the board.
pub const GAP: f32 = 22.0;

pub const DESIGN_W: f32 = 980.0;
pub const DESIGN_H: f32 = 620.0;
/// User zoom (⌘+ / ⌘-) is the only scaling: the board never grows with the
/// window (#19); a bigger window simply shows more of it.
pub const USER_ZOOM_MIN: f32 = 0.5;
pub const USER_ZOOM_MAX: f32 = 3.0;
pub const USER_ZOOM_STEP: f32 = 1.15;

/// The origin mark, against the Claude mark beside it. Smaller because
/// it answers the smaller question: what the node is comes first, where
/// it is running comes after (#122).
pub const ORIGIN_SIZE: f32 = 0.8;

/// The tick as a polyline, `w` across and centred on `at`. Down to the elbow,
/// then up and out: the long arm runs past the short one, which is what makes
/// it read as a tick at four pixels rather than as a bent line.
pub fn check_mark(at: Pt, w: f32) -> Vec<Vec<Pt>> {
    vec![vec![
        Pt::new(at.x - w * 0.46, at.y + w * 0.02),
        Pt::new(at.x - w * 0.14, at.y + w * 0.34),
        Pt::new(at.x + w * 0.46, at.y - w * 0.36),
    ]]
}

#[derive(Clone, Debug)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgba },
    Stroke { pieces: Vec<Vec<Pt>>, width: f32, color: Rgba },
    Text { text: String, scale: f32, stroke: f32, angle: f32, center: Pt, color: Rgba },
    /// A service's mark inside a node, drawn from the same kind of outline a
    /// label is (#80).
    Mark { mark: logos::Mark, size: f32, angle: f32, center: Pt, color: Rgba },
    /// A filled polygon. The comb's subagents are solid hexes (#96), and a
    /// stroke cannot say that.
    ///
    /// `screen` is how the fill is laid down: solid, or broken into a
    /// pattern of the same colour where the board prints rather than paints
    /// (#146, #149).
    Poly { points: Vec<Pt>, color: Rgba, screen: Screen },
}

/// How a fill is laid down (#149).
///
/// A board with one ink says a state with *texture*: at the size these are
/// drawn the eye reads the grain of a hatch — which way it runs and how fine
/// it is — long before it reads how dark the patch is. Both of these are a
/// fragment shader's work and cost one fill apiece, whatever area they
/// cover: the renderer has them already (gpui's `pattern_slash` and
/// `checkerboard`), so the screen is drawn at the pixel rather than at
/// whatever lattice the shapes were built on.
///
/// The sizes are in board pixels and do not follow the zoom: a screen
/// belongs to the sheet, not to what is drawn on it, so it stays the same
/// grain however far the board is zoomed out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Screen {
    Solid,
    /// Stripes at 45 degrees: `width` of ink, then `gap` of nothing.
    Slash { width: f32, gap: f32 },
}

impl Shape {
    /// A polygon filled the ordinary way.
    pub fn poly(points: Vec<Pt>, color: Rgba) -> Shape {
        Shape::Poly { points, color, screen: Screen::Solid }
    }
}

impl Shape {
    pub(crate) fn scale(&mut self, z: f32) {
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
            Shape::Text { center, scale, stroke, .. } => {
                *center = *center * z;
                *scale *= z;
                *stroke *= z;
            }
            Shape::Mark { center, size, .. } => {
                *center = *center * z;
                *size *= z;
            }
            Shape::Poly { points, .. } => {
                for p in points {
                    *p = *p * z;
                }
            }
        }
    }
}

/// The window in design units, at the user's zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub zoom: f32,
    pub width: f32,
    pub height: f32,
}

impl Layout {
    pub fn new(win_w: f32, win_h: f32, user_zoom: f32) -> Self {
        let zoom = user_zoom.clamp(USER_ZOOM_MIN, USER_ZOOM_MAX);
        Self { zoom, width: win_w / zoom, height: win_h / zoom }
    }
}
