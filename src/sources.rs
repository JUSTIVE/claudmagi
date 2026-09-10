//! Where session facts come from. `ClaudeSource` reads the real registry
//! that Claude Code maintains (plus each session's subagent transcripts);
//! `FakeSource` is an in-memory list the test tools edit. The board only
//! ever sees `SessionSource::snapshot()`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::model::{Phase, SessionInfo, SubagentInfo};
use crate::pr;
use crate::ticket;

pub trait SessionSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Current sessions, oldest first. May block on IO; call off the UI thread.
    fn snapshot(&self) -> Vec<SessionInfo>;
}

/// The login name of whoever runs the board, uppercased for the badge (#15).
pub fn machine_user() -> String {
    let name = std::env::var("USER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| dirs::home_dir().and_then(|h| h.file_name().map(|s| s.to_string_lossy().into_owned())))
        .unwrap_or_else(|| "CLAUDMAGI".into());
    name.to_ascii_uppercase()
}

fn millis(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Real sessions: ~/.claude/sessions/<pid>.json
// ---------------------------------------------------------------------------

/// Finished subagents stay on the board this long after their last write.
pub const SUBAGENT_LINGER: Duration = Duration::from_secs(45);
/// A transcript untouched for this long is treated as finished no matter what.
const SUBAGENT_STALE: Duration = Duration::from_secs(30 * 60);
const TAIL_BYTES: u64 = 64 * 1024;

#[derive(Clone)]
struct SubCache {
    mtime: SystemTime,
    size: u64,
    info: SubagentInfo,
}

#[derive(Default)]
pub struct ClaudeSource {
    subs: Mutex<HashMap<PathBuf, SubCache>>,
    dirs: Mutex<HashMap<String, Option<PathBuf>>>,
    transcripts: Mutex<HashMap<String, Option<PathBuf>>>,
    tabs: Mutex<Option<WarpTabs>>,
    prs: pr::Tracker,
    tickets: ticket::Tracker,
    /// Set once the first snapshot has waited on Warp; it never waits again.
    tabs_tried: std::sync::atomic::AtomicBool,
}

impl SessionSource for ClaudeSource {
    fn name(&self) -> &'static str {
        "live"
    }

    fn snapshot(&self) -> Vec<SessionInfo> {
        let mut list = read_sessions();
        let tabs = self.warp_tabs_settled(&list);
        for s in &mut list {
            s.subagents = self.subagents_for(s);
            if let Some(uuid) = &s.warp_session_uuid {
                if let Some(tab) = tabs.get(&uuid.replace('-', "")) {
                    s.group = format!("warp-tab:{tab}");
                }
            }
        }
        self.attach_links(&mut list);
        list
    }
}

// ---------------------------------------------------------------------------
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

/// Reads the pane → tab map straight from Warp's database. `None` when no
/// database is present or `sqlite3` fails; an empty map when Warp simply has
/// no terminal panes open.
fn fetch_warp_tabs_from_db() -> Option<HashMap<String, String>> {
    let db = warp_db_candidates().into_iter().find(|p| p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false))?;
    let mut cmd = std::process::Command::new("/usr/bin/sqlite3");
    cmd.args(["-readonly", "-batch", "-noheader", "-separator", " "]).arg(&db).arg(WARP_DB_QUERY);
    let text = run_with_timeout(cmd, WARPCTRL_TIMEOUT)?;
    Some(parse_pane_rows(&text))
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
fn fetch_warp_tabs() -> Option<HashMap<String, String>> {
    fetch_warp_tabs_from_db().or_else(fetch_warp_tabs_from_warpctrl)
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

impl ClaudeSource {
    /// Cached pane → tab map; empty when neither Warp's database nor
    /// `warpctrl` can answer.
    fn warp_tabs(&self) -> HashMap<String, String> {
        let mut guard = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = match guard.as_ref() {
            Some(t) if t.good => WARP_TABS_TTL,
            _ => WARP_TABS_RETRY,
        };
        if guard.as_ref().is_none_or(|t| t.fetched.elapsed() > ttl) {
            match fetch_warp_tabs() {
                Some(map) if !map.is_empty() => {
                    *guard = Some(WarpTabs { map, fetched: Instant::now(), good: true });
                }
                // A miss keeps whatever good map we already had — Warp being
                // briefly unreadable is not the same as having no tabs (#59).
                _ => match guard.as_mut() {
                    Some(t) => t.fetched = Instant::now(),
                    None => *guard = Some(WarpTabs { map: HashMap::new(), fetched: Instant::now(), good: false }),
                },
            }
        }
        guard.as_ref().map(|t| t.map.clone()).unwrap_or_default()
    }

    /// Warp's pane → tab map, insisting on a real answer the first time it is
    /// needed. Sessions are grouped by it, so painting before it lands means
    /// showing a layout that is simply wrong and then re-laying it out (#59).
    /// After the first attempt this never blocks again, so a machine without
    /// Warp pays the cost once.
    fn warp_tabs_settled(&self, list: &[SessionInfo]) -> HashMap<String, String> {
        let mut tabs = self.warp_tabs();
        let wants = list.iter().any(|s| s.warp_session_uuid.is_some());
        let first = !self.tabs_tried.swap(true, std::sync::atomic::Ordering::Relaxed);
        if first && wants && tabs.is_empty() {
            for _ in 0..FIRST_TABS_TRIES {
                std::thread::sleep(FIRST_TABS_WAIT);
                tabs = self.warp_tabs();
                if !tabs.is_empty() {
                    break;
                }
            }
        }
        tabs
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSession {
    pid: i32,
    session_id: String,
    cwd: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    waiting_for: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    tempo: Option<String>,
    #[serde(default)]
    started_at: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawMeta {
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

pub fn config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    dirs::home_dir().map(|h| h.join(".claude"))
}

pub fn sessions_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("sessions"))
}

/// Claude Code names project directories after the cwd with every
/// non-alphanumeric character replaced by `-`.
pub fn project_slug(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let r = unsafe { libc::kill(pid, 0) };
    if r == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Reads the environment of another process of the same user via
/// `sysctl(KERN_PROCARGS2)`.
fn proc_env(pid: i32) -> Option<HashMap<String, String>> {
    const KERN_PROCARGS2: libc::c_int = 49;
    let mut mib = [libc::CTL_KERN, KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    unsafe {
        if libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
    }
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
    }
    buf.truncate(size);
    if buf.len() < 4 {
        return None;
    }
    let argc = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let mut i = 4;
    while i < buf.len() && buf[i] != 0 {
        i += 1;
    }
    while i < buf.len() && buf[i] == 0 {
        i += 1;
    }
    let mut skipped = 0;
    while skipped < argc && i < buf.len() {
        while i < buf.len() && buf[i] != 0 {
            i += 1;
        }
        i += 1;
        skipped += 1;
    }
    let mut out = HashMap::new();
    while i < buf.len() {
        let start = i;
        while i < buf.len() && buf[i] != 0 {
            i += 1;
        }
        if start == i {
            break;
        }
        let s = String::from_utf8_lossy(&buf[start..i]);
        if let Some((k, v)) = s.split_once('=') {
            out.insert(k.to_string(), v.to_string());
        }
        i += 1;
    }
    Some(out)
}

fn ttys_for(pids: &[i32]) -> HashMap<i32, String> {
    let mut map = HashMap::new();
    if pids.is_empty() {
        return map;
    }
    let list = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    if let Ok(out) = std::process::Command::new("ps").args(["-o", "pid=,tty=", "-p", &list]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut it = line.split_whitespace();
            if let (Some(pid), Some(tty)) = (it.next(), it.next()) {
                if let Ok(pid) = pid.parse::<i32>() {
                    if tty != "??" {
                        map.insert(pid, tty.to_string());
                    }
                }
            }
        }
    }
    map
}

/// Snapshot of every live session, oldest first (without subagents).
pub fn read_sessions() -> Vec<SessionInfo> {
    let Some(dir) = sessions_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };

    let mut raws = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(raw) = serde_json::from_str::<RawSession>(&text) else { continue };
        if !pid_alive(raw.pid) {
            continue;
        }
        raws.push(raw);
    }

    let pids: Vec<i32> = raws.iter().map(|r| r.pid).collect();
    let ttys = ttys_for(&pids);

    let mut out: Vec<SessionInfo> = raws
        .into_iter()
        .map(|r| {
            let env = proc_env(r.pid).unwrap_or_default();
            SessionInfo {
                pid: r.pid,
                session_id: r.session_id,
                cwd: r.cwd,
                name: r.name.unwrap_or_default(),
                status: r.status.unwrap_or_else(|| "idle".into()),
                waiting_for: r.waiting_for,
                state: r.state,
                tempo: r.tempo,
                started_at: r.started_at.unwrap_or(0),
                tty: ttys.get(&r.pid).cloned(),
                warp_focus_url: env.get("WARP_FOCUS_URL").cloned(),
                warp_session_uuid: env.get("WARP_TERMINAL_SESSION_UUID").cloned(),
                synthetic: false,
                subagents: Vec::new(),
                group: match (env.get("WARP_TERMINAL_SESSION_UUID"), env.get("TERM_PROGRAM")) {
                    (Some(uuid), _) => format!("warp:{uuid}"),
                    (None, Some(prog)) => format!("term:{prog}"),
                    (None, None) => "desktop".into(),
                },
                prs: Vec::new(),
                ticket: None,
            }
        })
        .collect();
    out.sort_by_key(|s| (s.started_at, s.pid));
    out
}

impl ClaudeSource {
    /// `~/.claude/projects/<slug>/<session>/subagents`, resolved once per
    /// session (with a scan fallback in case the slug rule differs).
    fn subagents_dir(&self, session: &SessionInfo) -> Option<PathBuf> {
        let mut dirs = self.dirs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = dirs.get(&session.session_id) {
            if d.as_ref().is_some_and(|p| p.is_dir()) {
                return d.clone();
            }
            // Not found last time: retry only occasionally by falling through.
        }
        let projects = config_dir()?.join("projects");
        let direct = projects.join(project_slug(&session.cwd)).join(&session.session_id).join("subagents");
        let found = if direct.is_dir() {
            Some(direct)
        } else {
            std::fs::read_dir(&projects).ok().and_then(|rd| {
                rd.flatten()
                    .map(|e| e.path().join(&session.session_id).join("subagents"))
                    .find(|p| p.is_dir())
            })
        };
        dirs.insert(session.session_id.clone(), found.clone());
        found
    }

    /// Fetches every PR in one snapshot instead of rationing them, for the
    /// `--prs` diagnostic. The board never wants this: it would stall the poll.
    pub fn eager() -> Self {
        let mut s = Self::default();
        s.prs.eager = true;
        s.tickets.eager = true;
        s
    }

    /// `~/.claude/projects/<slug>/<session>.jsonl`, the session's own
    /// transcript — the file the PR and Linear links are read out of (#56,
    /// #57). It is resolved on its own rather than through `subagents_dir`:
    /// that directory only exists once a session has spawned a subagent, so
    /// hanging the transcript off it left every subagent-less session with no
    /// PR and no ticket (#65).
    fn transcript(&self, session: &SessionInfo) -> Option<PathBuf> {
        let mut cache = self.transcripts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.get(&session.session_id) {
            if hit.as_ref().is_some_and(|p| p.is_file()) {
                return hit.clone();
            }
            // Not found last time: fall through and look again, since a
            // transcript appears as soon as the session writes its first turn.
        }
        let projects = config_dir()?.join("projects");
        let name = format!("{}.jsonl", session.session_id);
        let direct = projects.join(project_slug(&session.cwd)).join(&name);
        let found = if direct.is_file() {
            Some(direct)
        } else {
            std::fs::read_dir(&projects)
                .ok()
                .and_then(|rd| rd.flatten().map(|e| e.path().join(&name)).find(|p| p.is_file()))
        };
        cache.insert(session.session_id.clone(), found.clone());
        found
    }

    /// Attaches each session's pull request and Linear issue, sharing one `gh`
    /// budget across the snapshot so a board full of PRs never stalls the poll
    /// (#56). The ticket is resolved after the PR because a PR title's tag is
    /// one of the things that corroborates it (#57).
    fn attach_links(&self, list: &mut [SessionInfo]) {
        let paths: Vec<Option<PathBuf>> = list.iter().map(|s| self.transcript(s)).collect();
        let mut want_prs = Vec::new();
        for (s, transcript) in list.iter_mut().zip(&paths) {
            let refs = self.prs.resolve(&s.session_id, &s.cwd, transcript.as_deref());
            s.prs = refs.iter().filter_map(|id| self.prs.cached(id)).collect();
            want_prs.extend(refs);
            // Pool every transcript's Linear links before judging any of them,
            // so the team prefixes are all known by the time we pick (#57).
            self.tickets.scan(&s.session_id, transcript.as_deref());
        }
        // Naming what the board shows before reading it back means an eager
        // tracker has already fetched everything by the time we look (#64).
        self.prs.want(want_prs);
        for (s, transcript) in list.iter_mut().zip(&paths) {
            let refs = self.prs.resolve(&s.session_id, &s.cwd, transcript.as_deref());
            s.prs = refs.iter().filter_map(|id| self.prs.cached(id)).collect();
        }

        let titles: Vec<Vec<String>> = list.iter().map(|s| s.prs.iter().map(|p| p.title.clone()).collect()).collect();
        let want_keys: Vec<String> = list
            .iter()
            .zip(&titles)
            .filter_map(|(s, t)| self.tickets.key_for(&s.session_id, &s.label(), t))
            .collect();
        self.tickets.want(want_keys);
        for (s, t) in list.iter_mut().zip(&titles) {
            let name = s.label();
            s.ticket = self.tickets.pick(&s.session_id, &name, t);
        }
    }

    fn subagents_for(&self, session: &SessionInfo) -> Vec<SubagentInfo> {
        let Some(dir) = self.subagents_dir(session) else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
        let now = SystemTime::now();
        let mut out = Vec::new();
        let mut cache = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
            let size = meta.len();
            let info = match cache.get(&path) {
                Some(c) if c.mtime == mtime && c.size == size => c.info.clone(),
                _ => {
                    let info = read_subagent(&path, &meta, now);
                    cache.insert(path.clone(), SubCache { mtime, size, info: info.clone() });
                    info
                }
            };
            let recent = now.duration_since(mtime).map(|d| d < SUBAGENT_LINGER).unwrap_or(true);
            if info.running || recent {
                out.push(info);
            }
        }
        out.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.agent_id.cmp(&b.agent_id)));
        out
    }
}

