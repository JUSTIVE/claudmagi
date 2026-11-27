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

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use nsh_metro::{FlyoutWidth, label, palette as metro_palette};

use crate::logos::{self, Mark};
use crate::model::{Phase, Target};
use crate::theme::{self, Palette, Rgba};
use crate::ui::panel::{Which, flyout};
use crate::ui::state::{Board, Ui};
use crate::{pr, ticket};

/// How tall one row comes out, for working out how many of them fit.
const ROW_H: f32 = 26.0;
/// The query line over the rows and the key hints under them.
const CHROME_H: f32 = 62.0;
/// The accent header gpui-metro puts on a flyout.
const HEADER_H: f32 = 80.0;

/// How many rows a window `h` tall has room for (#193).
///
/// The search is a panel now, so nothing about it is centred on the board
/// and there is no card to keep off the window's edges: it runs the full
/// height down the right, and what bounds the list is the window itself —
/// the flyout's header, the query line, the hints and the status bar.
fn rows_that_fit(h: f32) -> usize {
    let room = h - HEADER_H - CHROME_H - crate::ui::state::STATUS_H;
    ((room / ROW_H).floor().max(1.0) as usize).min(MAX_ROWS)
}
/// One size for every chip in the list, whatever it stands for (#79). On the
/// board a connector is smaller than a session chip because it has less room;
/// in a list that difference only reads as noise, and the mark inside says
/// what the node is far better than its height did.
const CHIP_W: f32 = 168.0;
/// The column a tagalong row's step stands in.
const STEP: f32 = 9.0;
/// The tick beside a signed-off number, in px.
const TICK: f32 = 11.0;
const CHIP_H: f32 = 20.0;
const LOGO: f32 = 11.0;
/// How many matches the search gathers at once. A card could not have shown
/// many more without running off the board; a full-height panel can (#193).
const MAX_ROWS: usize = 20;

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
pub(crate) fn card_bg(theme: Palette) -> Rgba {
    theme::with_alpha(theme::lerp(theme.bg, theme.ink, CARD_LIFT), CARD_ALPHA)
}

/// What the board would draw for this node, so the row can draw the same
/// thing. The colours come from `scene`'s own tables, untouched: the themes
/// have already solved reading these against their own boards — the orange
/// board's waiting chip is maroon precisely because orange on orange vanishes
/// — and the card is that board lifted 7%. (#76, #78)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// A session chip in one of its phases.
    Chip(Phase),
    /// A pull request connector, whether CI is still going (#57, #62), and
    /// whether review has signed off (#116).
    Pr { look: pr::Look, running: bool, signed_off: bool },
    /// The Linear node at the head of a lane; `None` before Orca answers.
    Ticket(Option<ticket::Status>),
    /// A page the session published (#137), in the teal that is its own on
    /// the board (#238).
    Artifact,
    /// A Linear notification still asking (#241). It wears the colour the
    /// board already keeps for "a person is wanted" rather than inventing
    /// one: that is exactly what it says, and the Linear mark inside the
    /// chip says whose asking it is (#79).
    Inbox,
}

/// A node's body: fill, outline, and the label that goes on it. `None` where
/// the board leaves it off, which is how filled reads as further along than
/// outlined (#58).
pub struct Body {
    pub fill: Option<Hsla>,
    pub border: Option<Hsla>,
    pub text: Hsla,
}

