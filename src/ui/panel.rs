//! Shared chrome for the panels (test tools, settings, the session list):
//! gpui-metro's settings flyout (#175), and the small widgets the panels are
//! built from.

use nsh_core::prelude::*;
use nsh_metro::settings_flyout;

pub use crate::ui::metro::{chip as button, hint, section};

use crate::ui::metro::{BODY, PAD};
use crate::ui::state::{Board, Ui};

/// Which panel a header drag belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Dev,
    Settings,
    /// The list down the right-hand side (#103).
    List,
    /// The ⌘K search (#71), a panel like the rest since #193.
    Search,
}

impl Which {
    /// Every panel, for the sweep that closes the others (#163).
    pub const ALL: [Which; 4] = [Which::Dev, Which::Settings, Which::List, Which::Search];

    /// Its place in that sweep. A `match` rather than a search through `ALL`,
    /// so a panel added later cannot quietly miss it.
    fn slot(self) -> usize {
        match self {
            Which::Dev => 0,
            Which::Settings => 1,
            Which::List => 2,
            Which::Search => 3,
        }
    }
}

/// What is up after `which` is pressed: it flips, everything else closes
/// (#163). The search is one of them since #193, so ⌘K puts away whatever
/// panel was open and a panel button puts away the search.
fn after_toggle(open: [bool; Which::ALL.len()], which: Which) -> [bool; Which::ALL.len()] {
    let mut next = [false; Which::ALL.len()];
    next[which.slot()] = !open[which.slot()];
    next
}

/// Opens `which` and closes the others, or closes `which` if it was the one
/// open: the panels are never up together (#163).
pub fn toggle(b: &mut Board, which: Which) {
    let open = Which::ALL.map(|w| *panel_open(b, w));
    for (w, up) in Which::ALL.into_iter().zip(after_toggle(open, which)) {
        *panel_open(b, w) = up;
    }
    if which == Which::Search && b.palette.open {
        crate::ui::palette::refresh(b);
    }
}

/// Whether a panel is up. The search keeps its own state, and that state's
/// flag stays the only truth about whether it is open (#193).
pub fn panel_open(b: &mut Board, which: Which) -> &mut bool {
    match which {
        Which::Dev => &mut b.dev.open,
        Which::Settings => &mut b.settings_panel.open,
        Which::List => &mut b.list_panel.open,
        Which::Search => &mut b.palette.open,
    }
}

/// A panel as nsh-metro's settings flyout (#175): full height down the right
/// edge under an accent header with a back button and the panel's lower-case
/// title, the body scrolling under it. It slides in when its panel opens and
/// out when it closes; a press outside it closes it (light dismiss), as the
/// back button does. Its body is built when it opens and binds what changes.
pub fn flyout(
    ui: &Ui,
    which: Which,
    width: nsh_metro::FlyoutWidth,
    title: impl Fn(&Board) -> String + 'static, body: impl Fn(&Ui) -> AnyElement + 'static) -> AnyElement {
    // A memo, not a closure over the board: the flyout slides in whenever
    // what it reads changes, and the board changes every poll. Only a change
    // of the flag itself may move it.
    let open = {
        let ui = ui.clone();
        computed(move || ui.read(|b| match which {
            Which::Dev => b.dev.open,
            Which::Settings => b.settings_panel.open,
            Which::List => b.list_panel.open,
            Which::Search => b.palette.open,
        }))
    };
    let title = untracked(|| ui.peek(|b| title(b)));
    let (shut, content) = (ui.clone(), ui.clone());
    let body = std::rc::Rc::new(body);
    settings_flyout(move || open.get())
        .title(title)
        .width(width)
        // The sections carry the panels' own side margin; the flyout brings
        // Metro's, so one of them goes.
        .child(move || {
            view().mx(px(-PAD)).child(body(&content))
        })
        .on_dismiss(move || shut.update(|b| *panel_open(b, which) = false))
        .into_element()
}

/// A flyout's own layer over the window: the flyout places itself against
/// the box it is given, so each gets the whole of it.
fn layer(flyout: AnyElement) -> View {
    view().absolute().inset_0().pointer_events(PointerEvents::BoxNone).child(flyout)
}

/// The four panels, over everything but the status bar, whose buttons open
/// and close them.
pub fn flyouts(ui: Ui) -> AnyElement {
    let _ = BODY;
    view()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom(px(crate::ui::state::STATUS_H))
        .pointer_events(PointerEvents::BoxNone)
        .child(layer(crate::ui::settings::panel(&ui)))
        .child(layer(crate::ui::devtools::panel(&ui)))
        .child(layer(crate::ui::list::panel(&ui)))
        .child(layer(crate::ui::palette::panel(&ui)))
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panels are never up together (#163), and since #193 the ⌘K search
    /// is one of them: it puts away whatever was open, and whatever opens
    /// next puts it away.
    #[test]
    fn only_one_panel_is_ever_up() {
        let none = [false; Which::ALL.len()];
        let dev = after_toggle(none, Which::Dev);
        assert!(dev[Which::Dev.slot()]);

        let search = after_toggle(dev, Which::Search);
        assert!(search[Which::Search.slot()], "⌘K opens the search");
        assert!(!search[Which::Dev.slot()], "and puts the test tools away");

        let settings = after_toggle(search, Which::Settings);
        assert!(settings[Which::Settings.slot()] && !settings[Which::Search.slot()], "and the other way round");

        assert_eq!(after_toggle(settings, Which::Settings), none, "pressing the same one closes it");
        assert_eq!(Which::ALL.map(|w| w.slot()), [0, 1, 2, 3], "every panel has a place of its own in the sweep");
    }
}
