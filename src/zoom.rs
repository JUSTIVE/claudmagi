//! The window's size and the user's zoom.

/// How far one line of a mouse wheel scrolls.
pub const GAP: f32 = 22.0;

/// The window a fresh board opens at.
pub const DESIGN_W: f32 = 980.0;
pub const DESIGN_H: f32 = 620.0;
/// User zoom (⌘+ / ⌘-) is the only scaling: the board never grows with the
/// window (#19); a bigger window simply shows more of it.
pub const USER_ZOOM_MIN: f32 = 0.5;
pub const USER_ZOOM_MAX: f32 = 3.0;
pub const USER_ZOOM_STEP: f32 = 1.15;

/// The window in design units, at the user's zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub zoom: f32,
    pub width: f32,
    pub height: f32,
}

impl Layout {
    pub fn new(win_w: f32, win_h: f32, user_zoom: f32) -> Self {
        let zoom = user_zoom.clamp(USER_ZOOM_MIN, USER_ZOOM_MAX);
        Self { zoom, width: win_w / zoom, height: win_h / zoom }
    }
}
