//! The Linear issue a session is working on (#57).
//!
//! Sessions here are named after their ticket (`PJM-1953`), which is the
//! strongest signal there is — the user typed it. It is not sufficient on its
//! own, though: `YOSHI-60` has the shape of an issue key without being one. So
//! a name-derived key is only believed when something else repeats it, either
//! a `linear.app` link in the transcript or the ticket tag GitHub PR titles
//! carry. Sessions named something else fall back to those two sources.
//!
//! Status comes from `orca linear issue <KEY> --json`, which works whenever
//! the Orca app is running — unlike `warpctrl` in #43, this one has a
//! documented way to turn it on. With Orca closed the CLI answers
//! `runtime_unavailable` and the node falls back to showing just the key, so
//! the board degrades instead of breaking. Failures are cached for longer than
//! hits so a closed Orca is not re-asked every second. (#58)

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::pr::tail;

/// How much of a transcript's tail is searched, matching the PR scan.
const TAIL_BYTES: u64 = 2 * 1024 * 1024;
/// How long a fetched issue stays fresh.
pub const TTL: Duration = Duration::from_secs(60);
/// How long a failure sticks. Orca is usually closed rather than briefly
/// unhappy, and retrying every minute per ticket would be pure noise.
pub const MISS_TTL: Duration = Duration::from_secs(5 * 60);
/// `orca` calls allowed per snapshot, matching the `gh` ration.
const FETCH_BUDGET: usize = 2;

/// Linear's canonical workflow state types, which every custom status maps on
/// to. Names and colours are per team; the type is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// `triage` and `backlog`: not picked up.
    Backlog,
    /// `unstarted`: ready for someone.
    Todo,
    /// `started`: in progress.
    Started,
    /// `completed`.
    Done,
    /// `canceled` and `duplicate`.
    Cancelled,
}

impl Status {
    pub const ALL: [Status; 5] = [Status::Backlog, Status::Todo, Status::Started, Status::Done, Status::Cancelled];

    pub fn from_type(t: &str) -> Option<Status> {
        Some(match t {
            "triage" | "backlog" => Status::Backlog,
            "unstarted" => Status::Todo,
            "started" => Status::Started,
            "completed" => Status::Done,
            "canceled" | "duplicate" => Status::Cancelled,
            _ => return None,
        })
    }

    pub fn short(self) -> &'static str {
        match self {
            Status::Backlog => "BACKLOG",
            Status::Todo => "TODO",
            Status::Started => "DOING",
            Status::Done => "DONE",
            Status::Cancelled => "CANCELLED",
        }
    }

    /// Single letter for the test panel's state row.
    pub fn letter(self) -> &'static str {
        match self {
            Status::Backlog => "B",
            Status::Todo => "T",
            Status::Started => "S",
            Status::Done => "D",
            Status::Cancelled => "C",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// `PJM-1953`.
    pub key: String,
    /// The Linear workspace slug, when one has been seen in a link.
    pub workspace: Option<String>,
    /// Filled in once Orca answers; `None` while it is closed.
    pub status: Option<Status>,
    /// The team's own name for the status, e.g. `🔨진행 중`.
    pub state_name: String,
    pub title: String,
    /// Linear's own URL for the issue, which beats one built from a slug.
    pub canonical_url: Option<String>,
}

impl Ticket {
    pub fn label(&self) -> String {
        self.key.clone()
    }

    /// The web URL: Linear's own when known, else one built from a slug seen
    /// in the transcript.
    pub fn url(&self) -> Option<String> {
        self.canonical_url
            .clone()
            .or_else(|| self.workspace.as_ref().map(|w| format!("https://linear.app/{w}/issue/{}", self.key)))
    }

    /// The workspace slug, preferring the one inside Linear's own URL.
    fn slug(&self) -> Option<String> {
        let from_url = self
            .canonical_url
            .as_deref()
            .and_then(|u| u.strip_prefix("https://linear.app/"))
            .and_then(|rest| rest.split('/').next())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        from_url.or_else(|| self.workspace.clone())
    }

    /// Deep link into the desktop app. macOS routes the `linear` scheme to
    /// Linear.app, which registers it at runtime rather than in its
    /// Info.plist. The canonical URL's trailing title slug is dropped: the
    /// key alone resolves, and the slug carries non-ASCII (#60).
    pub fn app_url(&self) -> Option<String> {
        Some(format!("linear://{}/issue/{}", self.slug()?, self.key))
    }

