//! Floating test tools: switch between live and sandbox data, and create /
//! retarget / destroy synthetic sessions to exercise every board state.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};

use crate::model::Phase;
use crate::pr;
use crate::ticket;
use crate::theme::{self, PALETTE, Rgba};
use crate::ui::board::{Board, Mode};
use crate::ui::panel::{Which, button, c, dim, section};

pub const PANEL_W: f32 = 330.0;

fn phase_color(phase: Phase) -> Rgba {
    match phase {
        Phase::Working => PALETTE.text_on,
        Phase::NeedsUser => PALETTE.packet, // the needs-input chip's orange (#50)
        Phase::Idle => theme::AMBER_DIM,
    }
}

impl Board {
    pub(crate) fn toggle_devtools(&mut self) {
        self.dev.toggle();
    }

    pub(crate) fn render_devtools(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.mode;
        let sandbox = self.sandbox.clone();
        let auto = sandbox.auto();
        let rows: Vec<_> = self
            .model
            .live()
            .enumerate()
            .map(|(i, chip)| (i, chip.info.clone()))
            .collect();

        let mut panel = self.panel_chrome(Which::Dev, "TEST TOOLS", PANEL_W, cx)
            // Data source.
            .child(
                section("SOURCE").child(
                    div()
                        .flex()
                        .gap_1()
                        .child(button("mode-live", "LIVE", mode == Mode::Live).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.set_mode(Mode::Live, cx),
                        )))
                        .child(button("mode-sandbox", "SANDBOX", mode == Mode::Sandbox).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.set_mode(Mode::Sandbox, cx)),
                        )),
                ),
            );

        if mode == Mode::Sandbox {
            let add = |id: &'static str, label: &'static str, phase: Phase| {
                let sb = sandbox.clone();
                button(id, label, false).on_click(move |_, _, _| {
                    sb.add(phase);
                })
            };
            let sb_fill = sandbox.clone();
            let sb_clear = sandbox.clone();
            let sb_auto = sandbox.clone();
            panel = panel.child(
                section("SANDBOX SESSIONS")
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(add("add-working", "+ WORKING", Phase::Working))
                            .child(add("add-needs", "+ NEEDS INPUT", Phase::NeedsUser))
                            .child(add("add-idle", "+ IDLE", Phase::Idle)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(button("fill-8", "FILL ×8", false).on_click(move |_, _, _| sb_fill.fill(8)))
                            .child(button("clear-all", "CLEAR", false).on_click(move |_, _, _| sb_clear.clear()))
                            .child(button("auto-churn", "AUTO CHURN", auto).on_click(move |_, _, _| {
                                sb_auto.set_auto(!sb_auto.auto());
                            })),
                    ),
            );
        }

        // Session rows (both modes).
        let mut list = div()
            .id("dev-session-list")
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(280.))
            .overflow_y_scroll();
        if rows.is_empty() {
            list = list.child(div().text_color(dim(0.5)).child(if mode == Mode::Sandbox {
                "no synthetic sessions yet — add one above"
            } else {
                "no live Claude Code sessions"
            }));
        }
        for (i, info) in rows {
            let phase = info.phase();
            let label = div()
                .id(SharedString::from(format!("label-{i}")))
                .flex_1()
                .overflow_hidden()
                .text_color(c(phase_color(phase)))
                .child({
                    let mut line = format!("{} · {}", info.label(), phase.label());
                    if let Some(t) = &info.ticket {
                        line = format!("{} · {}", line, t.label());
                    }
                    if let Some(p) = &info.pr {
                        line = format!("{} · {} {}", line, p.label(), p.look().short());
                    }
                    line
                });
            let label = if info.synthetic {
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                label.cursor_pointer().hover(|s| s.bg(dim(0.08))).on_click(move |_, _, _| sb.cycle(&sid))
            } else {
                label
            };
            let mut row = div().flex().items_center().gap_1().child(label);
            if info.synthetic {
                for p in Phase::ALL {
                    let sb = sandbox.clone();
                    let sid = info.session_id.clone();
                    row = row.child(
                        button(SharedString::from(format!("phase-{i}-{}", p.short())), p.short(), p == phase)
                            .on_click(move |_, _, _| sb.set_phase(&sid, p)),
                    );
                }
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                row = row.child(
                    button(SharedString::from(format!("add-sub-{i}")), "+SUB", false)
                        .on_click(move |_, _, _| {
                            sb.add_sub(&sid, true);
                        }),
                );
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                row = row.child(
                    button(SharedString::from(format!("remove-{i}")), "×", false).on_click(move |_, _, _| sb.remove(&sid)),
                );
            } else {
                let sid = info.session_id.clone();
                row = row.child(
                    button(SharedString::from(format!("focus-{i}")), "→ WARP", false).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Some(idx) = this.model.chips.iter().position(|c| c.info.session_id == sid) {
                                this.activate(crate::model::Target::Session(idx), cx);
                            }
                        },
                    )),
                );
            }
            list = list.child(row);

            // Link nodes get their own row: five PR states plus "none" is more
            // than the session row can hold, and cycling through them to reach
            // one was tedious (#58).
            if info.synthetic {
                let look = info.pr.as_ref().map(|p| p.look());
                let mut links = div().flex().items_center().gap_1().pl_3().child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(dim(0.55))
                        .child(match &info.pr {
                            Some(p) => p.label(),
                            None => "PR".into(),
                        }),
                );
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                links = links.child(
                    button(SharedString::from(format!("pr-none-{i}")), "−", look.is_none())
                        .on_click(move |_, _, _| sb.set_pr(&sid, None)),
                );
                for l in pr::Look::ALL {
                    let sb = sandbox.clone();
                    let sid = info.session_id.clone();
                    links = links.child(
                        button(SharedString::from(format!("pr-{i}-{}", l.letter())), l.letter(), look == Some(l))
                            .on_click(move |_, _, _| sb.set_pr(&sid, Some(l))),
                    );
                }
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                let ci = info.pr.as_ref().is_some_and(|p| p.running());
                links = links.child(
                    button(SharedString::from(format!("pr-ci-{i}")), "CI", ci)
                        .on_click(move |_, _, _| sb.set_pr_running(&sid, !ci)),
                );
                list = list.child(links);

                // And the same for the Linear node at the other end (#58).
                let status = info.ticket.as_ref().map(|t| t.status);
                let mut tickets = div().flex().items_center().gap_1().pl_3().child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(dim(0.55))
                        .child(match &info.ticket {
                            Some(t) => t.label(),
                            None => "LINEAR".into(),
                        }),
                );
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                tickets = tickets.child(
                    button(SharedString::from(format!("tk-none-{i}")), "−", status.is_none())
                        .on_click(move |_, _, _| sb.set_ticket(&sid, None)),
                );
                for st in ticket::Status::ALL {
                    let sb = sandbox.clone();
                    let sid = info.session_id.clone();
                    tickets = tickets.child(
                        button(
                            SharedString::from(format!("tk-{i}-{}", st.letter())),
                            st.letter(),
                            status == Some(Some(st)),
                        )
                        .on_click(move |_, _, _| sb.set_ticket(&sid, Some(Some(st)))),
                    );
                }
                list = list.child(tickets);
            }

            // Subagents, indented under their session.
            for (j, sub) in info.subagents.iter().enumerate() {
                let sub_phase = sub.phase();
                let mut sub_row = div().flex().items_center().gap_1().pl_3().child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .text_size(px(10.))
                        .text_color(c(theme::with_alpha(phase_color(sub_phase), 0.85)))
                        .child(format!(
                            "└ {} · {} · {}",
                            sub.label(),
                            if sub.description.is_empty() { "-" } else { sub.description.as_str() },
                            if sub.running { "running" } else { "done" }
                        )),
                );
                if info.synthetic {
                    let sb = sandbox.clone();
                    let aid = sub.agent_id.clone();
                    let running = sub.running;
                    sub_row = sub_row.child(
                        button(
                            SharedString::from(format!("sub-toggle-{i}-{j}")),
                            if running { "DONE" } else { "RUN" },
                            false,
                        )
                        .on_click(move |_, _, _| sb.set_sub_running(&aid, !running)),
                    );
                    let sb = sandbox.clone();
                    let aid = sub.agent_id.clone();
                    sub_row = sub_row.child(
                        button(SharedString::from(format!("sub-remove-{i}-{j}")), "×", false)
                            .on_click(move |_, _, _| sb.remove_sub(&aid)),
                    );
                }
                list = list.child(sub_row);
            }
        }
        let title: &'static str = match mode {
            Mode::Sandbox => "SESSIONS · click a name to cycle · +SUB adds a subagent",
            Mode::Live => "SESSIONS",
        };
        let count = if mode == Mode::Sandbox { sandbox.len() } else { self.model.live().count() };
        panel = panel.child(
            section(title)
                .child(div().text_color(dim(0.45)).text_size(px(10.)).child(format!("{count} total")))
                .child(list),
        );

        panel.child(
            div()
                .px_3()
                .py_1p5()
                .border_t_1()
                .border_color(dim(0.12))
                .text_size(px(10.))
                .text_color(dim(0.45))
                .child("T toggles this panel · Esc closes"),
        )
    }
}
