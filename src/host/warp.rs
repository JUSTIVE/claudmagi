//! Warp: the host claudmagi was written against (#118).
//!
//! Warp exports `WARP_TERMINAL_SESSION_UUID` and `WARP_FOCUS_URL` into every
//! shell it spawns, so a session's pane is in its environment; opening the
//! focus URL brings that pane to the front. The tab a pane sits in is not in
//! the environment, though, and the tab is what makes a cluster (#42), so it
//! has to be read out of Warp's own state — which is what drags Full Disk
//! Access into this (#67, #69). Orca, by contrast, hands its tab over in the
//! environment and needs none of this machinery.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat, Seated, run_with_timeout};


#[derive(Default)]
pub struct Warp {
    tabs: Mutex<Option<WarpTabs>>,
    /// Set once the first snapshot has waited on Warp; it never waits again.
    tabs_tried: std::sync::atomic::AtomicBool,
    /// Where each pane stands with Warp's own list (#186, #251).
    missing: Mutex<HashMap<String, Presence>>,
}

/// Whether a seat is still there, as Warp's list answers for it (#186,
/// #192, #251).
///
/// The rule this holds is that **only Warp condemns a pane**: a wait, and
/// then two answers saying the same thing. It used to live as a map of
/// "first missing at" plus the subtraction at the call site, which meant
/// the three states a seat can be in were never written down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Warp lists it.
    Listed,
    /// A good answer did not list it, since this moment.
    Missing(Instant),
    /// It has been missing for its whole wait: the board lets it go.
    Gone,
}

/// What one reading of Warp's list says about one seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sighting {
    /// A good answer lists it.
    There,
    /// A good answer does not list it.
    NotThere,
    /// There was no good answer — the database is missing, locked or
    /// refused (#67). This says **nothing** about the seat, and a board
    /// that drops sessions on a maybe is worse than one that keeps a stale
    /// tile.
    Unknown,
}

impl Presence {
    /// The whole table. `wait` is this seat's own grace (`wait_for`).
    pub fn next(self, seen: Sighting, now: Instant, wait: Duration) -> Presence {
        match (self, seen) {
            (_, Sighting::There) => Presence::Listed,
            // Not knowing forgets what was: the clock starts again when
            // there is something to start it from.
            (_, Sighting::Unknown) => Presence::Listed,
            (Presence::Listed, Sighting::NotThere) => Presence::Missing(now),
            (Presence::Missing(since), Sighting::NotThere) => {
                if now.duration_since(since) >= wait { Presence::Gone } else { Presence::Missing(since) }
            }
            (Presence::Gone, Sighting::NotThere) => Presence::Gone,
        }
    }
}

impl Host for Warp {
    fn id(&self) -> &'static str {
        "warp"
    }

    fn seat(&self, env: &HashMap<String, String>) -> Option<Seat> {
        let uuid = env.get("WARP_TERMINAL_SESSION_UUID")?;
        Some(Seat {
            host: "warp",
            handle: uuid.clone(),
            // Warp keeps the tab to itself; `windows` goes and gets it.
            window: None,
            focus_url: env.get("WARP_FOCUS_URL").cloned(),
        })
    }

    fn windows(&self, handles: &[&str]) -> HashMap<String, String> {
        let tabs = self.tabs_settled(!handles.is_empty());
        handles
            .iter()
            .filter_map(|h| tabs.get(&h.replace('-', "")).map(|tab| ((*h).to_string(), tab.clone())))
            .collect()
    }

    /// The panes Warp has stopped listing (#186).
    ///
    /// Warp keeps its open panes in the same table the tabs are read from, so
    /// a pane that is no longer there is a pane that was closed. The session
    /// in it can outlive that — a background job keeps running, and the board
    /// went on drawing it under a tab that no longer opens.
    ///
    /// Two things hold this back from being wrong. Only a **good** answer can
    /// condemn a pane: a database that is missing, locked or refused (#67)
    /// means we do not know, and nothing is dropped on not knowing. And a
    /// pane has to be missing for a whole minute before it counts, because a
    /// pane Warp has only just opened is not in the file yet — a session
    /// would otherwise blink out of the board in the moment it was started.
    fn gone(&self, seats: &[Seated]) -> Vec<String> {
        self.gone_at(seats, Instant::now())
    }

    fn focus(&self, seat: &Seat) -> Result<Outcome> {
        if let Some(url) = seat.focus_url.as_deref() {
            if open_url(url)? {
                return Ok(Outcome::Focused(url.to_string()));
            }
        }
        if !seat.handle.is_empty() {
            let url = format!("warp://session/{}", seat.handle);
            if open_url(&url)? {
                return Ok(Outcome::Focused(url));
            }
        }
        Command::new("open").args(["-a", "Warp"]).status().context("activating Warp")?;
        Ok(Outcome::ActivatedApp("Warp".into()))
    }

    fn note(&self) -> Option<&'static str> {
        self.tabs_miss()?.note()
    }
}