    pub fn new(key: String, workspace: Option<String>) -> Self {
        Self { key, workspace, status: None, state_name: String::new(), title: String::new(), canonical_url: None }
    }

    pub fn synthetic(seq: u32, status: Option<Status>) -> Self {
        let key = format!("PJM-{}", 1900 + seq);
        Self {
            state_name: status.map(|s| s.short().to_string()).unwrap_or_default(),
            title: format!("synthetic issue {key}"),
            status,
            canonical_url: None,
            workspace: None,
            key,
        }
    }
}

/// Asks Orca for one issue. `None` when Orca is closed, the CLI is missing, or
/// the key does not resolve — all of which just mean "no status on the node".
pub fn fetch(key: &str) -> Option<Ticket> {
    let out = Command::new("orca").args(["linear", "issue", key, "--json"]).output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        return None;
    }
    let issue = v.get("result")?.get("issue")?;
    let state = issue.get("state");
    let text = |v: Option<&serde_json::Value>, k: &str| {
        v.and_then(|v| v.get(k)).and_then(|s| s.as_str()).unwrap_or_default().to_string()
    };
    Some(Ticket {
        key: text(Some(issue), "identifier"),
        workspace: None,
        status: state.and_then(|s| s.get("type")).and_then(|t| t.as_str()).and_then(Status::from_type),
        state_name: text(state, "name"),
        title: text(Some(issue), "title"),
        canonical_url: issue.get("url").and_then(|u| u.as_str()).map(|s| s.to_string()),
    })
}

/// `ABC-123` shape, uppercased. Deliberately loose: it says the string could
/// be an issue key, not that it is one.
pub fn key_shaped(name: &str) -> Option<String> {
    let up = name.trim().to_ascii_uppercase();
    let (team, number) = up.split_once('-')?;
    let team_ok = (2..=10).contains(&team.len())
        && team.starts_with(|c: char| c.is_ascii_alphabetic())
        && team.chars().all(|c| c.is_ascii_alphanumeric());
    let number_ok = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    (team_ok && number_ok).then_some(up)
}

/// Issue keys tagged in a PR title, e.g. `💄 [YO] [PJM-1953] 드롭다운`. This is
/// shape only, so it also returns things like `UTF-8`; `choose` is what
/// decides which of them is a real ticket.
pub fn keys_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == '-' {
            let dash = i;
            i += 1;
            let digits = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > digits {
                let word: String = bytes[start..i].iter().collect();
                // A key never runs straight into more letters (`utf-8mode`).
                let clean = i >= bytes.len() || !bytes[i].is_ascii_alphanumeric();
                if clean {
                    if let Some(k) = key_shaped(&word) {
                        out.push(k);
                    }
                }
                continue;
            }
            i = dash + 1;
        }
    }
    out
}

/// Every `linear.app/<workspace>/issue/<KEY>` in the transcript's tail, newest
/// last, with the workspace slugs they came from.
pub fn scan_transcript(path: &Path) -> (Vec<String>, Vec<String>) {
    let Some(text) = tail(path, TAIL_BYTES) else { return (Vec::new(), Vec::new()) };
    const NEEDLE: &str = "linear.app/";
    let (mut keys, mut spaces) = (Vec::new(), Vec::new());
    let mut from = 0;
    while let Some(i) = text[from..].find(NEEDLE) {
        let at = from + i + NEEDLE.len();
        from = at;
        let rest = &text[at..];
        let Some((workspace, tail_of)) = rest.split_once("/issue/") else { continue };
        if workspace.is_empty() || workspace.contains('/') || workspace.len() > 60 {
            continue;
        }
        let word: String = tail_of.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        if let Some(k) = key_shaped(&word) {
            keys.push(k);
            spaces.push(workspace.to_string());
        }
    }
    (keys, spaces)
}

