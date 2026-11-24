//! Glue between the board settings and the nsh-metro theme, plus the
//! compact Metro pieces the panels are built from.
//!
//! nsh-metro's own controls are sized for a phone (52 px buttons); the
//! panels here are dense, so these keep its look (square corners, 2 px
//! borders, the accent filling a pressed or chosen control, lower-case
//! headers) at panel scale. Their colours are bound to the theme, so a
//! change of accent or ground re-skins them without building them again.

use nsh_core::prelude::*;
use nsh_metro::{Accent, Mode, Theme, label, palette};

use crate::settings::{BoardTheme, DEFAULT_ACCENT, Settings};

/// Side padding inside a panel.
pub const PAD: f32 = 16.;
/// Body text inside the panels.
pub const BODY: f32 = 12.;
/// Captions and section headers.
pub const SMALL: f32 = 11.;
/// Seconds between the live tiles' waves of slides (#184).
const LIVE_TILE_BEAT: f32 = 3.0;

pub fn accent_of(name: &str) -> Accent {
    let find = |n: &str| Accent::ALL.into_iter().find(|a| a.name() == n);
    find(name).or_else(|| find(DEFAULT_ACCENT)).unwrap_or(Accent::Orange)
}

/// The dark board gets the dark Metro ground; the white and orange boards
/// get the light one.
pub fn mode_of(theme: BoardTheme) -> Mode {
    match theme {
        BoardTheme::Dark => Mode::Dark,
        BoardTheme::White | BoardTheme::Orange => Mode::Light,
    }
}

/// Installs the Metro theme in the board's own face: the chip labels are cut
/// from the embedded D-DIN, so the chrome around them is set in it too (#55).
/// Call before the first component is built.
pub fn init(settings: &Settings) {
    nsh_metro::theme::set_font_family(crate::theme::UI_FONT);
    // The live tiles slide in waves on one beat (#184).
    nsh_metro::set_live_tile_beat(Some(LIVE_TILE_BEAT));
    sync(settings);
}

/// Pushes the board settings into the Metro theme. Writes only what
/// changed: everything bound to the palette follows a write.
pub fn sync(settings: &Settings) {
    let (mode, accent) = (mode_of(settings.theme), accent_of(&settings.accent));
    let now = global::<Theme>().get_untracked();
    if now.mode != mode {
        Theme::set_mode(mode);
    }
    if now.accent != accent {
        Theme::set_accent(accent);
    }
}

/// A small flat Metro button: 2 px border, filled with the accent while it
/// is the chosen one and while it is pressed.
pub fn chip(text: impl Into<SharedString>, active: impl Fn() -> bool + 'static, on_press: impl Fn() + 'static) -> View {
    let active = std::rc::Rc::new(active);
    let (a1, a2, a3) = (active.clone(), active.clone(), active);
    view()
        .flex_shrink_0()
        .px(px(6.))
        .border(px(2.))
        .border_color(move || if a1() { palette().accent } else { palette().border })
        .bg(move || if a2() { palette().accent } else { Hsla::transparent_black() })
        .cursor_pointer()
        .active(|s| s.opacity(0.7))
        .on_press(move || on_press())
        .child(
            label(text.into())
                .text_size(px(BODY))
                .text_color(move || if a3() { palette().accent_ink } else { palette().foreground }),
        )
}

/// A titled block of a panel: a subtle lower-case header over its controls.
pub fn section(title: impl Into<SharedString>) -> View {
    let title: SharedString = title.into();
    view()
        .gap(px(6.))
        .px(px(PAD))
        .pb(px(14.))
        .child(label(SharedString::from(title.to_lowercase())).text_size(px(SMALL)).text_color(move || palette().subtle))
}

/// The footer line of a panel.
pub fn hint(content: impl IntoReactive<SharedString>) -> View {
    view()
        .px(px(PAD))
        .pt(px(2.))
        .pb(px(10.))
        .child(label(content).text_size(px(SMALL - 1.)).text_color(move || palette().subtle))
}

/// A phase as a small square of the board's own colour: the chip greens and
/// ambers are made to sit on a chip, not to be read as text on Metro chrome.
pub fn phase_mark(phase: crate::model::Phase, size: f32) -> View {
    view().flex_shrink_0().size(px(size)).bg(crate::theme::hsla(crate::ui::devtools::phase_color(phase)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_names_round_trip_and_unknown_falls_back() {
        for a in Accent::ALL {
            assert_eq!(accent_of(a.name()), a);
        }
        assert_eq!(accent_of("chartreuse"), Accent::Orange);
        assert_eq!(mode_of(BoardTheme::Dark), Mode::Dark);
        assert_eq!(mode_of(BoardTheme::Orange), Mode::Light);
        assert_eq!(mode_of(BoardTheme::White), Mode::Light);
    }
}