fn open_url(url: &str) -> Result<bool> {
    Ok(Command::new("open").arg(url).status().context("spawning `open`")?.success())
}

impl Warp {
    /// Cached pane → tab map; empty when neither Warp's database nor
    /// `warpctrl` can answer.
    fn tabs(&self) -> HashMap<String, String> {
        let mut guard = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = match guard.as_ref() {
            Some(t) if t.good => WARP_TABS_TTL,
            _ => WARP_TABS_RETRY,
        };
        if guard.as_ref().is_none_or(|t| t.fetched.elapsed() > ttl) {
            let got = fetch_warp_tabs();
            let miss = got.as_ref().err().copied();
            match got {
                Ok(map) if !map.is_empty() => {
                    *guard = Some(WarpTabs { map, fetched: Instant::now(), good: true, miss: None });
                }
                // A miss keeps whatever good map we already had — Warp being
                // briefly unreadable is not the same as having no tabs (#59).
                _ => match guard.as_mut() {
                    Some(t) => {
                        t.fetched = Instant::now();
                        t.miss = miss;
                    }
                    None => {
                        *guard = Some(WarpTabs { map: HashMap::new(), fetched: Instant::now(), good: false, miss })
                    }
                },
            }
        }
        guard.as_ref().map(|t| t.map.clone()).unwrap_or_default()
    }

    /// `gone`, with the clock handed in so it can be tested (#186).
    fn gone_at(&self, seats: &[Seated], now: Instant) -> Vec<String> {
        let answered = self.tabs.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|t| (t.map.clone(), t.good));
        // A bad answer is not an empty one: every seat is simply unknown,
        // and the table is what forgets (`Unknown` → `Listed`).
        let listing = match answered {
            Some((map, true)) => Some(map),
            _ => None,
        };
        let mut standing = self.missing.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for seat in seats {
            let seen = match &listing {
                None => Sighting::Unknown,
                Some(map) if map.contains_key(&seat.handle.replace('-', "")) => Sighting::There,
                Some(_) => Sighting::NotThere,
            };
            let here = standing.get(seat.handle).copied().unwrap_or(Presence::Listed);
            let there = here.next(seen, now, wait_for(seat.running_for));
            if there == Presence::Gone {
                out.push(seat.handle.to_string());
            }
            match there {
                Presence::Listed => standing.remove(seat.handle),
                other => standing.insert(seat.handle.to_string(), other),
            };
        }
        standing.retain(|h, _| seats.iter().any(|s| s.handle == h));
        out
    }

    /// Why the last fetch came up empty, for the status bar (#67).
    pub(crate) fn tabs_miss(&self) -> Option<TabsMiss> {
        self.tabs.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|t| t.miss)
    }

    /// Warp's pane → tab map, insisting on a real answer the first time it is
    /// needed. Sessions are grouped by it, so painting before it lands means
    /// showing a layout that is simply wrong and then re-laying it out (#59).
    /// After the first attempt this never blocks again, so a machine without
    /// Warp pays the cost once.
    fn tabs_settled(&self, wants: bool) -> HashMap<String, String> {
        let mut tabs = self.tabs();
        let first = !self.tabs_tried.swap(true, std::sync::atomic::Ordering::Relaxed);
        // A refusal is not a race: waiting 1.5s for it to resolve only
        // delays the first paint (#67).
        if first && wants && tabs.is_empty() && self.tabs_miss() != Some(TabsMiss::Denied) {
            for _ in 0..FIRST_TABS_TRIES {
                std::thread::sleep(FIRST_TABS_WAIT);
                tabs = self.tabs();
                if !tabs.is_empty() {
                    break;
                }
            }
        }
        tabs
    }
}

