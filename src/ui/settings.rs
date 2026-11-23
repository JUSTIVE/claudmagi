//! Settings panel (⌘, or the settings button): view, board background,
//! Metro accent and UI size.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};
use gpui_metro::{Accent, Theme, radio_button, slider};

use crate::render::scene;
use crate::settings::{BoardTheme, BoardView};
use crate::ui::board::Board;
use crate::ui::metro;
use crate::ui::panel::{Which, button, hint, section};

pub const PANEL_W: f32 = 320.0;
const ZOOM_PRESETS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
/// The twenty accents as a 10 × 2 grid of swatches, the way the phone's
/// accent picker laid them out as tiles.
const SWATCH: f32 = 26.0;
const SWATCH_GAP: f32 = 2.0;

/// Zoom ↔ slider position (0–1), linear over the allowed range.
fn zoom_to_unit(zoom: f32) -> f32 {
    ((zoom - scene::USER_ZOOM_MIN) / (scene::USER_ZOOM_MAX - scene::USER_ZOOM_MIN)).clamp(0.0, 1.0)
}

fn unit_to_zoom(v: f32) -> f32 {
    scene::USER_ZOOM_MIN + v.clamp(0.0, 1.0) * (scene::USER_ZOOM_MAX - scene::USER_ZOOM_MIN)
}

impl Board {
    pub(crate) fn toggle_settings(&mut self) {
        self.settings_panel.toggle();
    }

    pub(crate) fn set_view(&mut self, view: BoardView, cx: &mut Context<Self>) {
        if self.settings.view != view {
            self.settings.view = view;
            self.settings.save();
            // The views measure their content differently, so the old scroll
            // means nothing in the new one (#86).
            self.scroll_y = 0.0;
            // Opening the comb plays its arrival again, from its first
            // painted frame (#92, #93).
            self.comb_since = None;
            cx.notify();
        }
    }

    pub(crate) fn set_theme(&mut self, theme: BoardTheme, cx: &mut Context<Self>) {
        if self.settings.theme != theme {
            self.settings.theme = theme;
            self.settings.save();
            // The panels follow the board: dark board, dark Metro ground.
            metro::sync(&self.settings, cx);
            cx.notify();
        }
    }

    pub(crate) fn set_accent(&mut self, accent: Accent, cx: &mut Context<Self>) {
        if self.settings.accent != accent.name() {
            self.settings.accent = accent.name().into();
            self.settings.save();
            metro::sync(&self.settings, cx);
            cx.notify();
        }
    }

    pub(crate) fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::palette(cx);
        let theme = self.settings.theme;
        let view = self.settings.view;
        let zoom = self.settings.zoom;
        let current = metro::accent_of(&self.settings.accent);
        let board = cx.entity().downgrade();

        let mut views = div().flex().flex_wrap().gap(px(4.));
        for v in BoardView::ALL {
            views = views.child(
                button(SharedString::from(format!("view-{}", v.label())), v.label().to_lowercase(), v == view)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_view(v, cx))),
            );
        }

        let mut themes = div().flex().flex_col();
        for t in BoardTheme::ALL {
            let board = board.clone();
            themes = themes.child(
                radio_button(SharedString::from(format!("theme-{}", t.label())), t == theme)
                    .label(t.label().to_lowercase())
                    .on_select(move |_, cx| {
                        let _ = board.update(cx, |this, cx| this.set_theme(t, cx));
                    }),
            );
        }

        let mut swatches = div().flex().flex_wrap().gap(px(SWATCH_GAP)).w(px(10.0 * SWATCH + 9.0 * SWATCH_GAP));
        for a in Accent::ALL {
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("accent-{}", a.name())))
                    .size(px(SWATCH))
                    .bg(a.color())
                    .cursor_pointer()
                    .when(a == current, |d| d.border_2().border_color(p.foreground))
                    .hover(|s| s.opacity(0.8))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_accent(a, cx))),
            );
        }

        let mut presets = div().flex().flex_wrap().gap(px(4.));
        for z in ZOOM_PRESETS {
            let active = (zoom - z).abs() < 0.01;
            presets = presets.child(
                button(SharedString::from(format!("zoom-{}", (z * 100.0) as u32)), format!("{:.0}%", z * 100.0), active)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_zoom(z, cx))),
            );
        }

        let body = div()
            .flex()
            .flex_col()
            .child(section("view").child(views))
            .child(section("background").child(themes))
            .child(section("accent").child(swatches).child(div().text_color(p.accent).child(current.name())))
            .child(
                section("ui size")
                    .child(
                        slider("settings-zoom", zoom_to_unit(zoom))
                            .header(format!("{:.0}%", zoom * 100.0))
                            .on_change(move |v, _, cx| {
                                let _ = board.update(cx, |this, cx| this.set_zoom(unit_to_zoom(v), cx));
                            }),
                    )
                    .child(presets),
            )
            .child(hint("⌘+ ⌘− ⌘0 · ⌘+wheel zoom · ⌘, toggles this panel · settings persist"));
        self.panel_chrome(Which::Settings, "settings", PANEL_W, body, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_slider_maps_the_whole_range() {
        assert_eq!(zoom_to_unit(scene::USER_ZOOM_MIN), 0.0);
        assert_eq!(zoom_to_unit(scene::USER_ZOOM_MAX), 1.0);
        for z in ZOOM_PRESETS {
            assert!((unit_to_zoom(zoom_to_unit(z)) - z).abs() < 1e-5);
        }
    }
}
