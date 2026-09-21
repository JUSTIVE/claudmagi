//! ⌘K: one list over everything the board draws — session chips, the Linear
//! node at the head of a lane, the pull requests chained at its end — so a
//! name, an issue key or a PR number all lead to the same place a click on
//! that node would (#71).
//!
//! The scoring is the point. Letters may be scattered across a candidate, but
//! a run of them landing back to back is worth far more than the same letters
//! spread out: `RUN` is paid per letter *and* scaled by how deep into the run
//! that letter sits, so an unbroken run of `n` is worth `RUN * n * (n - 1) / 2`
//! on top of the base. Typing `pjm-20` puts the tickets that carry it whole
//! above anything that merely owns those six letters somewhere.

use gpui::{ClickEvent, Context, KeyDownEvent, MouseButton, SharedString, div, prelude::*, px};

use crate::model::{Phase, Target};
use crate::theme::{self, Palette, Rgba};
use crate::{pr, ticket};
use crate::ui::board::Board;

pub const PALETTE_W: f32 = 520.0;
/// How many matches the list shows at once.
const MAX_ROWS: usize = 9;

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// Paid for every letter that matched at all.
const MATCH: i32 = 10;
/// Paid per letter of an unbroken run, times how deep into the run it sits.
/// Large enough that a run outweighs the boundary bonus a scattered match
/// collects by landing on every word start.
const RUN: i32 = 60;
/// A match at the start of the candidate, or straight after a separator.
const BOUNDARY: i32 = 18;
/// Charged per letter skipped between two matches.
const GAP: i32 = 2;
/// Nothing is skipped more expensively than this, so a long candidate does not
/// lose to a short one on distance alone.
const MAX_SKIP: i32 = 10;

fn is_boundary(hay: &[char], j: usize) -> bool {
    j == 0 || matches!(hay[j - 1], '-' | '_' | '/' | '.' | ' ' | '#' | ':' | '~' | '@')
}

/// Best score for `query` against `hay`, or `None` when the query's letters do
/// not all appear in `hay` in order. An empty query matches everything at 0.
///
/// Every alignment is considered rather than the first one a greedy scan
/// finds: with the run bonus in play the last place a letter fits is often
/// worth more than the first, and the candidates here are a handful of short
/// strings.
pub fn score(query: &str, hay: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(0);
    }
    let h: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    if q.len() > h.len() {
        return None;
    }
    // `best[j]`: the score of the best match of the query so far that ends on
    // `h[j]`, and how many letters of the run ending there came before it.
    let mut best: Vec<Option<(i32, i32)>> = vec![None; h.len()];
    for (i, qc) in q.iter().enumerate() {
        let mut next: Vec<Option<(i32, i32)>> = vec![None; h.len()];
        for j in 0..h.len() {
            if h[j] != *qc {
                continue;
            }
            if i == 0 {
                // The first letter pays for how deep it sits, so the earlier
                // of two otherwise equal candidates wins.
                let lead = (j as i32).min(MAX_SKIP);
                next[j] = Some((MATCH + boundary_bonus(&h, j) - lead, 0));
                continue;
            }
            for k in 0..j {
                let Some((prev, run)) = best[k] else { continue };
                let cell = if k + 1 == j {
                    (prev + MATCH + RUN * (run + 1), run + 1)
                } else {
                    let skipped = ((j - k - 1) as i32).min(MAX_SKIP);
                    (prev + MATCH + boundary_bonus(&h, j) - GAP * skipped, 0)
                };
                if next[j].is_none_or(|(s, _)| cell.0 > s) {
                    next[j] = Some(cell);
                }
            }
        }
        best = next;
        if best.iter().all(Option::is_none) {
            return None;
        }
    }
    best.iter().filter_map(|cell| cell.map(|(s, _)| s)).max()
}

