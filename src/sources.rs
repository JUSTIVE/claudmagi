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
}

impl SessionSource for ClaudeSource {
    fn name(&self) -> &'static str {
        "live"
    }

    fn snapshot(&self) -> Vec<SessionInfo> {
        let mut list = read_sessions();
        for s in &mut list {
            s.subagents = self.subagents_for(s);
        }
        list
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
    fn project_slug_matches_claude_code_naming() {
        assert_eq!(project_slug("/Users/ben/git/personal"), "-Users-ben-git-personal");
        assert_eq!(project_slug("/Users/ben/.claude/x"), "-Users-ben--claude-x");
    }
}
