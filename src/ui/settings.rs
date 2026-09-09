//! Settings panel (⌘, or the SETTINGS button): board background and UI size.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};

use crate::render::scene;
use crate::settings::BoardTheme;
use crate::ui::board::Board;
use crate::ui::panel::{Which, button, dim, section};

pub const PANEL_W: f32 = 300.0;
const ZOOM_PRESETS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];

impl Board {
    pub(crate) fn toggle_settings(&mut self) {
        self.settings_panel.toggle();
    }

    pub(crate) fn set_theme(&mut self, theme: BoardTheme, cx: &mut Context<Self>) {
        if self.settings.theme != theme {
            self.settings.theme = theme;
            self.settings.save();
            cx.notify();
        }
    }

    pub(crate) fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.settings.theme;
        let zoom = self.settings.zoom;
        let zoom_pct = format!("{:.0}%", zoom * 100.0);

        let mut themes = div().flex().flex_wrap().gap_1();
        for t in BoardTheme::ALL {
            themes = themes.child(
                button(SharedString::from(format!("theme-{}", t.label())), t.label(), t == theme)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_theme(t, cx))),
            );
        }

        let mut presets = div().flex().flex_wrap().gap_1();
        for z in ZOOM_PRESETS {
            let active = (zoom - z).abs() < 0.01;
            presets = presets.child(
                button(SharedString::from(format!("zoom-{}", (z * 100.0) as u32)), format!("{:.0}%", z * 100.0), active)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_zoom(z, cx))),
            );
        }

        self.panel_chrome(Which::Settings, "SETTINGS", PANEL_W, cx)
            .child(section("BACKGROUND").child(themes))
            .child(
                section("UI SIZE")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(button("settings-zoom-out", "−", false).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| this.zoom_by(1.0 / scene::USER_ZOOM_STEP, cx),
                            )))
                            .child(div().w(px(48.)).text_center().child(zoom_pct))
                            .child(button("settings-zoom-in", "+", false).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| this.zoom_by(scene::USER_ZOOM_STEP, cx),
                            )))
                            .child(div().text_color(dim(0.45)).text_size(px(10.)).child("⌘+ ⌘− ⌘0 · ⌘+wheel")),
                    )
                    .child(presets),
            )
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(dim(0.12))
                    .text_size(px(10.))
                    .text_color(dim(0.45))
                    .child("⌘, toggles this panel · settings persist across launches"),
            )
    }
}