fn boundary_bonus(hay: &[char], j: usize) -> i32 {
    if is_boundary(hay, j) { BOUNDARY } else { 0 }
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// How opaque the card is over the board it covers.
const CARD_ALPHA: f32 = 0.97;
/// How far the card's background is lifted off the board's, towards the ink.
/// Enough to read as a card of its own against the board behind it, not so
/// much that it stops being the same material.
const CARD_LIFT: f32 = 0.07;

/// The card is the board's own surface lifted a little: background from
/// `bg`, text from `ink`, accents from `packet`, which is the same recipe the
/// status bar follows and the only one that reads on all three boards.
///
/// Not the chip colour, tempting as a chip-shaped card was: the white and
/// orange boards share `chip` and `text_on`, so a card built from those came
/// out identical on both and looked like neither (#72, #73).
fn card_bg(theme: Palette) -> Rgba {
    theme::with_alpha(theme::lerp(theme.bg, theme.ink, CARD_LIFT), CARD_ALPHA)
}

/// What the board would draw for this node. The row's dot copies it: filled
/// where the board fills and outlined where it outlines (#58), in the colour
/// it would wear out there. The themes have already solved reading these
/// against their own boards — the orange board's waiting chip is maroon
/// precisely because orange on orange vanishes — and the card is that board
/// lifted 7%, so nothing needs adjusting on the way in. (#76)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// A session chip in one of its phases.
    Chip(Phase),
    /// A pull request connector (#57).
    Pr(pr::Look),
    /// The Linear node at the head of a lane; `None` before Orca answers.
    Ticket(Option<ticket::Status>),
}

impl Style {
    /// `(filled, colour)`, from the same tables `scene` paints with.
    fn dot(self, theme: Palette) -> (bool, gpui::Hsla) {
        let at = theme::hsla;
        let ink = |a: f32| theme::hsla(theme::with_alpha(theme.ink, a));
        match self {
            Style::Chip(Phase::Working) => (true, at(theme.chip)),
            Style::Chip(Phase::NeedsUser) => (true, at(theme.chip_needs)),
            Style::Chip(Phase::Idle) => (true, at(theme.chip_idle)),
            Style::Pr(pr::Look::Draft) => (false, ink(0.8)),
            Style::Pr(pr::Look::Open) => (false, at(theme.text_on)),
            Style::Pr(pr::Look::Approved) => (true, at(theme.text_on)),
            Style::Pr(pr::Look::Failing) => (true, at(theme.alarm)),
            Style::Pr(pr::Look::Merged) => (true, at(theme.chip)),
            Style::Pr(pr::Look::Closed) => (false, ink(0.3)),
            Style::Ticket(Some(ticket::Status::Started)) => (true, at(theme.text_idle)),
            Style::Ticket(Some(ticket::Status::Done)) => (true, at(theme.chip)),
            Style::Ticket(Some(ticket::Status::Cancelled)) => (false, ink(0.3)),
            Style::Ticket(Some(ticket::Status::Todo)) => (false, ink(0.8)),
            Style::Ticket(None | Some(ticket::Status::Backlog)) => (false, ink(0.45)),
        }
    }
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Session,
    Ticket,
    Pr,
}

impl Kind {
    /// The node's shape on the board, as close as a glyph gets, hollow when
    /// the board would draw it as an outline.
    fn glyph(self, filled: bool) -> &'static str {
        match (self, filled) {
            (Kind::Session, true) => "▪",
            (Kind::Session, false) => "▫",
            (Kind::Ticket, true) => "◆",
            (Kind::Ticket, false) => "◇",
            (Kind::Pr, true) => "●",
            (Kind::Pr, false) => "○",
        }
    }
}

/// What picking a row does: the same thing clicking that node does.
#[derive(Clone)]
pub enum Action {
    /// Jump to the session's terminal.
    Session(usize),
    /// Open a link, preferring `first` and falling back to `then` (#60).
    Open { label: String, first: Option<String>, then: Option<String>, what: &'static str },
}

#[derive(Clone)]
pub struct Entry {
    pub kind: Kind,
    pub label: String,
    /// Where the node stands, said the way the board says it (#75, #76).
    pub style: Style,
    pub detail: String,
    pub action: Action,
    /// What the query is scored against. The label leads; the rest are worth
    /// less, so a query that hits a name beats one that only hits a path.
    fields: Vec<String>,
}

/// How much a field beyond the label is worth, in percent.
const SECONDARY: i32 = 65;

impl Entry {
    fn score(&self, query: &str) -> Option<i32> {
        let mut best: Option<i32> = None;
        for (i, field) in self.fields.iter().enumerate() {
            let Some(s) = score(query, field) else { continue };
            let s = if i == 0 { s } else { s * SECONDARY / 100 };
            if best.is_none_or(|b| s > b) {
                best = Some(s);
            }
        }
        best
    }
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    /// The rows on screen, rebuilt whenever the query or the board changes
    /// rather than every frame: scoring is cheap, but not 30 times a second
    /// for nothing.
    pub results: Vec<Entry>,
}

impl Board {
    pub(crate) fn toggle_palette(&mut self) {
        self.palette.open = !self.palette.open;
        self.palette.query.clear();
        self.palette.selected = 0;
        self.palette_refresh();
    }

