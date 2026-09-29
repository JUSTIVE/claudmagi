//! Colour palette sampled from the reference video.

pub use gpui::Rgba;
use gpui::{Hsla, rgb};

/// Typeface for the gpui-drawn UI (status bar, panels). The board's chip
/// labels are drawn from the same face's outlines in `font.rs`; the family is
/// registered with gpui at startup from the embedded file. (#46, #47, #55)
pub const UI_FONT: &str = "D-DIN";

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgba,
    pub packet: Rgba,
    pub line: Rgba,
    pub chip: Rgba,
    pub chip_needs: Rgba,
    pub chip_idle: Rgba,
    pub text_on: Rgba,
    pub text_needs: Rgba,
    pub text_idle: Rgba,
    pub outline: Rgba,
    pub ink: Rgba,
    /// Failing CI on a PR connector — the one alarm colour (#56), and the
    /// label that reads on top of it (#58).
    pub alarm: Rgba,
    pub on_alarm: Rgba,
    /// Border on a PR whose CI is still running (#62).
    pub busy: Rgba,
    /// A landed pull request, and the label that reads on it (#82). Its own
    /// colour rather than the chip's black: merged is the one PR state that
    /// is neither good news nor bad, and it is worth telling apart at a
    /// glance from a session chip sitting on the same lane.
    pub merged: Rgba,
    pub on_merged: Rgba,
    /// A lane whose work finished end to end (#84). The chip greens are made
    /// to sit on a chip; this one has to read as a hairline on the board.
    pub done: Rgba,
    /// Whether a board has to blink what is waiting on a person rather than
    /// shout it in colour (#99). The orange board gave up the loud orange for
    /// its waiting chips because orange on orange is invisible (#50), and a
    /// pulse is what gives that back.
    pub pulse: bool,
}

pub const PALETTE: Palette = Palette {
    bg: Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },             // white (#16)
    packet: Rgba { r: 0.941, g: 0.290, b: 0.055, a: 1.0 },   // #F04A0E, the bumps on the traces (#16)
    line: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },     // #120A05
    chip: Rgba { r: 0.055, g: 0.043, b: 0.039, a: 1.0 },     // #0E0B0A
    chip_needs: Rgba { r: 0.941, g: 0.290, b: 0.055, a: 1.0 }, // #F04A0E — the waiting chip is the loud one (#50)
    chip_idle: Rgba { r: 0.227, g: 0.078, b: 0.063, a: 1.0 }, // #3A1410 — a finished session sinks into maroon (#50)
    text_on: Rgba { r: 0.231, g: 0.890, b: 0.541, a: 1.0 },  // #3BE38A
    text_needs: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 }, // ink on the orange needs-input chip
    text_idle: Rgba { r: 1.0, g: 0.690, b: 0.227, a: 1.0 }, // #FFB03A
    outline: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },   // hover ring: black (#11)
    ink: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },
    alarm: Rgba { r: 0.941, g: 0.290, b: 0.055, a: 1.0 },    // #F04A0E — a failing PR is orange (#58)
    on_alarm: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 }, // ink, as on the orange needs-input chip
    busy: Rgba { r: 0.851, g: 0.643, b: 0.0, a: 1.0 },       // #D9A400 — yellow that still reads on white
    merged: Rgba { r: 0.510, g: 0.314, b: 0.875, a: 1.0 },   // #8250DF — the purple a merged PR wears on GitHub
    on_merged: Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
    done: Rgba { r: 0.078, g: 0.545, b: 0.318, a: 1.0 },     // #148B51 — deep enough to read as a line on white
    pulse: false,
};

/// Dull amber for idle text where an orange chip would vanish (orange theme)
/// and for the panels' session lists.
pub const AMBER_DIM: Rgba = Rgba { r: 0.788, g: 0.467, b: 0.227, a: 1.0 }; // #C9773A

/// Reference-clip look: orange board, black traces and packets. Neither
/// detached chip can be orange here — it would vanish into the board — so
/// needs-input keeps the maroon + amber it had before the swap (#50), and
/// idle goes a shade deeper still.
pub const ORANGE: Palette = Palette {
    bg: Rgba { r: 0.941, g: 0.290, b: 0.055, a: 1.0 },
    packet: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },
    chip_needs: Rgba { r: 0.227, g: 0.078, b: 0.063, a: 1.0 }, // #3A1410
    text_needs: Rgba { r: 1.0, g: 0.690, b: 0.227, a: 1.0 },   // #FFB03A
    chip_idle: Rgba { r: 0.165, g: 0.078, b: 0.063, a: 1.0 }, // #2A1410
    text_idle: AMBER_DIM,
    alarm: Rgba { r: 0.549, g: 0.114, b: 0.094, a: 1.0 }, // #8C1D18 — orange on orange is invisible
    on_alarm: AMBER_DIM,
    busy: Rgba { r: 1.0, g: 0.878, b: 0.400, a: 1.0 }, // #FFE066, bright against the orange board
    // Deep enough to hold its own against the orange board, where the white
    // board's purple would read as another warm blob.
    merged: Rgba { r: 0.294, g: 0.161, b: 0.549, a: 1.0 }, // #4B298C
    on_merged: Rgba { r: 0.937, g: 0.902, b: 1.0, a: 1.0 }, // #EFE6FF
    done: Rgba { r: 0.055, g: 0.318, b: 0.196, a: 1.0 }, // #0E5132 — a lighter green disappears into orange
    pulse: true,
    ..PALETTE
};

