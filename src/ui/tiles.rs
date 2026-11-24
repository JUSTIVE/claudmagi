//! The board as a Metro start screen (#157), built from nsh-metro's own
//! `tile_grid` and live `tile` (#176, #195).
//!
//! The panels around the board speak Windows Phone Metro (#156); this is the
//! board in the same language, drawn by nsh-metro: the turnstile, the live
//! tiles' peek, the tilt under the pointer and the packing are all its own,
//! and all of it is the platform's animation, not frames the app draws.
//!
//! Tiles are grouped by the *work*, not by the window (#162): everything on
//! one Linear issue sits in that issue's group — failing an issue, one pull
//! request, and failing that, one directory — whichever Warp tab its
//! sessions run in. A group reads as the work does: the issue, the sessions
//! as live tiles (with their subagents as small tiles after them), and what
//! the work produced, a tile per pull request or page.

use std::rc::Rc;

use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use nsh_metro::theme::{LIGHT, SEMILIGHT, size};
use nsh_metro::{Tile, TileSize, label, palette as metro_palette, tile, tile_grid};

use crate::ink;
use crate::logos::Mark;
use crate::model::{BoardModel, ChipState, Phase, SessionInfo};
use crate::theme::{self, Palette, Rgba};
use crate::ui::state::{Board, STATUS_H, TileMenu, Ui};

/// A small tile's side and the gutter after it, in design px: a session is
/// a medium tile, two of them and the gutter a side.
const UNIT: f32 = 64.0;
const GAP: f32 = 8.0;
/// A group is one medium tile wide, or two when any group has more than
/// two tiles' worth in it (`columns`).
const NARROW: usize = 2;
const WIDE: usize = 4;
/// Kept clear around the screen, and under the traffic lights.
const MARGIN: f32 = 24.0;
const TOP: f32 = 22.0;
/// Between one row of groups and the next.
const ROW_GAP: f32 = 28.0;
/// A live tile's cycle, gpui-metro's own default.
const PERIOD: f32 = 12.0;
/// The mark in a tile's free corner (#187), in design px.
const MARK: f32 = 11.0;
/// A session's source mark, alone in the middle of a front with nothing
/// else on it: Metro's 64 px tile glyph, at this board's tile size.
const SOURCE: f32 = 40.0;
/// The most letters a group's title runs to before it is cut, per small
/// tile of the group's width: the title is set in one line over the group.
const TITLE_CHARS_PER_UNIT: usize = 6;

/// What a session is working on, as the group it goes under (#162): its
/// Linear issue, or failing that its pull request, or failing that the
/// directory it works in — and there, the seat it sits in (#191).
///
/// A ticket or a pull request is a session saying what it is on, and two
/// sessions saying the same thing belong together whatever window they run
/// in. A shared directory says nothing of the kind: one repository holds
/// every piece of work anybody does in it. So where that is all there is,
/// the place breaks the tie — another tab, or another app entirely, is its
/// own heading rather than another tile under a folder's name.
pub fn work_unit(info: &SessionInfo) -> String {
    if let Some(t) = &info.ticket {
        return format!("issue:{}", t.key);
    }
    if let Some(p) = info.prs.last() {
        return format!("pr:{}", p.url());
    }
    if let Some(r) = info.prs_unanswered.last() {
        return format!("pr:{}", r.url());
    }
    format!("dir:{}@{}", info.cwd, info.group)
}

/// A group's title: the names its Claude sessions go by (#165) — what
/// `/rename` set, as written, or the session's directory where it has none —
/// each once, in board order.
pub fn section_name<'a>(sessions: impl Iterator<Item = &'a SessionInfo>) -> String {
    let mut names: Vec<String> = Vec::new();
    for info in sessions {
        let name = info.name.trim();
        let name = if name.is_empty() { info.label().to_lowercase() } else { name.to_string() };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.join("  ·  ")
}

/// One tile of the start screen, by what it shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// The Linear issue of the group, carried by this session.
    Issue(usize),
    /// A session, as a medium live tile.
    Session(usize),
    /// One of a session's subagents, as a small tile: the session and the
    /// subagent's place among its live ones. The fourth of more than four
    /// counts the rest.
    Agent(usize, usize),
    /// What a session produced: one of its pull requests.
    Pr(usize, usize),
    /// A pull request number GitHub would not answer for (#112).
    Unanswered(usize, usize),
    /// The newest page a session published, when it has no pull request.
    Page(usize),
}

/// The start screen's groups in board order, each with its title and its
/// tiles: the issue once, the sessions each followed by their subagents,
/// then everything they produced, each thing once.
pub fn groups(model: &BoardModel) -> Vec<(String, Vec<Kind>)> {
    keyed_groups(model).into_iter().map(|(_, title, tiles)| (title, tiles)).collect()
}

/// [`groups`], each with what it is known by from one poll to the next: its
/// first session's id. Not the work it is the group of (`work_unit`): that
/// changes as the work does — a session that opens a pull request or picks
/// up a Linear issue moves from its directory's group to the issue's — and
/// a group known by it would be dropped and built again each time, where
/// the same group should take the new tile in and push the rest along
/// (#200).
pub fn keyed_groups(model: &BoardModel) -> Vec<(String, String, Vec<Kind>)> {
    let mut units: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, c) in model.chips.iter().enumerate() {
        if c.anim.gone {
            continue;
        }
        let key = work_unit(&c.info);
        match units.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(i),
            None => units.push((key, vec![i])),
        }
    }
    units
        .into_iter()
        .map(|(_, members)| {
            let unit = model.chips[members[0]].info.session_id.clone();
            let title = section_name(members.iter().map(|&i| &model.chips[i].info));
            let mut tiles = Vec::new();
            if let Some(&i) = members.iter().find(|&&i| model.chips[i].info.ticket.is_some()) {
                tiles.push(Kind::Issue(i));
            }
            for &i in &members {
                tiles.push(Kind::Session(i));
                let live = model.chips[i].subs.iter().filter(|s| !s.anim.gone).count();
                tiles.extend((0..live.min(4)).map(|k| Kind::Agent(i, k)));
            }
            let mut shown: Vec<String> = Vec::new();
            let mut once = |key: String| {
                let fresh = !shown.contains(&key);
                if fresh {
                    shown.push(key);
                }
                fresh
            };
            for &i in &members {
                let info = &model.chips[i].info;
                for (k, p) in info.prs.iter().enumerate() {
                    if once(p.url()) {
                        tiles.push(Kind::Pr(i, k));
                    }
                }
                for (k, r) in info.prs_unanswered.iter().enumerate() {
                    if once(r.url()) {
                        tiles.push(Kind::Unanswered(i, k));
                    }
                }
                if info.prs.is_empty() && info.prs_unanswered.is_empty() {
                    if let Some(a) = info.artifacts.last() {
                        if once(a.url()) {
                            tiles.push(Kind::Page(i));
                        }
                    }
                }
            }
            (unit, title, tiles)
        })
        .collect()
}

