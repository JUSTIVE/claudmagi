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
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat};


#[derive(Default)]
pub struct Warp {
    tabs: Mutex<Option<WarpTabs>>,
    /// Set once the first snapshot has waited on Warp; it never waits again.
    tabs_tried: std::sync::atomic::AtomicBool,
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

/// Runs a command with a wall-clock limit and returns its stdout on success.
/// Stdout is drained on a helper thread so a chatty child never blocks on a
/// full pipe.
fn run_with_timeout(mut cmd: std::process::Command, timeout: Duration) -> Option<String> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).ok().map(|_| text)
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    reader.join().ok().flatten()
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
