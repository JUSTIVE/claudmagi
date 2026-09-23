//! The marks the search draws inside each chip (#79).
//!
//! gpui reads an SVG through an `AssetSource` and renders it as a mask: the
//! silhouette is all that survives, and the colour comes from the element's
//! `text_color`, which is how a logo takes its chip's label colour without
//! being told. The files are compiled in the way the typeface is (#55), so an
//! installed bundle carries them with no resources lookup at runtime.
//!
//! See `assets/logos/README.md` for where they came from and what a
//! replacement has to look like.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use gpui::{AssetSource, SharedString};

use crate::font;

const GITHUB: &[u8] = include_bytes!("../assets/logos/github.svg");
const LINEAR: &[u8] = include_bytes!("../assets/logos/linear.svg");
const CLAUDE: &[u8] = include_bytes!("../assets/logos/claude.svg");

pub const GITHUB_PATH: &str = "logos/github.svg";
pub const LINEAR_PATH: &str = "logos/linear.svg";
pub const CLAUDE_PATH: &str = "logos/claude.svg";

/// Which mark a node wears (#80).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mark {
    /// A session, and the assistant working in it.
    Claude,
    /// A pull request.
    GitHub,
    /// A Linear issue.
    Linear,
}

impl Mark {
    pub const ALL: [Mark; 3] = [Mark::Claude, Mark::GitHub, Mark::Linear];

    pub fn path(self) -> &'static str {
        match self {
            Mark::Claude => CLAUDE_PATH,
            Mark::GitHub => GITHUB_PATH,
            Mark::Linear => LINEAR_PATH,
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Mark::Claude => CLAUDE,
            Mark::GitHub => GITHUB,
            Mark::Linear => LINEAR,
        }
    }
}

/// Every mark flattened once into the board's own path commands, in a box one
/// unit across centred on the origin. The board draws its labels from glyph
/// outlines (`font::outline`), and a mark is the same kind of thing, so it
/// goes down the same pipe: the gpui canvas and an `--svg` export both get it
/// for free. (#80)
fn unit_outline(mark: Mark) -> &'static Vec<font::Cmd> {
    static CACHE: OnceLock<Mutex<HashMap<Mark, &'static Vec<font::Cmd>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(mark).or_insert_with(|| Box::leak(Box::new(flatten(mark))))
}

fn flatten(mark: Mark) -> Vec<font::Cmd> {
    let Ok(tree) = usvg::Tree::from_data(mark.bytes(), &usvg::Options::default()) else {
        log::warn!("{} is not renderable", mark.path());
        return Vec::new();
    };
    // Into a unit box: the file's own size is whatever its viewBox says, and
    // the caller asks in pixels.
    let size = tree.size();
    let k = 1.0 / size.width().max(size.height()).max(1.0);
    let (ox, oy) = (size.width() / 2.0, size.height() / 2.0);
    let mut out = Vec::new();
    walk(tree.root(), &mut |path| {
        let t = path.abs_transform();
        let at = |x: f32, y: f32| {
            let mut p = usvg::tiny_skia_path::Point::from_xy(x, y);
            t.map_point(&mut p);
            ((p.x - ox) * k, (p.y - oy) * k)
        };
        for seg in path.data().segments() {
            out.push(match seg {
                usvg::tiny_skia_path::PathSegment::MoveTo(p) => {
                    let (x, y) = at(p.x, p.y);
                    font::Cmd::Move(x, y)
                }
                usvg::tiny_skia_path::PathSegment::LineTo(p) => {
                    let (x, y) = at(p.x, p.y);
                    font::Cmd::Line(x, y)
                }
                usvg::tiny_skia_path::PathSegment::QuadTo(c, p) => {
                    let ((cx, cy), (x, y)) = (at(c.x, c.y), at(p.x, p.y));
                    font::Cmd::Quad { cx, cy, x, y }
                }
                usvg::tiny_skia_path::PathSegment::CubicTo(a, b, p) => {
                    let ((c1x, c1y), (c2x, c2y), (x, y)) = (at(a.x, a.y), at(b.x, b.y), at(p.x, p.y));
                    font::Cmd::Cubic { c1x, c1y, c2x, c2y, x, y }
                }
                usvg::tiny_skia_path::PathSegment::Close => font::Cmd::Close,
            });
        }
    });
    out
}