fn read_subagent(path: &Path, meta: &std::fs::Metadata, now: SystemTime) -> SubagentInfo {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("agent-unknown");
    let agent_id = stem.strip_prefix("agent-").unwrap_or(stem).to_string();
    let raw_meta: RawMeta = std::fs::read_to_string(path.with_extension("meta.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
    let stale = now.duration_since(mtime).map(|d| d > SUBAGENT_STALE).unwrap_or(false);
    let running = !stale && transcript_running(path).unwrap_or(false);
    SubagentInfo {
        agent_id,
        agent_type: raw_meta.agent_type.unwrap_or_else(|| "agent".into()),
        description: raw_meta.description.unwrap_or_default(),
        running,
        started_at: millis(meta.created().unwrap_or(mtime)),
        updated_at: millis(mtime),
    }
}

/// Reads the tail of a subagent transcript and decides whether the agent is
/// still going: a trailing `SubagentStop` hook record or a final assistant
/// message without tool calls means done; anything else means running.
fn transcript_running(path: &Path) -> std::io::Result<bool> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 {
        lines.remove(0); // partial line
    }
    Ok(tail_running(lines.iter().rev().copied()))
}

/// `lines` are transcript records newest first.
pub fn tail_running<'a>(lines: impl Iterator<Item = &'a str>) -> bool {
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("attachment").and_then(|a| a.get("hookEvent")).and_then(|h| h.as_str()) == Some("SubagentStop") {
            return false;
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                let has_tool_use = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                    .is_some_and(|blocks| {
                        blocks.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
                    });
                return has_tool_use;
            }
            Some("user") => return true,
            _ => continue,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Sandbox sessions, edited from the test panel
// ---------------------------------------------------------------------------

const NAMES: [&str; 12] = [
    "PLAI", "PMBI", "SANI", "SAUI", "SMRI", "TOLI", "YOGI", "GENI", "FAKI", "MNAI", "LUWI", "BKBI",
];
const AGENT_TYPES: [(&str, &str); 5] = [
    ("Explore", "Find the relevant files"),
    ("oh-my-claudecode:executor", "Implement the change"),
    ("Plan", "Design the approach"),
    ("oh-my-claudecode:architect", "Review the architecture"),
    ("general-purpose", "Research the docs"),
];

pub const CHURN_INTERVAL: Duration = Duration::from_millis(1400);

struct FakeState {
    sessions: Vec<SessionInfo>,
    seq: u32,
    agent_seq: u32,
    auto: bool,
    last_churn: Option<Instant>,
    rng: u64,
}

impl FakeState {
    fn next_rand(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 8
    }

    fn push_session(&mut self, phase: Phase) -> String {
        self.seq += 1;
        let seq = self.seq;
        let name = format!("{}-{:02}", NAMES[(seq as usize - 1) % NAMES.len()], seq);
        let info = SessionInfo::synthetic(seq, &name, phase);
        let label = info.label();
        self.sessions.push(info);
        label
    }

    fn push_sub(&mut self, session_id: &str, running: bool) -> Option<String> {
        self.agent_seq += 1;
        let seq = self.agent_seq;
        let (ty, desc) = AGENT_TYPES[(seq as usize - 1) % AGENT_TYPES.len()];
        let s = self.sessions.iter_mut().find(|s| s.session_id == session_id)?;
        let info = SubagentInfo::synthetic(seq, ty, desc, running);
        let id = info.agent_id.clone();
        s.subagents.push(info);
        Some(id)
    }
}

/// A stable, distinct PR number per sandbox session, so connectors do not all
/// read the same (#56). Synthetic pids are `90_000 + seq`.
fn fake_pr_number(pid: i32) -> u32 {
    8700 + (pid - 90_000).max(0) as u32
}

/// In-memory sessions you can create, retarget and destroy at will.
pub struct FakeSource {
    inner: Mutex<FakeState>,
}

impl Default for FakeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSource {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(FakeState {
                sessions: Vec::new(),
                seq: 0,
                agent_seq: 0,
                auto: false,
                last_churn: None,
                rng: 0x9E37_79B9_7F4A_7C15,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a session in `phase`; returns its label.
    pub fn add(&self, phase: Phase) -> String {
        self.lock().push_session(phase)
    }

    pub fn fill(&self, n: usize) {
        for i in 0..n {
            self.add(Phase::ALL[i % 3]);
        }
    }

    pub fn remove(&self, session_id: &str) {
        self.lock().sessions.retain(|s| s.session_id != session_id);
    }

    pub fn clear(&self) {
        self.lock().sessions.clear();
    }

    pub fn set_phase(&self, session_id: &str, phase: Phase) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            s.set_phase(phase);
        }
    }

    /// Sets the sandbox session's first PR, or clears them all.
    pub fn set_pr(&self, session_id: &str, look: Option<pr::Look>) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            let n = s.prs.first().map(|p| p.id.number).unwrap_or_else(|| fake_pr_number(s.pid));
            s.prs = look.map(|l| vec![pr::Pr::synthetic(n, l)]).unwrap_or_default();
        }
    }

    /// Chains another synthetic PR onto the session, to exercise the case of
    /// one ticket with several (#63).
    pub fn add_pr(&self, session_id: &str, look: pr::Look) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            if s.prs.len() >= pr::MAX_PER_SESSION {
                return;
            }
            let n = s.prs.iter().map(|p| p.id.number).max().unwrap_or_else(|| fake_pr_number(s.pid));
            s.prs.push(pr::Pr::synthetic(n + 1, look));
        }
    }

    /// Toggles a synthetic Linear issue on a sandbox session (#57).
    /// Sets a sandbox session's Linear node, or removes it (#58).
    pub fn set_ticket(&self, session_id: &str, status: Option<Option<ticket::Status>>) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            let seq = (s.pid - 90_000).max(0) as u32;
            s.ticket = status.map(|st| ticket::Ticket::synthetic(seq, st));
        }
    }

    /// Puts a sandbox PR's CI in flight, or stops it (#62).
    pub fn set_pr_running(&self, session_id: &str, on: bool) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            for p in s.prs.iter_mut() {
                p.pending = if on { 2 } else { 0 };
            }
        }
    }

    pub fn cycle_ticket(&self, session_id: &str) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            s.ticket = match &s.ticket {
                None => Some(ticket::Ticket::synthetic((s.pid - 90_000).max(0) as u32, Some(ticket::Status::Started))),
                Some(_) => None,
            };
        }
    }

    pub fn cycle(&self, session_id: &str) {
        if let Some(s) = self.lock().sessions.iter_mut().find(|s| s.session_id == session_id) {
            let next = s.phase().next();
            s.set_phase(next);
        }
    }

    /// Spawns a synthetic subagent under `session_id`; returns its id.
    pub fn add_sub(&self, session_id: &str, running: bool) -> Option<String> {
        self.lock().push_sub(session_id, running)
    }

    pub fn remove_sub(&self, agent_id: &str) {
        for s in &mut self.lock().sessions {
            s.subagents.retain(|a| a.agent_id != agent_id);
        }
    }

    pub fn set_sub_running(&self, agent_id: &str, running: bool) {
        for s in &mut self.lock().sessions {
            if let Some(a) = s.subagents.iter_mut().find(|a| a.agent_id == agent_id) {
                a.running = running;
            }
        }
    }

    pub fn auto(&self) -> bool {
        self.lock().auto
    }

    pub fn set_auto(&self, on: bool) {
        let mut st = self.lock();
        st.auto = on;
        st.last_churn = None;
    }

    pub fn len(&self) -> usize {
        self.lock().sessions.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// When auto mode is on, every `CHURN_INTERVAL` one random thing happens:
    /// a subagent is spawned, finished or removed, a session is added or
    /// removed, or a session changes phase. Returns true if something changed.
    pub fn churn(&self, now: Instant) -> bool {
        let mut st = self.lock();
        if !st.auto {
            return false;
        }
        if st.last_churn.is_some_and(|t| now.duration_since(t) < CHURN_INTERVAL) {
            return false;
        }
        st.last_churn = Some(now);
        let roll = st.next_rand() % 100;
        let n = st.sessions.len();
        let pick = |st: &mut FakeState| (st.next_rand() % st.sessions.len().max(1) as u64) as usize;
        match roll {
            // 30%: spawn a subagent under a session that has room.
            0..=29 if n > 0 => {
                let idx = pick(&mut st);
                if st.sessions[idx].subagents.len() < 3 {
                    let id = st.sessions[idx].session_id.clone();
                    st.push_sub(&id, true);
                } else {
                    // Full: finish one instead so the chain keeps moving.
                    if let Some(a) = st.sessions[idx].subagents.iter_mut().find(|a| a.running) {
                        a.running = false;
                    }
                }
            }
            // 20%: a running subagent finishes (unplugs).
            30..=49 if n > 0 => {
                let idx = pick(&mut st);
                if let Some(a) = st.sessions[idx].subagents.iter_mut().find(|a| a.running) {
                    a.running = false;
                } else if !st.sessions[idx].subagents.is_empty() {
                    st.sessions[idx].subagents.remove(0);
                }
            }
            // 15%: a finished subagent disappears (oldest first).
            50..=64 if n > 0 => {
                let idx = pick(&mut st);
                let subs = &mut st.sessions[idx].subagents;
                if let Some(pos) = subs.iter().position(|a| !a.running) {
                    subs.remove(pos);
                } else if !subs.is_empty() {
                    subs[0].running = false;
                }
            }
            // 8%: a new session arrives.
            65..=72 if n < 14 => {
                let phase = Phase::ALL[(st.next_rand() % 3) as usize];
                st.push_session(phase);
            }
            // 7%: a session goes away.
            73..=79 if n > 2 => {
                let idx = pick(&mut st);
                st.sessions.remove(idx);
            }
            // Rest: a session changes phase.
            _ if n > 0 => {
                let idx = pick(&mut st);
                let next = st.sessions[idx].phase().next();
                st.sessions[idx].set_phase(next);
            }
            _ => {
                st.push_session(Phase::Working);
            }
        }
        true
    }
}

