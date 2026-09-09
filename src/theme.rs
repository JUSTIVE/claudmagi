//! Colour palette sampled from the reference video.

pub use gpui::Rgba;
use gpui::{Hsla, rgb};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgba,
    pub line: Rgba,
    pub chip: Rgba,
    pub chip_needs: Rgba,
    pub chip_idle: Rgba,
    pub text_on: Rgba,
    pub text_needs: Rgba,
    pub text_idle: Rgba,
    pub outline: Rgba,
    pub ink: Rgba,
}

pub const PALETTE: Palette = Palette {
    bg: Rgba { r: 0.941, g: 0.290, b: 0.055, a: 1.0 },       // #F04A0E
    line: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },     // #120A05
    chip: Rgba { r: 0.055, g: 0.043, b: 0.039, a: 1.0 },     // #0E0B0A
    chip_needs: Rgba { r: 0.227, g: 0.078, b: 0.063, a: 1.0 }, // #3A1410
    chip_idle: Rgba { r: 0.165, g: 0.078, b: 0.063, a: 1.0 }, // #2A1410
    text_on: Rgba { r: 0.231, g: 0.890, b: 0.541, a: 1.0 },  // #3BE38A
    text_needs: Rgba { r: 1.0, g: 0.690, b: 0.227, a: 1.0 }, // #FFB03A
    text_idle: Rgba { r: 0.788, g: 0.467, b: 0.227, a: 1.0 }, // #C9773A
    outline: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },   // hover ring: black (#11)
    ink: Rgba { r: 0.070, g: 0.040, b: 0.020, a: 1.0 },
};

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
