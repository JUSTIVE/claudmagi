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

use anyhow::Result;
use gpui::{AssetSource, SharedString};

const GITHUB: &[u8] = include_bytes!("../assets/logos/github.svg");
const LINEAR: &[u8] = include_bytes!("../assets/logos/linear.svg");
const CLAUDE: &[u8] = include_bytes!("../assets/logos/claude.svg");

pub const GITHUB_PATH: &str = "logos/github.svg";
pub const LINEAR_PATH: &str = "logos/linear.svg";
pub const CLAUDE_PATH: &str = "logos/claude.svg";

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