/// How many small tiles wide every group is: narrow — one medium tile, so
/// groups of one or two things stand close — unless some group holds more
/// than two medium tiles' worth, which would make it a tall column (#173).
pub fn columns(groups: &[(String, Vec<Kind>)]) -> usize {
    let worth = |kinds: &Vec<Kind>| {
        let small = kinds.iter().filter(|k| matches!(k, Kind::Agent(..))).count();
        kinds.len() - small + small.div_ceil(4)
    };
    if groups.iter().any(|(_, k)| worth(k) > 2) { WIDE } else { NARROW }
}

/// Between one group and the next, and the height of a group's title row:
/// nsh-metro's own.
const GROUP_GAP: f32 = nsh_metro::tile::GROUP_GAP;
const GROUP_HEADER: f32 = nsh_metro::tile::GROUP_HEADER;

/// The most groups — pieces of work — a row holds, however wide the
/// window (#178).
const MAX_PER_ROW: usize = 4;

/// How many groups `columns` small tiles wide stand side by side in `room`
/// design px: as many as fit, never more than `MAX_PER_ROW`, and always one.
pub fn groups_per_row(room: f32, columns: usize) -> usize {
    let (group, gap) = (group_width(columns), GROUP_GAP);
    (((room + gap) / (group + gap)).floor() as usize).clamp(1, MAX_PER_ROW)
}

/// One group's width in design px, `columns` small tiles across.
fn group_width(columns: usize) -> f32 {
    columns as f32 * UNIT + columns.saturating_sub(1) as f32 * GAP
}

/// How far in from the margin the start screen sits so its widest row is
/// centred in `room` (#179): `groups` in all, `per_row` to a row. Nothing
/// when the row fills the room.
pub fn centring(room: f32, columns: usize, groups: usize, per_row: usize) -> f32 {
    let k = per_row.min(groups).max(1) as f32;
    let row = k * group_width(columns) + (k - 1.0) * GROUP_GAP;
    ((room - row) / 2.0).max(0.0)
}

/// `text` cut to `n` characters, with an ellipsis if it was longer.
fn clip(text: &str, n: usize) -> String {
    if text.chars().count() <= n { text.to_string() } else { format!("{}…", text.chars().take(n - 1).collect::<String>()) }
}

/// Where in its cycle a session's tile starts, from its id: every tile
/// keeps its own clock, so a screen of them never turns in step.
fn phase_of(info: &SessionInfo) -> f32 {
    let mut h: u32 = 2_166_136_261;
    for b in info.session_id.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    (h % 1000) as f32 / 1000.0
}

/// How long ago `started` was, short: `40m`, `3h 05m`, `2d`. `started` is
/// whatever the session file said, seconds or milliseconds.
fn running_for(started: u64) -> Option<String> {
    let started = if started > 100_000_000_000 { started / 1000 } else { started };
    // Before 2001 is not a start time anybody wrote down; the sandbox's
    // sessions carry a placeholder.
    if started < 1_000_000_000 {
        return None;
    }
    let secs = crate::usage::now_secs().checked_sub(started)?;
    let (m, h, d) = (secs / 60, secs / 3600, secs / 86_400);
    Some(if d > 0 {
        format!("{d}d")
    } else if h > 0 {
        format!("{h}h {:02}m", m % 60)
    } else {
        format!("{m}m")
    })
}

/// The back of a session's tile: what it is up to, a line large and a few
/// small. Waiting, what for; working, what its subagents are on, or where
/// it works on its own; idle, where it is and how long it has been up.
fn back_lines(info: &SessionInfo, chip: &ChipState) -> (String, Vec<String>) {
    let place = info.short_cwd();
    match info.phase() {
        Phase::NeedsUser => {
            let mut small: Vec<String> = info.waiting_for.iter().map(|w| w.to_lowercase()).collect();
            small.push(place);
            ("needs you".into(), small)
        }
        Phase::Working => {
            let busy: Vec<String> = chip
                .subs
                .iter()
                .filter(|x| !x.anim.gone && x.info.running)
                .map(|x| {
                    let what = x.info.description.trim();
                    if what.is_empty() { x.info.label().to_lowercase() } else { what.to_lowercase() }
                })
                .collect();
            if busy.is_empty() {
                (place, vec!["on its own".into()])
            } else {
                (format!("{} at work", busy.len()), busy.into_iter().take(3).collect())
            }
        }
        Phase::Idle => (place, running_for(info.started_at).map(|up| vec![format!("up {up}")]).unwrap_or_default()),
    }
}

/// Whether a pull request can be put away (#194): only one that has landed.
///
/// Everything else is work in flight, and the board is for work in flight —
/// a draft, an open one, a red one and even a closed one can still change
/// what they say, so hiding them would be hiding the board from itself.
pub fn can_put_away(p: &crate::pr::Pr) -> bool {
    p.look() == crate::pr::Look::Merged
}

/// Takes the pull requests somebody put away out of a snapshot (#194), by
/// url: a number belongs to its repository, and two repositories have a #1.
pub fn without_dismissed(list: &mut [SessionInfo], dismissed: &[String]) {
    if dismissed.is_empty() {
        return;
    }
    for s in list.iter_mut() {
        s.prs.retain(|p| !dismissed.iter().any(|d| *d == p.url()));
    }
}

const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

/// The colours of the board the tiles stand on.
#[derive(Clone, Copy)]
struct Inks {
    pal: Palette,
    accent: Rgba,
}