/// Picks the issue for a session. The name wins when it is believable — the
/// transcript or the PR title repeats it, or its team prefix is one the board
/// has seen in a real Linear link. Otherwise the PR title's tag, and only then
/// whatever the session linked to last. `teams` is what keeps `YOSHI-60` from
/// passing for a ticket while `PJM-1924` goes through untouched.
pub fn choose(name: &str, pr_title: Option<&str>, linked: &[String], teams: &HashSet<String>) -> Option<String> {
    let from_title: Vec<String> = pr_title.map(keys_in).unwrap_or_default();
    let known = |key: &str| key.split_once('-').is_some_and(|(team, _)| teams.contains(team));
    if let Some(named) = key_shaped(name) {
        if linked.contains(&named) || from_title.contains(&named) || known(&named) {
            return Some(named);
        }
    }
    from_title
        .iter()
        .find(|k| teams.is_empty() || known(k))
        .cloned()
        .or_else(|| linked.last().cloned())
}

/// The team half of an issue key.
fn team_of(key: &str) -> Option<String> {
    key.split_once('-').map(|(t, _)| t.to_string())
}

/// Per-session issue resolution, rescanned only when a transcript grows.
pub struct Tracker {
    /// session id → (transcript size when scanned, keys, workspace).
    seen: Mutex<HashMap<String, (u64, Vec<String>, Option<String>)>>,
    /// The workspace slug seen most often, shared by every session.
    workspace: Mutex<HashMap<String, usize>>,
    /// Team prefixes that have appeared in a real `linear.app` link, pooled
    /// across sessions — the board's answer to "is this a Linear team?".
    teams: Mutex<HashSet<String>>,
    /// Issue key → what Orca last said, and when. `None` is cached too, for
    /// longer, so a closed Orca is not re-asked constantly (#58).
    issues: Mutex<HashMap<String, (Option<Ticket>, Instant)>>,
    /// `orca` calls allowed per snapshot; `--prs` lifts it.
    pub budget: usize,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            budget: FETCH_BUDGET,
            seen: Mutex::default(),
            workspace: Mutex::default(),
            teams: Mutex::default(),
            issues: Mutex::default(),
        }
    }
}

impl Tracker {
    /// Reads a transcript once, pooling its workspace slugs and team prefixes.
    /// Every session is scanned before any is judged, so the answer does not
    /// depend on which session the snapshot happened to reach first.
    pub fn scan(&self, session_id: &str, transcript: Option<&Path>) {
        let Some(path) = transcript else { return };
        let size = std::fs::metadata(path).ok().map(|m| m.len()).unwrap_or(0);
        if self.seen.lock().is_ok_and(|m| m.get(session_id).is_some_and(|(seen, ..)| *seen == size)) {
            return;
        }
        let (keys, spaces) = scan_transcript(path);
        if let Ok(mut tally) = self.workspace.lock() {
            for w in &spaces {
                *tally.entry(w.clone()).or_insert(0) += 1;
            }
        }
        if let Ok(mut teams) = self.teams.lock() {
            teams.extend(keys.iter().filter_map(|k| team_of(k)));
        }
        if let Ok(mut m) = self.seen.lock() {
            m.insert(session_id.to_string(), (size, keys, spaces.last().cloned()));
        }
    }

    /// The issue for a session, from what `scan` has pooled so far, with its
    /// Linear status when Orca could be reached (#58).
    pub fn pick(&self, session_id: &str, name: &str, pr_title: Option<&str>, budget: &mut usize, now: Instant) -> Option<Ticket> {
        let linked = self
            .seen
            .lock()
            .ok()
            .and_then(|m| m.get(session_id).map(|(_, keys, _)| keys.clone()))
            .unwrap_or_default();
        let teams = self.teams.lock().map(|t| t.clone()).unwrap_or_default();
        let key = choose(name, pr_title, &linked, &teams)?;
        let workspace = self.workspace();
        Some(match self.state(&key, budget, now) {
            Some(mut issue) => {
                issue.workspace = workspace;
                issue
            }
            None => Ticket::new(key, workspace),
        })
    }

    /// Cached Linear state for an issue, rationed like the PR fetches.
    fn state(&self, key: &str, budget: &mut usize, now: Instant) -> Option<Ticket> {
        if let Ok(map) = self.issues.lock() {
            if let Some((hit, at)) = map.get(key) {
                let ttl = if hit.is_some() { TTL } else { MISS_TTL };
                if now.duration_since(*at) < ttl {
                    return hit.clone();
                }
            }
        }
        if *budget == 0 {
            return self.issues.lock().ok()?.get(key).and_then(|(hit, _)| hit.clone());
        }
        *budget -= 1;
        let fetched = fetch(key);
        if let Ok(mut map) = self.issues.lock() {
            map.insert(key.to_string(), (fetched.clone(), now));
        }
        fetched
    }