/// Dark board: light traces; working chips are white with a deep green label
/// (#32), so they pop the way black chips do on the white board.
pub const DARK: Palette = Palette {
    bg: Rgba { r: 0.078, g: 0.071, b: 0.063, a: 1.0 },       // #141210
    line: Rgba { r: 0.929, g: 0.906, b: 0.871, a: 1.0 },     // #EDE7DE
    chip: Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
    text_on: Rgba { r: 0.078, g: 0.545, b: 0.318, a: 1.0 },  // #148B51
    chip_idle: Rgba { r: 0.290, g: 0.110, b: 0.086, a: 1.0 }, // #4A1C16 — lifted off the dark board (#50)
    outline: Rgba { r: 0.929, g: 0.906, b: 0.871, a: 1.0 },
    ink: Rgba { r: 0.929, g: 0.906, b: 0.871, a: 1.0 },
    busy: Rgba { r: 0.949, g: 0.788, b: 0.298, a: 1.0 }, // #F2C94C, lifted for the dark board
    // Lifted the way the rest of the dark board is, with the board's own
    // near-black for the label on top.
    merged: Rgba { r: 0.639, g: 0.443, b: 0.969, a: 1.0 }, // #A371F7
    on_merged: Rgba { r: 0.078, g: 0.071, b: 0.063, a: 1.0 }, // #141210
    done: Rgba { r: 0.231, g: 0.890, b: 0.541, a: 1.0 }, // #3BE38A — the board's own green, lifted for the dark
    ..PALETTE
};

impl Palette {
    pub fn for_theme(theme: crate::settings::BoardTheme) -> Palette {
        use crate::settings::BoardTheme;
        match theme {
            BoardTheme::White => PALETTE,
            BoardTheme::Orange => ORANGE,
            BoardTheme::Dark => DARK,
        }
    }
}

/// How long one blink takes, and how far down it dips.
const PULSE_PERIOD: f32 = 1.1;
const PULSE_LOW: f32 = 0.35;

/// The blink at `t` seconds: 1 at the top, `PULSE_LOW` at the bottom. A
/// board that does not blink is always at the top (#99).
pub fn pulse_at(pal: Palette, t: f32) -> f32 {
    if !pal.pulse {
        return 1.0;
    }
    let phase = (t / PULSE_PERIOD) * std::f32::consts::TAU;
    PULSE_LOW + (1.0 - PULSE_LOW) * (0.5 + 0.5 * phase.cos())
}

pub fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// Relative luminance, and WCAG's contrast ratio from it.
pub fn luma(c: Rgba) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (hi, lo) = if luma(a) > luma(b) { (luma(a), luma(b)) } else { (luma(b), luma(a)) };
    (hi + 0.05) / (lo + 0.05)
}

/// `colour` made to read against `on`, by mixing in as little `ink` as the
/// background allows.
///
/// A chip fills itself, so the palette's chip colours only ever have to read
/// against their own chip. Draw one straight onto the board instead — which
/// is what the comb does, its cells being outlines with nothing behind them
/// (#86) — and the pairing can collapse: amber on the orange board comes out
/// at 1.2:1. Contrast along the path to the ink is not monotonic, since the
/// two pass the background's own brightness, so the search keeps its upper
/// bound on a value that already clears the floor.
pub fn readable(colour: Rgba, on: Rgba, ink: Rgba, floor: f32) -> Rgba {
    if contrast(colour, on) >= floor {
        return colour;
    }
    if contrast(ink, on) < floor {
        return ink;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if contrast(lerp(colour, ink, mid), on) >= floor {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    lerp(colour, ink, hi)
}

pub fn with_alpha(c: Rgba, a: f32) -> Rgba {
    Rgba { a: (c.a * a).clamp(0.0, 1.0), ..c }
}

pub fn hsla(c: Rgba) -> Hsla {
    c.into()
}

#[allow(dead_code)]
pub fn hex(v: u32) -> Rgba {
    rgb(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only the orange board blinks, and what it does is a blink: it starts
    /// at full, dips, comes back, and never goes out (#99).
    #[test]
    fn the_orange_board_is_the_one_that_blinks() {
        for pal in [PALETTE, DARK] {
            for t in [0.0f32, 0.3, 0.55, 1.0, 7.3] {
                assert_eq!(pulse_at(pal, t), 1.0, "a board that shouts in colour has no need to blink");
            }
        }
        assert!(ORANGE.pulse);
        assert_eq!(pulse_at(ORANGE, 0.0), 1.0, "it starts at full");
        assert!((pulse_at(ORANGE, PULSE_PERIOD / 2.0) - PULSE_LOW).abs() < 1e-5, "and dips to the floor");
        assert!((pulse_at(ORANGE, PULSE_PERIOD) - 1.0).abs() < 1e-5, "one period later it is back");
        let mut lo = f32::MAX;
        for step in 0..120 {
            let v = pulse_at(ORANGE, step as f32 * PULSE_PERIOD / 60.0);
            assert!((PULSE_LOW - 1e-5..=1.0 + 1e-5).contains(&v), "the blink stays between its bounds");
            lo = lo.min(v);
        }
        assert!(lo > 0.0, "it never goes out entirely, which would read as a chip that vanished");
    }
}
