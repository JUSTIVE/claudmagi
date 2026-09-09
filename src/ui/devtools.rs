//! Floating test tools: switch between live and sandbox data, and create /
//! retarget / destroy synthetic sessions to exercise every board state.

use gpui::{ClickEvent, Context, ElementId, MouseButton, MouseDownEvent, SharedString, Stateful, div, prelude::*, px};

use crate::geom::Pt;
use crate::model::Phase;
use crate::theme::{self, PALETTE, Rgba};
use crate::ui::board::{Board, Mode};

pub const PANEL_W: f32 = 330.0;

#[derive(Default)]
pub struct DevState {
    pub open: bool,
    pub pos: Option<Pt>,
    /// Mouse offset from the panel origin while the header is being dragged.
    pub drag: Option<Pt>,
}

fn c(rgba: Rgba) -> gpui::Hsla {
    theme::hsla(rgba)
}

fn dim(alpha: f32) -> gpui::Hsla {
    theme::hsla(theme::with_alpha(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, alpha))
}

fn phase_color(phase: Phase) -> Rgba {
    match phase {
        Phase::Working => PALETTE.text_on,
        Phase::NeedsUser => PALETTE.text_needs,
        Phase::Idle => PALETTE.text_idle,
    }
}

fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Stateful<gpui::Div> {
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

fn section(title: &'static str) -> gpui::Div {
    div().flex().flex_col().gap_1p5().px_3().py_2().border_t_1().border_color(dim(0.12)).child(
        div().text_color(c(PALETTE.text_on)).text_size(px(10.)).child(title),
    )
}

impl Board {
    pub(crate) fn toggle_devtools(&mut self, win_w: f32) {
        self.dev.open = !self.dev.open;
        if self.dev.open && self.dev.pos.is_none() {
            self.dev.pos = Some(Pt::new((win_w - PANEL_W - 18.0).max(8.0), 18.0));
        }
    }

    pub(crate) fn render_devtools(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let pos = self.dev.pos.unwrap_or(Pt::new(20.0, 20.0));
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
            .absolute()
            .left(px(pos.x))
            .top(px(pos.y))
            .w(px(PANEL_W))
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(c(theme::with_alpha(PALETTE.text_on, 0.45)))
            .bg(c(theme::with_alpha(PALETTE.chip, 0.96)))
            .shadow_lg()
            .font_family("Menlo")
            .text_size(px(11.))
            .text_color(dim(0.85))
            // Keep clicks and wheel events off the board underneath.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            // Header doubles as the drag handle.
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .cursor_grab()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                            let m = Pt::new(f32::from(ev.position.x), f32::from(ev.position.y));
                            let pos = this.dev.pos.unwrap_or_default();
                            this.dev.drag = Some(m - pos);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .child(div().text_color(c(PALETTE.text_on)).child("TEST TOOLS"))
                    .child(
                        div()
                            .id("dev-close")
                            .px_1p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(dim(0.12)))
                            .child("×")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.dev.open = false;
                                cx.notify();
                            })),
                    ),
            )
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
                .child(format!("{} · {}", info.label(), phase.label()));
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
                .child("T toggles this panel · drag header to move · Esc closes"),
        )
    }
}
