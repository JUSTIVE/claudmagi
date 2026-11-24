//! Floating test tools: switch between live and sandbox data, and create /
//! retarget / destroy synthetic sessions to exercise every board state.

use std::sync::Arc;

use nsh_core::prelude::*;
use nsh_metro::{label, palette};

use crate::model::{Phase, SessionInfo};
use crate::pr;
use crate::sources::{self, FakeSource};
use crate::theme::{self, PALETTE, Rgba};
use crate::ticket;
use crate::ui::metro::{SMALL, phase_mark};
use crate::ui::panel::{Which, button, flyout, hint, section};
use crate::ui::state::{Mode, Ui};

pub(crate) fn phase_color(phase: Phase) -> Rgba {
    match phase {
        Phase::Working => PALETTE.text_on,
        Phase::NeedsUser => PALETTE.packet, // the needs-input chip's orange (#50)
        Phase::Idle => theme::AMBER_DIM,
    }
}

pub fn panel(ui: &Ui) -> AnyElement {
    flyout(ui, Which::Dev, nsh_metro::FlyoutWidth::Narrow, |_| "test tools".into(), |ui| {
        // What the controls show: rebuilt when that changes.
        let shown = ui.memo(|b| (b.mode, b.sandbox.auto(), (b.sandbox.rate() * 100.0) as i32, b.sandbox.groups()));
        let mode = ui.memo(|b| b.mode);
        let (a, r) = (ui.clone(), ui.clone());
        view()
            .child(dynamic(move || {
                let (mode, ..) = shown.get();
                tools(&a, mode)
            }))
            // In live mode this was only ever a list, and the list has a
            // panel of its own (⌘L, #103); in the sandbox these rows are the
            // controls themselves.
            .child(when(move || mode.get() == Mode::Sandbox, move || sessions(&r)))
            .child(hint("T toggles this panel · ⌘L lists the sessions · Esc closes"))
            .into_element()
    })
}

/// A button of the test tools. What it does is read off the sandbox at
/// once rather than at the next poll, so the board and the buttons show it
/// straight away.
fn act(ui: &Ui, text: impl Into<SharedString>, active: bool, f: impl Fn() + 'static) -> View {
    let ui = ui.clone();
    button(text, move || active, move || {
        f();
        ui.sync_sandbox();
    })
}

/// A sandbox action on one session.
fn on(ui: &Ui, sb: &Arc<FakeSource>, sid: &str, text: impl Into<SharedString>, active: bool, f: impl Fn(&FakeSource, &str) + 'static) -> View {
    let (sb, sid) = (sb.clone(), sid.to_string());
    act(ui, text, active, move || f(&sb, &sid))
}

fn dim(a: f32) -> impl Fn() -> Hsla + Clone + 'static {
    move || palette().foreground.opacity(a)
}

fn caption(text: impl Into<SharedString>) -> impl IntoElement {
    label(text.into()).w(px(56.)).flex_shrink_0().text_size(px(SMALL)).text_color(dim(0.55))
}

/// The source switch and the sandbox's own controls. Built again only
/// when what they show changes, not on every poll.
fn tools(ui: &Ui, mode: Mode) -> AnyElement {
    let sandbox = ui.peek(|b| b.sandbox.clone());
    let (to_live, to_sandbox) = (ui.clone(), ui.clone());
    let mut panel = view().child(
        section("SOURCE").child(
            view()
                .flex_row()
                .gap(px(4.))
                .child(act(ui, "live", mode == Mode::Live, move || to_live.update(|b| b.set_mode(Mode::Live))))
                .child(act(ui, "sandbox", mode == Mode::Sandbox, move || to_sandbox.update(|b| b.set_mode(Mode::Sandbox)))),
        ),
    );

    if mode == Mode::Sandbox {
        let add = |text: &'static str, phase: Phase| {
            let sb = sandbox.clone();
            act(ui, text, false, move || {
                sb.add(phase);
            })
        };
        let (sb_fill, sb_clear, sb_auto) = (sandbox.clone(), sandbox.clone(), sandbox.clone());
        let auto = sandbox.auto();
        // How fast the churn runs (#106).
        let rate = sandbox.rate();
        let mut rates = view().flex_row().items_center().flex_wrap().gap(px(4.)).child(caption("rate"));
        for r in sources::CHURN_RATES {
            let sb = sandbox.clone();
            rates = rates.child(act(ui, format!("{r}\u{00d7}"), (rate - r).abs() < 0.01, move || sb.set_rate(r)));
        }
        // Growing a cluster a pane at a time (#107).
        let mut grow = view().flex_row().items_center().flex_wrap().gap(px(4.)).child(caption("grow"));
        for (group, n) in sandbox.groups() {
            let sb = sandbox.clone();
            let text = format!("{}+{n}", group.rsplit(':').next().unwrap_or(&group));
            grow = grow.child(act(ui, text, false, move || {
                sb.add_to_group(&group, Phase::Working);
            }));
        }
        panel = panel.child(
            section("SANDBOX SESSIONS")
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.))
                        .child(add("+ working", Phase::Working))
                        .child(add("+ needs input", Phase::NeedsUser))
                        .child(add("+ idle", Phase::Idle)),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.))
                        .child(act(ui, "fill ×8", false, move || sb_fill.fill(8)))
                        .child(act(ui, "clear", false, move || sb_clear.clear()))
                        .child(act(ui, "auto churn", auto, move || sb_auto.set_auto(!sb_auto.auto()))),
                )
                .child(rates)
                .child(grow),
        );
    }

    panel.into_element()
}

