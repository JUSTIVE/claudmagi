//! Text on the board: D-DIN glyph outlines, filled, so labels can ride the
//! chips at any angle (#47). `scale` keeps the meaning it had with the old
//! stroke font — the cap height is `GLYPH_H * scale` px — so the chip styles
//! and the title badge did not have to change.
//!
//! The face is compiled into the binary (#55) rather than read from
//! `/System/Library/Fonts`, so the board looks the same on any machine and
//! the bundle carries no path dependency. D-DIN is © 2017 Datto Inc. under
//! the SIL Open Font License 1.1, which permits exactly this embedding; the
//! notice travels in `assets/fonts/OFL.txt` and in the file's own name table.

use std::sync::OnceLock;

use gpui::{Path, PathBuilder, PathStyle, Pixels, point, px};
use lyon::tessellation::{FillOptions, FillRule};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// Cap height in grid units; `scale` is px per unit.
pub const GLYPH_H: f32 = 6.0;
/// Advance per character when no font file could be loaded, in grid units.
const FALLBACK_ADVANCE: f32 = 5.7;

/// The board typeface, embedded so it is always present. Also handed to gpui
/// (`ui::register_fonts`) so the status bar and panels use the same face.
pub const FONT_TTF: &[u8] = include_bytes!("../assets/fonts/D-DIN.ttf");

struct Loaded {
    face: Face<'static>,
    /// Cap height in font units.
    cap: f32,
}

fn loaded() -> Option<&'static Loaded> {
    static FACE: OnceLock<Option<Loaded>> = OnceLock::new();
    FACE.get_or_init(|| {
        let face = Face::parse(FONT_TTF, 0).ok()?;
        let upem = face.units_per_em() as f32;
        let cap = face.capital_height().map(|c| c as f32).filter(|c| *c > 0.0).unwrap_or(upem * 0.72);
        Some(Loaded { face, cap })
    })
    .as_ref()
}

/// Name of the face in use, for diagnostics.
pub fn face_name() -> Option<String> {
    loaded()?
        .face
        .names()
        .into_iter()
        .find(|n| n.name_id == ttf_parser::name_id::FULL_NAME)
        .map(|n| n.to_string().unwrap_or_else(|| String::from_utf8_lossy(n.name).into_owned()))
}

/// Px per font unit at `scale`.
fn unit_px(l: &Loaded, scale: f32) -> f32 {
    GLYPH_H * scale / l.cap
}

fn glyph_of(l: &Loaded, c: char) -> Option<GlyphId> {
    l.face.glyph_index(c).or_else(|| l.face.glyph_index('?'))
}

fn kerning(l: &Loaded, left: GlyphId, right: GlyphId) -> f32 {
    let Some(kern) = l.face.tables().kern else { return 0.0 };
    kern.subtables
        .into_iter()
        .filter(|st| st.horizontal && !st.variable)
        .find_map(|st| st.glyphs_kerning(left, right))
        .map(|k| k as f32)
        .unwrap_or(0.0)
}

/// Glyphs with their pen x (font units) and the total advance.
fn shape(l: &Loaded, text: &str) -> (Vec<(GlyphId, f32)>, f32) {
    let mut out = Vec::new();
    let mut pen = 0.0f32;
    let mut prev: Option<GlyphId> = None;
    for c in text.chars() {
        let Some(g) = glyph_of(l, c) else { continue };
        if let Some(p) = prev {
            pen += kerning(l, p, g);
        }
        out.push((g, pen));
        pen += l.face.glyph_hor_advance(g).unwrap_or(0) as f32;
        prev = Some(g);
    }
    (out, pen)
}

/// Width of `text` in pixels at the given scale.
pub fn measure(text: &str, scale: f32) -> f32 {
    let n = text.chars().count();
    if n == 0 {
        return 0.0;
    }
    match loaded() {
        Some(l) => shape(l, text).1 * unit_px(l, scale),
        None => n as f32 * FALLBACK_ADVANCE * scale,
    }
}

/// Cap height in pixels; the text is centred on this box, descenders hang
/// below it.
pub fn height(scale: f32) -> f32 {
    GLYPH_H * scale
}