    /// Rebuilds the rows. Cheap enough to call on every keystroke and every
    /// session poll, which between them cover everything that can change it.
    pub(crate) fn palette_refresh(&mut self) {
        if !self.palette.open {
            self.palette.results.clear();
            return;
        }
        let query = self.palette.query.clone();
        let mut scored: Vec<(i32, usize, Entry)> = self
            .palette_entries()
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| e.score(&query).map(|s| (s, i, e)))
            .collect();
        // Board order breaks ties, so an unfiltered list reads top to bottom
        // the way the lanes do.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.truncate(MAX_ROWS);
        self.palette.results = scored.into_iter().map(|(_, _, e)| e).collect();
        self.palette.selected = self.palette.selected.min(self.palette.results.len().saturating_sub(1));
    }

    /// Everything on the board, as rows: each session, then the ticket and the
    /// pull requests on its lane. A key or a number that two lanes share is
    /// listed once, under the first lane that carries it.
    fn palette_entries(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        let mut seen: Vec<(Kind, String)> = Vec::new();
        for (i, chip) in self.model.chips.iter().enumerate() {
            let info = &chip.info;
            let label = info.label();
            out.push(Entry {
                kind: Kind::Session,
                style: Style::Chip(info.phase()),
                detail: info.short_cwd(),
                action: Action::Session(i),
                fields: vec![label.clone(), info.short_cwd(), info.group_label()],
                label,
            });
            if let Some(t) = &info.ticket {
                let key = (Kind::Ticket, t.key.clone());
                if !seen.contains(&key) {
                    seen.push(key);
                    out.push(Entry {
                        kind: Kind::Ticket,
                        label: t.key.clone(),
                        style: Style::Ticket(t.status),
                        detail: t.title.clone(),
                        action: Action::Open {
                            label: t.key.clone(),
                            first: t.app_url(),
                            then: t.url(),
                            what: "Linear issue",
                        },
                        fields: vec![t.key.clone(), t.title.clone()],
                    });
                }
            }
            for pr in &info.prs {
                let label = pr.label();
                let key = (Kind::Pr, format!("{}#{}", pr.id.repo, pr.id.number));
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                out.push(Entry {
                    kind: Kind::Pr,
                    style: Style::Pr(pr.look()),
                    detail: format!("{} · {}", pr.id.repo, pr.title),
                    action: Action::Open {
                        label: label.clone(),
                        first: None,
                        then: Some(pr.url()),
                        what: "pull request",
                    },
                    // The bare number matters as much as `#1234`: nobody types
                    // the hash when they are after a PR.
                    fields: vec![label.clone(), pr.id.number.to_string(), pr.title.clone(), pr.id.repo.clone()],
                    label,
                });
            }
        }
        out
    }

