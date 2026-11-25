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
/// the board first, then made to hold a label drawn in the board's colour.
pub fn floor_color(c: Rgba, pal: Palette) -> Rgba {
    legible(theme::over(c, pal.bg), pal.bg, pal)
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
    if p.running() {
        return pal.busy;
    }
    match p.look() {
        Look::Merged => pal.merged,
        Look::Open => pal.text_on,
        Look::Failing => pal.alarm,
        Look::Draft => theme::with_alpha(pal.ink, 0.7),
        Look::Closed => theme::with_alpha(pal.ink, 0.35),
    }
}


