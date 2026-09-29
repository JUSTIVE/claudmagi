//! The list panel (⌘L): everything the board is drawing, written down the
//! right-hand side (#103).
//!
//! The board says how things stand at a glance and the comb says what there
//! is; neither says it in words you can scan. This does, in board order, one
//! session per block with its issue, its pull requests and its subagents
//! underneath. Every line is the same click its node on the board is: the
//! session jumps to its Warp tab, the issue opens in Linear, a pull request
//! opens on GitHub.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};

use crate::model::Target;
use crate::theme::{self, PALETTE};
use crate::ui::board::{Board, STATUS_H};
use crate::ui::panel::{Which, c, dim};

pub const PANEL_W: f32 = 330.0;

impl Board {
    pub(crate) fn toggle_list(&mut self) {
        self.list_panel.toggle();
    }

    pub(crate) fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<(usize, crate::model::SessionInfo)> =
            self.model.live().enumerate().map(|(i, chip)| (i, chip.info.clone())).collect();
        let subs: Vec<Vec<crate::model::SubagentInfo>> = self
            .model
            .live()
            .map(|chip| chip.subs.iter().filter(|s| !s.anim.gone).map(|s| s.info.clone()).collect())
            .collect();
        let total = rows.len();

        let mut list = div().flex().flex_col().gap_2().px_3().py_2();
        for ((i, info), subs) in rows.into_iter().zip(subs) {
            let phase = info.phase();
            let mut block = div().flex().flex_col().gap_0p5();

            // The session itself.
            let session_id = info.session_id.clone();
            block = block.child(
                div()
                    .id(SharedString::from(format!("list-session-{i}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(dim(0.08)))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(c(crate::ui::devtools::phase_color(phase)))
                            .child(info.label()),
                    )
                    .child(div().flex_none().text_size(px(10.)).text_color(dim(0.4)).child(phase.label()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(at) = this.model.chips.iter().position(|c| c.info.session_id == session_id) {
                            this.activate(Target::Session(at), cx);
                        }
                    })),
            );
            block = block.child(
                div().text_size(px(10.)).text_color(dim(0.4)).overflow_hidden().child(info.short_cwd()),
            );

            // Its issue.
            if let Some(t) = &info.ticket {
                let (key, app, web) = (t.label(), t.app_url(), t.url());
                let state =
                    t.status.map(|s| s.short().to_string()).unwrap_or_else(|| "NO STATUS".into());
                block = block.child(
                    div()
                        .id(SharedString::from(format!("list-ticket-{i}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .cursor_pointer()
                        .hover(|s| s.bg(dim(0.08)))
                        .text_size(px(10.))
                        .child(div().flex_1().overflow_hidden().text_color(dim(0.75)).child(t.label()))
                        .child(div().flex_none().text_color(dim(0.4)).child(state))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.open_link(key.clone(), app.clone(), web.clone(), "Linear issue", cx)
                        })),
                );
            }

            // Its pull requests, newest last, the way the board chains them.
            for (k, pr) in info.prs.iter().enumerate() {
                let (label, url) = (pr.label(), pr.url());
                let state = if pr.running() {
                    format!("{} \u{27f3}", pr.look().short())
                } else {
                    pr.look().short().to_string()
                };
                block = block.child(
                    div()
                        .id(SharedString::from(format!("list-pr-{i}-{k}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .cursor_pointer()
                        .hover(|s| s.bg(dim(0.08)))
                        .text_size(px(10.))
                        .child(div().flex_1().overflow_hidden().text_color(dim(0.75)).child(pr.label()))
                        .child(div().flex_none().text_color(dim(0.4)).child(state))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.open_link(label.clone(), None, Some(url.clone()), "pull request", cx)
                        })),
                );
            }

            // And its subagents, which have nowhere of their own to go.
            for sub in &subs {
                block = block.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .text_size(px(10.))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_color(dim(0.55))
                                .child(format!("\u{2514} {}", sub.label())),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(dim(0.4))
                                .child(if sub.running { "running" } else { "done" }),
                        ),
                );
            }
            list = list.child(block);
        }

        // Docked down the right edge rather than in the bottom corner: a list
        // wants height, and the board has more of it than it has width.
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom(px(STATUS_H))
            .w(px(PANEL_W.min(self.window.0 - 36.0).max(200.0)))
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(c(theme::with_alpha(PALETTE.text_on, 0.45)))
            .bg(c(theme::with_alpha(PALETTE.chip, 0.96)))
            .font_family(theme::UI_FONT)
            .text_size(px(11.))
            .text_color(dim(0.85))
            .overflow_hidden()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(dim(0.12))
                    .child(div().text_color(c(PALETTE.text_on)).child("SESSIONS"))
                    .child(div().text_size(px(10.)).text_color(dim(0.45)).child(format!("{total}")))
                    .child(
                        div()
                            .id("list-close")
                            .px_1p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(dim(0.12)))
                            .child("\u{00d7}")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.panel_mut(Which::List).open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().overflow_hidden().child(list))
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(dim(0.12))
                    .text_size(px(10.))
                    .text_color(dim(0.45))
                    .child("\u{2318}L toggles this panel"),
            )
    }
}
