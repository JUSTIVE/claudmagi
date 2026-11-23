//! Shared chrome for the docked panels (test tools, settings): a flat Metro
//! sheet with the app title over a light lower-case page title and a round
//! close glyph, and the small widgets the panels are built from.

use gpui::{ClickEvent, Context, MouseButton, div, prelude::*, px};
use gpui_metro::{Icon, TextStyle, round_button, text};

use crate::ui::board::Board;
use crate::ui::metro::{BODY, PAD};

#[derive(Default)]
pub struct PanelState {
    pub open: bool,
}

pub use crate::ui::metro::{chip as button, hint, section};

/// Which panel a header drag belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Dev,
    Settings,
    /// The list down the right-hand side (#103).
    List,
}

impl Board {
    /// Opens `which` and closes the other two, or closes `which` if it was
    /// the one open: the test tools, the settings and the list are never up
    /// together (#163).
    pub(crate) fn toggle_panel(&mut self, which: Which) {
        let was = self.panel_mut(which).open;
        for w in [Which::Dev, Which::Settings, Which::List] {
            self.panel_mut(w).open = false;
        }
        self.panel_mut(which).open = !was;
    }

    pub(crate) fn panel_mut(&mut self, which: Which) -> &mut PanelState {
        match which {
            Which::Dev => &mut self.dev,
            Which::Settings => &mut self.settings_panel,
            Which::List => &mut self.list_panel,
        }
    }

    /// A docked panel: `CLAUDMAGI` over the panel's lower-case title, with a
    /// round close glyph. The board lays the open panels out side by side in
    /// the left edge, side by side and top to bottom, so they never
    /// overlap (#25, #158). The header stays put and `body` scrolls under it
    /// when the window is too short for it.
    pub(crate) fn panel_chrome(
        &mut self,
        which: Which,
        title: &'static str,
        width: f32,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (close_id, scroll_id): (&'static str, &'static str) = match which {
            Which::Dev => ("dev-close", "dev-scroll"),
            Which::Settings => ("settings-close", "settings-scroll"),
            Which::List => ("list-close", "list-scroll"),
        };
        let p = gpui_metro::Theme::palette(cx);
        div()
            // A panel wider than the window would push its own close button
            // off the screen (#83).
            .w(px(width.min(self.window.0 - 36.0).max(180.0)))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            // Metro: a flat sheet of chrome, square, an accent rule on top.
            .bg(p.chrome)
            .border_t_2()
            .border_color(p.accent)
            .font_family(gpui_metro::Theme::font(cx))
            .text_size(px(BODY))
            .text_color(p.foreground)
            // Keep clicks and wheel events off the board underneath.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .pl(px(PAD))
                    .pr(px(8.))
                    .pt(px(8.))
                    .pb(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(text(TextStyle::AppTitle, "CLAUDMAGI"))
                            .child(div().ml(px(-2.)).child(text(TextStyle::Title2, title).lowercase())),
                    )
                    .child(round_button(close_id, Icon::Close).diameter(28.).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.panel_mut(which).open = false;
                            cx.notify();
                        },
                    ))),
            )
            // The body takes the rest of the height and scrolls in it.
            .child(div().id(scroll_id).flex_1().min_h_0().overflow_y_scroll().child(body))
    }
}
