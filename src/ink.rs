//! The colours a tile is painted in, worked out against the board behind it
//! (#139, #140). The honeycomb drew straight onto the board and these were
//! its; the start screen paints tiles with them.

use crate::theme::{self, Palette, Rgba};

/// Contrast every colour has to clear against what it is drawn on.
const READABLE: f32 = 3.0;

/// A colour made to read against whatever it is being drawn on (#140).
/// Which way to mix is the background's own question: against a dark fill a
/// colour has to go lighter and against the white board darker, and the
/// palette's two ends are the only colours guaranteed to be there to go
/// towards.
pub fn legible(c: Rgba, on: Rgba, pal: Palette) -> Rgba {
    let toward = if theme::contrast(pal.bg, on) > theme::contrast(pal.ink, on) { pal.bg } else { pal.ink };
    // Laid on first: a see-through colour does not answer the question until
    // it is standing on something (#139).
    theme::readable(theme::over(c, on), on, toward, READABLE)
}

/// A colour as something to paint with rather than write in (#139): laid on
/// the board first, then taken dark enough (or light enough) to hold a
/// label.
///
/// It moves the lightness and keeps the chroma (#236). Mixing the board's
/// ink in instead is what [`legible`] does for text, and on a fill it comes
/// out muddy — the board's green lost a third of its saturation that way.
pub fn floor_color(c: Rgba, pal: Palette) -> Rgba {
    theme::deepened(theme::over(c, pal.bg), pal.bg, READABLE)
}

pub fn ticket_color(t: &crate::ticket::Ticket, pal: Palette) -> Rgba {
    use crate::ticket::Status;
    match t.status {
        Some(Status::Started) => pal.text_idle,
        Some(Status::Done) => pal.done,
        Some(Status::Cancelled) | None => theme::with_alpha(pal.ink, 0.45),
        Some(_) => theme::with_alpha(pal.ink, 0.7),
    }
}

/// How far round the wheel a published page sits from the board's green:
/// into the teals, the one stretch of the wheel no state uses (#238). A
/// page is not a step along the way to landing, so it stands away from the
/// green-to-purple run rather than inside it.
const ARTIFACT_HUE: f32 = 40.0;

/// A page the session published (#137). Quiet grey was the colour of
/// everything the board had nothing to say about — an idle session, a pull
/// request GitHub would not answer for — and a page is not nothing.
pub fn artifact_color(pal: Palette) -> Rgba {
    theme::turned_hue(pal.text_on, ARTIFACT_HUE)
}

/// How far round the wheel a finished run nobody has opened sits from the
/// board's green: **right round**, to the colour opposite it (#259).
///
/// Green is work going well — a session running, a pull request open — so
/// the far side of the wheel is the one place a stopped run can stand
/// without borrowing somebody else's meaning. It is also, as it happens,
/// the widest gap left on this board: the accent and the alarm sit at 16°,
/// the ticket's amber and the running border at 36° and 45°, the greens at
/// 148°, the page's teal at 188°, review's blue at 226°, merged at 261°
/// and the queue at 289°. 328° is nobody's.
///
/// It is deliberately **not** the accent. The accent is a session calling
/// you; this one is not calling. It only has something you have not read.
const UNREAD_HUE: f32 = 180.0;

/// A run that finished with something to look at that nobody has opened
/// (#259).
pub fn unread_color(pal: Palette) -> Rgba {
    theme::turned_hue(pal.text_on, UNREAD_HUE)
}

/// How far round the wheel a pull request on its way in sits from one that
/// is in (#233): towards magenta, so the queue reads as the purple family
/// — landing — without being mistaken for the blue of a pull request that
/// is waiting on you.
const MERGING_HUE: f32 = 28.0;

/// How far round the wheel a signed-off pull request sits from the board's
/// green: into the blues (#225). Open is green while it is being looked
/// at; once review says yes it is a different thing waiting to land, and a
/// different colour says that at a glance where a second green could not
/// (#217, #224). Taken from the green rather than set as a colour of its
/// own, so it keeps the weight the board paints its tiles at.
const APPROVED_HUE: f32 = 78.0;

/// The fill of a pull request's tile. Colour is the whole signal (#57):
/// green while it is open and being looked at, **blue** once review has
/// signed it off and it is only waiting to land, purple once it has (#225).
/// Lightness cannot carry this — [`floor_color`] sets a fill's lightness so
/// a label can be read on it, which is why two greens came out of it all
/// but identical (#217) — so the blue is the green turned round the wheel.
pub fn pr_fill(p: &crate::pr::Pr, pal: Palette) -> Rgba {
    let c = pr_color(p, pal);
    // CI running has a colour of its own and says something else; a draft
    // and a closed one are already dim.
    if !p.running() && p.look() == crate::pr::Look::Open && p.approved {
        floor_color(theme::turned_hue(c, APPROVED_HUE), pal)
    } else {
        floor_color(c, pal)
    }
}

pub fn pr_color(p: &crate::pr::Pr, pal: Palette) -> Rgba {
    use crate::pr::Look;
    // In the queue the checks running are the queue's own, and what the
    // tile has to say is that it is going in (#231).
    if p.running() && p.look() != Look::Merging {
        return pal.busy;
    }
    match p.look() {
        // On its way in wears where it is going, a step round the wheel
        // from being in (#233).
        Look::Merging => theme::turned_hue(pal.merged, MERGING_HUE),
        Look::Merged => pal.merged,
        Look::Open => pal.text_on,
        Look::Failing => pal.alarm,
        Look::Draft => theme::with_alpha(pal.ink, 0.7),
        Look::Closed => theme::with_alpha(pal.ink, 0.35),
    }
}






#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::prelude::Hsla;

    /// A fill goes darker, not muddier (#236). The paint's hue and chroma
    /// survive the contrast floor, and the floor is still cleared — mixing
    /// the board's ink in instead cost the board's green a third of its
    /// saturation and made every tile read as earth.
    #[test]
    fn a_fill_keeps_the_colour_it_was_given() {
        for pal in [theme::PALETTE, theme::DARK] {
            for paint in
                [pal.text_on, pal.merged, pal.alarm, pal.text_idle, theme::turned_hue(pal.text_on, 78.0), unread_color(pal)]
            {
                let fill = floor_color(paint, pal);
                assert!(
                    theme::contrast(fill, pal.bg) >= READABLE - 0.01,
                    "{fill:?} does not read on the board: {:.2}",
                    theme::contrast(fill, pal.bg)
                );
                let (was, now): (Hsla, Hsla) = (paint.into(), fill.into());
                assert!((was.s - now.s).abs() < 0.02, "chroma lost: {:.2} -> {:.2}", was.s, now.s);
                let turn = (was.h - now.h).abs();
                assert!(turn.min(1.0 - turn) < 0.01, "hue moved: {:.3} -> {:.3}", was.h, now.h);
            }
        }
    }

    /// The board's greens were the proof: both of them used to come out of
    /// the floor at the same place, which is what sent the review states to
    /// hue in the first place (#217, #224). They still differ by hue alone.
    #[test]
    fn two_greens_still_cannot_be_told_apart_by_weight() {
        let pal = theme::PALETTE;
        let (a, b) = (floor_color(pal.text_on, pal), floor_color(pal.done, pal));
        assert!(theme::contrast(a, b) < 1.3, "{a:?} vs {b:?}");
    }
}
