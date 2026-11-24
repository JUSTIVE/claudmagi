//! The list panel (⌘L): everything the board is drawing, written down the
//! right-hand side (#103).
//!
//! The start screen says how things stand at a glance; this says it in
//! words you can scan, in board order, one session per block with its
//! issue, its pull requests and its subagents underneath. Every line is the
//! same click its tile is.

use nsh_core::prelude::*;
use nsh_metro::{label, palette};

use crate::ui::metro::{BODY, PAD, SMALL, phase_mark};
use crate::ui::panel::{Which, flyout, hint};
use crate::ui::state::Ui;

pub fn panel(ui: &Ui) -> AnyElement {
    flyout(
        ui,
        Which::List,
        nsh_metro::FlyoutWidth::Narrow,
        // The count rides in the title.
        |b| format!("sessions  {}", b.model.live().count()),
        |ui| {
            let ui = ui.clone();
            view()
                .child(dynamic(move || rows(&ui)))
                .child(hint("\u{2318}L toggles this panel"))
                .into_element()
        },
    )
}

/// A row that opens something: hover shading, the press, a label and the
/// state beside it.
fn line(text: String, state: String, on_press: impl Fn() + 'static) -> View {
    let dim = |a: f32| move || palette().foreground.opacity(a);
    view()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .pl(px(8.))
        .cursor_pointer()
        .hover(|s| s.opacity(0.75))
        .on_press(move || on_press())
        .child(label(text).flex_1().text_size(px(SMALL)).truncate().text_color(dim(0.75)))
        .child(label(state).flex_shrink_0().text_size(px(SMALL)).text_color(dim(0.4)))
}

fn rows(ui: &Ui) -> AnyElement {
    let dim = |a: f32| move || palette().foreground.opacity(a);
    let rows: Vec<(crate::model::SessionInfo, Vec<crate::model::SubagentInfo>)> = ui.read(|b| {
        b.model
            .live()
            .map(|chip| (chip.info.clone(), chip.subs.iter().filter(|s| !s.anim.gone).map(|s| s.info.clone()).collect()))
            .collect()
    });
    let mut list = view().gap(px(12.)).px(px(PAD)).py(px(8.));
    for (info, subs) in rows {
        let phase = info.phase();
        let mut block = view().gap(px(2.));

        // The session itself.
        let (go, id) = (ui.clone(), info.session_id.clone());
        block = block.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .hover(|s| s.opacity(0.75))
                .on_press(move || go.activate_session(&id))
                .child(phase_mark(phase, 10.))
                .child(label(info.label()).flex_1().text_size(px(BODY + 2.)).truncate().text_color(dim(1.0)))
                .child(label(phase.label()).flex_shrink_0().text_size(px(SMALL)).text_color(dim(0.4))),
        );
        block = block.child(label(info.short_cwd()).text_size(px(SMALL)).truncate().text_color(dim(0.4)));

        // Its issue.
        if let Some(t) = &info.ticket {
            let (go, key, app, web) = (ui.clone(), t.label(), t.app_url(), t.url());
            let state = t.status.map(|s| s.short().to_string()).unwrap_or_else(|| "NO STATUS".into());
            block = block.child(line(t.label(), state, move || {
                go.open_link(key.clone(), app.clone(), web.clone(), "Linear issue")
            }));
        }

        // Its pull requests, newest last; review and CI ride on either side
        // of the word (#116).
        for pr in &info.prs {
            let mut state = pr.look().short().to_string();
            if pr.signed_off() {
                state = format!("\u{2713} {state}");
            }
            if pr.running() {
                state = format!("{state} \u{27f3}");
            }
            let (go, name, url) = (ui.clone(), pr.label(), pr.url());
            block = block.child(line(pr.label(), state, move || go.open_link(name.clone(), None, Some(url.clone()), "pull request")));
        }

        // And the pages it published (#137).
        for a in &info.artifacts {
            let state = a.version_label().unwrap_or_else(|| "ARTIFACT".into());
            let (go, name, url) = (ui.clone(), a.label(), a.url());
            block = block.child(line(a.label(), state, move || {
                go.open_link(name.clone(), None, Some(url.clone()), crate::made::ARTIFACT)
            }));
        }

        // And its subagents, which have nowhere of their own to go.
        for sub in &subs {
            block = block.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(8.))
                    .child(label(format!("\u{2514} {}", sub.label())).flex_1().text_size(px(SMALL)).truncate().text_color(dim(0.55)))
                    .child(label(if sub.running { "running" } else { "done" }).text_size(px(SMALL)).text_color(dim(0.4))),
            );
        }
        list = list.child(block);
    }
    list.into_element()
}