    /// The most-seen workspace slug. Example text in skills and docs mentions
    /// other workspaces, so the popular one wins rather than the first.
    pub fn workspace(&self) -> Option<String> {
        let tally = self.workspace.lock().ok()?;
        tally.iter().max_by_key(|(_, n)| **n).map(|(w, _)| w.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_link_drops_the_title_slug() {
        let mut t = Ticket::new("PJM-1953".into(), Some("fallback".into()));
        t.canonical_url = Some("https://linear.app/cookieplace/issue/PJM-1953/combobox-list".into());
        assert_eq!(t.app_url().as_deref(), Some("linear://cookieplace/issue/PJM-1953"));
        assert_eq!(t.url().as_deref(), Some("https://linear.app/cookieplace/issue/PJM-1953/combobox-list"));

        // Orca closed: the slug seen in the transcript still builds a link.
        let bare = Ticket::new("PJM-1953".into(), Some("cookieplace".into()));
        assert_eq!(bare.app_url().as_deref(), Some("linear://cookieplace/issue/PJM-1953"));
        assert_eq!(Ticket::new("PJM-1".into(), None).app_url(), None, "no workspace, no link");
    }

    #[test]
    fn issue_keys_need_a_team_and_a_number() {
        assert_eq!(key_shaped("PJM-1953").as_deref(), Some("PJM-1953"));
        assert_eq!(key_shaped("pjm-1953").as_deref(), Some("PJM-1953"), "session names arrive lowercased");
        assert_eq!(key_shaped("YOSHI-60").as_deref(), Some("YOSHI-60"), "shape alone cannot rule this out");
        assert_eq!(key_shaped("YOSHI-DARK"), None);
        assert_eq!(key_shaped("claudmagi-f6"), None);
        assert_eq!(key_shaped("A-1"), None, "one-letter teams are not a thing");
        assert_eq!(key_shaped("PJM1953"), None);
    }

    #[test]
    fn titles_give_up_their_tags() {
        assert_eq!(keys_in("💄 [YO] [PJM-1953] 공통 드롭다운"), vec!["PJM-1953"]);
        assert_eq!(keys_in("🐛 [YO][NATIVE] [PJM-1974] 웹뷰"), vec!["PJM-1974"]);
        assert!(keys_in("nothing to see").is_empty());
        assert_eq!(keys_in("utf-8 and x-1"), vec!["UTF-8"], "shape only: one-letter teams are out, UTF is not");
    }

    #[test]
    fn a_named_session_needs_corroboration() {
        let none = HashSet::new();
        let known: HashSet<String> = ["PJM".to_string(), "DEV".to_string()].into_iter().collect();
        // The name is right and the PR title repeats it.
        assert_eq!(choose("PJM-1974", Some("🐛 [YO] [PJM-1974] 웹뷰"), &[], &none).as_deref(), Some("PJM-1974"));
        // The name is right and the transcript linked it.
        assert_eq!(choose("pjm-1953", None, &["PJM-1953".into()], &none).as_deref(), Some("PJM-1953"));
        // Neither, but PJM is a team the board has seen: believe the name
        // rather than some other ticket the session merely read about.
        assert_eq!(
            choose("PJM-1924", None, &["DEV-5066".into()], &known).as_deref(),
            Some("PJM-1924"),
            "a known team vouches for the name"
        );
        // Key-shaped, unknown team, nothing backs it up.
        assert_eq!(choose("YOSHI-60", Some("🐛 [YO] 비-GraphQL 분리"), &[], &known), None);
        // Not key-shaped at all: fall back to the PR's tag.
        assert_eq!(choose("yoshi-f2", Some("✨ [YO] [PJM-1991] 실물 확인"), &[], &known).as_deref(), Some("PJM-1991"));
        // A title tag whose team is unknown is not preferred over nothing.
        assert_eq!(choose("scratch", Some("bump utf-8 handling"), &[], &known), None);
        // Nothing anywhere.
        assert_eq!(choose("claudmagi-f6", None, &[], &known), None);
        // No name match, no title: the last thing it linked to.
        assert_eq!(
            choose("scratch", None, &["PJM-1284".into(), "DEV-5108".into()], &known).as_deref(),
            Some("DEV-5108")
        );
    }
}