// Warp tabs: which pane (terminal session uuid) sits in which tab (#42)
//
// Two sources, tried in order:
//  1. Warp's own state database (`warp.sqlite`), which Warp keeps current
//     for session restore. Read-only through `/usr/bin/sqlite3`; works
//     without any Warp setting.
//  2. The Warp Control CLI (`warpctrl pane list`), which needs Warp's local
//     control server to be turned on.
// ---------------------------------------------------------------------------

/// How long a pane has to be missing from Warp's own list before the board
/// takes the session out of the window it was sitting in (#186).
///
/// Long enough to cover a pane Warp has opened but not written down yet, and
/// short enough that a tab closed by hand is off the board before anyone
/// goes looking for it.
const GONE_AFTER: Duration = Duration::from_secs(60);

/// The same wait for a session that was already running before this board
/// started (#192).
///
/// The minute above buys one thing: a pane Warp has only just opened and has
/// not written down yet. A session that has been running for longer than
/// that minute was not opened in it a moment ago, so the excuse cannot
/// apply, and holding its tile for a further minute after every launch only
/// puts a week-old background job back on the board — loudest tile first,
/// since what it last said was that it wanted a person.
///
/// It is not zero. Age is never a reason to drop anything on its own: the
/// only thing that condemns a pane is Warp not listing it, and this wait is
/// what keeps one unlucky read from being enough. It spans more than one
/// fetch (`WARP_TABS_TTL`), so a list that is briefly short of a pane — a
/// restore, a write in flight — has to say the same thing twice.
const GONE_AFTER_SETTLED: Duration = Duration::from_secs(6);

/// How long a seat waits before a missing pane counts, by how long its
/// session has been running.
fn wait_for(running_for: Duration) -> Duration {
    if running_for > GONE_AFTER { GONE_AFTER_SETTLED } else { GONE_AFTER }
}

/// How often to ask Warp for its pane → tab map.
const WARP_TABS_TTL: Duration = Duration::from_secs(5);
/// How soon to retry after a miss. A failed read must not pass for "no tabs"
/// until the normal TTL expires, or the board groups by pane, lays itself out
/// wrong, and then slides every row when the real answer lands (#59).
const WARP_TABS_RETRY: Duration = Duration::from_millis(400);
/// How hard the first snapshot insists on an answer before giving up, once
/// per process. Sessions are grouped by it, so a wrong first layout is worse
/// than a slightly later first paint.
const FIRST_TABS_TRIES: usize = 6;
const FIRST_TABS_WAIT: Duration = Duration::from_millis(250);
const WARPCTRL_TIMEOUT: Duration = Duration::from_millis(1500);

struct WarpTabs {
    map: HashMap<String, String>,
    fetched: Instant,
    /// Whether this came from a fetch that actually returned panes.
    good: bool,
    /// Why the last fetch came up empty, when it did (#67).
    miss: Option<TabsMiss>,
}

/// Why Warp's pane -> tab map is missing. `Denied` is the one worth saying out
/// loud: the database is right there and macOS will not open it (#67).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsMiss {
    /// No Warp database on this machine.
    NoDatabase,
    /// `~/Library/Group Containers` is Full Disk Access territory, and the
    /// app has not been granted it. A terminal never sees this -- it runs
    /// under the grant its own terminal app holds -- so it only ever shows up
    /// in the installed bundle.
    Denied,
    /// The database opened but `sqlite3` or the query did not answer.
    Unreadable,
}

impl TabsMiss {
    /// What the status bar says. Only a denial gets a line: the other two
    /// mean "no Warp here", which the empty board already shows.
    fn note(self) -> Option<&'static str> {
        match self {
            Self::Denied => Some("NO WARP TABS \u{2014} GRANT FULL DISK ACCESS"),
            Self::NoDatabase | Self::Unreadable => None,
        }
    }
}

/// Warp's state databases, most likely first. Warp writes the live one under
/// its app-group container; the plain Application Support copy is an older
/// location kept as a fallback.
pub fn warp_db_candidates() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let mut out = Vec::new();
    for flavour in ["dev.warp.Warp-Stable", "dev.warp.Warp-Preview", "dev.warp.Warp"] {
        out.push(
            home.join("Library/Group Containers/2BBY89MBSN.dev.warp/Library/Application Support")
                .join(flavour)
                .join("warp.sqlite"),
        );
        out.push(home.join("Library/Application Support").join(flavour).join("warp.sqlite"));
    }
    out
}