fn walk(group: &usvg::Group, f: &mut impl FnMut(&usvg::Path)) {
    for node in group.children() {
        match node {
            usvg::Node::Path(p) => f(p),
            usvg::Node::Group(g) => walk(g, f),
            _ => {}
        }
    }
}

/// A mark's outline at `size` pixels across, relative to its centre, the way
/// `font::outline` gives a label's.
pub fn outline(mark: Mark, size: f32) -> Vec<font::Cmd> {
    unit_outline(mark).iter().map(|c| scale_cmd(*c, size)).collect()
}

fn scale_cmd(cmd: font::Cmd, k: f32) -> font::Cmd {
    match cmd {
        font::Cmd::Move(x, y) => font::Cmd::Move(x * k, y * k),
        font::Cmd::Line(x, y) => font::Cmd::Line(x * k, y * k),
        font::Cmd::Quad { cx, cy, x, y } => font::Cmd::Quad { cx: cx * k, cy: cy * k, x: x * k, y: y * k },
        font::Cmd::Cubic { c1x, c1y, c2x, c2y, x, y } => font::Cmd::Cubic {
            c1x: c1x * k,
            c1y: c1y * k,
            c2x: c2x * k,
            c2y: c2y * k,
            x: x * k,
            y: y * k,
        },
        font::Cmd::Close => font::Cmd::Close,
    }
}

/// Serves the three marks and nothing else.
pub struct Logos;

impl AssetSource for Logos {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            GITHUB_PATH => Some(Cow::Borrowed(GITHUB)),
            LINEAR_PATH => Some(Cow::Borrowed(LINEAR)),
            CLAUDE_PATH => Some(Cow::Borrowed(CLAUDE)),
            _ => None,
        })
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![GITHUB_PATH.into(), LINEAR_PATH.into(), CLAUDE_PATH.into()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mark that does not parse renders as nothing at all, and an empty chip
    /// reads as a bug rather than a missing file, so check them here.
    #[test]
    fn every_mark_is_a_single_colour_svg_with_a_viewbox() {
        for path in [GITHUB_PATH, LINEAR_PATH, CLAUDE_PATH] {
            let bytes = Logos.load(path).unwrap().expect("the source serves it");
            let text = String::from_utf8(bytes.to_vec()).expect("svg is text");
            assert!(text.contains("viewBox"), "{path} needs a viewBox to scale into the chip");
            assert!(text.contains("<path"), "{path} has nothing to draw");
            // gpui paints the mask in one colour, so a file carrying its own
            // palette would arrive as a flat silhouette and surprise whoever
            // dropped it in.
            assert!(!text.contains("linearGradient"), "{path} would lose its gradient to the mask");
        }
        assert!(Logos.load("logos/nothing.svg").unwrap().is_none());
    }

    /// The renderer's own parser, on the renderer's own terms: a mark it
    /// cannot read, or one that covers nothing, draws an empty chip, and an
    /// empty chip looks like a bug in the board rather than a bad file.
    #[test]
    fn every_mark_parses_and_covers_its_box() {
        for path in [GITHUB_PATH, LINEAR_PATH, CLAUDE_PATH] {
            let bytes = Logos.load(path).unwrap().unwrap();
            let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())
                .unwrap_or_else(|e| panic!("{path} is not something gpui can render: {e}"));
            let (w, h) = (tree.size().width(), tree.size().height());
            assert!(w > 0.0 && h > 0.0, "{path} has no size to draw into");
            let ink = tree.root().abs_bounding_box();
            assert!(
                ink.width() > w * 0.5 && ink.height() > h * 0.5,
                "{path} draws in a corner of its own box, so it would arrive tiny"
            );
        }
    }
}
