//! The list panel (⌘L): everything the board is drawing, written down the
//! right-hand side (#103).
//!
//! The board says how things stand at a glance and the comb says what there
//! is; neither says it in words you can scan. This does, in board order, one
//! session per block with its issue, its pull requests and its subagents
//! underneath. Every line is the same click its node on the board is: the
//! session jumps to wherever its host seats it, the issue opens in Linear, a
//! pull request opens on GitHub.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};

use crate::model::Target;
use gpui_metro::Theme;

use crate::ui::board::Board;
use crate::ui::metro::{BODY, PAD, SMALL, phase_mark};
use crate::ui::panel::{Which, hint};

impl Board {
    pub(crate) fn toggle_list(&mut self) {
        self.toggle_panel(crate::ui::panel::Which::List);
    }

    pub(crate) fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::palette(cx);
        let fg = p.foreground;
        let dim = move |a: f32| fg.opacity(a);
        let rows: Vec<(usize, crate::model::SessionInfo)> =
            self.model.live().enumerate().map(|(i, chip)| (i, chip.info.clone())).collect();
        let subs: Vec<Vec<crate::model::SubagentInfo>> = self
            .model
            .live()
            .map(|chip| chip.subs.iter().filter(|s| !s.anim.gone).map(|s| s.info.clone()).collect())
            .collect();
        let total = rows.len();

        let mut list = div().flex().flex_col().gap_3().px(px(PAD)).py_2();
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
                    .hover(move |s| s.bg(dim(0.08)))
                    .child(phase_mark(phase, 10.))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(BODY + 2.))
                            .child(info.label()),
                    )
                    .child(div().flex_none().text_size(px(SMALL)).text_color(dim(0.4)).child(phase.label()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(at) = this.model.chips.iter().position(|c| c.info.session_id == session_id) {
                            this.activate(Target::Session(at), cx);
                        }
                    })),
            );
            block = block.child(
                div().text_size(px(SMALL)).text_color(dim(0.4)).overflow_hidden().child(info.short_cwd()),
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
                        .hover(move |s| s.bg(dim(0.08)))
                        .text_size(px(SMALL))
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
                // The word is the state and the checks; review and CI ride on
                // either side of it, the way they ride on the connector (#116).
                let mut state = pr.look().short().to_string();
                if pr.signed_off() {
                    state = format!("\u{2713} {state}");
                }
                if pr.running() {
                    state = format!("{state} \u{27f3}");
                }
                block = block.child(
                    div()
                        .id(SharedString::from(format!("list-pr-{i}-{k}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .cursor_pointer()
                        .hover(move |s| s.bg(dim(0.08)))
                        .text_size(px(SMALL))
                        .child(div().flex_1().overflow_hidden().text_color(dim(0.75)).child(pr.label()))
                        .child(div().flex_none().text_color(dim(0.4)).child(state))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.open_link(label.clone(), None, Some(url.clone()), "pull request", cx)
                        })),
                );
            }

            // And the pages it published, which share the cell's bottom line
            // with the pull requests and read the same way here (#137).
            for (k, a) in info.artifacts.iter().enumerate() {
                let (label, url) = (a.label(), a.url());
                let state = a.version_label().unwrap_or_else(|| "ARTIFACT".into());
                block = block.child(
                    div()
                        .id(SharedString::from(format!("list-artifact-{i}-{k}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .cursor_pointer()
                        .hover(move |s| s.bg(dim(0.08)))
                        .text_size(px(SMALL))
                        .child(div().flex_1().overflow_hidden().text_color(dim(0.75)).child(a.label()))
                        .child(div().flex_none().text_color(dim(0.4)).child(state))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.open_link(label.clone(), None, Some(url.clone()), crate::made::ARTIFACT, cx)
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
                        .text_size(px(SMALL))
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

        // Down the right edge as the settings flyout, like the other panels
        // (#175): a list wants height, and the board has more of it than it
        // has width. The count rides in the title.
        let body = div().flex().flex_col().child(list).child(hint("\u{2318}L toggles this panel"));
        self.flyout(Which::List, format!("sessions  {total}"), body, cx)
    }
}