/// `terminal_panes.uuid` is the pane's `WARP_TERMINAL_SESSION_UUID`;
/// `pane_nodes` ties the pane to its tab. Rows come out as `uuid window tab`.
const WARP_DB_QUERY: &str = "select lower(hex(tp.uuid)), t.window_id, pn.tab_id \
     from terminal_panes tp \
     join pane_nodes pn on pn.id = tp.id \
     join tabs t on t.id = pn.tab_id";

/// Reads the pane → tab map straight from Warp's database. An empty map when
/// Warp simply has no terminal panes open; a `TabsMiss` when there is nothing
/// to read or nothing is allowed to read it.
fn fetch_warp_tabs_from_db() -> Result<HashMap<String, String>, TabsMiss> {
    let db = warp_db_candidates()
        .into_iter()
        .find(|p| p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))
        .ok_or(TabsMiss::NoDatabase)?;
    // Open it here rather than letting `sqlite3` be the first to try. A TCC
    // refusal reaches us from `sqlite3` as "authorization denied" on a stderr
    // we discard plus a non-zero exit — the same shape as a corrupt file —
    // whereas `File::open` says `PermissionDenied` and nothing else does.
    // `stat` is not restricted, so the candidate above is still found (#67).
    if let Err(e) = std::fs::File::open(&db) {
        return Err(match e.kind() {
            std::io::ErrorKind::PermissionDenied => TabsMiss::Denied,
            _ => TabsMiss::Unreadable,
        });
    }
    let mut cmd = std::process::Command::new("/usr/bin/sqlite3");
    cmd.args(["-readonly", "-batch", "-noheader", "-separator", " "]).arg(&db).arg(WARP_DB_QUERY);
    let text = run_with_timeout(cmd, WARPCTRL_TIMEOUT).ok_or(TabsMiss::Unreadable)?;
    Ok(parse_pane_rows(&text))
}

/// Parses `uuid window tab` lines into `uuid → "<window>-<tab>"`. Tab ids are
/// unique on their own, but the window keeps the key readable.
pub fn parse_pane_rows(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let uuid = parts.next()?;
            let window = parts.next()?;
            let tab = parts.next()?;
            if !looks_like_uuid(uuid) || window.parse::<u64>().is_err() || tab.parse::<u64>().is_err() {
                return None;
            }
            Some((uuid.replace('-', "").to_ascii_lowercase(), format!("{window}-{tab}")))
        })
        .collect()
}

/// The Warp Control CLI: an installed `warpctrl` if there is one, else the
/// Warp app binary itself in its hidden `--warpctrl` mode. Returns the
/// program and the leading arguments.
pub fn warpctrl_command() -> Option<(PathBuf, Vec<&'static str>)> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("warpctrl")).collect())
        .unwrap_or_default();
    candidates.push(PathBuf::from("/usr/local/bin/warpctrl"));
    candidates.push(PathBuf::from("/opt/homebrew/bin/warpctrl"));
    if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
        return Some((p, vec![]));
    }
    for app in ["/Applications/Warp.app", "/Applications/WarpPreview.app"] {
        let dir = PathBuf::from(app).join("Contents/MacOS");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            if let Some(bin) = rd.flatten().map(|e| e.path()).find(|p| p.is_file()) {
                return Some((bin, vec!["--warpctrl"]));
            }
        }
    }
    None
}

/// Runs `warpctrl --output-format json pane list` with a timeout. Empty when
/// Warp's local control server is off (Settings → Scripting).
fn fetch_warp_tabs_from_warpctrl() -> Option<HashMap<String, String>> {
    let (bin, lead) = warpctrl_command()?;
    let mut cmd = std::process::Command::new(bin);
    cmd.args(lead).args(["--output-format", "json", "pane", "list"]);
    let text = run_with_timeout(cmd, WARPCTRL_TIMEOUT)?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(parse_pane_tabs(&value))
}