/// One sandbox session's controls: its line, its phase and subagent
/// buttons, and its detail rows when they are open (#180).
fn session_row(ui: &Ui, sandbox: &Arc<FakeSource>, info: SessionInfo, open: bool) -> View {
    let sandbox = sandbox.clone();
    let mut list = view().gap(px(4.));
    let phase = info.phase();
    let sid = info.session_id.clone();
    let mut line = format!("{} · {}", info.label(), phase.label());
    if let Some(t) = &info.ticket {
        line = format!("{line} · {}", t.label());
    }
    for p in &info.prs {
        line = format!("{line} · {} {}", p.label(), p.look().short());
    }
    let cycle = {
        let (sb, sid, ui) = (sandbox.clone(), sid.clone(), ui.clone());
        move || {
            sb.cycle(&sid);
            ui.sync_sandbox();
        }
    };
    let toggle = {
        let (ui, sid) = (ui.clone(), sid.clone());
        move || {
            ui.update(|b| {
                if !b.dev_expanded.remove(&sid) {
                    b.dev_expanded.insert(sid.clone());
                }
            })
        }
    };
    let mut row = view()
        .flex_row()
        .items_center()
        .gap(px(4.))
        .child(phase_mark(phase, 8.))
        .child(
            label(line)
                .flex_1()
                .text_size(px(crate::ui::metro::BODY))
                .truncate()
                .text_color(dim(1.0))
                .cursor_pointer()
                .on_press(cycle),
        )
        // The detail rows open per session (#180).
        .child(button(if open { "▾" } else { "▸" }, move || open, toggle));
    for p in Phase::ALL {
        row = row.child(on(ui, &sandbox, &sid, p.short(), p == phase, move |sb, id| sb.set_phase(id, p)));
    }
    row = row.child(on(ui, &sandbox, &sid, "+sub", false, |sb, id| {
        sb.add_sub(id, true);
    }));
    row = row.child(on(ui, &sandbox, &sid, "×", false, |sb, id| sb.remove(id)));
    list = list.child(row);

    if open {
        // The pull requests (#58, #112, #116, #134), the pages (#137).
        let look = info.prs.last().map(|p| p.look());
        let head =
            info.prs.last().map(|p| p.label()).or_else(|| info.prs_unanswered.last().map(|r| format!("{}?", r.label())));
        let chain = info.prs.len() + info.prs_unanswered.len();
        let mut links = view().flex_row().items_center().flex_wrap().gap(px(4.)).pl(px(12.)).child(caption(match (head, chain) {
            (None, _) => "PR".to_string(),
            (Some(h), 0 | 1) => h,
            (Some(h), n) => format!("{h} +{}", n - 1),
        }));
        links = links.child(on(ui, &sandbox, &sid, "−", look.is_none(), |sb, id| sb.set_pr(id, None)));
        for l in pr::Look::ALL {
            links = links.child(on(ui, &sandbox, &sid, l.letter(), look == Some(l), move |sb, id| sb.set_pr(id, Some(l))));
        }
        let ci = info.prs.last().is_some_and(|p| p.running());
        links = links.child(on(ui, &sandbox, &sid, "CI", ci, move |sb, id| sb.set_pr_running(id, !ci)));
        let ok = info.prs.last().is_some_and(|p| p.approved);
        links = links.child(on(ui, &sandbox, &sid, "OK", ok, move |sb, id| sb.set_pr_approved(id, !ok)));
        let next = pr::Look::ALL[info.prs.len() % pr::Look::ALL.len()];
        links = links.child(on(ui, &sandbox, &sid, "+", false, move |sb, id| sb.add_pr(id, next)));
        links = links.child(on(ui, &sandbox, &sid, "?", false, |sb, id| sb.add_unanswered(id)));
        links = links.child(on(ui, &sandbox, &sid, "A+", false, |sb, id| sb.add_artifact(id)));
        links = links.child(on(ui, &sandbox, &sid, "A−", info.artifacts.is_empty(), |sb, id| sb.clear_artifacts(id)));
        list = list.child(links);

        // The Linear issue (#58).
        let status = info.ticket.as_ref().map(|t| t.status);
        let mut tickets = view().flex_row().items_center().flex_wrap().gap(px(4.)).pl(px(12.)).child(caption(match &info.ticket {
            Some(t) => t.label(),
            None => "LINEAR".into(),
        }));
        tickets = tickets.child(on(ui, &sandbox, &sid, "−", status.is_none(), |sb, id| sb.set_ticket(id, None)));
        for st in ticket::Status::ALL {
            tickets = tickets.child(on(ui, &sandbox, &sid, st.letter(), status == Some(Some(st)), move |sb, id| {
                sb.set_ticket(id, Some(Some(st)))
            }));
        }
        list = list.child(tickets);

        // Where the session runs (#124).
        let seated = info.seat.as_ref().map(|s| s.host);
        let mut hosts = view().flex_row().items_center().flex_wrap().gap(px(4.)).pl(px(12.)).child(caption("host"));
        hosts = hosts.child(on(ui, &sandbox, &sid, "−", seated.is_none(), |sb, id| sb.set_host(id, None)));
        for (host, name) in crate::host::NAMED {
            hosts = hosts.child(on(ui, &sandbox, &sid, name, seated == Some(host), move |sb, id| sb.set_host(id, Some(host))));
        }
        list = list.child(hosts);
    }

    // Subagents, indented under their session.
    for sub in &info.subagents {
        let text = format!(
            "└ {} · {} · {}",
            sub.label(),
            if sub.description.is_empty() { "-" } else { sub.description.as_str() },
            if sub.running { "running" } else { "done" }
        );
        let (aid, running) = (sub.agent_id.clone(), sub.running);
        let (sb1, sb2, aid2) = (sandbox.clone(), sandbox.clone(), aid.clone());
        list = list.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .pl(px(12.))
                .child(phase_mark(sub.phase(), 5.))
                .child(label(text).flex_1().text_size(px(SMALL)).truncate().text_color(dim(0.6)))
                .child(act(ui, if running { "done" } else { "run" }, false, move || sb1.set_sub_running(&aid, !running)))
                .child(act(ui, "×", false, move || sb2.remove_sub(&aid2))),
        );
    }
    list
}

