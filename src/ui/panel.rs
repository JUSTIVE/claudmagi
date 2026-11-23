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
}

impl Board {
    /// Opens `which` and closes the other two, or closes `which` if it was
    /// the one open: the test tools, the settings and the list are never up
    /// together (#163).
    pub(crate) fn toggle_panel(&mut self, which: Which) {
        let was = self.panel_mut(which).open;
        for w in [Which::Dev, Which::Settings, Which::List] {
            self.panel_mut(w).open = false;
        }
        self.panel_mut(which).open = !was;
    }

    pub(crate) fn panel_mut(&mut self, which: Which) -> &mut PanelState {
        match which {
            Which::Dev => &mut self.dev,
            Which::Settings => &mut self.settings_panel,
            Which::List => &mut self.list_panel,
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
        };
        let open = self.panel_mut(which).open;
        settings_flyout(id, open)
            .title(title)
            // The sections carry the panels' own side margin; the flyout
            // brings Metro's, so one of them goes.
            .child(div().mx(px(-PAD)).text_size(px(BODY)).child(body))
            .on_dismiss(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.panel_mut(which).open = false;
                this.touch(cx);
            }))
    }
}