impl SessionSource for FakeSource {
    fn name(&self) -> &'static str {
        "sandbox"
    }

    fn snapshot(&self) -> Vec<SessionInfo> {
        self.lock().sessions.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_source_creates_retargets_and_destroys() {
        let src = FakeSource::new();
        let label = src.add(Phase::Working);
        assert_eq!(label, "PLAI-01");
        src.fill(2);
        assert_eq!(src.len(), 3);
        let snap = src.snapshot();
        assert_eq!(snap[0].phase(), Phase::Working);
        src.set_phase(&snap[0].session_id, Phase::Idle);
        assert_eq!(src.snapshot()[0].phase(), Phase::Idle);
        src.cycle(&snap[0].session_id);
        assert_eq!(src.snapshot()[0].phase(), Phase::Working);
        src.remove(&snap[1].session_id);
        assert_eq!(src.len(), 2);
        src.clear();
        assert!(src.is_empty());
    }

    #[test]
    fn fake_subagents_attach_to_their_session() {
        let src = FakeSource::new();
        src.add(Phase::Working);
        let sid = src.snapshot()[0].session_id.clone();
        let a = src.add_sub(&sid, true).unwrap();
        let b = src.add_sub(&sid, true).unwrap();
        assert_eq!(src.snapshot()[0].subagents.len(), 2);
        src.set_sub_running(&a, false);
        assert!(!src.snapshot()[0].subagents[0].running);
        src.remove_sub(&b);
        assert_eq!(src.snapshot()[0].subagents.len(), 1);
        assert!(src.add_sub("nope", true).is_none());
    }

    #[test]
    fn churn_only_runs_in_auto_mode_and_rate_limits() {
        let src = FakeSource::new();
        src.fill(3);
        let t0 = Instant::now();
        assert!(!src.churn(t0));
        src.set_auto(true);
        assert!(src.churn(t0));
        assert!(!src.churn(t0 + Duration::from_millis(100)));
        assert!(src.churn(t0 + CHURN_INTERVAL + Duration::from_millis(1)));
        assert!(src.snapshot().iter().all(|s| s.synthetic));
    }

    #[test]
    fn churn_spawns_finishes_and_removes_subagents() {
        let src = FakeSource::new();
        src.fill(3);
        src.set_auto(true);
        let mut t = Instant::now();
        let (mut spawned, mut finished, mut removed) = (0, 0, 0);
        let mut prev = src.snapshot();
        for _ in 0..300 {
            t += CHURN_INTERVAL + Duration::from_millis(1);
            assert!(src.churn(t));
            let cur = src.snapshot();
            for s in &cur {
                if let Some(p) = prev.iter().find(|p| p.session_id == s.session_id) {
                    spawned += s.subagents.len().saturating_sub(p.subagents.len());
                    removed += p.subagents.len().saturating_sub(s.subagents.len());
                    finished += s
                        .subagents
                        .iter()
                        .filter(|a| !a.running && p.subagents.iter().any(|b| b.agent_id == a.agent_id && b.running))
                        .count();
                }
            }
            prev = cur;
        }
        assert!(spawned >= 20, "spawned {spawned}");
        assert!(finished >= 10, "finished {finished}");
        assert!(removed >= 10, "removed {removed}");
        assert!(src.snapshot().iter().all(|s| s.subagents.len() <= 3));
    }

    #[test]
    fn transcript_tail_classifies_running_and_done() {
        let done = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"all done"}]}}"#;
        let tool = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#;
        let user = r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#;
        let stop = r#"{"type":"attachment","attachment":{"hookEvent":"SubagentStop"}}"#;
        let junk = r#"{"type":"system","subtype":"turn_duration"}"#;
        assert!(!tail_running([done].into_iter()));
        assert!(tail_running([tool].into_iter()));
        assert!(tail_running([user].into_iter()));
        assert!(tail_running([junk, user].into_iter()));
        assert!(!tail_running([stop, tool].into_iter()), "stop hook after a tool call means finished");
        assert!(tail_running([user, done].into_iter()), "a new prompt after the final answer resumes it");
    }

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

    #[test]
    fn project_slug_matches_claude_code_naming() {
        assert_eq!(project_slug("/Users/ben/git/personal"), "-Users-ben-git-personal");
        assert_eq!(project_slug("/Users/ben/.claude/x"), "-Users-ben--claude-x");
    }
}