/// The database first (always available), then `warpctrl` (only with Warp's
/// local control on). A successful but empty answer from the database is
/// final: Warp has no terminal panes, so there is nothing for warpctrl to add.
fn fetch_warp_tabs() -> Result<HashMap<String, String>, TabsMiss> {
    match fetch_warp_tabs_from_db() {
        Ok(map) => Ok(map),
        // `warpctrl` needs no Full Disk Access, so it is worth trying even
        // when the database was the thing that was refused.
        Err(miss) => fetch_warp_tabs_from_warpctrl().ok_or(miss),
    }
}

fn looks_like_uuid(s: &str) -> bool {
    let hex = s.chars().filter(|c| c.is_ascii_hexdigit()).count();
    (s.len() == 32 && hex == 32) || (s.len() == 36 && hex == 32 && s.matches('-').count() == 4)
}

/// Extracts `session uuid → tab id` from `warpctrl pane list` JSON without
/// depending on its exact shape: any object carrying a session-ish uuid is a
/// pane, and its tab is either a `tab*` field on the pane or the id of an
/// enclosing object reached through a `tab*` key.
pub fn parse_pane_tabs(value: &serde_json::Value) -> HashMap<String, String> {
    fn id_of(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
        ["id", "uuid", "tab_id", "tabId"].iter().find_map(|k| obj.get(*k).and_then(scalar))
    }
    fn scalar(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    fn walk(v: &serde_json::Value, parent_key: &str, tab: Option<String>, out: &mut HashMap<String, String>) {
        match v {
            serde_json::Value::Object(obj) => {
                let tab_here = if parent_key.to_ascii_lowercase().contains("tab") { id_of(obj).or(tab.clone()) } else { tab.clone() };
                let own_tab = obj
                    .iter()
                    .find(|(k, v)| k.to_ascii_lowercase().contains("tab") && scalar(v).is_some())
                    .and_then(|(_, v)| scalar(v))
                    .or(tab_here.clone());
                let session = obj.iter().find_map(|(k, v)| {
                    let k = k.to_ascii_lowercase();
                    if !k.contains("session") {
                        return None;
                    }
                    match v {
                        serde_json::Value::String(s) if looks_like_uuid(s) => Some(s.clone()),
                        serde_json::Value::Object(inner) => id_of(inner).filter(|s| looks_like_uuid(s)),
                        _ => None,
                    }
                });
                if let (Some(sess), Some(t)) = (session, own_tab.clone()) {
                    out.insert(sess.replace('-', ""), t);
                }
                for (k, child) in obj {
                    walk(child, k, tab_here.clone(), out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, parent_key, tab.clone(), out);
                }
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    walk(value, "", None, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warp_db_rows_map_panes_to_window_and_tab() {
        let rows = "0762e1f78808469c8809535d4eb65068 1 1\n\
                    0bd9a62647ef4dc3ab9a54e9966df502 1 2\n\
                    80602DAB-5730-4503-B001-386BF769BCD9 1 1\n\
                    garbage line\n\
                    notauuid 1 1\n";
        let m = parse_pane_rows(rows);
        assert_eq!(m.len(), 3);
        assert_eq!(m.get("0762e1f78808469c8809535d4eb65068").map(String::as_str), Some("1-1"));
        assert_eq!(m.get("80602dab57304503b001386bf769bcd9").map(String::as_str), Some("1-1"), "dashes and case are normalised");
        assert_eq!(m.get("0bd9a62647ef4dc3ab9a54e9966df502").map(String::as_str), Some("1-2"));
        assert!(parse_pane_rows("").is_empty());
    }

    /// A pane Warp has stopped listing is a tab that was closed, and the
    /// session in it may well still be running (#186). Two things keep that
    /// from being read too eagerly: nothing is dropped unless Warp actually
    /// Every reading moves a pane one step, and a reading that is not an
    /// answer moves it back (#251). The rule the table holds is that only
    /// Warp condemns a pane: a wait, and then the same thing said twice.
    #[test]
    fn a_bad_reading_is_not_an_empty_one() {
        let t0 = Instant::now();
        let wait = Duration::from_secs(60);
        let step = |p: Presence, seen: Sighting, at: Duration| p.next(seen, t0 + at, wait);

        let missing = step(Presence::Listed, Sighting::NotThere, Duration::ZERO);
        assert_eq!(missing, Presence::Missing(t0), "the clock starts the first time it is not listed");
        assert_eq!(step(missing, Sighting::NotThere, wait / 2), missing, "half a wait is not a wait");
        assert_eq!(step(missing, Sighting::NotThere, wait), Presence::Gone, "and then it is let go");
        assert_eq!(step(Presence::Gone, Sighting::NotThere, wait * 9), Presence::Gone, "gone stays gone");

        // The two ways back.
        assert_eq!(step(missing, Sighting::There, wait * 9), Presence::Listed, "listed again is listed");
        assert_eq!(step(Presence::Gone, Sighting::There, wait * 9), Presence::Listed, "even after it was let go");
        assert_eq!(
            step(missing, Sighting::Unknown, wait * 9),
            Presence::Listed,
            "a database that is missing, locked or refused says nothing about the pane (#67)"
        );
        for here in [Presence::Listed, Presence::Missing(t0), Presence::Gone] {
            assert_ne!(step(here, Sighting::Unknown, wait * 9), Presence::Gone, "{here:?}: not knowing never condemns");
            assert_ne!(step(here, Sighting::There, wait * 9), Presence::Gone, "{here:?}: nor does being listed");
        }
    }

    /// answered, and a pane has to stay missing for a minute first.
    #[test]
    fn a_pane_warp_has_stopped_listing_is_gone_after_a_minute() {
        let warp = Warp::default();
        let here = "0762e1f78808469c8809535d4eb65068";
        let closed = "0bd9a62647ef4dc3ab9a54e9966df502";
        // Both only just started, so both get the newcomer's whole minute
        // (#192).
        let seat = |h: &'static str| Seated { handle: h, running_for: Duration::from_secs(3) };
        let handles = [seat(here), seat(closed)];
        let t0 = Instant::now();

        // Nothing read: nothing is gone, however long it is missing.
        assert!(warp.gone_at(&handles, t0).is_empty(), "nothing was read, so nothing is missing");
        assert!(warp.gone_at(&handles, t0 + GONE_AFTER * 2).is_empty());

        // A denial is not an answer either — the database is right there and
        // macOS will not open it (#67).
        *warp.tabs.lock().unwrap() = Some(WarpTabs {
            map: HashMap::new(),
            fetched: Instant::now(),
            good: false,
            miss: Some(TabsMiss::Denied),
        });
        assert!(warp.gone_at(&handles, t0 + GONE_AFTER * 2).is_empty(), "a refusal is not an empty Warp");

        // A good answer with one of them in it.
        *warp.tabs.lock().unwrap() = Some(WarpTabs {
            map: HashMap::from([(here.to_string(), "1-1".to_string())]),
            fetched: Instant::now(),
            good: true,
            miss: None,
        });
        assert!(warp.gone_at(&handles, t0).is_empty(), "a pane that has only just gone missing is not gone");
        assert!(warp.gone_at(&handles, t0 + GONE_AFTER / 2).is_empty());
        assert_eq!(warp.gone_at(&handles, t0 + GONE_AFTER), vec![closed.to_string()]);
        assert_eq!(warp.gone_at(&handles, t0 + GONE_AFTER * 3), vec![closed.to_string()], "and stays gone");

        // A pane that comes back forgets it was ever missing, so the minute
        // starts again rather than condemning it on the next blink.
        *warp.tabs.lock().unwrap() = Some(WarpTabs {
            map: HashMap::from([(here.to_string(), "1-1".to_string()), (closed.to_string(), "1-2".to_string())]),
            fetched: Instant::now(),
            good: true,
            miss: None,
        });
        assert!(warp.gone_at(&handles, t0 + GONE_AFTER * 3).is_empty());
        *warp.tabs.lock().unwrap() = Some(WarpTabs {
            map: HashMap::from([(here.to_string(), "1-1".to_string())]),
            fetched: Instant::now(),
            good: true,
            miss: None,
        });
        assert!(warp.gone_at(&handles, t0 + GONE_AFTER * 3).is_empty(), "the clock starts again");

    }

    /// The minute above buys one thing: a pane Warp has opened and not
    /// written down yet. Only a session young enough to be in that position
    /// gets it (#192) — otherwise every launch put a week-old background job
    /// back on the board for a minute, wearing the colour that calls for a
    /// person.
    ///
    /// Age still condemns nothing on its own. What drops a pane is Warp not
    /// listing it, and an old seat has to be missing from more than one
    /// look at that list before the board believes it.
    #[test]
    fn a_session_that_has_been_up_for_a_week_does_not_get_the_newcomers_minute() {
        let here = "0762e1f78808469c8809535d4eb65068";
        let gone = "0bd9a62647ef4dc3ab9a54e9966df502";
        let listed = |warp: &Warp| {
            *warp.tabs.lock().unwrap() = Some(WarpTabs {
                map: HashMap::from([(here.to_string(), "1-1".to_string())]),
                fetched: Instant::now(),
                good: true,
                miss: None,
            });
        };
        let t0 = Instant::now();
        let week = Duration::from_secs(60 * 60 * 24 * 7);

        let old = Warp::default();
        listed(&old);
        let seats = [Seated { handle: here, running_for: week }, Seated { handle: gone, running_for: week }];
        assert!(old.gone_at(&seats, t0).is_empty(), "one unlucky read is never enough");
        assert!(old.gone_at(&seats, t0 + GONE_AFTER_SETTLED / 2).is_empty());
        assert_eq!(old.gone_at(&seats, t0 + GONE_AFTER_SETTLED), vec![gone.to_string()]);
        assert!(
            GONE_AFTER_SETTLED > WARP_TABS_TTL,
            "the short wait has to span more than one look at Warp's list, or a single stale read condemns a pane"
        );

        // The pane of a session that started a moment ago may be the one
        // Warp has not written down yet, so that one keeps the minute.
        let young = Warp::default();
        listed(&young);
        let new_seat = [Seated { handle: gone, running_for: Duration::from_secs(3) }];
        // Its clock starts here, at the first answer that does not list it.
        let first = t0 + GONE_AFTER_SETTLED * 2;
        assert!(young.gone_at(&new_seat, first).is_empty(), "a pane this new may simply be unwritten");
        assert!(young.gone_at(&new_seat, first + GONE_AFTER / 2).is_empty());
        assert_eq!(young.gone_at(&new_seat, first + GONE_AFTER), vec![gone.to_string()]);
    }

    #[test]
    fn only_a_refusal_is_worth_a_line_in_the_status_bar() {
        // No Warp and a broken Warp both look like "no tabs", which the board
        // already shows by grouping per pane. A refusal looks identical and
        // is the one the user can do something about (#67).
        assert!(TabsMiss::Denied.note().is_some());
        assert_eq!(TabsMiss::NoDatabase.note(), None);
        assert_eq!(TabsMiss::Unreadable.note(), None);
    }

    #[test]
    fn pane_list_json_maps_sessions_to_tabs_in_either_shape() {
        let flat = serde_json::json!({
            "panes": [
                {"id": "p1", "tab_id": "t1", "session_id": "0762e1f78808469c8809535d4eb65068"},
                {"id": "p2", "tab_id": "t1", "session": {"id": "8691e9de-2600-40af-9792-b25feb1846d9"}},
                {"id": "p3", "tab_id": "t2", "session_id": "9899800ccc604e0da7bb74b6c2e42b2e"}
            ]
        });
        let m = parse_pane_tabs(&flat);
        assert_eq!(m.get("0762e1f78808469c8809535d4eb65068").map(String::as_str), Some("t1"));
        assert_eq!(m.get("8691e9de260040af9792b25feb1846d9").map(String::as_str), Some("t1"));
        assert_eq!(m.get("9899800ccc604e0da7bb74b6c2e42b2e").map(String::as_str), Some("t2"));

        let nested = serde_json::json!({
            "windows": [{"id": "w1", "tabs": [
                {"id": "tab-a", "panes": [{"session_id": "0762e1f78808469c8809535d4eb65068"}]},
                {"id": "tab-b", "panes": [{"sessionId": "9899800ccc604e0da7bb74b6c2e42b2e"}]}
            ]}]
        });
        let m = parse_pane_tabs(&nested);
        assert_eq!(m.get("0762e1f78808469c8809535d4eb65068").map(String::as_str), Some("tab-a"));
        assert_eq!(m.get("9899800ccc604e0da7bb74b6c2e42b2e").map(String::as_str), Some("tab-b"));
        assert!(parse_pane_tabs(&serde_json::json!({"ok": true})).is_empty());
    }
}