impl Style {
    pub(crate) fn body(self, theme: Palette) -> Body {
        let at = theme::hsla;
        let ink = |a: f32| theme::hsla(theme::with_alpha(theme.ink, a));
        let body = |fill, border, text| Body { fill, border, text };
        match self {
            // A plugged-in chip wears the board's chip colour; one that has
            // pulled out of its socket wears the colour of why it did.
            Style::Chip(Phase::Working) => body(Some(at(theme.chip)), None, at(theme.text_on)),
            Style::Chip(Phase::NeedsUser) => body(Some(at(theme.chip_needs)), None, at(theme.text_needs)),
            Style::Chip(Phase::Idle) => body(Some(at(theme.chip_idle)), None, at(theme.text_idle)),
            Style::Pr { look, running, .. } => {
                let mut out = match look {
                    pr::Look::Draft => body(None, Some(ink(0.8)), ink(0.85)),
                    pr::Look::Open => body(None, Some(at(theme.text_on)), ink(0.85)),
                    pr::Look::Failing => body(Some(at(theme.alarm)), None, at(theme.on_alarm)),
                    pr::Look::Merging | pr::Look::Merged => body(Some(at(theme.merged)), None, at(theme.on_merged)),
                    pr::Look::Closed => body(None, Some(ink(0.3)), ink(0.42)),
                };
                // CI in flight is a border laid over whatever it already is,
                // never a look of its own (#62).
                if running {
                    out.border = Some(at(theme.busy));
                }
                out
            }
            Style::Ticket(Some(ticket::Status::Started)) => {
                body(Some(at(theme.text_idle)), None, at(theme.ink))
            }
            Style::Ticket(Some(ticket::Status::Done)) => body(Some(at(theme.chip)), None, at(theme.bg)),
            Style::Ticket(Some(ticket::Status::Cancelled)) => body(None, Some(ink(0.3)), ink(0.42)),
            Style::Ticket(Some(ticket::Status::Todo)) => body(Some(ink(0.10)), Some(ink(0.8)), ink(0.85)),
            Style::Ticket(None | Some(ticket::Status::Backlog)) => {
                body(Some(ink(0.10)), Some(ink(0.45)), ink(0.6))
            }
            Style::Inbox => body(Some(at(theme.chip_needs)), None, at(theme.text_needs)),
            Style::Artifact => {
                let fill = crate::ink::floor_color(crate::ink::artifact_color(theme), theme);
                let white = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
                body(Some(at(fill)), None, at(crate::ink::legible(white, fill, theme)))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Session,
    Ticket,
    Pr,
    Artifact,
    /// A Linear notification (#241): not a node on the board at all, which
    /// is the point of it being here.
    Inbox,
    /// A pull request waiting on your review (#257). It sits in the inbox
    /// beside the notifications and wears the same colour, but the mark
    /// says GitHub, because that is whose thing it is (#79) and where
    /// pressing it goes.
    Review,
}

impl Kind {
    /// The mark that goes inside the chip: whose thing this is (#79).
    fn logo(self) -> Mark {
        match self {
            Kind::Session => Mark::Claude,
            Kind::Ticket => Mark::Linear,
            Kind::Pr => Mark::GitHub,
            Kind::Artifact => Mark::Artifact,
            Kind::Inbox => Mark::Linear,
            Kind::Review => Mark::GitHub,
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
    /// The session this belongs to. A PR number is often all anyone
    /// remembers, and finding it should bring its issue and its session along
    /// (#85).
    pub owner: usize,
    /// True when this row came along with a match rather than matching
    /// itself.
    pub related: bool,
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
    /// Review has signed off on this row's pull request (#116).
    fn signed_off(&self) -> bool {
        matches!(self.style, Style::Pr { signed_off: true, .. })
    }

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

/// The rows a query earns, best lane first.
///
/// A hit brings its lane with it: a pull request number is often all anybody
/// remembers, and from there the issue and the session are what was actually
/// wanted (#85). Lanes are ranked by their best hit, and inside a lane the hit
/// leads with the rest stepped in behind it. Board order breaks ties, so an
/// empty query reads top to bottom the way the lanes do.
fn rank(entries: Vec<Entry>, query: &str) -> Vec<Entry> {
    let scores: Vec<Option<i32>> = entries.iter().map(|e| e.score(query)).collect();
    let mut lanes: Vec<(i32, usize, usize)> = Vec::new(); // best score, anchor, owner
    for (i, (e, s)) in entries.iter().zip(&scores).enumerate() {
        let Some(s) = *s else { continue };
        match lanes.iter_mut().find(|(_, _, owner)| *owner == e.owner) {
            Some(lane) if s > lane.0 => *lane = (s, i, e.owner),
            Some(_) => {}
            None => lanes.push((s, i, e.owner)),
        }
    }
    lanes.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut out: Vec<Entry> = Vec::new();
    for (_, anchor, owner) in lanes {
        if out.len() >= MAX_ROWS {
            break;
        }
        out.push(entries[anchor].clone());
        for (i, e) in entries.iter().enumerate() {
            if i == anchor || e.owner != owner || out.len() >= MAX_ROWS {
                continue;
            }
            let mut e = e.clone();
            // Only what came along for the ride is stepped in; something that
            // matched on its own is a hit like any other.
            e.related = scores[i].is_none();
            out.push(e);
        }
    }
    out
}

/// Opens or closes the search, through the panels: opening it puts away
/// whatever panel was up and opening one of those puts it away (#163, #193).
pub fn toggle(b: &mut Board) {
    crate::ui::panel::toggle(b, Which::Search);
    b.palette.query.clear();
    b.palette.selected = 0;
    refresh(b);
}

/// Rebuilds the rows. Cheap enough to call on every keystroke and every
/// session poll, which between them cover everything that can change it.
pub fn refresh(b: &mut Board) {
    if !b.palette.open {
        b.palette.results.clear();
        return;
    }
    let query = b.palette.query.clone();
    b.palette.results = rank(entries(b), &query);
    b.palette.selected = b.palette.selected.min(b.palette.results.len().saturating_sub(1));
}

/// Everything on the board, as rows: each session, then the ticket and the
/// pull requests on its lane. A key or a number that two lanes share is
/// listed once, under the first lane that carries it.
fn entries(b: &Board) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut seen: Vec<(Kind, String)> = Vec::new();
    // What Linear is still asking of you leads the list (#241). It is the
    // one thing in the panel that is not already drawn on the board behind
    // it, and the board fills twenty rows on its own: put the inbox under
    // the lanes and an empty query would never reach it.
    for (i, note) in b.inbox.notes.iter().enumerate() {
        let label = note.label();
        let web = (!note.url.is_empty()).then(|| note.url.clone());
        out.push(Entry {
            kind: if crate::inbox::about_a_pull_request(&note.kind) { Kind::Review } else { Kind::Inbox },
            style: Style::Inbox,
            // A lane of its own. Most of these are about work no session
            // here is doing, and one that does name a lane should not drag
            // that lane's rows along behind it (#85) — the inbox is a list
            // of what is being asked, not another way into the board.
            owner: usize::MAX - i,
            related: false,
            detail: note.detail(),
            action: Action::Open {
                label: label.clone(),
                first: web.as_deref().and_then(crate::inbox::app_url),
                then: web,
                what: "Linear notification",
            },
            fields: vec![label.clone(), note.title.clone(), note.body.clone(), note.actor.clone()],
            label,
        });
    }
    for (i, chip) in b.model.chips.iter().enumerate() {
        let info = &chip.info;
        let label = info.label();
        out.push(Entry {
            kind: Kind::Session,
            style: Style::Chip(info.phase()),
                owner: i,
                related: false,
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
                    owner: i,
                    related: false,
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
                style: Style::Pr { look: pr.look(), running: pr.running(), signed_off: pr.signed_off() },
                owner: i,
                related: false,
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
        // And the pages it published, which the cell shows on the same
        // line as the pull request and are otherwise only reachable by
        // dropping that line's list (#137). Searching by name is the
        // whole point here: a page is named after the work, not numbered.
        for a in &info.artifacts {
            let key = (Kind::Artifact, a.id.clone());
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let label = a.label();
            out.push(Entry {
                kind: Kind::Artifact,
                style: Style::Artifact,
                owner: i,
                related: false,
                detail: a.version_label().map(|v| format!("artifact · {v}")).unwrap_or("artifact".into()),
                action: Action::Open {
                    label: label.clone(),
                    first: None,
                    then: Some(a.url()),
                    what: crate::made::ARTIFACT,
                },
                fields: vec![label.clone(), a.id.clone()],
                label,
            });
        }
    }
    out
}

/// Keys while the search is open. Everything lands here, so `t` types a `t`
/// instead of opening the test panel.
pub fn key(ui: &Ui, ev: &KeyEvent) {
    let mods = &ev.modifiers;
    let key = ev.key.as_ref();
    let picked = ui.update(|b| {
        match key {
            "Escape" => {
                b.palette.open = false;
                b.palette.results.clear();
                return None;
            }
            "Enter" => return pick(b),
            "ArrowUp" => {
                b.palette.selected = b.palette.selected.saturating_sub(1);
                return None;
            }
            "ArrowDown" => {
                let last = b.palette.results.len().saturating_sub(1);
                b.palette.selected = (b.palette.selected + 1).min(last);
                return None;
            }
            "Backspace" if mods.cmd || mods.alt => b.palette.query.clear(),
            "Backspace" => {
                b.palette.query.pop();
            }
            // A modified key is a command, not typing, and named keys never
            // read as text.
            text if !mods.cmd && !mods.ctrl && text.chars().count() == 1 && !text.chars().any(char::is_control) => {
                b.palette.query.push_str(text)
            }
            _ => return None,
        }
        b.palette.selected = 0;
        refresh(b);
        None
    });
    if let Some(action) = picked {
        run(ui, action);
    }
}

/// Takes the selected row and puts the search away.
fn pick(b: &mut Board) -> Option<Action> {
    let entry = b.palette.results.get(b.palette.selected).cloned()?;
    b.palette.open = false;
    b.palette.results.clear();
    Some(entry.action)
}

fn run(ui: &Ui, action: Action) {
    match action {
        Action::Session(i) => ui.activate(Target::Session(i)),
        Action::Open { label, first, then, what } => ui.open_link(label, first, then, what),
    }
}

/// The search, as the panel it is since #193: the flyout down the right
/// edge the settings and the test tools wear, at the wider of Metro's two
/// widths because a row here is a node and a line about it.
pub fn panel(ui: &Ui) -> AnyElement {
    flyout(ui, Which::Search, FlyoutWidth::Wide, |_| "search".into(), |ui| {
        let ui = ui.clone();
        dynamic(move || body(&ui))
    })
}

fn body(ui: &Ui) -> AnyElement {
    let (fits, query, selected, results, theme, orange) = ui.read(|b| {
        (
            rows_that_fit(b.window.1),
            b.palette.query.clone(),
            b.palette.selected,
            b.palette.results.clone(),
            b.palette(),
            b.settings.theme == crate::settings::BoardTheme::Orange,
        )
    });
    // Only as many rows as the window has room for, keeping the selected
    // one among them.
    let first = selected.saturating_sub(fits.saturating_sub(1));
    let rows: Vec<(usize, Entry)> = results.into_iter().enumerate().skip(first).take(fits).collect();
    let empty = rows.is_empty();
    let ink = theme.ink;
    let at = move |a: f32| theme::hsla(theme::with_alpha(ink, a));
    // The rows keep the board's colours, so a node reads here the way it
    // reads there (#78); the sheet under them is the panel's.
    let accent = if orange { theme::hsla(theme.packet) } else { untracked(metro_palette).accent };
    let (rule, sel_bg, hover_bg) = (at(0.15), at(0.13), at(0.06));

    let mut list = view();
    for (i, entry) in rows {
        let on = i == selected;
        let body = entry.style.body(theme);
        let go = ui.clone();
        let mut chip = view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .w(px(CHIP_W))
            .h(px(CHIP_H))
            .px(px(7.))
            // Square: a Metro tile has no corner radius, and the row is a
            // small one (#228).
            .overflow_hidden()
            .child(svg(ImageSrc::svg_markup(logos::markup(entry.kind.logo()))).size(px(LOGO)).flex_shrink_0().text_color(body.text))
            .child(label(entry.label.clone()).text_size(px(11.)).truncate().text_color(body.text))
            // Review's tick, beside the number it signed off on (#116),
            // the same one the board draws (#221, #228).
            .children(entry.signed_off().then(|| {
                svg(ImageSrc::svg_markup(crate::ui::metro::TICK_SVG))
                    .size(px(TICK))
                    .flex_shrink_0()
                    .text_color(body.text)
            }));
        if let Some(c) = body.fill {
            chip = chip.bg(c);
        }
        if let Some(c) = body.border {
            chip = chip.border(px(1.)).border_color(c);
        }
        let row = view()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(3.))
            .cursor_pointer()
            .border_l(px(2.))
            .border_color(if on { accent } else { Hsla::transparent_black() })
            .bg(if on { sel_bg } else { Hsla::transparent_black() })
            .on_press(move || {
                let picked = go.update(|b| {
                    b.palette.selected = i;
                    pick(b)
                });
                if let Some(action) = picked {
                    run(&go, action);
                }
            })
            // A row that came along with a hit is stepped in behind it, so
            // the thing actually searched for still leads (#85).
            // A row that came along with a hit is stepped in behind it by
            // a bar in the board's ink rather than by an arrow glyph (#228).
            .child(
                view().w(px(STEP)).h(px(CHIP_H)).flex_shrink_0().items_center().justify_center().children(
                    entry
                        .related
                        .then(|| view().w(px(2.)).h(px(CHIP_H - 8.)).bg(at(0.25))),
                ),
            )
            // The row *is* the node: the same body the board would give it
            // (#78), with the mark of the service it belongs to (#79).
            .child(chip)
            .child(
                label(entry.detail.clone())
                    .flex_1()
                    .text_size(px(10.))
                    .truncate()
                    .text_color(at(if entry.related { 0.4 } else { 0.55 })),
            );
        let row = if on { row } else { row.hover(move |s| s.bg(hover_bg)) };
        list = list.child(row);
    }

    // The query line reads as a field: the key that opened the panel, what
    // has been typed, and the way back out of it.
    let field = view()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .border_b(px(1.))
        .border_color(rule)
        .child(label("⌘k").flex_shrink_0().text_size(px(11.)).text_color(accent))
        .child(label(format!("{query}▌")).flex_1().text_size(px(11.)).text_color(at(0.85)))
        .children((!query.is_empty()).then(|| label("⌫ 지우기").flex_shrink_0().text_size(px(10.)).text_color(at(0.4))));

    view()
        .child(field)
        .child(if empty {
            view().px(px(12.)).py(px(8.)).child(label("찾는 노드가 없습니다").text_size(px(11.)).text_color(at(0.45)))
        } else {
            list
        })
        .child(
            view()
                .px(px(12.))
                .py(px(6.))
                .border_t(px(1.))
                .border_color(rule)
                .child(label("↑↓ 이동 · ⏎ 열기 · esc 닫기").text_size(px(10.)).text_color(at(0.45))),
        )
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES: [Palette; 3] = [theme::PALETTE, theme::ORANGE, theme::DARK];

    fn pr_style(look: pr::Look, running: bool) -> Style {
        Style::Pr { look, running, signed_off: false }
    }

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
    /// than outlined (#58). A row is the node, so the law holds there too and
    /// a glance down the list sorts landed from pending (#76, #78).
    #[test]
    fn a_filled_body_always_means_further_along() {
        let filled = |s: Style| s.body(theme::PALETTE).fill.is_some();
        for done in [pr::Look::Failing, pr::Look::Merged] {
            assert!(filled(pr_style(done, false)), "{done:?} is filled on the board");
        }
        for pending in [pr::Look::Draft, pr::Look::Open, pr::Look::Closed] {
            assert!(!filled(pr_style(pending, false)), "{pending:?} is an outline on the board");
        }
        assert!(filled(Style::Ticket(Some(ticket::Status::Started))));
        assert!(filled(Style::Ticket(Some(ticket::Status::Done))));
        assert!(!filled(Style::Ticket(Some(ticket::Status::Cancelled))));
        for phase in Phase::ALL {
            assert!(filled(Style::Chip(phase)), "a chip is a solid thing in every phase");
        }
    }

    /// Approval is a tick laid over whatever the pull request is, never a
    /// body of its own (#116). The body answers to GitHub and to the checks;
    /// approval does not touch it at any look, in any theme.
    #[test]
    fn approval_never_changes_the_body_it_rides_on() {
        for t in THEMES {
            for look in pr::Look::ALL {
                let bare = Style::Pr { look, running: false, signed_off: false };
                let ok = Style::Pr { look, running: false, signed_off: true };
                let (a, b) = (bare.body(t), ok.body(t));
                assert_eq!(
                    (a.fill, a.border, a.text),
                    (b.fill, b.border, b.text),
                    "{look:?}: a sign-off repainted the connector"
                );
            }
        }
        // Which is what lets the two be read at once. Every one of these used
        // to cost the other: a single enum had to pick one and picked wrong.
        use crate::pr::{Look, Pr, State};
        let failing = Pr { approved: true, ..Pr::synthetic(7, Look::Failing) };
        assert_eq!(failing.look(), Look::Failing, "the checks are what the colour says");
        assert!(failing.signed_off(), "and the tick still says review signed off");
        assert!(Pr::synthetic(7, Look::Merged).signed_off(), "nothing lands without a sign-off");
        let draft = Pr { approved: true, ..Pr::synthetic(7, Look::Draft) };
        assert!(draft.signed_off() && draft.look() == Look::Draft);
        assert_eq!(Pr::synthetic(7, Look::Merged).state, State::Merged);
    }

    /// Running CI is a border over whatever the pull request already is, not
    /// a look of its own (#62).
    #[test]
    fn ci_in_flight_borders_a_body_without_replacing_it() {
        for look in pr::Look::ALL {
            let (calm, busy) =
                (pr_style(look, false).body(theme::PALETTE), pr_style(look, true).body(theme::PALETTE));
            assert_eq!(calm.fill, busy.fill, "{look:?} keeps its body while CI runs");
            assert_eq!(busy.border, Some(theme::hsla(theme::PALETTE.busy)), "{look:?} takes the border");
        }
    }

    /// Every phase has to be told apart by the chip alone, since the word
    /// that used to say so is gone (#76).
    #[test]
    fn each_phase_gets_its_own_chip_colours() {
        for t in THEMES {
            let bodies: Vec<(Option<Hsla>, Hsla)> =
                Phase::ALL.iter().map(|p| Style::Chip(*p).body(t)).map(|b| (b.fill, b.text)).collect();
            for (i, a) in bodies.iter().enumerate() {
                for b in &bodies[i + 1..] {
                    assert_ne!(a, b, "two phases would draw the same chip");
                }
            }
        }
    }

    /// A label has to read on the body it is printed on, and the board's own
    /// pairings are the ones being copied, so this catches a theme that
    /// pairs a colour with itself rather than any judgement about taste.
    #[test]
    fn a_label_never_lands_on_its_own_colour() {
        for t in THEMES {
            let styles = [
                Style::Chip(Phase::Working),
                Style::Chip(Phase::NeedsUser),
                Style::Chip(Phase::Idle),
                pr_style(pr::Look::Merged, false),
                pr_style(pr::Look::Failing, false),
                Style::Ticket(Some(ticket::Status::Started)),
                Style::Ticket(Some(ticket::Status::Done)),
                Style::Artifact,
                Style::Inbox,
            ];
            for s in styles {
                let body = s.body(t);
                if let Some(fill) = body.fill {
                    assert_ne!(fill, body.text, "a label would vanish into its own body");
                }
            }
        }
    }

    /// The inbox leads the list and stands alone (#241). The board fills
    /// the panel by itself, so rows put in behind the lanes would never be
    /// reached on an empty query; and a notification is not another way
    /// into a lane, so it brings none along behind it (#85).
    #[test]
    fn what_linear_is_asking_leads_the_list_and_stands_alone() {
        use crate::model::{Phase, SessionInfo};
        let mut b = crate::ui::state::Board::new();
        let many: Vec<SessionInfo> =
            (0..MAX_ROWS as u32 + 4).map(|i| SessionInfo::synthetic(i, &format!("S-{i}"), Phase::Working)).collect();
        b.take(many, None);
        b.inbox.notes = vec![crate::inbox::Note {
            kind: "issueCommentMention".into(),
            actor: "Z".into(),
            key: Some("PJM-3292".into()),
            title: "키보드가 덮은 입력칸을 키보드 위로 스크롤합니다".into(),
            url: "https://linear.app/cookieplace/issue/PJM-3292/yo#comment-1".into(),
            ..Default::default()
        }];

        let rows = rank(entries(&b), "");
        assert_eq!(rows.len(), MAX_ROWS, "the board alone is enough to fill the list");
        assert_eq!(rows[0].kind, Kind::Inbox, "what is being asked of you leads");
        assert_eq!(rows[0].label, "PJM-3292", "the key is what the chip carries");
        assert!(rows[1..].iter().all(|r| r.owner != rows[0].owner), "it drags no lane behind it");
        assert!(rows[1..].iter().all(|r| !r.related), "and nothing rides in as its tagalong");
    }

    /// The panel runs the window's full height, so the rows it shows have to
    /// stop above the status bar however short the window is — and a search
    /// with no rows is no search (#83, #193).
    #[test]
    fn the_rows_stop_before_the_window_does() {
        for h in [240.0, 300.0, 420.0, 620.0, 1400.0] {
            let rows = rows_that_fit(h);
            assert!(rows >= 1, "{h}: a search with no rows is no search");
            let used = HEADER_H + CHROME_H + rows as f32 * ROW_H;
            assert!(
                used <= h - crate::ui::state::STATUS_H + 0.01 || rows == 1,
                "{h}: the rows run {used} into a window of {h}"
            );
        }
        assert!(rows_that_fit(2400.0) <= MAX_ROWS, "it never shows more than the search gathers");
        assert!(
            rows_that_fit(1400.0) > rows_that_fit(420.0),
            "a taller window is the whole reason for making this a panel"
        );
    }

    /// The rows used to sit on a card the palette painted itself, out of the
    /// board's own colours. The panel's sheet is Metro's, so what has to be
    /// checked now is that the board's ink still reads on it — on every
    /// board, since the Metro mode follows the board's (#193).
    #[test]
    fn the_boards_ink_reads_on_the_panels_sheet() {
        use crate::settings::BoardTheme;
        use nsh_metro::Accent;
        for (board, theme) in [(BoardTheme::White, theme::PALETTE), (BoardTheme::Orange, theme::ORANGE), (BoardTheme::Dark, theme::DARK)] {
            let sheet = nsh_metro::Palette::resolve(crate::ui::metro::mode_of(board), Accent::Orange).background;
            let sheet = Rgba::from(sheet);
            let ratio = contrast(theme.ink, sheet);
            assert!(ratio > 4.5, "{board:?}: the board's ink has to read on the panel (got {ratio:.1}:1)");
        }
    }

    fn entry(owner: usize, kind: Kind, label: &str, fields: &[&str]) -> Entry {
        Entry {
            kind,
            owner,
            related: false,
            label: label.into(),
            style: Style::Chip(Phase::Working),
            detail: String::new(),
            action: Action::Session(owner),
            fields: fields.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// One lane's worth of board: the session, its issue, its pull request.
    fn lane(owner: usize, name: &str, key: &str, pr: &str) -> Vec<Entry> {
        vec![
            entry(owner, Kind::Session, name, &[name]),
            entry(owner, Kind::Ticket, key, &[key]),
            entry(owner, Kind::Pr, &format!("#{pr}"), &[&format!("#{pr}"), pr]),
        ]
    }

    /// The ask behind #85: a number is all anybody remembers, and what they
    /// wanted was the work around it.
    #[test]
    fn a_hit_brings_its_lane_along() {
        let mut all = lane(0, "PJM-2033", "PJM-2033", "9052");
        all.extend(lane(1, "CREPE-7F", "PJM-2042", "8760"));
        let out = rank(all, "9052");

        assert_eq!(out.len(), 3, "only the lane that owns 9052");
        assert_eq!(out[0].label, "#9052", "the hit leads");
        assert!(!out[0].related);
        let rest: Vec<&str> = out[1..].iter().map(|e| e.label.as_str()).collect();
        assert_eq!(rest, vec!["PJM-2033", "PJM-2033"], "its session and its issue follow");
        assert!(out[1..].iter().all(|e| e.related), "they came along rather than matched");
    }

    #[test]
    fn the_best_member_of_a_lane_is_the_one_that_leads_it() {
        let out = rank(lane(0, "PJM-2033", "PJM-2033", "9052"), "pjm-2033");
        assert_eq!(out[0].label, "PJM-2033");
        assert!(!out[0].related && !out[1].related, "both the session and the issue matched on their own");
        assert!(out.iter().any(|e| e.label == "#9052" && e.related), "the pull request rides along");
    }

    #[test]
    fn a_lane_nothing_matched_in_stays_out() {
        let mut all = lane(0, "PJM-2033", "PJM-2033", "9052");
        all.extend(lane(1, "CREPE-7F", "PJM-2042", "8760"));
        let out = rank(all, "8760");
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|e| e.owner == 1), "the other lane is not on screen at all");
    }

    #[test]
    fn an_empty_query_keeps_the_board_order_and_marks_nothing_as_a_tagalong() {
        let mut all = lane(0, "PJM-2033", "PJM-2033", "9052");
        all.extend(lane(1, "CREPE-7F", "PJM-2042", "8760"));
        let out = rank(all, "");
        assert_eq!(out.len(), 6);
        assert!(out.iter().all(|e| !e.related));
        let owners: Vec<usize> = out.iter().map(|e| e.owner).collect();
        assert_eq!(owners, vec![0, 0, 0, 1, 1, 1], "lanes stay together, in board order");
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