    /// Keys while the palette is open. Everything lands here, so `t` types a
    /// `t` instead of opening the test panel.
    pub(crate) fn palette_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let mods = &ev.keystroke.modifiers;
        match key {
            "escape" => {
                self.palette.open = false;
                self.palette.results.clear();
                return;
            }
            "enter" => {
                self.palette_activate(cx);
                return;
            }
            "up" => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
                return;
            }
            "down" => {
                let last = self.palette.results.len().saturating_sub(1);
                self.palette.selected = (self.palette.selected + 1).min(last);
                return;
            }
            "backspace" if mods.platform || mods.alt => self.palette.query.clear(),
            "backspace" => {
                self.palette.query.pop();
            }
            _ => match ev.keystroke.key_char.as_deref() {
                // A modified key is a command, not typing; `key_char` is
                // already `None` for ⌘-anything, and control keys never read
                // as text.
                Some(text) if !mods.platform && !mods.control && !text.chars().any(char::is_control) => {
                    self.palette.query.push_str(text)
                }
                _ => return,
            },
        }
        self.palette.selected = 0;
        self.palette_refresh();
    }

    fn palette_activate(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.palette.results.get(self.palette.selected).cloned() else { return };
        self.palette.open = false;
        self.palette.results.clear();
        match entry.action {
            Action::Session(i) => self.activate(Target::Session(i), cx),
            Action::Open { label, first, then, what } => self.open_link(label, first, then, what, cx),
        }
    }

    pub(crate) fn render_palette(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.palette.query.clone();
        let selected = self.palette.selected;
        let rows: Vec<(usize, Entry)> = self.palette.results.iter().cloned().enumerate().collect();
        let empty = rows.is_empty();

        // `self.palette()` is the board's colours; `self.palette` is this
        // search. Bind the colours once so the rest reads unambiguously.
        let theme = self.palette();
        let (ink, card_rgba) = (theme.ink, card_bg(theme));
        let at = |a: f32| theme::hsla(theme::with_alpha(ink, a));
        let (accent, card, edge) = (theme::hsla(theme.packet), theme::hsla(card_rgba), at(0.35));
        let (rule, sel_bg, hover_bg) = (at(0.15), at(0.13), at(0.06));

        let mut list = div().flex().flex_col();
        for (i, entry) in rows {
            let on = i == selected;
            list = list.child(
                div()
                    .id(SharedString::from(format!("palette-row-{i}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .when(on, |d| d.bg(sel_bg))
                    .when(!on, |d| d.hover(move |s| s.bg(hover_bg)))
                    .child({
                        let (filled, colour) = entry.style.dot(theme);
                        div().w(px(12.)).flex_none().text_size(px(12.)).text_color(colour).child(entry.kind.glyph(filled))
                    })
                    .child(
                        div()
                            .w(px(132.))
                            .flex_none()
                            .overflow_hidden()
                            .text_color(at(if on { 1.0 } else { 0.85 }))
                            .child(entry.label.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_size(px(10.))
                            .text_color(at(0.55))
                            .child(entry.detail.clone()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.palette.selected = i;
                        this.palette_activate(cx);
                        cx.notify();
                    })),
            );
        }

        div()
            .absolute()
            .top(px(96.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(PALETTE_W))
                    .flex()
                    .flex_col()
                    .rounded_lg()
                    .border_1()
                    .border_color(edge)
                    .bg(card)
                    .shadow_lg()
                    .font_family(theme::UI_FONT)
                    .text_size(px(11.))
                    .text_color(at(0.85))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(rule)
                            .child(div().flex_none().text_color(accent).child("⌘K"))
                            .child(div().flex_1().child(format!("{query}▌")))
                            .when(!query.is_empty(), |d| {
                                d.child(div().flex_none().text_size(px(10.)).text_color(at(0.4)).child("⌫ 지우기"))
                            }),
                    )
                    .when(empty, |d| {
                        d.child(div().px_3().py_2().text_color(at(0.45)).child("찾는 노드가 없습니다"))
                    })
                    .when(!empty, |d| d.child(list))
                    .child(
                        div()
                            .px_3()
                            .py_1p5()
                            .border_t_1()
                            .border_color(rule)
                            .text_size(px(10.))
                            .text_color(at(0.45))
                            .child("↑↓ 이동 · ⏎ 열기 · esc 닫기"),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES: [Palette; 3] = [theme::PALETTE, theme::ORANGE, theme::DARK];

    fn luma(c: Rgba) -> f32 {
        0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
    }

    /// WCAG's ratio, which is what "readable" means here.
    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let (hi, lo) = if luma(a) > luma(b) { (luma(a), luma(b)) } else { (luma(b), luma(a)) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Every board has to get a card of its own, readable and distinct from
    /// the board behind it. Building the card from `chip` failed exactly here:
    /// the white and orange boards share that colour (#72).
    #[test]
    fn every_board_gets_its_own_readable_card() {
        // The ratio rather than a raw difference of brightnesses: the orange
        // board's card is mid-luma and its ink is dark, which reads fine and
        // no subtraction says so.
        let themes = THEMES;
        for t in themes {
            let card = card_bg(t);
            let ratio = contrast(card, t.ink);
            assert!(ratio > 4.5, "the board's ink has to read on its own card (got {ratio:.1}:1)");
            assert!(
                (luma(card) - luma(t.bg)).abs() > 0.005,
                "the card has to lift off the board behind it"
            );
        }
        let cards: Vec<Rgba> = themes.iter().map(|t| card_bg(*t)).collect();
        for (i, a) in cards.iter().enumerate() {
            for b in &cards[i + 1..] {
                let apart = (a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs();
                assert!(apart > 0.05, "no two boards may hand out the same card");
            }
        }
    }

    /// The board's one law about connectors: filled is always further along
    /// than outlined (#58). The row's dot repeats it, so a glance down the
    /// list sorts landed from pending without reading a word (#76).
    #[test]
    fn a_filled_dot_always_means_further_along() {
        let filled = |s: Style| s.dot(theme::PALETTE).0;
        for done in [pr::Look::Approved, pr::Look::Failing, pr::Look::Merged] {
            assert!(filled(Style::Pr(done)), "{done:?} is filled on the board");
        }
        for pending in [pr::Look::Draft, pr::Look::Open, pr::Look::Closed] {
            assert!(!filled(Style::Pr(pending)), "{pending:?} is an outline on the board");
        }
        assert!(filled(Style::Ticket(Some(ticket::Status::Started))));
        assert!(filled(Style::Ticket(Some(ticket::Status::Done))));
        for early in [ticket::Status::Backlog, ticket::Status::Todo, ticket::Status::Cancelled] {
            assert!(!filled(Style::Ticket(Some(early))), "{early:?} is an outline on the board");
        }
        assert!(!filled(Style::Ticket(None)), "an issue Orca has not answered for yet");
        for phase in Phase::ALL {
            assert!(filled(Style::Chip(phase)), "a chip is a solid thing in every phase");
        }
    }

    /// Every phase has to be told apart by the dot alone, since the word that
    /// used to say so is gone (#76).
    #[test]
    fn each_phase_gets_its_own_chip_colour() {
        for t in THEMES {
            let colours: Vec<gpui::Hsla> = Phase::ALL.iter().map(|p| Style::Chip(*p).dot(t).1).collect();
            for (i, a) in colours.iter().enumerate() {
                for b in &colours[i + 1..] {
                    assert_ne!(a, b, "two phases would draw the same dot");
                }
            }
        }
    }

    #[test]
    fn an_unbroken_run_beats_the_same_letters_scattered() {
        let run = score("pjm20", "PJM-2041").unwrap();
        let scattered = score("pjm20", "p j m 2 0").unwrap();
        assert!(run > scattered * 2, "run {run} should dwarf scattered {scattered}");
    }

    #[test]
    fn the_longer_the_run_the_bigger_the_lead() {
        let short = score("pj", "PJM-2041").unwrap() - score("pj", "p-j").unwrap();
        let long = score("pjm20", "PJM-2041").unwrap() - score("pjm20", "p-j-m-2-0").unwrap();
        assert!(long > short, "a five letter run ({long}) leads by more than a two letter one ({short})");
    }

    #[test]
    fn a_query_that_is_not_in_the_candidate_does_not_match() {
        assert!(score("zzz", "PJM-2041").is_none());
        assert!(score("2041x", "PJM-2041").is_none());
        assert!(score("longer than the hay", "hay").is_none());
    }

    #[test]
    fn matching_is_case_insensitive_and_an_empty_query_takes_everything() {
        assert!(score("pjm", "PJM-2041").is_some());
        assert!(score("PJM", "pjm-2041").is_some());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn a_hit_at_a_boundary_beats_one_buried_mid_word() {
        let boundary = score("crepe", "~/git/cookieplace/crepe").unwrap();
        let buried = score("crepe", "~/git/cookieplacecrepe").unwrap();
        assert!(boundary > buried, "boundary {boundary} should beat buried {buried}");
    }

    #[test]
    fn an_earlier_hit_wins_between_otherwise_equal_candidates() {
        let early = score("pjm", "PJM-2041").unwrap();
        let late = score("pjm", "yoshi-dark PJM-2041").unwrap();
        assert!(early > late, "early {early} should beat late {late}");
    }

    #[test]
    fn a_pr_is_found_by_its_bare_number() {
        assert!(score("8792", "8792").is_some());
        let whole = score("8792", "8792").unwrap();
        let partial = score("8792", "8_7_9_2").unwrap();
        assert!(whole > partial);
    }
}