impl Inks {
    /// A tile's fill and the ink on it. A board that prints has one ink
    /// (#146): every tile is that ink, lettered in the paper.
    fn on(self, fill: Rgba) -> (Rgba, Rgba) {
        if self.pal.screened {
            return (self.pal.ink, self.pal.bg);
        }
        // White on the accent, which is what Metro sets on it whatever the
        // accent is; everything else is worked out against its own fill.
        let ink = if fill == self.accent { WHITE } else { ink::legible(WHITE, fill, self.pal) };
        (fill, ink)
    }

    /// Working tiles wear the board's chip colour; one waiting on you is the
    /// one in the accent; an idle one goes grey (#161).
    ///
    /// The accent is the session's own voice (#188). A pull request with red
    /// checks wants a person too, and the comb's cell had to say so — there
    /// the pull request is a row *inside* the cell, and the cell's colour is
    /// the only thing that carries across the board. On a start screen it is
    /// a tile of its own, already alarm-coloured and lettered `fail`, so the
    /// session saying it again only put the loudest colour on a tile whose
    /// own word is `idle`, while the status bar counted nobody waiting.
    fn session(self, info: &SessionInfo) -> Rgba {
        match info.phase() {
            Phase::NeedsUser => self.accent,
            Phase::Working => self.pal.chip,
            Phase::Idle => self.quiet(),
        }
    }

    fn quiet(self) -> Rgba {
        theme::lerp(self.pal.bg, self.pal.ink, 0.32)
    }
}

/// A tile by what it is of, rather than by where it sits in the board's
/// lists: a tile keeps its key from one poll to the next while the indices
/// under it shift, so its faces and its press look it up again each time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TileKey {
    Issue(String),
    Session(String),
    Agent(String, usize),
    Pr(String),
    Unanswered(String),
    Page(String),
}

impl TileKey {
    fn of(kind: &Kind, model: &BoardModel) -> Option<TileKey> {
        let info = |i: usize| model.chips.get(i).map(|c| &c.info);
        Some(match *kind {
            Kind::Issue(i) => TileKey::Issue(info(i)?.ticket.as_ref()?.key.clone()),
            Kind::Session(i) => TileKey::Session(info(i)?.session_id.clone()),
            Kind::Agent(i, k) => TileKey::Agent(info(i)?.session_id.clone(), k),
            Kind::Pr(i, k) => TileKey::Pr(info(i)?.prs.get(k)?.url()),
            Kind::Unanswered(i, k) => TileKey::Unanswered(info(i)?.prs_unanswered.get(k)?.url()),
            Kind::Page(i) => TileKey::Page(info(i)?.artifacts.last()?.url()),
        })
    }

    fn id(&self) -> String {
        match self {
            TileKey::Issue(k) => format!("i-{k}"),
            TileKey::Session(s) => format!("s-{s}"),
            TileKey::Agent(s, k) => format!("a-{s}-{k}"),
            TileKey::Pr(u) => format!("p-{u}"),
            TileKey::Unanswered(u) => format!("u-{u}"),
            TileKey::Page(u) => format!("g-{u}"),
        }
    }

    fn size(&self) -> TileSize {
        match self {
            TileKey::Agent(..) => TileSize::Small,
            _ => TileSize::Medium,
        }
    }
}

/// One face of a medium tile: a corner mark, a large line, small lines under
/// it, and the tile's title at the bottom-left where Metro keeps it.
#[derive(Clone, Debug, PartialEq, Default)]
struct Face {
    corner: Option<String>,
    big: String,
    small: Vec<String>,
    title: String,
    foot: Option<String>,
}

/// Everything a tile shows, looked up from the board by its key.
#[derive(Clone, Debug, PartialEq)]
struct Shows {
    fill: Rgba,
    ink: Rgba,
    front: Face,
    /// A session's back face: it is live.
    back: Option<Face>,
    mark: Option<Mark>,
    /// A session's source — the app it runs in, or Claude where it sits in
    /// none — set large in the middle of a front with nothing else to say.
    source: Option<Mark>,
    /// A review still pending, said in a band across the top (#153).
    waiting: bool,
    /// A small tile: a subagent's name, or `+n` for the rest.
    agent: Option<(String, bool)>,
}

