//! The marks the start screen and the search draw beside what they name (#79).
//!
//! The platform draws an SVG as authored, its `currentColor` the element's
//! text colour; [`markup`] hands it each mark recoloured that way, so a logo
//! takes the colour of the text beside it without being told. The files are
//! compiled in the way the typeface is (#55), so an installed bundle carries
//! them with no resources lookup at runtime.
//!
//! See `assets/logos/README.md` for where they came from and what a
//! replacement has to look like.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const GITHUB: &[u8] = include_bytes!("../assets/logos/github.svg");
const LINEAR: &[u8] = include_bytes!("../assets/logos/linear.svg");
const CLAUDE: &[u8] = include_bytes!("../assets/logos/claude.svg");
const WARP: &[u8] = include_bytes!("../assets/logos/warp.svg");
const ORCA: &[u8] = include_bytes!("../assets/logos/orca.svg");
const HERDR: &[u8] = include_bytes!("../assets/logos/herdr.svg");
const ANTHROPIC: &[u8] = include_bytes!("../assets/logos/anthropic.svg");
const CLOUD: &[u8] = include_bytes!("../assets/logos/cloud.svg");
const ARTIFACT: &[u8] = include_bytes!("../assets/logos/artifact.svg");

/// Which mark a node wears (#80).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mark {
    /// A session, and the assistant working in it.
    Claude,
    /// A pull request.
    GitHub,
    /// A Linear issue.
    Linear,
    /// Where a session is running, drawn beside the Claude mark (#122). The
    /// Claude one says what a node is; these say where it is.
    Warp,
    Orca,
    /// Herdr, whose terminals outlive the window they are shown in (#189).
    Herdr,
    /// The Claude desktop app. Anthropic's own mark rather than Claude's,
    /// because Claude's is already in the slot next door and two of them
    /// side by side read as a mistake.
    ClaudeApp,
    /// Not on this machine at all (#120).
    Cloud,
    /// A page the session published on claude.ai (#137). A window, because
    /// that is what an artifact is: something made here that opens over
    /// there.
    Artifact,
}

impl Mark {
    /// Every mark, for the tests that check the files themselves.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: [Mark; 9] = [
        Mark::Claude,
        Mark::GitHub,
        Mark::Linear,
        Mark::Warp,
        Mark::Orca,
        Mark::Herdr,
        Mark::ClaudeApp,
        Mark::Cloud,
        Mark::Artifact,
    ];

    /// The mark for a seat's host, or `None` where the board cannot say
    /// where the session is beyond "a terminal" (#122).
    pub fn host(host: &str) -> Option<Mark> {
        match host {
            "warp" => Some(Mark::Warp),
            "orca" => Some(Mark::Orca),
            "herdr" => Some(Mark::Herdr),
            "desktop" => Some(Mark::ClaudeApp),
            "cloud" => Some(Mark::Cloud),
            _ => None,
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Mark::Claude => CLAUDE,
            Mark::GitHub => GITHUB,
            Mark::Linear => LINEAR,
            Mark::Warp => WARP,
            Mark::Orca => ORCA,
            Mark::Herdr => HERDR,
            Mark::ClaudeApp => ANTHROPIC,
            Mark::Cloud => CLOUD,
            Mark::Artifact => ARTIFACT,
        }
    }
}

/// Every mark's markup, with its fills set to `currentColor` (#79): drawn as
/// authored, it takes the text colour of the element it sits in, which is
/// how a mark takes its tile's colour without one of its own.
pub fn markup(mark: Mark) -> &'static str {
    static CACHE: OnceLock<Mutex<HashMap<Mark, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    cache.entry(mark).or_insert_with(|| {
        let text = String::from_utf8_lossy(mark.bytes()).into_owned();
        Box::leak(tint(&text).into_boxed_str())
    })
}

/// `svg` with every colour it names put to `currentColor`, and a fill on the
/// root so shapes that name none take it too.
fn tint(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len() + 32);
    let mut rest = svg;
    while let Some(at) = rest.find("fill=\"") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 6..];
        let end = after.find('"').unwrap_or(after.len());
        let value = &after[..end];
        if value == "none" {
            out.push_str("fill=\"none\"");
        } else {
            out.push_str("fill=\"currentColor\"");
        }
        rest = &after[(end + 1).min(after.len())..];
    }
    out.push_str(rest);
    match out.find("<svg") {
        Some(at) => {
            let (head, tail) = out.split_at(at + 4);
            format!("{head} fill=\"currentColor\"{tail}")
        }
        None => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mark that does not parse renders as nothing at all, and an empty slot
    /// reads as a bug rather than a missing file, so check them here.
    #[test]
    fn every_mark_is_a_single_colour_svg_with_a_viewbox() {
        for (mark, path) in Mark::ALL.map(|m| (m, format!("{m:?}"))) {
            let bytes = mark.bytes();
            let text = String::from_utf8(bytes.to_vec()).expect("svg is text");
            assert!(text.contains("viewBox"), "{path} needs a viewBox to scale into its slot");
            assert!(text.contains("<path"), "{path} has nothing to draw");
            // A mark is drawn in its tile's one colour, so a file carrying its own
            // palette would arrive as a flat silhouette and surprise whoever
            // dropped it in.
            assert!(!text.contains("linearGradient"), "{path} would lose its gradient to the recolouring");
        }
        for mark in Mark::ALL {
            assert!(markup(mark).contains("currentColor"), "{mark:?} would not take its tile's colour");
        }
    }

    /// The parser the platform's SVG support is built on: a mark it cannot
    /// read, or one that covers nothing, draws an empty slot, and an empty
    /// slot looks like a bug in the board rather than a bad file.
    #[test]
    fn every_mark_parses_and_covers_its_box() {
        for (mark, path) in Mark::ALL.map(|m| (m, format!("{m:?}"))) {
            let bytes = mark.bytes();
            let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
                .unwrap_or_else(|e| panic!("{path} is not something the platform can draw: {e}"));
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
