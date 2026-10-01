//! gpui views: the board itself and the floating panels.

pub mod board;
pub mod devtools;
pub mod list;
pub mod palette;
pub mod panel;
pub mod settings;

/// Hands gpui the embedded board typeface so `theme::UI_FONT` resolves to the
/// same face the chip labels are drawn from (#55).
pub fn register_fonts(cx: &mut gpui::App) {
    if let Err(e) = cx.text_system().add_fonts(vec![std::borrow::Cow::Borrowed(crate::font::FONT_TTF)]) {
        log::warn!("could not register {}: {e}", crate::theme::UI_FONT);
    }
    // Registering it is not the same as gpui finding it, and a family it
    // cannot find renders as nothing at all rather than as a fallback: every
    // panel, the status bar and the search go mute together, with no other
    // sign. Say so here. (#115)
    if !cx.text_system().all_font_names().iter().any(|n| n == crate::theme::UI_FONT) {
        log::warn!("{} is not among the families gpui knows, so the UI will have no text", crate::theme::UI_FONT);
    }
}
