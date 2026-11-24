//! Shared chrome for the panels (test tools, settings, the session list):
//! gpui-metro's settings flyout (#175), and the small widgets the panels are
//! built from.

use gpui::{ClickEvent, Context, SharedString, div, prelude::*, px};
use gpui_metro::{SettingsFlyout, settings_flyout};

use crate::ui::board::Board;
use crate::ui::metro::{BODY, PAD};

#[derive(Default)]
pub struct PanelState {
    pub open: bool,
}

pub use crate::ui::metro::{chip as button, hint, section};

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

impl Board {
    /// Opens `which` and closes the other two, or closes `which` if it was
    /// the one open: the test tools, the settings and the list are never up
    /// together (#163).
    pub(crate) fn toggle_panel(&mut self, which: Which) {
        let open = Which::ALL.map(|w| *self.panel_open(w));
        for (w, up) in Which::ALL.into_iter().zip(after_toggle(open, which)) {
            *self.panel_open(w) = up;
        }
    }

    /// Whether a panel is up. The flag rather than the state, because the
    /// search keeps its own (the query and the rows it found) and that
    /// state's flag has to stay the only truth about whether it is open
    /// (#193).
    pub(crate) fn panel_open(&mut self, which: Which) -> &mut bool {
        match which {
            Which::Dev => &mut self.dev.open,
            Which::Settings => &mut self.settings_panel.open,
            Which::List => &mut self.list_panel.open,
            Which::Search => &mut self.palette.open,
        }
    }

    /// A panel as gpui-metro's settings flyout (#175): full height down the
    /// right edge under an accent header with a back button and the panel's
    /// lower-case title, the body scrolling under it. It is drawn every frame
    /// with whether it is open, so it slides in on open and back out on
    /// close by itself; a press outside it closes it and stops there (light
    /// dismiss), as the back button does.
    pub(crate) fn flyout(
        &mut self,
        which: Which,
        title: impl Into<SharedString>,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> SettingsFlyout {
        let id = match which {
            Which::Dev => "dev-flyout",
            Which::Settings => "settings-flyout",
            Which::List => "list-flyout",
            Which::Search => "search-flyout",
        };
        let open = *self.panel_open(which);
        settings_flyout(id, open)
            .title(title)
            // The sections carry the panels' own side margin; the flyout
            // brings Metro's, so one of them goes.
            .child(div().mx(px(-PAD)).text_size(px(BODY)).child(body))
            .on_dismiss(cx.listener(move |this, _: &ClickEvent, _, cx| {
                *this.panel_open(which) = false;
                this.touch(cx);
            }))
    }
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
