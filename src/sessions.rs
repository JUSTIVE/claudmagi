//! Discovers live Claude Code sessions from `~/.claude/sessions/<pid>.json`
//! and derives a display phase for each.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Claude is working; the chip stays plugged in.
    Working,
    /// A permission dialog / question is waiting on the user.
    NeedsUser,
    /// The turn finished; the session is idle at the prompt.
    Idle,
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub pid: i32,
    pub session_id: String,
    pub cwd: String,
    pub name: String,
    pub status: String,
    pub waiting_for: Option<String>,
    pub state: Option<String>,
    pub tempo: Option<String>,
    pub detail: Option<String>,
    pub started_at: u64,
    pub status_updated_at: u64,
    pub kind: String,
    pub version: Option<String>,
    pub tty: Option<String>,
    pub warp_focus_url: Option<String>,
    pub warp_session_uuid: Option<String>,
}

impl SessionInfo {
    pub fn phase(&self) -> Phase {
        if self.waiting_for.is_some() {
            return Phase::NeedsUser;
        }
        if self.status == "busy" {
            return Phase::Working;
        }
        let blocked = |v: &Option<String>| v.as_deref() == Some("blocked");
        if blocked(&self.tempo) || blocked(&self.state) {
            Phase::NeedsUser
        } else {
            Phase::Idle
        }
    }

    pub fn label(&self) -> String {
        let raw = if self.name.trim().is_empty() {
            PathBuf::from(&self.cwd)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("PID{}", self.pid))
        } else {
            self.name.clone()
        };
        raw.to_ascii_uppercase()
    }

    pub fn short_cwd(&self) -> String {
        if let Some(home) = dirs::home_dir() {
            if let Ok(rest) = PathBuf::from(&self.cwd).strip_prefix(&home) {
                return format!("~/{}", rest.display());
            }
        }
        self.cwd.clone()
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
    detail: Option<String>,
    #[serde(default)]
    started_at: Option<u64>,
    #[serde(default)]
    status_updated_at: Option<u64>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    version: Option<String>,
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
                detail: r.detail,
                started_at: r.started_at.unwrap_or(0),
                status_updated_at: r.status_updated_at.unwrap_or(0),
                kind: r.kind.unwrap_or_else(|| "interactive".into()),
                version: r.version,
                tty: ttys.get(&r.pid).cloned(),
                warp_focus_url: env.get("WARP_FOCUS_URL").cloned(),
                warp_session_uuid: env.get("WARP_TERMINAL_SESSION_UUID").cloned(),
            }
        })
        .collect();
    out.sort_by_key(|s| (s.started_at, s.pid));
    out
}
