//! The pull request list a cell drops down (#134).
//!
//! A cell has one line for GitHub and draws the newest pull request it has,
//! so when a session owns several the others are neither seen nor reachable:
//! the line opened the one it was showing, and nothing said there were more.
//! Pressing the line now drops the whole set down instead, one row each, and
//! a row opens its own pull request.
//!
//! The rows are the nodes: each wears the body the board would give that pull
//! request (#78), so the list reads the way the board does. A number GitHub
//! would not answer for is in the list as well, drawn as not knowing rather
//! than left out (#112) — it can still be opened, since the number is all a
//! link needs.

use gpui::{
    Anchor, AnyElement, ClickEvent, Context, Edges, MouseButton, SharedString, anchored, deferred,
    div, point, prelude::*, px, svg,
};

use crate::logos;
use crate::pr;
use crate::render::comb;
use crate::theme;
use crate::ui::board::{Board, STATUS_H};
use crate::ui::palette::{Body, Style, card_bg};

/// A row of the list, and the card's own padding.
const ROW_H: f32 = 22.0;
const PAD: f32 = 4.0;
/// How wide the card may run. The number is short and the title is as long as
/// somebody felt like making it, so the title is what gives way.
const MAX_W: f32 = 320.0;
/// Between the row it came from and the card.
const DROP: f32 = 6.0;
/// Kept clear of the window's edges, and of the status bar along the bottom.
const EDGE: f32 = 8.0;
/// The node body. Narrower than the palette's, which carries a session name:
/// everything in here is a number.
const CHIP_W: f32 = 66.0;
const CHIP_H: f32 = 18.0;
const LOGO: f32 = 10.0;

impl Board {
    /// The open cell's pull requests, as a card hanging off its GitHub row.
    ///
    /// Nothing unless the comb is the view showing and the session is still
    /// on it: the card is anchored to a cell, and a cell is the only thing
    /// that can be short of room for its pull requests.
    pub(crate) fn render_pr_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.pr_menu.clone()?;
        let i = self.model.chips.iter().position(|c| c.info.session_id == id)?;
        let (at, _, h) = comb::slot_box(self.comb.as_ref()?, i, comb::Slot::Pr)?;
        let info = &self.model.chips[i].info;
        let rows = pr::rows(&info.prs, &info.prs_unanswered);
        // The sandbox's pull requests are made up, so they go nowhere, the
        // same way its sessions have no terminal to jump to.
        let real = !info.synthetic;
        if rows.len() < 2 {
            return None;
        }

        // The cell is in the grid's own units and scrolls with the grid; the
        // card is in window pixels and does not.
        let zoom = self.layout.zoom;
        let (x, y) = (at.x * zoom, (at.y - self.scroll_y) * zoom);
        let reach = h * zoom / 2.0;
        let tall = rows.len() as f32 * ROW_H + PAD * 2.0;
        // Below the row it came from, unless the status bar is in the way, in
        // which case it goes up instead — the one thing the window's own
        // snapping cannot work out, since the bar is painted over the board
        // rather than taken out of it.
        let under = self.window.1 - STATUS_H - EDGE;
        let (anchor, from) = match y + reach + DROP + tall <= under {
            true => (Anchor::TopCenter, y + reach + DROP),
            false => (Anchor::BottomCenter, y - reach - DROP),
        };

        let theme = self.palette();
        let ink = |a: f32| theme::hsla(theme::with_alpha(theme.ink, a));
        let mut list = div().flex().flex_col().py(px(PAD));
        for (k, row) in rows.into_iter().enumerate() {
            let body = match row.look {
                Some(look) => {
                    Style::Pr { look, running: row.running, signed_off: row.signed_off }.body(theme)
                }
                // No state to draw, so it is drawn as not knowing (#112).
                None => Body { fill: None, border: Some(ink(0.3)), text: ink(0.5) },
            };
            let detail = match row.title.trim() {
                "" => "GitHub would not answer".to_string(),
                title => title.to_string(),
            };
            let (label, url) = (row.label.clone(), row.url.clone());
            list = list.child(
                div()
                    .id(SharedString::from(format!("pr-menu-{i}-{k}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px(px(PAD + 4.0))
                    .h(px(ROW_H))
                    .cursor_pointer()
                    .hover(move |s| s.bg(ink(0.08)))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .w(px(CHIP_W))
                            .h(px(CHIP_H))
                            .px(px(6.))
                            .rounded(px(4.))
                            .when_some(body.fill, |d, c| d.bg(c))
                            .when_some(body.border, |d, c| d.border_1().border_color(c))
                            .text_color(body.text)
                            .overflow_hidden()
                            .child(
                                svg()
                                    .path(logos::GITHUB_PATH)
                                    .size(px(LOGO))
                                    .flex_none()
                                    .text_color(body.text),
                            )
                            .child(row.label.clone())
                            // Review's tick, beside the number it signed off
                            // on, exactly as the cell draws it (#116).
                            .when(row.signed_off, |d| {
                                d.child(div().flex_none().child("\u{2713}"))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_size(px(10.))
                            .text_color(ink(if row.look.is_some() { 0.55 } else { 0.4 }))
                            .child(detail),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.pr_menu = None;
                        this.open_link(
                            label.clone(),
                            None,
                            real.then(|| url.clone()),
                            "pull request",
                            cx,
                        );
                    })),
            );
        }

        let card = div()
            .w(px(MAX_W.min(self.window.0 - 2.0 * EDGE).max(160.0)))
            .flex()
            .flex_col()
            .rounded_lg()
            .border_1()
            .border_color(ink(0.35))
            .bg(theme::hsla(card_bg(theme)))
            .shadow_lg()
            .font_family(theme::UI_FONT)
            .text_size(px(11.))
            .text_color(ink(0.85))
            // The board closes the list on any press that reaches it, so the
            // card has to keep its own presses to itself.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(list);

        Some(
            deferred(
                anchored()
                    .anchor(anchor)
                    .position(point(px(x), px(from)))
                    .snap_to_window_with_margin(Edges {
                        top: px(EDGE),
                        right: px(EDGE),
                        bottom: px(STATUS_H + EDGE),
                        left: px(EDGE),
                    })
                    .child(card),
            )
            .priority(1)
            .into_any_element(),
        )
    }
}