/// What a press on a tile does, once the grid has swung away.
enum Press {
    Session(String),
    Link { label: String, first: Option<String>, then: Option<String>, what: &'static str },
}

/// The pull request a tile is of, if it is one that can be put away (#194).
fn landed(b: &Board, key: &TileKey) -> Option<(String, String)> {
    let TileKey::Pr(url) = key else { return None };
    b.model.live().flat_map(|c| c.info.prs.iter()).find(|p| p.url() == *url).filter(|p| can_put_away(p)).map(|p| (p.url(), p.label()))
}

fn press_of(b: &Board, key: &TileKey) -> Option<Press> {
    let infos = || b.model.live().map(|c| &c.info);
    Some(match key {
        TileKey::Session(s) | TileKey::Agent(s, _) => Press::Session(s.clone()),
        TileKey::Issue(k) => {
            let t = infos().filter_map(|i| i.ticket.as_ref()).find(|t| t.key == *k)?;
            Press::Link { label: t.label(), first: t.app_url(), then: t.url(), what: "Linear issue" }
        }
        TileKey::Pr(u) => {
            let p = infos().flat_map(|i| i.prs.iter()).find(|p| p.url() == *u)?;
            Press::Link { label: p.label(), first: None, then: Some(p.url()), what: "pull request" }
        }
        TileKey::Unanswered(u) => {
            let r = infos().flat_map(|i| i.prs_unanswered.iter()).find(|r| r.url() == *u)?;
            Press::Link { label: format!("#{}", r.number), first: None, then: Some(r.url()), what: "pull request" }
        }
        TileKey::Page(u) => {
            let a = infos().flat_map(|i| i.artifacts.iter()).find(|a| a.url() == *u)?;
            Press::Link { label: a.label(), first: None, then: Some(a.url()), what: "page" }
        }
    })
}

/// The inks the tiles are painted in now.
fn inks_of(b: &Board) -> Inks {
    let pal = b.palette();
    let accent = if b.settings.theme == crate::settings::BoardTheme::Orange {
        pal.ink
    } else {
        Rgba::from(untracked(metro_palette).accent)
    };
    Inks { pal, accent }
}

/// What the tile `key` shows, or `None` once it is no longer on the board.
fn shows(b: &Board, key: &TileKey) -> Option<Shows> {
    let inks = inks_of(b);
    let chip_of = |id: &str| b.model.chips.iter().find(|c| c.info.session_id == id && !c.anim.gone);
    let infos = || b.model.live().map(|c| &c.info);
    let plain = |fill: Rgba, ink: Rgba, front: Face, mark: Option<Mark>| Shows {
        fill,
        ink,
        front,
        back: None,
        mark,
        source: None,
        waiting: false,
        agent: None,
    };
    Some(match key {
        TileKey::Session(id) => {
            let chip = chip_of(id)?;
            let info = &chip.info;
            let (fill, ink) = inks.on(inks.session(info));
            let running = chip.subs.iter().filter(|x| !x.anim.gone && x.info.running).count();
            let corner = if info.phase() == Phase::NeedsUser {
                Some("!".to_string())
            } else {
                (running > 0).then(|| running.to_string())
            };
            let name = info.label();
            let (big, small) = back_lines(info, chip);
            Shows {
                fill,
                ink,
                front: Face { corner, title: name.clone(), foot: Some(info.phase().label().to_lowercase()), ..Face::default() },
                back: Some(Face { big, small, title: name, ..Face::default() }),
                // Where it runs, in the same mark the cell wore (#123).
                mark: info.seat.as_ref().and_then(|s| Mark::host(s.host)),
                source: Some(info.seat.as_ref().and_then(|s| Mark::host(s.host)).unwrap_or(Mark::Claude)),
                waiting: false,
                agent: None,
            }
        }
        TileKey::Agent(id, k) => {
            let chip = chip_of(id)?;
            let agents: Vec<_> = chip.subs.iter().filter(|x| !x.anim.gone).collect();
            let n = agents.len();
            let agent = agents.get(*k)?;
            let over = *k == 3 && n > 4;
            let base = if inks.pal.screened { inks.pal.ink } else { inks.pal.chip };
            // A finished one is the colour given back to the board.
            let fill = if over || agent.info.running { base } else { theme::lerp(inks.pal.bg, base, 0.45) };
            let ink = if inks.pal.screened { inks.pal.bg } else { ink::legible(WHITE, fill, inks.pal) };
            let text = if over { format!("+{}", n - 3) } else { agent.info.label().to_lowercase() };
            Shows { agent: Some((text, over)), ..plain(fill, ink, Face::default(), None) }
        }
        TileKey::Issue(k) => {
            let t = infos().filter_map(|i| i.ticket.as_ref()).find(|t| t.key == *k)?;
            let (fill, ink) = inks.on(ink::floor_color(ink::ticket_color(t, inks.pal), inks.pal));
            let state = if t.state_name.is_empty() {
                t.status.map(|st| st.short().to_lowercase()).unwrap_or_default()
            } else {
                t.state_name.to_lowercase()
            };
            let front = Face { big: t.label(), small: wrap_lines(&t.title), foot: Some(state), ..Face::default() };
            plain(fill, ink, front, Some(Mark::Linear))
        }
        TileKey::Pr(u) => {
            let p = infos().flat_map(|i| i.prs.iter()).find(|p| p.url() == *u)?;
            let (fill, ink) = inks.on(ink::floor_color(ink::pr_color(p, inks.pal), inks.pal));
            let front = Face {
                big: p.label(),
                small: wrap_lines(&p.title),
                foot: Some(p.look().short().to_lowercase()),
                ..Face::default()
            };
            Shows { waiting: p.awaiting_review(), ..plain(fill, ink, front, Some(Mark::GitHub)) }
        }
        TileKey::Unanswered(u) => {
            let r = infos().flat_map(|i| i.prs_unanswered.iter()).find(|r| r.url() == *u)?;
            let (fill, ink) = inks.on(inks.quiet());
            let front = Face { big: format!("#{}", r.number), foot: Some("github did not answer".into()), ..Face::default() };
            plain(fill, ink, front, Some(Mark::GitHub))
        }
        TileKey::Page(u) => {
            let a = infos().flat_map(|i| i.artifacts.iter()).find(|a| a.url() == *u)?;
            let (fill, ink) = inks.on(inks.quiet());
            let front = Face { big: a.label(), small: wrap_lines(&a.title), foot: Some("published".into()), ..Face::default() };
            plain(fill, ink, front, Some(Mark::Artifact))
        }
    })
}

/// How the start screen is set out: how wide a group is, how many stand in
/// a row, where the screen stands and at what zoom. The screen is built
/// again only when this changes; groups and tiles come and go inside it.
#[derive(Clone, Debug, PartialEq)]
struct Frame {
    columns: usize,
    per_row: usize,
    inset: f32,
    zoom: f32,
}

fn frame_of(b: &Board) -> Frame {
    let groups = groups(&b.model);
    let columns = columns(&groups);
    let zoom = b.layout.zoom;
    // A window is not a screen: the groups that fit across it make a row,
    // and the rest go on in rows below (#177).
    let room = b.window.0 / zoom.max(0.1) - 2.0 * MARGIN;
    let per_row = groups_per_row(room, columns);
    // On a wide window the screen stands in the middle (#179).
    let inset = centring(room, columns, groups.len(), per_row);
    Frame { columns, per_row, inset, zoom }
}

/// One group as the screen shows it: its title and its tiles' keys.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    title: String,
    tiles: Vec<TileKey>,
}

fn group_of(b: &Board, unit: &str, columns: usize) -> Option<Group> {
    let (_, title, kinds) = keyed_groups(&b.model).into_iter().find(|(u, ..)| u == unit)?;
    Some(Group {
        // One short of the room: the last group of a row wraps instead.
        title: clip(&title, columns * TITLE_CHARS_PER_UNIT - 1),
        tiles: kinds.iter().filter_map(|k| TileKey::of(k, &b.model)).collect(),
    })
}

/// The tiles view: a page header and the start screen under it, scrolling
/// in the space above the status bar.
pub fn start_screen(ui: Ui) -> AnyElement {
    let frame = ui.memo(frame_of);
    // Bumped after a tile's press, so the screen turns back in (#176).
    let replay = signal(0usize);
    {
        let ui = ui.clone();
        effect(move || {
            let r = ui.read(|b| b.tiles_replay);
            untracked(|| replay.set(r));
        });
    }
    let bg = ui.clone();
    scroll()
        .flex_1()
        .bg(move || bg.read(|b| theme::hsla(b.palette().bg)))
        // A press on the bare screen moves the window, as on gpui (#166);
        // the tiles keep their own presses.
        .window_drag(true)
        .child(view().window_drag(true).child(dynamic(move || screen(ui.clone(), frame.get(), replay))))
        .into_element()
}

