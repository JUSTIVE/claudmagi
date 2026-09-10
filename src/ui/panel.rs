//! Shared chrome for the docked panels (test tools, settings): title bar,
//! close button, and the small widgets they are built from.

use gpui::{ClickEvent, Context, ElementId, MouseButton, SharedString, Stateful, div, prelude::*, px};

use crate::theme::{self, PALETTE, Rgba};
use crate::ui::board::Board;

#[derive(Default)]
pub struct PanelState {
    pub open: bool,
}

impl PanelState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }
}

pub fn c(rgba: Rgba) -> gpui::Hsla {
    theme::hsla(rgba)
}

pub fn dim(alpha: f32) -> gpui::Hsla {
    theme::hsla(theme::with_alpha(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, alpha))
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(dim(if active { 0.9 } else { 0.25 }))
        .cursor_pointer()
        .when(active, |d| d.bg(dim(0.92)).text_color(c(PALETTE.chip)))
        .when(!active, |d| d.text_color(dim(0.85)).hover(|s| s.bg(dim(0.12))))
        .child(label.into())
}

pub fn section(title: impl Into<SharedString>) -> gpui::Div {
    div().flex().flex_col().gap_1p5().px_3().py_2().border_t_1().border_color(dim(0.12)).child(
        div().text_color(c(PALETTE.text_on)).text_size(px(10.)).child(title.into()),
    )
}

/// Which panel a header drag belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Dev,
    Settings,
}

impl Board {
    pub(crate) fn panel_mut(&mut self, which: Which) -> &mut PanelState {
        match which {
            Which::Dev => &mut self.dev,
            Which::Settings => &mut self.settings_panel,
        }
    }

    /// A docked panel: dark card with a title bar and a close button. The
    /// board lays the open panels out side by side in the bottom-right
    /// corner (#25), so they never overlap. Append body children to the
    /// returned element.
    pub(crate) fn panel_chrome(
        &mut self,
        which: Which,
        title: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let close_id: &'static str = match which {
            Which::Dev => "dev-close",
            Which::Settings => "settings-close",
        };
        div()
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(c(theme::with_alpha(PALETTE.text_on, 0.45)))
            .bg(c(theme::with_alpha(PALETTE.chip, 0.96)))
            .shadow_lg()
            .font_family(theme::UI_FONT)
            .text_size(px(11.))
            .text_color(dim(0.85))
            // Keep clicks and wheel events off the board underneath.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .child(div().text_color(c(PALETTE.text_on)).child(title))
                    .child(
                        div()
                            .id(close_id)
                            .px_1p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(dim(0.12)))
                            .child("×")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.panel_mut(which).open = false;
                                cx.notify();
                            })),
                    ),
            )
    }
}
