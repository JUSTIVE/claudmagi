//! Shared chrome for the panels (test tools, settings, the session list):
//! gpui-metro's settings flyout (#175), and the small widgets the panels are
//! built from.

use nsh_core::prelude::*;
use nsh_metro::motion::{EXP6_IN, EXP6_OUT};
use nsh_metro::theme::reduced_motion;
use nsh_metro::{FlyoutWidth, Icon, label as metro_label, palette as metro_palette};

pub use crate::ui::metro::{chip as button, hint, section};

use crate::ui::metro::{BODY, PAD};
use crate::ui::state::{Board, Ui};

/// Which panel a header drag belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Dev,
    Settings,
    /// The list down the right-hand side (#103).
    List,
    /// The ⌘K search (#71), a panel like the rest since #193.
    Search,
    /// What Linear is asking of you (#244).
    Inbox,
}

impl Which {
    /// Every panel, for the sweep that closes the others (#163).
    pub const ALL: [Which; 5] = [Which::Dev, Which::Settings, Which::List, Which::Search, Which::Inbox];

    /// Its place in that sweep. A `match` rather than a search through `ALL`,
    /// so a panel added later cannot quietly miss it.
    fn slot(self) -> usize {
        match self {
            Which::Dev => 0,
            Which::Settings => 1,
            Which::List => 2,
            Which::Search => 3,
            Which::Inbox => 4,
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
        Which::Inbox => &mut b.inbox_panel.open,
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
            Which::Inbox => b.inbox_panel.open,
        }))
    };
    let title = untracked(|| ui.peek(|b| title(b)));
    let (shut, content) = (ui.clone(), ui.clone());
    let body = std::rc::Rc::new(body);
    sheet(
        open,
        width,
        title,
        // The sections carry the panels' own side margin; the sheet brings
        // Metro's, so one of them goes.
        move || view().mx(px(-PAD)).child(body(&content)).into_element(),
        move || shut.update(|b| *panel_open(b, which) = false),
    )
}

/// How long the sheet takes to come in and to go, in ms: Metro's own
/// `showPanel` timings, a long hard deceleration in and a quicker
/// acceleration out.
const IN_MS: f32 = 550.;
const OUT_MS: f32 = 250.;
/// The header band over a panel's body.
const HEADER: f32 = 80.;

/// A panel's own chrome: full height down the right edge, sliding in from it
/// and out again, a press outside closing it.
///
/// nsh-metro's `settings_flyout` is this with an **accent** header band, and
/// it is fixed there (#245). The board wants the accent to mean one thing at
/// a time, and a sheet of colour at the top of every panel is not that: here
/// the header is the sheet's own ground with the title in ink, and the one
/// place the accent is left in a panel is the name of whoever is asking for
/// something (`ui::inbox`).
fn sheet(
    open: Memo<bool>,
    width: FlyoutWidth,
    title: String,
    body: impl Fn() -> AnyElement + 'static,
    dismiss: impl Fn() + 'static,
) -> AnyElement {
    let w = width.px();
    let initially = untracked(|| open.get());
    // How far off the right edge it sits: 0 is in place.
    let x = animated_value(if initially { 0. } else { w });
    let mounted = signal(initially);
    let is_open = signal(initially);
    effect(move || {
        let up = open.get();
        untracked(|| {
            is_open.set(up);
            if up {
                mounted.set(true);
                if reduced_motion() {
                    x.set(0.);
                } else {
                    x.set(w);
                    let _ = x.timing(0., IN_MS, EXP6_OUT);
                }
            } else if reduced_motion() {
                mounted.set(false);
            } else {
                let gone = x.timing(w, OUT_MS, EXP6_IN);
                spawn(async move {
                    if gone.await && !is_open.get_untracked() {
                        mounted.set(false);
                    }
                });
            }
        });
    });
    let dismiss = std::rc::Rc::new(dismiss);
    let title = title.to_lowercase();
    let panel = move || {
        let ink = move || metro_palette().foreground;
        let back = dismiss.clone();
        let header = view()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .gap(px(14.))
            .h(px(HEADER))
            .px(px(24.))
            .child(
                view()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded(px(16.))
                    .border(px(2.))
                    .border_color(ink)
                    .cursor_pointer()
                    .active(|s| s.opacity(0.6))
                    .on_press(move || back())
                    .child(Icon::Back.render(16., ink)),
            )
            .child(metro_label(title.clone()).text_size(px(24.)).truncate().text_color(ink));
        let out = dismiss.clone();
        view()
            .absolute()
            .inset_0()
            .child(when(move || is_open.get(), {
                let out = out.clone();
                move || {
                    let out = out.clone();
                    view().absolute().inset_0().on_press(move || out())
                }
            }))
            .child(
                view()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(w))
                    .bg(move || metro_palette().background)
                    .border_l_1()
                    .border_color(move || metro_palette().border)
                    .bind_translate_x(&x)
                    // Swallows presses, so the board behind does not take
                    // them and the sheet does not dismiss itself.
                    .on_press(|| {})
                    .child(header)
                    .child(scroll().flex_1().child(view().px(px(24.)).py(px(20.)).gap(px(18.)).child(body()))),
            )
            .into_element()
    };
    view().absolute().inset_0().pointer_events(PointerEvents::BoxNone).child(when(move || mounted.get(), panel)).into_element()
}

/// A flyout's own layer over the window: the flyout places itself against
/// the box it is given, so each gets the whole of it.
fn layer(flyout: AnyElement) -> View {
    view().absolute().inset_0().pointer_events(PointerEvents::BoxNone).child(flyout)
}

/// The panels, over everything but the status bar, whose buttons and keys
/// open and close them.
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
        .child(layer(crate::ui::inbox::panel(&ui)))
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
        let inbox = after_toggle(settings, Which::Inbox);
        assert!(inbox[Which::Inbox.slot()] && !inbox[Which::Settings.slot()], "⌘A is one of them too (#244)");
        assert_eq!(after_toggle(inbox, Which::Inbox), none, "and closes on a second press");
        assert_eq!(Which::ALL.map(|w| w.slot()), [0, 1, 2, 3, 4], "every panel has a place of its own in the sweep");
    }
}