fn screen(ui: Ui, frame: Frame, replay: Signal<usize>) -> AnyElement {
    let z = frame.zoom;
    let (tile_unit, gap) = (UNIT * z, GAP * z);
    let group_w = group_width(frame.columns) * z;
    let row_w = frame.per_row as f32 * group_w + (frame.per_row as f32 - 1.0) * GROUP_GAP * z;
    let header = label("start")
        .text_size(px(size::HUGE * z))
        .font_weight(LIGHT)
        .line_height(nsh_core::LineHeight::Points(px((size::HUGE + 8.) * z)))
        .text_color(move || metro_palette().foreground);
    let units = ui.memo(|b| keyed_groups(&b.model).into_iter().map(|(unit, ..)| unit).collect::<Vec<_>>());
    // The group whose tile was just pressed: it swings away on its own, the
    // tapped tile last, and every other group swings away with it, as the
    // start screen did when it was one grid (#176).
    let tapped: Signal<Option<String>> = signal(None);
    {
        let ui = ui.clone();
        effect(move || {
            // The screen comes back in after the press has landed.
            ui.read(|b| b.tiles_replay);
            untracked(|| tapped.set(None));
        });
    }
    let groups = {
        let ui = ui.clone();
        each(
            move || units.get(),
            |unit: &String| unit.clone(),
            move |unit, at| {
                let unit = untracked(|| unit.get());
                let at = untracked(|| at.get());
                let hinge = (MARGIN + frame.inset) * z + (at % frame.per_row) as f32 * (group_w + GROUP_GAP * z);
                group(ui.clone(), unit, frame.columns, (tile_unit, gap), z, hinge, replay, tapped)
            },
        )
    };
    let empty = units;
    view()
        .pt(px(TOP * z))
        .pb(px(MARGIN * z))
        .pr(px(MARGIN * z))
        .pl(px(frame.inset * z))
        .child(view().pt(px(12. * z)).pb(px(20. * z)).px(px(MARGIN * z)).child(view().ml(px(-3. * z)).child(header)))
        .child(when(move || empty.get().is_empty(), move || {
            view()
                .pl(px(MARGIN * z))
                .child(label("no claude code sessions").text_size(px(size::NORMAL * z)).text_color(move || metro_palette().subtle))
        }))
        // The groups wrap at `per_row` to a line, as the rows of grids did
        // (#177); a group that comes or goes moves the ones after it along,
        // and the rest stay where they are.
        .child(view().pl(px(MARGIN * z)).child(
            view().flex_row().flex_wrap().items_start().w(px(row_w)).gap_x(px(GROUP_GAP * z)).gap_y(px(ROW_GAP * z)).child(groups),
        ))
        .into_element()
}

/// One group: its title over its tiles. A grid of its own, so a tile that
/// comes or goes reflows its group as nsh-metro does it — the new one swings
/// open in place, the rest slide over — and the screen is not built again.
#[allow(clippy::too_many_arguments)]
fn group(
    ui: Ui,
    unit: String,
    columns: usize,
    metrics: (f32, f32),
    z: f32,
    hinge: f32,
    replay: Signal<usize>,
    tapped: Signal<Option<String>>,
) -> View {
    let (unit_px, gap) = metrics;
    let shown = {
        let unit = unit.clone();
        ui.memo(move |b| group_of(b, &unit, columns))
    };
    // Every other group swings away while a tile of this one is pressed.
    let leave = signal(false);
    {
        let unit = unit.clone();
        effect(move || {
            let away = tapped.get().is_some_and(|t| t != unit);
            untracked(|| leave.set(away));
        });
    }
    let down: Rc<dyn Fn()> = {
        let unit = unit.clone();
        Rc::new(move || tapped.set(Some(unit.clone())))
    };
    let grid = tile_grid()
        .columns(columns)
        .metrics(unit_px, gap)
        .turnstile(true)
        .replay(replay.read_only())
        .leave(leave.read_only())
        .hinge(hinge)
        .source(move || {
            let keys = shown.get().map(|g| g.tiles).unwrap_or_default();
            keys.into_iter().map(|k| tile_for(ui.clone(), k, unit_px, gap, z, down.clone())).collect()
        });
    // A group that the layout moves — one came in ahead of it, or the group
    // above grew a row — slides to its new place the way a tile slides
    // inside a grid, instead of jumping there (#199).
    let (dx, dy) = (animated_value(0.), animated_value(0.));
    let seen: Rc<std::cell::Cell<Option<(f32, f32)>>> = Rc::default();
    view()
        .flex_shrink_0()
        .bind_translate_x(&dx)
        .bind_translate_y(&dy)
        .on_layout(move |r: &Bounds<Points>| {
            let at = (f32::from(r.origin.x), f32::from(r.origin.y));
            let Some(was) = seen.replace(Some(at)) else { return };
            let (ox, oy) = (was.0 - at.0, was.1 - at.1);
            if ox.abs() < 0.5 && oy.abs() < 0.5 {
                return;
            }
            dx.set(ox);
            dy.set(oy);
            if nsh_metro::theme::reduced_motion() {
                dx.set(0.);
                dy.set(0.);
            } else {
                let _ = dx.timing(0., nsh_metro::tile::REFLOW, nsh_metro::motion::EXP6_OUT);
                let _ = dy.timing(0., nsh_metro::tile::REFLOW, nsh_metro::motion::EXP6_OUT);
            }
        })
        // The title runs on over the gap to the next group, as nsh-metro's
        // own group headers do; `clip` has already kept it short of the
        // next one's.
        .child(
            view().relative().h(px(GROUP_HEADER * z)).child(
                label(move || SharedString::from(shown.get().map(|g| g.title.to_lowercase()).unwrap_or_default()))
                    .absolute()
                    .left_0()
                    .top_0()
                    .number_of_lines(1u32)
                    .text_size(px(size::MEDIUM_LARGE * z))
                    .font_weight(SEMILIGHT)
                    .text_color(move || metro_palette().foreground),
            ),
        )
        .child(grid)
}

