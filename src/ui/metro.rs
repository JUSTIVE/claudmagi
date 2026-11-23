//! Glue between the board settings and the gpui-metro theme, plus the
//! compact Metro pieces the panels are built from.
//!
//! gpui-metro's own controls are sized for a phone (52 px buttons); the
//! panels here are dense, so these keep its look (square corners, 2 px
//! borders, the accent filling a pressed or chosen control, lower-case
//! headers) at panel scale. They are `RenderOnce` so they read the live
//! theme when drawn, and their call sites do not need a palette in hand.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, RenderOnce, SharedString,
    Window, div, prelude::*, px,
};
use gpui_metro::{Accent, Mode, Theme};

use crate::settings::{BoardTheme, DEFAULT_ACCENT, Settings};

/// Side padding inside a panel.
pub const PAD: f32 = 16.;
/// Body text inside the panels.
pub const BODY: f32 = 12.;
/// Captions and section headers.
pub const SMALL: f32 = 11.;

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
/// Call after `register_fonts`.
pub fn init(settings: &Settings, cx: &mut App) {
    gpui_metro::init(cx);
    cx.global_mut::<Theme>().font_family = crate::theme::UI_FONT.into();
    sync(settings, cx);
}

/// Pushes the board settings into the global Metro theme. Writes only what
/// changed, since every write refreshes all windows.
pub fn sync(settings: &Settings, cx: &mut App) {
    let (mode, accent) = (mode_of(settings.theme), accent_of(&settings.accent));
    if Theme::get(cx).mode != mode {
        Theme::set_mode(mode, cx);
    }
    if Theme::get(cx).accent != accent {
        Theme::set_accent(accent, cx);
    }
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A small flat Metro button: 2 px border, filled with the accent while it
/// is the chosen one and while it is pressed.
#[derive(IntoElement)]
pub struct Chip {
    id: ElementId,
    label: SharedString,
    active: bool,
    on_click: Option<ClickHandler>,
}

pub fn chip(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Chip {
    Chip { id: id.into(), label: label.into(), active, on_click: None }
}

impl Chip {
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Theme::palette(cx);
        div()
            .id(self.id)
            .flex_none()
            .px(px(6.))
            .border_2()
            .border_color(if self.active { p.accent } else { p.border })
            .when(self.active, |d| d.bg(p.accent).text_color(p.accent_ink))
            .when(!self.active, |d| d.text_color(p.foreground))
            .cursor_pointer()
            .active(move |s| s.bg(p.accent).border_color(p.accent).text_color(p.accent_ink))
            .child(self.label)
            .when_some(self.on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
    }
}

/// A titled block of a panel: a subtle lower-case header over its controls.
#[derive(IntoElement)]
pub struct Section {
    title: SharedString,
    children: Vec<AnyElement>,
}

pub fn section(title: impl Into<SharedString>) -> Section {
    Section { title: title.into(), children: Vec::new() }
}

impl ParentElement for Section {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Section {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Theme::palette(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(PAD))
            .pb(px(14.))
            .child(div().text_size(px(SMALL)).text_color(p.subtle).child(SharedString::from(self.title.to_lowercase())))
            .children(self.children)
    }
}

/// The footer line of a panel.
#[derive(IntoElement)]
pub struct Hint(SharedString);

pub fn hint(content: impl Into<SharedString>) -> Hint {
    Hint(content.into())
}

impl RenderOnce for Hint {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Theme::palette(cx);
        div().px(px(PAD)).pt(px(2.)).pb(px(10.)).text_size(px(SMALL - 1.)).text_color(p.subtle).child(self.0)
    }
}

/// A phase as a small square of the board's own colour: the chip greens and
/// ambers are made to sit on a chip, not to be read as text on Metro chrome.
pub fn phase_mark(phase: crate::model::Phase, size: f32) -> gpui::Div {
    div().flex_none().size(px(size)).bg(crate::theme::hsla(crate::ui::devtools::phase_color(phase)))
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
