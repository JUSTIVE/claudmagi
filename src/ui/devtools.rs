//! Floating test tools: switch between live and sandbox data, and create /
//! retarget / destroy synthetic sessions to exercise every board state.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};

use crate::model::Phase;
use crate::pr;
use crate::ticket;
use crate::theme::{self, PALETTE, Rgba};
use crate::sources;
use crate::ui::board::{Board, Mode};
use crate::ui::metro::{SMALL, phase_mark};
use crate::ui::panel::{Which, button, hint, section};

pub(crate) fn phase_color(phase: Phase) -> Rgba {
    match phase {
        Phase::Working => PALETTE.text_on,
        Phase::NeedsUser => PALETTE.packet, // the needs-input chip's orange (#50)
        Phase::Idle => theme::AMBER_DIM,
    }
}

impl Board {
    pub(crate) fn toggle_devtools(&mut self) {
        self.toggle_panel(Which::Dev);
    }

    pub(crate) fn render_devtools(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Panel text is Metro foreground at a strength, on either ground.
        let fg = gpui_metro::Theme::palette(cx).foreground;
        let dim = move |a: f32| fg.opacity(a);
        let mode = self.mode;
        let sandbox = self.sandbox.clone();
        let auto = sandbox.auto();
        let rows: Vec<_> = self
            .model
            .live()
            .enumerate()
            .map(|(i, chip)| (i, chip.info.clone()))
            .collect();

        let mut panel = div()
            .flex()
            .flex_col()
            // Data source.
            .child(
                section("SOURCE").child(
                    div()
                        .flex()
                        .gap_1()
                        .child(button("mode-live", "live", mode == Mode::Live).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.set_mode(Mode::Live, cx),
                        )))
                        .child(button("mode-sandbox", "sandbox", mode == Mode::Sandbox).on_click(
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
                            .child(add("add-working", "+ working", Phase::Working))
                            .child(add("add-needs", "+ needs input", Phase::NeedsUser))
                            .child(add("add-idle", "+ idle", Phase::Idle)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(button("fill-8", "fill ×8", false).on_click(move |_, _, _| sb_fill.fill(8)))
                            .child(button("clear-all", "clear", false).on_click(move |_, _, _| sb_clear.clear()))
                            .child(button("auto-churn", "auto churn", auto).on_click(move |_, _, _| {
                                sb_auto.set_auto(!sb_auto.auto());
                            })),
                    )
                    // How fast the churn runs. Watching a lane slide into
                    // place wants one end of this and a crowd arriving wants
                    // the other (#106).
                    .child({
                        let rate = sandbox.rate();
                        let mut row = div().flex().items_center().flex_wrap().gap_1().child(
                            div().w(px(40.)).flex_none().text_size(px(SMALL)).text_color(dim(0.55)).child("rate"),
                        );
                        for r in sources::CHURN_RATES {
                            let sb = sandbox.clone();
                            row = row.child(
                                button(
                                    SharedString::from(format!("churn-rate-{}", (r * 100.0) as u32)),
                                    format!("{r}\u{00d7}"),
                                    (rate - r).abs() < 0.01,
                                )
                                .on_click(move |_, _, _| sb.set_rate(r)),
                            );
                        }
                        row
                    })
                    // Growing a cluster a pane at a time: the sandbox starts
                    // a new group every third session otherwise, and a tab
                    // full of panes never turns up (#107).
                    .child({
                        let groups = sandbox.groups();
                        let mut row = div().flex().items_center().flex_wrap().gap_1().child(
                            div().w(px(40.)).flex_none().text_size(px(SMALL)).text_color(dim(0.55)).child("grow"),
                        );
                        for (group, n) in groups {
                            let sb = sandbox.clone();
                            let label = format!("{}+{n}", group.rsplit(':').next().unwrap_or(&group));
                            let id = SharedString::from(format!("grow-{group}"));
                            row = row.child(button(id, label, false).on_click(move |_, _, _| {
                                sb.add_to_group(&group, Phase::Working);
                            }));
                        }
                        row
                    }),
            );
        }

        // Session rows (both modes).
        let mut list = div()
            .id("dev-session-list")
            .flex()
            .flex_col()
            .gap_1();
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
                .whitespace_nowrap()
                .text_ellipsis()
                .child({
                    let mut line = format!("{} · {}", info.label(), phase.label());
                    if let Some(t) = &info.ticket {
                        line = format!("{} · {}", line, t.label());
                    }
                    for p in &info.prs {
                        line = format!("{} · {} {}", line, p.label(), p.look().short());
                    }
                    line
                });
            let label = if info.synthetic {
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                label.cursor_pointer().hover(move |s| s.bg(dim(0.08))).on_click(move |_, _, _| sb.cycle(&sid))
            } else {
                label
            };
            let mut row = div().flex().items_center().gap_1().child(phase_mark(phase, 8.)).child(label);
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
                    button(SharedString::from(format!("add-sub-{i}")), "+sub", false)
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
                    button(SharedString::from(format!("focus-{i}")), "→ warp", false).on_click(cx.listener(
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
                // The letters shape the newest, which is the one a cell
                // draws, so the row answers for that one too (#134).
                let look = info.prs.last().map(|p| p.look());
                // What the cell would be drawing, and how many are behind it:
                // the answered one it prefers, or the number nobody spoke for
                // (#112), with the rest of the chain counted off (#134).
                let head = info
                    .prs
                    .last()
                    .map(|p| p.label())
                    .or_else(|| info.prs_unanswered.last().map(|r| format!("{}?", r.label())));
                let chain = info.prs.len() + info.prs_unanswered.len();
                // Wrapping rather than shrinking: this row now carries ten
                // controls and a narrow window is not a reason to clip their
                // letters off, the way the GROW row above already works.
                let mut links = div().flex().items_center().flex_wrap().gap_1().pl_3().child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_size(px(SMALL))
                        .text_color(dim(0.55))
                        .child(match (head, chain) {
                            (None, _) => "PR".to_string(),
                            (Some(h), 0 | 1) => h,
                            (Some(h), n) => format!("{h} +{}", n - 1),
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
                let ci = info.prs.last().is_some_and(|p| p.running());
                links = links.child(
                    button(SharedString::from(format!("pr-ci-{i}")), "CI", ci)
                        .on_click(move |_, _, _| sb.set_pr_running(&sid, !ci)),
                );
                // Review is a switch of its own rather than one of the state
                // letters, which is exactly the shape of the thing (#116).
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                let ok = info.prs.last().is_some_and(|p| p.approved);
                links = links.child(
                    button(SharedString::from(format!("pr-ok-{i}")), "OK", ok)
                        .on_click(move |_, _, _| sb.set_pr_approved(&sid, !ok)),
                );
                // Chaining walks the states rather than laying down the
                // same one again: four identical nodes exercise nothing a
                // single one does not (#63, #134).
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                let next = pr::Look::ALL[info.prs.len() % pr::Look::ALL.len()];
                links = links.child(
                    button(SharedString::from(format!("pr-add-{i}")), "+", false)
                        .on_click(move |_, _, _| sb.add_pr(&sid, next)),
                );
                // And a number GitHub would not answer for, which the
                // sandbox had no way to make at all (#112, #134).
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                links = links.child(
                    button(SharedString::from(format!("pr-silent-{i}")), "?", false)
                        .on_click(move |_, _, _| sb.add_unanswered(&sid)),
                );
                // The pages the session published share this line on the cell
                // (#137), so they share this row here: one to hang another
                // page on, one to take them all away, the way `−` does for
                // the pull request.
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                links = links.child(
                    button(SharedString::from(format!("art-add-{i}")), "A+", false)
                        .on_click(move |_, _, _| sb.add_artifact(&sid)),
                );
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                let pages = info.artifacts.len();
                links = links.child(
                    button(SharedString::from(format!("art-none-{i}")), "A−", pages == 0)
                        .on_click(move |_, _, _| sb.clear_artifacts(&sid)),
                );
                list = list.child(links);

                // And the same for the Linear node at the other end (#58).
                let status = info.ticket.as_ref().map(|t| t.status);
                let mut tickets = div().flex().items_center().gap_1().pl_3().child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_size(px(SMALL))
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

                // And where the session is running, which is a badge rather
                // than a state but is just as hard to reach without a row of
                // its own (#124).
                let seated = info.seat.as_ref().map(|s| s.host);
                let mut hosts = div().flex().items_center().gap_1().pl_3().child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_size(px(SMALL))
                        .text_color(dim(0.55))
                        .child("host"),
                );
                let sb = sandbox.clone();
                let sid = info.session_id.clone();
                hosts = hosts.child(
                    button(SharedString::from(format!("hs-none-{i}")), "−", seated.is_none())
                        .on_click(move |_, _, _| sb.set_host(&sid, None)),
                );
                for (host, label) in crate::host::NAMED {
                    let sb = sandbox.clone();
                    let sid = info.session_id.clone();
                    hosts = hosts.child(
                        button(
                            SharedString::from(format!("hs-{i}-{host}")),
                            label,
                            seated == Some(host),
                        )
                        .on_click(move |_, _, _| sb.set_host(&sid, Some(host))),
                    );
                }
                list = list.child(hosts);
            }

            // Subagents, indented under their session.
            for (j, sub) in info.subagents.iter().enumerate() {
                let sub_phase = sub.phase();
                let mut sub_row = div().flex().items_center().gap_1().pl_3().child(phase_mark(sub_phase, 5.)).child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(SMALL))
                        .text_color(dim(0.6))
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
                            if running { "done" } else { "run" },
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
            Mode::Sandbox => "sessions · click a name to cycle · +sub adds a subagent",
            Mode::Live => "sessions",
        };
        let count = if mode == Mode::Sandbox { sandbox.len() } else { self.model.live().count() };
        // In live mode this was only ever a list, and the list has a panel of
        // its own now (Cmd-L, #103). In the sandbox these rows are the
        // controls themselves, so they stay.
        if mode == Mode::Sandbox {
            panel = panel.child(
            section(title)
                .child(div().text_color(dim(0.45)).text_size(px(SMALL)).child(format!("{count} total")))
                .child(list),
            );
        }

        let body = panel.child(hint("T toggles this panel · ⌘L lists the sessions · Esc closes"));
        self.flyout(Which::Dev, "test tools", body, cx)
    }
}
