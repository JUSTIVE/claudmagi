//! gpui views: the board itself and the floating panels.

pub mod board;
pub mod devtools;
pub mod panel;
pub mod settings;

/// Hands gpui the embedded board typeface so `theme::UI_FONT` resolves to the
/// same face the chip labels are drawn from (#55).
pub fn register_fonts(cx: &mut gpui::App) {
    if let Err(e) = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(crate::font::FONT_TTF)]) {
        log::warn!("could not register {}: {e}", crate::theme::UI_FONT);
    }
}
