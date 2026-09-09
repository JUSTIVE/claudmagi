//! A tiny stroke-based vector font (4×6 unit grid) so labels can be drawn as
//! paths and rotated along the traces, matching the PCB silkscreen look.

use gpui::{Path, PathBuilder, PathStyle, Pixels, StrokeOptions, point, px};
use lyon::path::{LineCap, LineJoin};

type Strokes = &'static [&'static [(f32, f32)]];

pub const GLYPH_W: f32 = 4.0;
pub const GLYPH_H: f32 = 6.0;
pub const ADVANCE: f32 = 5.7;

const A: Strokes = &[&[(0., 6.), (0., 2.), (2., 0.), (4., 2.), (4., 6.)], &[(0., 3.5), (4., 3.5)]];
const B: Strokes = &[
    &[(0., 0.), (0., 6.)],
    &[(0., 0.), (3., 0.), (4., 1.), (4., 2.), (3., 3.), (0., 3.)],
    &[(3., 3.), (4., 4.), (4., 5.), (3., 6.), (0., 6.)],
];
const C: Strokes = &[&[(4., 1.), (3., 0.), (1., 0.), (0., 1.), (0., 5.), (1., 6.), (3., 6.), (4., 5.)]];
const D: Strokes = &[&[(0., 0.), (0., 6.)], &[(0., 0.), (3., 0.), (4., 1.), (4., 5.), (3., 6.), (0., 6.)]];
const E: Strokes = &[&[(4., 0.), (0., 0.), (0., 6.), (4., 6.)], &[(0., 3.), (3., 3.)]];
const F: Strokes = &[&[(4., 0.), (0., 0.), (0., 6.)], &[(0., 3.), (3., 3.)]];
const G: Strokes = &[&[(4., 1.), (3., 0.), (1., 0.), (0., 1.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 3.), (2., 3.)]];
const H: Strokes = &[&[(0., 0.), (0., 6.)], &[(4., 0.), (4., 6.)], &[(0., 3.), (4., 3.)]];
const I: Strokes = &[&[(1., 0.), (3., 0.)], &[(2., 0.), (2., 6.)], &[(1., 6.), (3., 6.)]];
const J: Strokes = &[&[(4., 0.), (4., 5.), (3., 6.), (1., 6.), (0., 5.)]];
const K: Strokes = &[&[(0., 0.), (0., 6.)], &[(4., 0.), (0., 3.)], &[(1.2, 2.1), (4., 6.)]];
const L: Strokes = &[&[(0., 0.), (0., 6.), (4., 6.)]];
const M: Strokes = &[&[(0., 6.), (0., 0.), (2., 3.), (4., 0.), (4., 6.)]];
const N: Strokes = &[&[(0., 6.), (0., 0.), (4., 6.), (4., 0.)]];
const O: Strokes = &[&[(1., 0.), (3., 0.), (4., 1.), (4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 1.), (1., 0.)]];
const P: Strokes = &[&[(0., 6.), (0., 0.), (3., 0.), (4., 1.), (4., 2.), (3., 3.), (0., 3.)]];
const Q: Strokes = &[
    &[(1., 0.), (3., 0.), (4., 1.), (4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 1.), (1., 0.)],
    &[(2.5, 4.5), (4., 6.)],
];
const R: Strokes = &[&[(0., 6.), (0., 0.), (3., 0.), (4., 1.), (4., 2.), (3., 3.), (0., 3.)], &[(2., 3.), (4., 6.)]];
const S: Strokes = &[&[
    (4., 1.), (3., 0.), (1., 0.), (0., 1.), (0., 2.), (1., 3.), (3., 3.), (4., 4.), (4., 5.), (3., 6.), (1., 6.), (0., 5.),
]];
const T: Strokes = &[&[(0., 0.), (4., 0.)], &[(2., 0.), (2., 6.)]];
const U: Strokes = &[&[(0., 0.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 0.)]];
const V: Strokes = &[&[(0., 0.), (2., 6.), (4., 0.)]];
const W: Strokes = &[&[(0., 0.), (1., 6.), (2., 2.5), (3., 6.), (4., 0.)]];
const X: Strokes = &[&[(0., 0.), (4., 6.)], &[(4., 0.), (0., 6.)]];
const Y: Strokes = &[&[(0., 0.), (2., 3.), (4., 0.)], &[(2., 3.), (2., 6.)]];
const Z: Strokes = &[&[(0., 0.), (4., 0.), (0., 6.), (4., 6.)]];
const D0: Strokes = &[
    &[(1., 0.), (3., 0.), (4., 1.), (4., 5.), (3., 6.), (1., 6.), (0., 5.), (0., 1.), (1., 0.)],
    &[(1., 5.), (3., 1.)],
];
const D1: Strokes = &[&[(1., 1.), (2., 0.), (2., 6.)], &[(1., 6.), (3., 6.)]];
const D2: Strokes = &[&[(0., 1.), (1., 0.), (3., 0.), (4., 1.), (4., 2.), (0., 6.), (4., 6.)]];
const D3: Strokes = &[&[(0., 0.), (4., 0.), (2., 2.5), (3., 2.5), (4., 3.5), (4., 5.), (3., 6.), (1., 6.), (0., 5.)]];
const D4: Strokes = &[&[(3., 6.), (3., 0.), (0., 4.), (4., 4.)]];
const D5: Strokes = &[&[(4., 0.), (0., 0.), (0., 3.), (3., 3.), (4., 4.), (4., 5.), (3., 6.), (1., 6.), (0., 5.)]];
const D6: Strokes = &[&[(4., 1.), (3., 0.), (1., 0.), (0., 1.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 4.), (3., 3.), (0., 3.)]];
const D7: Strokes = &[&[(0., 0.), (4., 0.), (1.5, 6.)]];
const D8: Strokes = &[
    &[(1., 0.), (3., 0.), (4., 1.), (4., 2.), (3., 3.), (1., 3.), (0., 2.), (0., 1.), (1., 0.)],
    &[(1., 3.), (0., 4.), (0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 4.), (3., 3.)],
];
const D9: Strokes = &[&[(0., 5.), (1., 6.), (3., 6.), (4., 5.), (4., 1.), (3., 0.), (1., 0.), (0., 1.), (0., 2.), (1., 3.), (4., 3.)]];
const DASH: Strokes = &[&[(0.5, 3.), (3.5, 3.)]];
const UNDER: Strokes = &[&[(0., 6.), (4., 6.)]];
const DOT: Strokes = &[&[(2., 5.6), (2., 6.)]];
const SLASH: Strokes = &[&[(0., 6.), (4., 0.)]];
const COLON: Strokes = &[&[(2., 1.5), (2., 1.9)], &[(2., 4.5), (2., 4.9)]];
const QUESTION: Strokes = &[&[(0., 1.), (1., 0.), (3., 0.), (4., 1.), (4., 2.), (2., 3.5), (2., 4.3)], &[(2., 5.6), (2., 6.)]];
const BANG: Strokes = &[&[(2., 0.), (2., 4.)], &[(2., 5.6), (2., 6.)]];
const PLUS: Strokes = &[&[(0.5, 3.), (3.5, 3.)], &[(2., 1.5), (2., 4.5)]];
const TILDE: Strokes = &[&[(0., 3.5), (1., 2.5), (3., 3.5), (4., 2.5)]];
const AT: Strokes = &[&[(3., 4.), (3., 2.), (1.5, 2.), (1.5, 4.), (4., 4.), (4., 1.), (3., 0.), (1., 0.), (0., 1.), (0., 5.), (1., 6.), (3., 6.)]];
const SPACE: Strokes = &[];
const UNKNOWN: Strokes = &[&[(0.5, 0.5), (3.5, 0.5), (3.5, 5.5), (0.5, 5.5), (0.5, 0.5)]];