/// The tile for one of the start screen's things, pressing as what it
/// shows: a session (or its subagent) jumps to its terminal, an issue
/// opens in Linear, a pull request or a page in the browser. The grid
/// swings away first and the press lands after.
fn tile_for(ui: Ui, key: TileKey, unit: f32, gap: f32, z: f32, down: Rc<dyn Fn()>) -> Tile {
    let initial = ui.peek(|b| shows(b, &key));
    let live = initial.as_ref().is_some_and(|s| s.back.is_some());
    let phase = match &key {
        TileKey::Session(id) => ui.peek(|b| b.model.chips.iter().find(|c| c.info.session_id == *id).map(|c| phase_of(&c.info))),
        _ => None,
    };
    let data = {
        let (ui, key) = (ui.clone(), key.clone());
        computed(move || ui.read(|b| shows(b, &key)))
    };
    let press = {
        let (ui, key) = (ui.clone(), key.clone());
        move || {
            match ui.peek(|b| press_of(b, &key)) {
                Some(Press::Session(id)) => ui.activate_session(&id),
                Some(Press::Link { label, first, then, what }) => ui.open_link(label, first, then, what),
                None => {}
            }
            ui.update(|b| b.tiles_replay += 1);
        }
    };
    let menu = {
        let (ui, key) = (ui.clone(), key.clone());
        move |at: (f32, f32)| {
            // A merged pull request is work that has landed: right-clicking
            // its tile offers to put it away (#194).
            if let Some((url, label)) = ui.peek(|b| landed(b, &key)) {
                ui.update(|b| b.tile_menu = Some(TileMenu { at, url, label }));
            }
        }
    };
    let mut t = tile(key.id(), key.size())
        .metrics(unit, gap)
        .color(theme::hsla(initial.as_ref().map_or(WHITE, |s| s.fill)))
        .on_click(press);
    let front = {
        let menu = menu.clone();
        let down = down.clone();
        move |_: TileSize| face(data, false, z, menu.clone(), down.clone())
    };
    t = t.front(front);
    if live {
        t = t.back(move |_: TileSize| face(data, true, z, menu.clone(), down.clone())).period(PERIOD);
        if let Some(p) = phase {
            t = t.phase(p);
        }
    }
    t
}

/// A tile's face, bound to what the tile shows: its fill, its ink and its
/// words follow the board without the tile being built again.
fn face(data: Memo<Option<Shows>>, back: bool, z: f32, menu: impl Fn((f32, f32)) + Clone + 'static, down: Rc<dyn Fn()>) -> View {
    let get = move || data.get();
    let fill = move || get().map_or(Hsla::transparent_black(), |s| theme::hsla(s.fill));
    let ink = move || get().map_or(Hsla::transparent_black(), |s| theme::hsla(s.ink));
    let soft = move || ink().opacity(0.72);
    let side = move || get().map(|s| if back { s.back.unwrap_or_default() } else { s.front }).unwrap_or_default();
    let small_tile = untracked(get).is_some_and(|s| s.agent.is_some());
    let base = view()
        .size_full()
        .relative()
        .bg(fill)
        .on_context_menu(move |e: &ContextMenuEvent| menu((f32::from(e.page_position.x), f32::from(e.page_position.y))))
        // The moment a tile goes down, the other groups start to swing away.
        .on_press_down(move |_: &PressEvent| down());
    if small_tile {
        let text = move || SharedString::from(get().and_then(|s| s.agent).map(|a| a.0).unwrap_or_default());
        let over = untracked(get).and_then(|s| s.agent).is_some_and(|a| a.1);
        return if over {
            base.child(label(text).absolute().left(px(7. * z)).top(px(18. * z)).text_size(px(20. * z)).text_color(ink))
        } else {
            base.child(
                label(text)
                    .absolute()
                    .left(px(6. * z))
                    .right(px(4. * z))
                    .bottom(px(5. * z))
                    .text_size(px(9. * z))
                    .truncate()
                    .text_color(ink),
            )
        };
    }
    let mark = untracked(get).and_then(|s| s.mark);
    let source = untracked(get).and_then(|s| s.source).filter(|_| !back);
    // A front with nothing on it but the name: no `!`, no count of
    // subagents at work. It shows where the session runs, large in the
    // middle as a Metro tile shows its glyph, and the corner mark makes way.
    let plain = move || !back && get().is_some_and(|s| s.source.is_some() && s.front.corner.is_none() && s.front.big.is_empty());
    base.p(px(9. * z))
        .gap(px(3. * z))
        .child(
            label(move || SharedString::from(side().corner.unwrap_or_default()))
                .absolute()
                .top(px(6. * z))
                .right(px(9. * z))
                .text_size(px(20. * z))
                .text_color(ink),
        )
        .child(label(move || SharedString::from(side().big)).text_size(px(15. * z)).truncate().text_color(ink))
        .child(dynamic(move || {
            side()
                .small
                .into_iter()
                .map(|line| label(line).text_size(px(10. * z)).truncate().text_color(soft).into_element())
                .collect::<Vec<_>>()
        }))
        .child(
            view()
                .absolute()
                .left(px(9. * z))
                .right(px((MARK + 12.) * z))
                .bottom(px(7. * z))
                .child(label(move || SharedString::from(side().title)).text_size(px(13. * z)).truncate().text_color(ink))
                .child(label(move || SharedString::from(side().foot.unwrap_or_default())).text_size(px(10. * z)).text_color(soft)),
        )
        // What the tile is a tile *of*, in the one corner nothing else uses
        // (#187), in the ink the state line is drawn in.
        .children(mark.map(|m| {
            svg(ImageSrc::svg_markup(crate::logos::markup(m)))
                .absolute()
                .right(px(8. * z))
                .bottom(px(7. * z))
                .size(px(MARK * z))
                .text_color(soft)
                .opacity(move || if plain() { 0.0_f32 } else { 1.0 })
        }))
        .children(source.map(|m| {
            view()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .opacity(move || if plain() { 1.0_f32 } else { 0.0 })
                .child(svg(ImageSrc::svg_markup(crate::logos::markup(m))).size(px(SOURCE * z)).text_color(ink))
        }))
        // A review still pending, in a band across the top (#153).
        .child(
            view()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(5. * z))
                .bg(move || if get().is_some_and(|s| s.waiting) { ink().opacity(0.5) } else { Hsla::transparent_black() }),
        )
}

