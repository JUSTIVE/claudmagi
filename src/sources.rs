//! Where session facts come from. `ClaudeSource` reads the real registry
//! that Claude Code maintains; `FakeSource` is an in-memory list the test
//! tools edit. The board only ever sees `SessionSource::snapshot()`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::model::{Phase, SessionInfo};

pub trait SessionSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Current sessions, oldest first. May block on IO; call off the UI thread.
    fn snapshot(&self) -> Vec<SessionInfo>;
}

// ---------------------------------------------------------------------------
// Real sessions: ~/.claude/sessions/<pid>.json
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ClaudeSource;

impl SessionSource for ClaudeSource {
    fn name(&self) -> &'static str {
        "live"
    }

    fn snapshot(&self) -> Vec<SessionInfo> {
        read_sessions()
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

pub fn sessions_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir).join("sessions"));
    }
    dirs::home_dir().map(|h| h.join(".claude").join("sessions"))
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

/// Snapshot of every live session, oldest first.
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
            }
        })
        .collect();
    out.sort_by_key(|s| (s.started_at, s.pid));
    out
}

// ---------------------------------------------------------------------------
// Sandbox sessions, edited from the test panel
// ---------------------------------------------------------------------------

const NAMES: [&str; 12] = [
    "PLAI", "PMBI", "SANI", "SAUI", "SMRI", "TOLI", "YOGI", "GENI", "FAKI", "MNAI", "LUWI", "BKBI",
];

pub const CHURN_INTERVAL: Duration = Duration::from_millis(1400);

struct FakeState {
    sessions: Vec<SessionInfo>,
    seq: u32,
    auto: bool,
    last_churn: Option<Instant>,
    rng: u64,
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
        let mut st = self.lock();
        st.seq += 1;
        let seq = st.seq;
        let name = format!("{}-{:02}", NAMES[(seq as usize - 1) % NAMES.len()], seq);
        let info = SessionInfo::synthetic(seq, &name, phase);
        let label = info.label();
        st.sessions.push(info);
        label
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

    /// When auto mode is on, randomly retargets / adds / removes a session
    /// every `CHURN_INTERVAL`. Returns true if something changed.
    pub fn churn(&self, now: Instant) -> bool {
        let mut st = self.lock();
        if !st.auto {
            return false;
        }
        if st.last_churn.is_some_and(|t| now.duration_since(t) < CHURN_INTERVAL) {
            return false;
        }
        st.last_churn = Some(now);
        let roll = st.next_rand();
        let n = st.sessions.len();
        match roll % 10 {
            0 if n < 14 => {
                st.seq += 1;
                let seq = st.seq;
                let name = format!("{}-{:02}", NAMES[(seq as usize - 1) % NAMES.len()], seq);
                let phase = Phase::ALL[(roll / 10 % 3) as usize];
                st.sessions.push(SessionInfo::synthetic(seq, &name, phase));
            }
            1 if n > 2 => {
                let idx = (st.next_rand() % n as u64) as usize;
                st.sessions.remove(idx);
            }
            _ if n > 0 => {
                let idx = (st.next_rand() % n as u64) as usize;
                let next = st.sessions[idx].phase().next();
                st.sessions[idx].set_phase(next);
            }
            _ => {
                st.seq += 1;
                let seq = st.seq;
                let name = format!("{}-{:02}", NAMES[(seq as usize - 1) % NAMES.len()], seq);
                st.sessions.push(SessionInfo::synthetic(seq, &name, Phase::Working));
            }
        }
        true
    }
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
}
