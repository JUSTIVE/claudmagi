//! The Linear issue a session is working on (#57).
//!
//! Sessions here are named after their ticket (`PJM-1953`), which is the
//! strongest signal there is — the user typed it. It is not sufficient on its
//! own, though: `YOSHI-60` has the shape of an issue key without being one. So
//! a name-derived key is only believed when something else repeats it, either
//! a `linear.app` link in the transcript or the ticket tag GitHub PR titles
//! carry. Sessions named something else fall back to those two sources.
//!
//! There is no state here, only identity. `orca linear issue <KEY> --json`
//! would give status and assignee, but on this machine it answers
//! `runtime_unavailable` — the same dead end as `warpctrl` in #43 — so the
//! node shows the key and links out to it.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::pr::tail;

/// How much of a transcript's tail is searched, matching the PR scan.
const TAIL_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// `PJM-1953`.
    pub key: String,
    /// The Linear workspace slug, when one has been seen.
    pub workspace: Option<String>,
}

impl Ticket {
    pub fn label(&self) -> String {
        self.key.clone()
    }

    /// Where a click goes. Without a workspace slug there is no URL to build.
    pub fn url(&self) -> Option<String> {
        self.workspace.as_ref().map(|w| format!("https://linear.app/{w}/issue/{}", self.key))
    }

    pub fn synthetic(seq: u32) -> Self {
        Self { key: format!("PJM-{}", 1900 + seq), workspace: None }
    }
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
#[derive(Default)]
pub struct Tracker {
    /// session id → (transcript size when scanned, keys, workspace).
    seen: Mutex<HashMap<String, (u64, Vec<String>, Option<String>)>>,
    /// The workspace slug seen most often, shared by every session.
    workspace: Mutex<HashMap<String, usize>>,
    /// Team prefixes that have appeared in a real `linear.app` link, pooled
    /// across sessions — the board's answer to "is this a Linear team?".
    teams: Mutex<HashSet<String>>,
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

    /// The issue for a session, from what `scan` has pooled so far.
    pub fn pick(&self, session_id: &str, name: &str, pr_title: Option<&str>) -> Option<Ticket> {
        let linked = self
            .seen
            .lock()
            .ok()
            .and_then(|m| m.get(session_id).map(|(_, keys, _)| keys.clone()))
            .unwrap_or_default();
        let teams = self.teams.lock().map(|t| t.clone()).unwrap_or_default();
        let key = choose(name, pr_title, &linked, &teams)?;
        Some(Ticket { key, workspace: self.workspace() })
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
