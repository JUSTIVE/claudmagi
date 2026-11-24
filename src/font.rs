//! The board typeface, D-DIN, compiled into the binary (#55) rather than read
//! from `/System/Library/Fonts`, so the board looks the same on any machine
//! and the bundle carries no path dependency. D-DIN is © 2017 Datto Inc. under
//! the SIL Open Font License 1.1, which permits exactly this embedding; the
//! notice travels in `assets/fonts/OFL.txt` and in the file's own name table.
//!
//! The UI registers these bytes with the shell at start-up and sets its text
//! in them. D-DIN covers Latin-1 and a little punctuation and no Hangul; what
//! it lacks the shell takes from a system face on its own (#119).

/// The board typeface, embedded so it is always present. Registered with the
/// shell by `ui::app` so the start screen and its panels use the same face.
pub const FONT_TTF: &[u8] = include_bytes!("../assets/fonts/D-DIN.ttf");
