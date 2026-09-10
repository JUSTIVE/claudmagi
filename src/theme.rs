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
    /// Failing CI on a PR connector — the one alarm colour (#56).
    pub alarm: Rgba,
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
    alarm: Rgba { r: 0.851, g: 0.176, b: 0.125, a: 1.0 }, // #D92D20
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
    alarm: Rgba { r: 0.549, g: 0.114, b: 0.094, a: 1.0 }, // #8C1D18, deep enough to part from the black chips
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
    alarm: Rgba { r: 1.0, g: 0.353, b: 0.290, a: 1.0 }, // #FF5A4A, lifted for the dark board
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

pub fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
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