/// The sandbox's sessions, a row each. Keyed by what a row shows, so a poll
/// that changed nothing rebuilds nothing, and a change rebuilds its row
/// alone (#180): a panel of sixty sessions is some six hundred buttons.
fn sessions(ui: &Ui) -> AnyElement {
    let count = ui.memo(|b| b.sandbox.len());
    let items = ui.memo(|b| {
        b.model
            .live()
            .map(|c| (c.info.clone(), b.dev_expanded.contains(&c.info.session_id)))
            .collect::<Vec<_>>()
    });
    let sandbox = ui.peek(|b| b.sandbox.clone());
    let ui = ui.clone();
    section("sessions · click a name to cycle · +sub adds a subagent")
        .child(label(move || SharedString::from(format!("{} total", count.get()))).text_size(px(SMALL)).text_color(dim(0.45)))
        .child(when(
            move || count.get() == 0,
            || label("no synthetic sessions yet — add one above").text_size(px(SMALL)).text_color(dim(0.5)),
        ))
        .child(view().gap(px(4.)).child(each(
            move || items.get(),
            |(info, open): &(SessionInfo, bool)| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                format!("{info:?}{open}").hash(&mut h);
                (info.session_id.clone(), h.finish())
            },
            move |item, _| {
                let (info, open) = untracked(|| item.get());
                session_row(&ui, &sandbox, info, open)
            },
        )))
        .into_element()
}