/// Puts a merged pull request away: written down first, so a board that is
/// killed in the next second comes back without it, then taken off the
/// board in hand rather than at the next poll (#194).
fn put_pr_away(b: &mut Board, url: String) {
    if !b.settings.dismissed.contains(&url) {
        b.settings.dismissed.push(url.clone());
        b.settings.save();
    }
    for c in b.model.chips.iter_mut() {
        c.info.prs.retain(|p| p.url() != url);
    }
    b.tile_menu = None;
    crate::ui::palette::refresh(b);
}

/// Brings every put-away pull request back (#194). They land on the board
/// again at the next poll, which is where they come from.
pub fn restore_dismissed(b: &mut Board) {
    b.settings.dismissed.clear();
    b.settings.save();
}

/// The tile menu's width, and how close to the window's edges it may come.
const MENU_W: f32 = 140.0;
const MENU_EDGE: f32 = 8.0;

/// The little menu a merged tile drops (#194): one thing to do, at the
/// pointer that asked for it, on the same sheet the search wears. A press
/// anywhere else puts it away.
pub fn menu(ui: Ui) -> AnyElement {
    dynamic(move || {
        let Some((m, pal, window, orange)) =
            ui.read(|b| b.tile_menu.clone().map(|m| (m, b.palette(), b.window, b.settings.theme == crate::settings::BoardTheme::Orange)))
        else {
            return view().into_element();
        };
        let ink = move |a: f32| theme::hsla(theme::with_alpha(pal.ink, a));
        let rule = move || if orange { ink(1.0) } else { metro_palette().accent };
        let x = m.at.0.min(window.0 - MENU_W - MENU_EDGE).max(MENU_EDGE);
        let y = m.at.1.min(window.1 - STATUS_H - MENU_EDGE - 60.0).max(MENU_EDGE);
        let (away, go) = (ui.clone(), ui.clone());
        let url = m.url.clone();
        view()
            .absolute()
            .inset_0()
            .on_press(move || away.update(|b| b.tile_menu = None))
            .child(
                view()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(MENU_W))
                    .border(px(1.))
                    .border_color(ink(0.35))
                    .bg(theme::hsla(crate::ui::palette::card_bg(pal)))
                    .on_press(|| {})
                    .child(view().h(px(2.)).bg(rule))
                    .child(view().px(px(8.)).py(px(4.)).child(label(m.label).text_size(px(10.)).text_color(ink(0.45))))
                    .child(
                        view()
                            .px(px(8.))
                            .py(px(5.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(ink(0.08)))
                            .on_press(move || go.update(|b| put_pr_away(b, url.clone())))
                            .child(label("치우기").text_size(px(11.)).text_color(ink(0.85))),
                    ),
            )
            .into_element()
    })
}

