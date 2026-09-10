//! The GitHub pull request a session is working on (#56).
//!
//! Nothing in the session registry names a PR, and `cwd` is no help: every
//! session on this machine can sit in the same checkout on the same branch, so
//! `cwd → branch → PR` resolves them all to the same answer. The link that
//! does work is the session's own transcript — when a session creates or
//! inspects a PR the URL lands in a message. Two filters make that signal
//! clean:
//!
//! 1. Only look inside `message.content` blocks. A raw grep also finds links
//!    injected into every session's context, which show up as one popular PR
//!    number attached to sessions that never touched it.
//! 2. Only accept URLs for the session's own repository, or a session picks up
//!    a PR it merely read about in another repo.
//!
//! State comes from `gh`, cached with a TTL and rationed per snapshot so the
//! poll loop never stalls behind the network.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a fetched PR stays fresh before `gh` is asked again.
pub const TTL: Duration = Duration::from_secs(60);
/// `gh` calls allowed per snapshot. One takes about a second, so the budget
/// keeps a poll from turning into a stall; the board fills in over a few ticks.
const FETCH_BUDGET: usize = 2;
/// How much of a transcript's tail is searched. PRs are created and discussed
/// late in a session, and the whole file can run to tens of megabytes.
const TAIL_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrRef {
    /// `owner/repo`.
    pub repo: String,
    pub number: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Draft,
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pr {
    pub id: PrRef,
    pub title: String,
    pub state: State,
    /// Checks that concluded green, and ones that concluded red. Cancelled and
    /// skipped runs count as neither — a cancelled job is not a failure.
    pub passed: u32,
    pub failed: u32,
}

/// How the board draws a connector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Outline only: not asking for review yet.
    Draft,
    /// Filled: open and nothing is red.
    Open,
    /// Red pins: at least one check failed.
    Failing,
    /// Sunk into the board: landed.
    Merged,
    /// Dim outline: closed without merging.
    Closed,
}

impl Pr {
    pub fn look(&self) -> Look {
        match self.state {
            State::Merged => Look::Merged,
            State::Closed => Look::Closed,
            _ if self.failed > 0 => Look::Failing,
            State::Draft => Look::Draft,
            State::Open => Look::Open,
        }
    }

    /// What the connector is labelled with.
    pub fn label(&self) -> String {
        format!("#{}", self.id.number)
    }

    /// A made-up PR for the sandbox, one per look.
    pub fn synthetic(number: u32, look: Look) -> Self {
        let (state, passed, failed) = match look {
            Look::Draft => (State::Draft, 3, 0),
            Look::Open => (State::Open, 12, 0),
            Look::Failing => (State::Open, 8, 2),
            Look::Merged => (State::Merged, 14, 0),
            Look::Closed => (State::Closed, 0, 0),
        };
        Self {
            id: PrRef { repo: "sandbox/demo".into(), number },
            title: format!("synthetic pull request {number}"),
            state,
            passed,
            failed,
        }
    }
}

impl Look {
    pub const ALL: [Look; 5] = [Look::Draft, Look::Open, Look::Failing, Look::Merged, Look::Closed];