/// One outline command in screen pixels, relative to the text centre,
/// unrotated. Screen y grows downwards.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    Move(f32, f32),
    Line(f32, f32),
    Quad { cx: f32, cy: f32, x: f32, y: f32 },
    Cubic { c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32 },
    Close,
}

struct Sink<'a> {
    k: f32,
    dx: f32,
    base: f32,
    out: &'a mut Vec<Cmd>,
}

impl Sink<'_> {
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (self.dx + x * self.k, self.base - y * self.k)
    }
}

impl OutlineBuilder for Sink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.out.push(Cmd::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.out.push(Cmd::Line(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (cx, cy) = self.map(x1, y1);
        let (x, y) = self.map(x, y);
        self.out.push(Cmd::Quad { cx, cy, x, y });
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1x, c1y) = self.map(x1, y1);
        let (c2x, c2y) = self.map(x2, y2);
        let (x, y) = self.map(x, y);
        self.out.push(Cmd::Cubic { c1x, c1y, c2x, c2y, x, y });
    }
    fn close(&mut self) {
        self.out.push(Cmd::Close);
    }
}

/// Filled outline of `text` at `scale`, centred on the origin. Empty when
/// no font file could be loaded.
pub fn outline(text: &str, scale: f32) -> Vec<Cmd> {
    let mut out = Vec::new();
    let Some(l) = loaded() else { return out };
    let (glyphs, advance) = shape(l, text);
    let k = unit_px(l, scale);
    let x0 = -advance * k / 2.0;
    let base = height(scale) / 2.0;
    for (g, pen) in glyphs {
        let mut sink = Sink { k, dx: x0 + pen * k, base, out: &mut out };
        l.face.outline_glyph(g, &mut sink);
    }
    out
}

/// Builds a filled path for `text`, centred on `center`, rotated by
/// `angle_deg` (clockwise, screen coordinates).
pub fn build(text: &str, scale: f32, angle_deg: f32, center: (f32, f32)) -> Option<Path<Pixels>> {
    let cmds = outline(text, scale);
    if cmds.is_empty() {
        return None;
    }
    let style = PathStyle::Fill(FillOptions::default().with_fill_rule(FillRule::NonZero));
    let mut b = PathBuilder::fill().with_style(style);
    for c in cmds {
        match c {
            Cmd::Move(x, y) => b.move_to(point(px(x), px(y))),
            Cmd::Line(x, y) => b.line_to(point(px(x), px(y))),
            Cmd::Quad { cx, cy, x, y } => b.curve_to(point(px(x), px(y)), point(px(cx), px(cy))),
            Cmd::Cubic { c1x, c1y, c2x, c2y, x, y } => {
                b.cubic_bezier_to(point(px(x), px(y)), point(px(c1x), px(c1y)), point(px(c2x), px(c2y)))
            }
            Cmd::Close => b.close(),
        }
    }
    if angle_deg != 0.0 {
        b.rotate(angle_deg);
    }
    b.translate(point(px(center.0), px(center.1)));
    b.build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_face_loads_and_lays_out_text() {
        let name = face_name().expect("the embedded face parses");
        assert!(name.to_ascii_lowercase().contains("din"), "{name}");
        assert_eq!(measure("", 1.5), 0.0);
        let w1 = measure("CLAUDMAGI", 1.5);
        let w2 = measure("CLAUDMAGI-8B", 1.5);
        assert!(w1 > 0.0 && w2 > w1);
        // Cap height is what `scale` promises: 6 px per unit of scale.
        assert_eq!(height(1.5), 9.0);
        let cmds = outline("A", 1.5);
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Move(..))) && cmds.iter().any(|c| matches!(c, Cmd::Close)));
        // The outline sits on a baseline at +cap/2 and reaches up to -cap/2.
        let ys: Vec<f32> = cmds
            .iter()
            .filter_map(|c| match c {
                Cmd::Move(_, y) | Cmd::Line(_, y) => Some(*y),
                _ => None,
            })
            .collect();
        let (top, bottom) = ys.iter().fold((f32::MAX, f32::MIN), |(a, b), y| (a.min(*y), b.max(*y)));
        assert!((bottom - 4.5).abs() < 0.2, "baseline at +cap/2, got {bottom}");
        assert!((top + 4.5).abs() < 0.6, "cap top at -cap/2, got {top}");
    }
}