/// A title in up to three short lines under a tile's large line.
fn wrap_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > 22 {
            out.push(std::mem::take(&mut line));
            if out.len() == 3 {
                break;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if out.len() < 3 && !line.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Three sessions in three different Warp tabs: the first two on one
    /// Linear issue (the first with a pull request), the third on nothing.
    fn work() -> BoardModel {
        use crate::sources::SessionSource;
        let src = crate::sources::FakeSource::new();
        for _ in 0..3 {
            src.add(Phase::Working);
        }
        let ids: Vec<String> = src.snapshot().into_iter().map(|s| s.session_id).collect();
        src.set_ticket(&ids[0], Some(Some(crate::ticket::Status::ALL[0])));
        src.add_pr(&ids[0], crate::pr::Look::ALL[0]);
        let mut list = src.snapshot();
        list.sort_by_key(|s| ids.iter().position(|id| *id == s.session_id));
        list[1].ticket = list[0].ticket.clone();
        list[1].prs.clear();
        list[1].artifacts.clear();
        list[2].ticket = None;
        list[2].prs.clear();
        list[2].artifacts.clear();
        for (k, s) in list.iter_mut().enumerate() {
            s.group = format!("warp-tab:{k}");
            s.subagents.clear();
            s.prs_unanswered.clear();
        }
        list[0].artifacts.clear();
        let mut model = BoardModel::new();
        model.apply(list, Instant::now());
        model.settle();
        model
    }

    #[test]
    fn sessions_on_one_issue_share_a_group_whatever_tab_they_run_in() {
        let model = work();
        let groups = groups(&model);
        assert_eq!(groups.len(), 2, "one group for the issue, one for the loose session");
        assert_eq!(
            groups[0].1,
            vec![Kind::Issue(0), Kind::Session(0), Kind::Session(1), Kind::Pr(0, 0)],
            "the issue once, both sessions, then what they produced"
        );
        assert_eq!(groups[1].1, vec![Kind::Session(2)]);
    }

    /// A folder is not a piece of work (#191). Sessions that share only a
    /// directory are sorted by where they sit — so a Herdr tab, a Warp tab
    /// and the next tab along each keep their own heading — while a ticket
    /// or a pull request still gathers them whatever window they run in
    /// (#162).
    /// What is put away stays away, and only what landed can be (#194).
    /// The key is the url: a pull request number belongs to a repository,
    /// and the board watches several.
    #[test]
    fn only_a_merged_pull_request_can_be_put_away_and_it_goes_by_url() {
        use crate::pr::{Look, Pr};
        assert!(can_put_away(&Pr::synthetic(1, Look::Merged)));
        for live in [Look::Draft, Look::Open, Look::Failing, Look::Closed] {
            assert!(!can_put_away(&Pr::synthetic(1, live)), "{live:?} can still change what it says");
        }

        let mut s = SessionInfo::synthetic(1, "S-1", Phase::Idle);
        s.prs = vec![Pr::synthetic(7, Look::Merged), Pr::synthetic(8, Look::Open)];
        let (gone, kept) = (s.prs[0].url(), s.prs[1].url());
        assert_ne!(gone, kept);

        let mut list = [s.clone()];
        without_dismissed(&mut list, &[]);
        assert_eq!(list[0].prs.len(), 2, "with nothing put away nothing goes");

        let mut list = [s.clone()];
        without_dismissed(&mut list, &[gone.clone()]);
        assert_eq!(list[0].prs.iter().map(|p| p.url()).collect::<Vec<_>>(), vec![kept], "only the one put away");

        let mut list = [s];
        without_dismissed(&mut list, &["https://github.com/someone/else/pull/7".into()]);
        assert_eq!(list[0].prs.len(), 2, "the same number in another repository is another pull request");
    }

    #[test]
    fn a_shared_folder_alone_does_not_make_one_section() {
        let mut a = SessionInfo::synthetic(1, "A", Phase::Idle);
        let mut b = SessionInfo::synthetic(2, "B", Phase::Idle);
        for s in [&mut a, &mut b] {
            s.cwd = "/Users/x/git/crepe/yoshi".into();
            s.ticket = None;
            s.prs.clear();
            s.prs_unanswered.clear();
        }
        a.group = "warp-tab:1-1".into();
        b.group = "herdr-tab:93362db4".into();
        assert_ne!(work_unit(&a), work_unit(&b), "one folder, two places, two sections");

        b.group = "warp-tab:1-3".into();
        assert_ne!(work_unit(&a), work_unit(&b), "two tabs of one app are two places too");

        b.group = a.group.clone();
        assert_eq!(work_unit(&a), work_unit(&b), "the same folder in the same tab is one section");

        // What a session is on outranks where it sits, which is the whole
        // point of grouping by work (#162).
        b.group = "herdr-tab:93362db4".into();
        for s in [&mut a, &mut b] {
            s.ticket = Some(crate::ticket::Ticket::synthetic(7, Some(crate::ticket::Status::Started)));
        }
        assert_eq!(work_unit(&a), work_unit(&b), "one issue is one section, wherever it is worked on");
    }

    #[test]
    fn a_section_is_named_for_its_sessions() {
        let model = work();
        let mut a = model.chips[0].info.clone();
        a.name = "fix login".into();
        let mut b = model.chips[1].info.clone();
        b.name = String::new();
        let c = a.clone();
        let name = section_name([&a, &b, &c].into_iter());
        assert_eq!(name, format!("fix login  ·  {}", b.label().to_lowercase()), "renamed as written, unnamed by directory, each once");
    }

    #[test]
    fn subagents_follow_their_session_and_the_fourth_counts_the_rest() {
        let mut model = work();
        let info = model.chips[2].info.clone();
        let mut list: Vec<SessionInfo> = model.chips.iter().map(|c| c.info.clone()).collect();
        for k in 0..6 {
            list[2].subagents.push(crate::model::SubagentInfo::synthetic(k, "AGENT", "", true));
        }
        model.apply(list, Instant::now());
        model.settle();
        let groups = groups(&model);
        let loose = &groups.iter().find(|(_, k)| k.contains(&Kind::Session(2))).unwrap().1;
        assert_eq!(
            loose,
            &vec![Kind::Session(2), Kind::Agent(2, 0), Kind::Agent(2, 1), Kind::Agent(2, 2), Kind::Agent(2, 3)],
            "{}: four small tiles after the session, never more",
            info.label()
        );
    }

    #[test]
    fn groups_stay_one_tile_wide_until_one_needs_two() {
        let model = work();
        // The issue's group has four things in it: two tiles wide.
        assert_eq!(columns(&groups(&model)), WIDE);
        let one = vec![("a".to_string(), vec![Kind::Session(0), Kind::Pr(0, 0)])];
        assert_eq!(columns(&one), NARROW, "a session and its pull request fit one tile wide");
        let agents = vec![("a".to_string(), vec![Kind::Session(0), Kind::Agent(0, 0), Kind::Agent(0, 1)])];
        assert_eq!(columns(&agents), NARROW, "subagents take one medium tile's room between them");
    }

    #[test]
    fn groups_wrap_to_the_window_instead_of_running_off_it() {
        // One medium tile a group: 136 px and a 56 px gap each.
        assert_eq!(groups_per_row(136.0, NARROW), 1);
        assert_eq!(groups_per_row(136.0 * 3.0 + 56.0 * 2.0, NARROW), 3, "three fit exactly");
        assert_eq!(groups_per_row(136.0 * 3.0 + 56.0 * 2.0 - 1.0, NARROW), 2);
        assert_eq!(groups_per_row(20.0, WIDE), 1, "never none, however narrow");
        assert_eq!(groups_per_row(10_000.0, NARROW), MAX_PER_ROW, "four pieces of work a row at most");
    }

    #[test]
    fn a_wide_window_centres_the_widest_row() {
        // Three narrow groups (3 × 136 + 2 × 56 = 520) in 1000: 240 either side.
        assert_eq!(centring(1000.0, NARROW, 3, 4), 240.0);
        // Seven groups go four to a row: the four-wide row is what is centred.
        let four = 4.0 * 136.0 + 3.0 * 56.0;
        assert_eq!(centring(1000.0, NARROW, 7, 4), (1000.0 - four) / 2.0);
        assert_eq!(centring(300.0, NARROW, 3, 1), 82.0, "one to a row on a narrow window, still centred");
        assert_eq!(centring(100.0, WIDE, 1, 1), 0.0, "never pushed off the left");
    }

    /// The accent calls you over, so only the session may use it (#188). A
    /// pull request with red checks is its own tile on a start screen and
    /// says `fail` there; a session that is not asking anything stays grey
    /// however that tile reads, which is what the status bar counts.
    #[test]
    fn a_sleeping_session_is_not_coloured_for_its_pull_request() {
        let inks = Inks { pal: theme::PALETTE, accent: Rgba { r: 1.0, g: 0.44, b: 0.0, a: 1.0 } };
        let red = crate::pr::Pr::synthetic(1, crate::pr::Look::Failing);

        let mut idle = SessionInfo::synthetic(1, "S-1", Phase::Idle);
        idle.prs.push(red.clone());
        assert_eq!(inks.session(&idle), inks.quiet(), "a red pull request does not wake its session's tile");

        let mut busy = SessionInfo::synthetic(2, "S-2", Phase::Working);
        busy.prs.push(red);
        assert_eq!(inks.session(&busy), theme::PALETTE.chip, "nor does it stop one working");

        let asking = SessionInfo::synthetic(3, "S-3", Phase::NeedsUser);
        assert_eq!(inks.session(&asking), inks.accent, "the accent is for a session asking");
    }

    #[test]
    fn a_title_wraps_by_word_into_three_lines_at_most() {
        let lines = wrap_lines("make the start screen draw with the metro library instead of the board canvas please");
        assert!(lines.len() <= 3, "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 22 || !l.contains(' ')), "{lines:?}");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
    }
}