pub fn glyph(c: char) -> Strokes {
    match c.to_ascii_uppercase() {
        'A' => A, 'B' => B, 'C' => C, 'D' => D, 'E' => E, 'F' => F, 'G' => G, 'H' => H, 'I' => I,
        'J' => J, 'K' => K, 'L' => L, 'M' => M, 'N' => N, 'O' => O, 'P' => P, 'Q' => Q, 'R' => R,
        'S' => S, 'T' => T, 'U' => U, 'V' => V, 'W' => W, 'X' => X, 'Y' => Y, 'Z' => Z,
        '0' => D0, '1' => D1, '2' => D2, '3' => D3, '4' => D4, '5' => D5, '6' => D6, '7' => D7,
        '8' => D8, '9' => D9,
        '-' => DASH, '_' => UNDER, '.' => DOT, '/' => SLASH, ':' => COLON, '?' => QUESTION,
        '!' => BANG, '+' => PLUS, '~' => TILDE, '@' => AT, ' ' => SPACE,
        _ => UNKNOWN,
    }
}

/// Width of `text` in pixels at the given scale (px per grid unit).
pub fn measure(text: &str, scale: f32) -> f32 {
    let n = text.chars().count();
    if n == 0 {
        return 0.0;
    }
    ((n as f32 - 1.0) * ADVANCE + GLYPH_W) * scale
}

pub fn height(scale: f32) -> f32 {
    GLYPH_H * scale
}

/// Builds a stroked path for `text`, centred on `center`, rotated by
/// `angle_deg` (clockwise, screen coordinates).
pub fn build(
    text: &str,
    scale: f32,
    stroke: f32,
    angle_deg: f32,
    center: (f32, f32),
) -> Option<Path<Pixels>> {
    let width = measure(text, scale);
    if width <= 0.0 {
        return None;
    }
    let x0 = -width / 2.0;
    let y0 = -height(scale) / 2.0;

    let options = StrokeOptions::default()
        .with_line_width(stroke)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round);
    let mut builder = PathBuilder::stroke(px(stroke)).with_style(PathStyle::Stroke(options));
    let mut any = false;
    for (i, c) in text.chars().enumerate() {
        let gx = x0 + i as f32 * ADVANCE * scale;
        for stroke_pts in glyph(c) {
            for (k, (ux, uy)) in stroke_pts.iter().enumerate() {
                let p = point(px(gx + ux * scale), px(y0 + uy * scale));
                if k == 0 {
                    builder.move_to(p);
                } else {
                    builder.line_to(p);
                }
                any = true;
            }
        }
    }
    if !any {
        return None;
    }
    if angle_deg != 0.0 {
        builder.rotate(angle_deg);
    }
    builder.translate(point(px(center.0), px(center.1)));
    builder.build().ok()
}
