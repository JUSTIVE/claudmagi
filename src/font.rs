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
//!
//! D-DIN covers 232 codepoints, which is Latin-1 and a little punctuation and
//! no Hangul at all, so every Korean session name on the board came out as a
//! row of `?` — the glyph the lookup fell back to. The gpui-drawn panels never
//! showed it because gpui falls back to a system face on its own. This does
//! the same now: the faces are a chain, each character takes the first one
//! that has it, and only what D-DIN lacks ever reaches the system. (#119)

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

/// Faces to fall back on, in the order a character is offered to them, with
/// the index of the face to take out of a collection.
///
/// Mapped rather than read: the Korean one is 55MB and a board full of ASCII
/// never touches a page of it. They are macOS's own, so they are there on any
/// machine this runs on, and a missing one is simply skipped.
const SYSTEM_FALLBACKS: &[(&str, u32)] = &[
    ("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0),
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/System/Library/Fonts/Apple Symbols.ttf", 0),
];

struct Loaded {
    face: Face<'static>,
    /// Cap height in font units.
    cap: f32,
    /// Em size in font units, which is what the faces are matched on.
    upem: f32,
}

impl Loaded {
    fn new(data: &'static [u8], index: u32) -> Option<Self> {
        let face = Face::parse(data, index).ok()?;
        let upem = face.units_per_em() as f32;
        if upem <= 0.0 {
            return None;
        }
        let cap = face.capital_height().map(|c| c as f32).filter(|c| *c > 0.0).unwrap_or(upem * 0.72);
        Some(Loaded { face, cap, upem })
    }
}

/// The board face first, then whatever the system has for what it does not
/// cover. Empty only when the embedded face itself will not parse.
fn faces() -> &'static [Loaded] {
    static FACES: OnceLock<Vec<Loaded>> = OnceLock::new();
    FACES.get_or_init(|| {
        let Some(primary) = Loaded::new(FONT_TTF, 0) else { return Vec::new() };
        let mut out = vec![primary];
        for (path, index) in SYSTEM_FALLBACKS {
            let Ok(file) = std::fs::File::open(path) else { continue };
            // Safety: these are the OS's own font files, which do not change
            // under a running process. The map is leaked because the faces
            // borrow from it for the life of the program.
            let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else { continue };
            let bytes: &'static [u8] = Box::leak(Box::new(map));
            match Loaded::new(bytes, *index) {
                Some(l) => out.push(l),
                None => log::warn!("{path} is not a face this can read"),
            }
        }
        out
    })
}

fn loaded() -> Option<&'static Loaded> {
    faces().first()
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

/// Px per font unit for one face at `scale`.
///
/// Every face is drawn to the same cap height, which is the one thing `scale`
/// promises, so the promise holds whichever face a character came out of. A
/// text engine would match ems instead; here the two agree to within a few
/// percent, D-DIN and the system Korean face carrying almost the same cap to
/// em ratio, and cap matching is the one the rest of this module is written
/// in. A Hangul syllable ends up standing a little above the Latin caps
/// beside it and dipping a little below the baseline — less, either way, than
/// the lowercase ascenders and descenders the board already draws outside
/// that band. (#119)
fn unit_px_of(l: &Loaded, scale: f32) -> f32 {
    GLYPH_H * scale / l.cap
}