    pub fn short(self) -> &'static str {
        match self {
            Look::Draft => "DRAFT",
            Look::Open => "OPEN",
            Look::Failing => "FAIL",
            Look::Merged => "MERGED",
            Look::Closed => "CLOSED",
        }
    }

    pub fn next(self) -> Look {
        let i = Self::ALL.iter().position(|l| *l == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// `owner/repo` out of a remote URL, for the GitHub forms git actually emits.
pub fn parse_repo(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let name = parts.next().filter(|s| !s.is_empty())?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

fn repo_of(cwd: &str) -> Option<String> {
    let out = Command::new("git").args(["-C", cwd, "remote", "get-url", "origin"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_repo(String::from_utf8_lossy(&out.stdout).trim())
}

/// The last PR of `repo` mentioned in a message body in the transcript's tail.
pub fn scan_transcript(path: &Path, repo: &str) -> Option<u32> {
    let text = tail(path, TAIL_BYTES)?;
    let needle = format!("github.com/{repo}/pull/");
    let mut found = None;
    for line in text.lines() {
        // Cheap gate: parsing every record of a two-megabyte tail is the
        // expensive part, and almost no line mentions a PR.
        if !line.contains(&needle) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) else {
            continue;
        };
        for block in blocks {
            if let Some(n) = last_number(&block.to_string(), &needle) {
                found = Some(n);
            }
        }
    }
    found
}

fn last_number(haystack: &str, needle: &str) -> Option<u32> {
    let mut out = None;
    let mut from = 0;
    while let Some(i) = haystack[from..].find(needle) {
        let start = from + i + needle.len();
        let digits: String = haystack[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u32>() {
            out = Some(n);
        }
        from = start.max(from + i + 1);
    }
    out
}

/// Last `bytes` of a file, starting at a line boundary.
fn tail(path: &Path, bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut buf = Vec::new();
    if len > bytes {
        f.seek(SeekFrom::Start(len - bytes)).ok()?;
    }
    f.read_to_end(&mut buf).ok()?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if len > bytes {
        // The seek landed mid-record; drop the partial first line.
        if let Some(i) = text.find('\n') {
            text = text.split_off(i + 1);
        }
    }
    Some(text)
}

/// Asks `gh` for one PR. `None` when `gh` is missing, unauthenticated, or the
/// PR is gone — all of which simply mean "no connector".
pub fn fetch(id: &PrRef) -> Option<Pr> {
    let out = Command::new("gh")
        .args([
            "pr",
            "view",
            &id.number.to_string(),
            "-R",
            &id.repo,
            "--json",
            "number,title,state,isDraft,statusCheckRollup",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(parse_pr(id, &v))
}

fn parse_pr(id: &PrRef, v: &serde_json::Value) -> Pr {
    let draft = v.get("isDraft").and_then(|d| d.as_bool()).unwrap_or(false);
    let state = match v.get("state").and_then(|s| s.as_str()).unwrap_or("OPEN") {
        "MERGED" => State::Merged,
        "CLOSED" => State::Closed,
        _ if draft => State::Draft,
        _ => State::Open,
    };
    let (mut passed, mut failed) = (0, 0);
    if let Some(rollup) = v.get("statusCheckRollup").and_then(|r| r.as_array()) {
        for check in rollup {
            let verdict = check
                .get("conclusion")
                .and_then(|c| c.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| check.get("state").and_then(|s| s.as_str()))
                .unwrap_or("");
            match verdict {
                "SUCCESS" | "NEUTRAL" => passed += 1,
                "FAILURE" | "TIMED_OUT" | "ERROR" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => failed += 1,
                // CANCELLED, SKIPPED, PENDING, QUEUED, IN_PROGRESS: neither.
                _ => {}
            }
        }
    }
    Pr { id: id.clone(), title: v.get("title").and_then(|t| t.as_str()).unwrap_or_default().into(), state, passed, failed }
}

/// Per-session PR resolution plus the fetched state, all cached.
pub struct Tracker {
    /// `gh` calls allowed per snapshot. The board keeps the ration; `--prs`
    /// lifts it so one pass answers for every session.
    pub budget: usize,
    /// cwd → `owner/repo`; `None` means "checked, not a GitHub checkout".
    repos: Mutex<HashMap<String, Option<String>>>,
    /// session id → (transcript size when scanned, what it resolved to).
    refs: Mutex<HashMap<String, (u64, Option<PrRef>)>>,
    prs: Mutex<HashMap<PrRef, (Option<Pr>, Instant)>>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self { budget: FETCH_BUDGET, repos: Mutex::default(), refs: Mutex::default(), prs: Mutex::default() }
    }
}

impl Tracker {
    /// Resolves `cwd`'s repository, remembering the answer.
    pub fn repo(&self, cwd: &str) -> Option<String> {
        if let Some(hit) = self.repos.lock().ok()?.get(cwd) {
            return hit.clone();
        }
        let found = repo_of(cwd);
        if let Ok(mut map) = self.repos.lock() {
            map.insert(cwd.to_string(), found.clone());
        }
        found
    }

    /// The PR a session points at, rescanned only when its transcript grew.
    pub fn resolve(&self, session_id: &str, cwd: &str, transcript: Option<&Path>) -> Option<PrRef> {
        let path = transcript?;
        let size = std::fs::metadata(path).ok()?.len();
        if let Ok(map) = self.refs.lock() {
            if let Some((seen, hit)) = map.get(session_id) {
                if *seen == size {
                    return hit.clone();
                }
            }
        }
        let repo = self.repo(cwd)?;
        let found = scan_transcript(path, &repo).map(|number| PrRef { repo, number });
        if let Ok(mut map) = self.refs.lock() {
            map.insert(session_id.to_string(), (size, found.clone()));
        }
        found
    }

    /// Cached state for a PR, refreshing at most `FETCH_BUDGET` per call.
    /// `budget` is shared across one snapshot so a board full of PRs still
    /// polls promptly, filling in over successive ticks.
    pub fn state(&self, id: &PrRef, budget: &mut usize, now: Instant) -> Option<Pr> {
        if let Ok(map) = self.prs.lock() {
            if let Some((pr, at)) = map.get(id) {
                if now.duration_since(*at) < TTL {
                    return pr.clone();
                }
            }
        }
        if *budget == 0 {
            // Stale is better than blank; keep showing the last answer.
            return self.prs.lock().ok()?.get(id).and_then(|(pr, _)| pr.clone());
        }
        *budget -= 1;
        let fetched = fetch(id);
        if let Ok(mut map) = self.prs.lock() {
            map.insert(id.clone(), (fetched.clone(), now));
        }
        fetched
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_reduce_to_owner_and_repo() {
        for url in [
            "git@github.com:cookieplace/crepe.git",
            "https://github.com/cookieplace/crepe.git",
            "https://github.com/cookieplace/crepe",
            "ssh://git@github.com/cookieplace/crepe.git",
        ] {
            assert_eq!(parse_repo(url).as_deref(), Some("cookieplace/crepe"), "{url}");
        }
        assert_eq!(parse_repo("git@gitlab.com:x/y.git"), None, "only GitHub");
        assert_eq!(parse_repo("https://github.com/cookieplace"), None, "needs both halves");
        assert_eq!(parse_repo("https://github.com/a/b/c"), None, "no deeper paths");
    }

    #[test]
    fn the_last_pr_of_the_right_repo_wins() {
        let needle = "github.com/o/r/pull/";
        assert_eq!(last_number("see https://github.com/o/r/pull/12 please", needle), Some(12));
        assert_eq!(last_number("https://github.com/o/r/pull/12 then /pull/34", needle), Some(12));
        assert_eq!(
            last_number("https://github.com/o/r/pull/12 and https://github.com/o/r/pull/34", needle),
            Some(34),
            "the newest mention wins"
        );
        assert_eq!(last_number("nothing here", needle), None);
    }

    #[test]
    fn rollups_ignore_cancelled_and_skipped_runs() {
        let id = PrRef { repo: "o/r".into(), number: 7 };
        let v = serde_json::json!({
            "state": "OPEN", "isDraft": false, "title": "t",
            "statusCheckRollup": [
                {"conclusion": "SUCCESS"}, {"conclusion": "SKIPPED"}, {"conclusion": "CANCELLED"},
                {"conclusion": "FAILURE"}, {"state": "SUCCESS"}, {"conclusion": ""},
            ],
        });
        let pr = parse_pr(&id, &v);
        assert_eq!((pr.passed, pr.failed), (2, 1));
        assert_eq!(pr.look(), Look::Failing, "one red check colours the whole connector");
    }

    #[test]
    fn state_and_checks_decide_the_look() {
        let id = PrRef { repo: "o/r".into(), number: 1 };
        let base = Pr { id, title: String::new(), state: State::Open, passed: 1, failed: 0 };
        assert_eq!(base.look(), Look::Open);
        assert_eq!(Pr { state: State::Draft, ..base.clone() }.look(), Look::Draft);
        assert_eq!(Pr { failed: 1, ..base.clone() }.look(), Look::Failing);
        assert_eq!(Pr { state: State::Draft, failed: 1, ..base.clone() }.look(), Look::Failing, "red beats draft");
        assert_eq!(Pr { state: State::Merged, failed: 1, ..base.clone() }.look(), Look::Merged, "landed is landed");
        assert_eq!(Pr { state: State::Closed, ..base }.look(), Look::Closed);
    }
}
