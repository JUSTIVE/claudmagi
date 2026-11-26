//! The inbox panel (⌘A): what Linear is asking of you, written down the
//! right-hand side (#244).
//!
//! ⌘K is a search over everything the board draws, and a notification is
//! not something you go looking for: it is something you run your eye down.
//! So it gets a panel of its own, where the rows stand in the order they
//! arrived and every one of them is the way to Linear (#243) — straight to
//! the comment or the issue, not to Linear's own inbox page.
//!
//! Empty is the normal state and says so. A missing key says that instead:
//! an inbox that is clear and an inbox that was never asked look identical
//! from the outside, and only here is there room to tell them apart.

use nsh_core::prelude::*;
use nsh_metro::{label, palette};

use crate::inbox::Note;
use crate::ui::metro::{BODY, PAD, SMALL};
use crate::ui::panel::{Which, flyout, hint};
use crate::ui::state::Ui;

pub fn panel(ui: &Ui) -> AnyElement {
    flyout(
        ui,
        Which::Inbox,
        // A row is a line of somebody's writing, so it wants the wider of
        // Metro's two widths, as the search does (#193).
        nsh_metro::FlyoutWidth::Wide,
        // No count in the title: a flyout's title is taken once, when the
        // panel is built, so a number there would be the one the board
        // started with. The badge counts (#242); this lists.
        |_| "linear".into(),
        |ui| {
            let ui = ui.clone();
            view()
                .child(dynamic(move || rows(&ui)))
                .child(hint("\u{2318}A toggles this panel"))
                .into_element()
        },
    )
}

/// One notification: what it is about over who wants what, the whole block
/// a press that goes where the asking is.
fn row(ui: &Ui, note: &Note) -> View {
    let dim = |a: f32| move || palette().foreground.opacity(a);
    let (go, label_text, said, url) = (ui.clone(), note.label(), note.said(), note.url.clone());
    let actor = note.actor.clone();
    // The one place a panel still wears the accent (#245): who is asking.
    let accent = move || nsh_metro::palette().accent;
    let app = crate::inbox::app_url(&url);
    let web = (!url.is_empty()).then(|| url.clone());
    let category = crate::inbox::spaced(&note.category);
    view()
        .gap(px(2.))
        .cursor_pointer()
        .hover(|s| s.opacity(0.75))
        .on_press(move || go.open_link(label_text.clone(), app.clone(), web.clone(), "Linear notification"))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(label(note.label()).flex_1().text_size(px(BODY + 2.)).truncate().text_color(dim(1.0)))
                .child(label(category).flex_shrink_0().text_size(px(SMALL)).text_color(dim(0.4))),
        )
        .child(
            view()
                .flex_row()
                .items_baseline()
                .gap(px(6.))
                .children((!actor.is_empty()).then(|| {
                    label(actor.clone()).flex_shrink_0().text_size(px(SMALL)).text_color(accent)
                }))
                .child(label(said).flex_1().text_size(px(SMALL)).truncate().text_color(dim(0.75))),
        )
}

fn rows(ui: &Ui) -> AnyElement {
    let dim = |a: f32| move || palette().foreground.opacity(a);
    let (notes, trouble) = ui.read(|b| (b.inbox.notes.clone(), word_for(b.inbox_error.as_ref())));
    let mut list = view().gap(px(12.)).px(px(PAD)).py(px(8.));
    if notes.is_empty() {
        return list
            .child(label(trouble).text_size(px(SMALL)).text_color(dim(0.4)))
            .into_element();
    }
    for note in &notes {
        list = list.child(row(ui, note));
    }
    list.into_element()
}

/// What an empty panel says. A clear inbox is the normal state; anything
/// else is the one place the board admits Linear is not answering.
fn word_for(err: Option<&crate::inbox::Error>) -> String {
    use crate::inbox::Error;
    match err {
        None => "inbox clear".into(),
        Some(Error::NoKey) => format!("no linear api key in the keychain ({})", crate::inbox::KEYCHAIN_SERVICE),
        Some(e) => e.to_string().to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inbox::Error;

    /// Clear is the normal state and reads as itself; a key that was never
    /// put in reads as that instead, because from the outside the two look
    /// exactly alike (#244).
    #[test]
    fn an_empty_panel_says_which_kind_of_empty_it_is() {
        assert_eq!(word_for(None), "inbox clear");
        assert!(word_for(Some(&Error::NoKey)).contains("claudmagi-linear"));
        assert!(word_for(Some(&Error::Request("HTTP 400".into()))).contains("http 400"));
    }
}