/// The first face that has `c`, or the board face's `?` when none does.
fn glyph_of(c: char) -> Option<(usize, GlyphId)> {
    let faces = faces();
    faces
        .iter()
        .enumerate()
        .find_map(|(i, l)| l.face.glyph_index(c).map(|g| (i, g)))
        .or_else(|| faces.first()?.face.glyph_index('?').map(|g| (0, g)))
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

/// One glyph, placed: which face it came out of and where its pen sits.
struct Placed {
    face: usize,
    glyph: GlyphId,
    /// Pen position in pixels. Font units will not do any more: two faces in
    /// one run measure in different units.
    x: f32,
}

/// Glyphs with their pen x and the total advance, both in pixels.
fn shape(text: &str, scale: f32) -> (Vec<Placed>, f32) {
    let faces = faces();
    let mut out = Vec::new();
    let mut pen = 0.0f32;
    let mut prev: Option<(usize, GlyphId)> = None;
    for c in text.chars() {
        let Some((fi, glyph)) = glyph_of(c) else { continue };
        let Some(l) = faces.get(fi) else { continue };
        let k = unit_px_of(l, scale);
        // Kerning is a pair in one face; across a fallback boundary there is
        // no pair to look up.
        if let Some((pf, pg)) = prev {
            if pf == fi {
                pen += kerning(l, pg, glyph) * k;
            }
        }
        out.push(Placed { face: fi, glyph, x: pen });
        pen += l.face.glyph_hor_advance(glyph).unwrap_or(0) as f32 * k;
        prev = Some((fi, glyph));
    }
    (out, pen)
}

/// Width of `text` in pixels at the given scale.
pub fn measure(text: &str, scale: f32) -> f32 {
    let n = text.chars().count();
    if n == 0 {
        return 0.0;
    }
    if loaded().is_none() {
        return n as f32 * FALLBACK_ADVANCE * scale;
    }
    shape(text, scale).1
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
    let faces = faces();
    if faces.is_empty() {
        return out;
    }
    let (glyphs, advance) = shape(text, scale);
    let x0 = -advance / 2.0;
    let base = height(scale) / 2.0;
    for p in glyphs {
        let Some(l) = faces.get(p.face) else { continue };
        let mut sink = Sink { k: unit_px_of(l, scale), dx: x0 + p.x, base, out: &mut out };
        l.face.outline_glyph(p.glyph, &mut sink);
    }
    out
}

/// Builds a filled path for `text`, centred on `center`, rotated by
/// `angle_deg` (clockwise, screen coordinates).
pub fn build(text: &str, scale: f32, angle_deg: f32, center: (f32, f32)) -> Option<Path<Pixels>> {
    path_of(outline(text, scale), angle_deg, center)
}

/// Builds a filled path from outline commands, centred on `center` and
/// rotated by `angle_deg`. Labels come from glyphs and the chip marks come
/// from SVG (#80), and past this point they are the same thing.
pub fn path_of(cmds: Vec<Cmd>, angle_deg: f32, center: (f32, f32)) -> Option<Path<Pixels>> {
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

    /// Korean is the case that forced this (#119): D-DIN has 232 codepoints
    /// and not one of them is Hangul, so every Korean session name came out
    /// of `glyph_of` as the `?` it falls back to — a row of question marks
    /// that looked like a bug in the board rather than a gap in the face.
    #[test]
    fn a_character_the_board_face_lacks_comes_out_of_another_one() {
        assert!(faces().len() > 1, "the system has no fallback face to offer");
        assert!(
            super::loaded().unwrap().face.glyph_index('\u{ac1c}').is_none(),
            "this test is only about characters the board face does not have"
        );
        let (face, _) = glyph_of('\u{ac1c}').expect("something has it");
        assert!(face > 0, "Hangul has to come from a fallback");

        // The failure this replaces: every unmapped character used to render
        // as `?`, so a Korean label and a row of question marks were the same
        // picture and the same width.
        let hangul = outline("\u{ac1c}\u{c120}", 1.5);
        let marks = outline("??", 1.5);
        assert!(!hangul.is_empty(), "Hangul draws something");
        assert_ne!(hangul, marks, "Hangul is still rendering as question marks");
        assert!(
            (measure("\u{ac1c}\u{c120}", 1.5) - measure("??", 1.5)).abs() > 0.5,
            "and it is not even the same width"
        );

        // A face that cannot be found anywhere still has to produce
        // something, since a label that draws nothing reads as a missing
        // session rather than a missing glyph.
        assert!(!outline("\u{10ffff}", 1.5).is_empty(), "the last resort is still `?`");
    }

    /// Every face in the chain is drawn at one em, which is what keeps a
    /// mixed label sitting on one line at one size (#119).
    #[test]
    fn a_mixed_label_is_one_line_at_one_size() {
        let scale = 1.5;
        let latin = outline("HA", scale);
        let mixed = outline("H\u{ac1c}A", scale);
        let ys = |cmds: &[Cmd]| {
            cmds.iter()
                .filter_map(|c| match c {
                    Cmd::Move(_, y) | Cmd::Line(_, y) => Some(*y),
                    _ => None,
                })
                .fold((f32::MAX, f32::MIN), |(a, b), y| (a.min(y), b.max(y)))
        };
        let (latin_top, latin_base) = ys(&latin);
        let (mixed_top, mixed_base) = ys(&mixed);
        // Both faces draw on the one baseline, so a Hangul syllable sits on
        // the Latin's, reaching a little above the caps and dipping a little
        // under it rather than floating off somewhere of its own.
        assert!(mixed_top <= latin_top + 0.01, "Hangul should reach at least as high as a cap");
        assert!(mixed_base >= latin_base - 0.01, "and at least as far down as the baseline");

        // The board already draws outside the cap band — every descender does,
        // and D-DIN's reach below the baseline is wider than a Hangul
        // syllable's reach above the caps. What this has to catch is a face
        // scaled by the wrong units: the chain holds a 2048-unit face next to
        // two 1000-unit ones, so forgetting whose units a glyph is in puts it
        // out by a factor of two rather than by a fraction.
        // Scaling is checked on the faces themselves rather than through
        // characters. Every face the chain actually reaches here measures in
        // 1000 units as D-DIN does, so a units mix-up would be invisible in
        // any glyph that comes out of it — and silent until the day a face
        // that measures in 2048 is the one with the character.
        for (i, l) in faces().iter().enumerate() {
            let drawn = l.cap * unit_px_of(l, scale);
            assert!(
                (drawn - height(scale)).abs() < 0.01,
                "face {i} ({} units) draws a cap of {drawn} where {} was promised",
                l.upem,
                height(scale)
            );
        }

        let (_, descender) = ys(&outline("hqp", scale));
        assert!(mixed_base <= descender, "Hangul hangs below the board face's own descenders");

        // And the run is laid out left to right across the boundary.
        assert!(measure("H\u{ac1c}A", scale) > measure("HA", scale));
    }

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
