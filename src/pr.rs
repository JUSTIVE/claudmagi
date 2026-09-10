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
/// PRs kept per session. A ticket can have several (#63), but a transcript
/// that has wandered past this many is no longer describing one lane's work.
pub const MAX_PER_SESSION: usize = 4;
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
    /// True once review has signed off (`gh`'s `reviewDecision`).
    pub approved: bool,
    /// Checks that concluded green, and ones that concluded red. Cancelled and
    /// skipped runs count as neither — a cancelled job is not a failure.
    pub passed: u32,
    pub failed: u32,
    /// Checks still queued or running (#62).
    pub pending: u32,
}

/// How the board draws a connector. Colour is the whole signal (#57), and
/// filled always means "further along" than outlined (#58).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Ink outline: not asking for review yet.
    Draft,
    /// Green outline: open, waiting on review.
    Open,
    /// Filled green: review signed off.
    Approved,
    /// Filled orange: at least one check failed.
    Failing,
    /// Filled black: landed.
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
            State::Open if self.approved => Look::Approved,
            State::Open => Look::Open,
        }
    }

    /// CI is still working. Drawn as a border rather than a look of its own,
    /// so it can sit on top of whatever the PR already is (#62). A landed or
    /// abandoned PR is never busy, whatever a stale rollup says.
    pub fn running(&self) -> bool {
        self.pending > 0 && matches!(self.state, State::Open | State::Draft)
    }

    /// What the connector is labelled with.
    pub fn label(&self) -> String {
        format!("#{}", self.id.number)
    }

    /// Where the connector points a click.
    pub fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.id.repo, self.id.number)
    }

    /// A made-up PR for the sandbox, one per look.
    pub fn synthetic(number: u32, look: Look) -> Self {
        let (state, approved, passed, failed) = match look {
            Look::Draft => (State::Draft, false, 3, 0),
            Look::Open => (State::Open, false, 12, 0),
            Look::Approved => (State::Open, true, 12, 0),
            Look::Failing => (State::Open, false, 8, 2),
            Look::Merged => (State::Merged, false, 14, 0),
            Look::Closed => (State::Closed, false, 0, 0),
        };
        Self {
            id: PrRef { repo: "sandbox/demo".into(), number },
            title: format!("synthetic pull request {number}"),
            state,
            approved,
            passed,
            failed,
            pending: 0,
        }
    }
}

impl Look {
    pub const ALL: [Look; 6] =
        [Look::Draft, Look::Open, Look::Approved, Look::Failing, Look::Merged, Look::Closed];

    pub fn short(self) -> &'static str {
        match self {
            Look::Draft => "DRAFT",
            Look::Open => "OPEN",
            Look::Approved => "OK",
            Look::Failing => "FAIL",
            Look::Merged => "MERGED",
            Look::Closed => "CLOSED",
        }
    }

    /// Single letter for the test panel's state row (#58).
    pub fn letter(self) -> &'static str {
        match self {
            Look::Draft => "D",
            Look::Open => "O",
            Look::Approved => "A",
            Look::Failing => "F",
            Look::Merged => "M",
            Look::Closed => "C",
        }
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

/// Every PR of `repo` mentioned in a message body in the transcript's tail,
/// oldest number first. A ticket often has more than one (#63); the newest
/// are kept when there are more than `MAX_PER_SESSION`.
pub fn scan_transcript(path: &Path, repo: &str) -> Vec<u32> {
    let Some(text) = tail(path, TAIL_BYTES) else { return Vec::new() };
    let needle = format!("github.com/{repo}/pull/");
    let mut found: Vec<u32> = Vec::new();
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
            found.extend(numbers_in(&block.to_string(), &needle));
        }
    }
    found.sort_unstable();
    found.dedup();
    if found.len() > MAX_PER_SESSION {
        found.drain(..found.len() - MAX_PER_SESSION);
    }
    found
}

fn numbers_in(haystack: &str, needle: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = haystack[from..].find(needle) {
        let start = from + i + needle.len();
        let digits: String = haystack[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u32>() {
            out.push(n);
        }
        from = start.max(from + i + 1);
    }
    out
}

/// Last `bytes` of a file, starting at a line boundary. Shared with the
/// Linear scan in `ticket.rs`.
pub fn tail(path: &Path, bytes: u64) -> Option<String> {
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
            "number,title,state,isDraft,reviewDecision,statusCheckRollup",
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
    let (mut passed, mut failed, mut pending) = (0, 0, 0);
    if let Some(rollup) = v.get("statusCheckRollup").and_then(|r| r.as_array()) {
        for check in rollup {
            let field = |k: &str| check.get(k).and_then(|c| c.as_str()).filter(|s| !s.is_empty());
            // A `CheckRun` carries `status` until it finishes and `conclusion`
            // after; a `StatusContext` only ever carries `state`.
            let verdict = field("conclusion").or_else(|| field("status")).or_else(|| field("state")).unwrap_or("");
            match verdict {
                "SUCCESS" | "NEUTRAL" => passed += 1,
                "FAILURE" | "TIMED_OUT" | "ERROR" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => failed += 1,
                "QUEUED" | "IN_PROGRESS" | "PENDING" | "WAITING" | "REQUESTED" => pending += 1,
                // CANCELLED, SKIPPED, EXPECTED, COMPLETED-without-a-verdict.
                _ => {}
            }
        }
    }
    let approved = v.get("reviewDecision").and_then(|r| r.as_str()) == Some("APPROVED");
    Pr {
        id: id.clone(),
        title: v.get("title").and_then(|t| t.as_str()).unwrap_or_default().into(),
        state,
        approved,
        passed,
        failed,
        pending,
    }
}

/// Hands a PR URL to the browser (#57).
pub fn open(url: &str) -> std::io::Result<bool> {
    Command::new("open").arg(url).status().map(|st| st.success())
}

/// Per-session PR resolution plus the fetched state, all cached.
pub struct Tracker {
    /// `gh` calls allowed per snapshot. The board keeps the ration; `--prs`
    /// lifts it so one pass answers for every session.
    pub budget: usize,
    /// cwd → `owner/repo`; `None` means "checked, not a GitHub checkout".
    repos: Mutex<HashMap<String, Option<String>>>,
    /// session id → (transcript size when scanned, what it resolved to).
    refs: Mutex<HashMap<String, (u64, Vec<PrRef>)>>,
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

    /// The PRs a session points at, oldest first, rescanned only when its
    /// transcript grew.
    pub fn resolve(&self, session_id: &str, cwd: &str, transcript: Option<&Path>) -> Vec<PrRef> {
        let Some(path) = transcript else { return Vec::new() };
        let Ok(size) = std::fs::metadata(path).map(|m| m.len()) else { return Vec::new() };
        if let Ok(map) = self.refs.lock() {
            if let Some((seen, hit)) = map.get(session_id) {
                if *seen == size {
                    return hit.clone();
                }
            }
        }
        let Some(repo) = self.repo(cwd) else { return Vec::new() };
        let found: Vec<PrRef> =
            scan_transcript(path, &repo).into_iter().map(|number| PrRef { repo: repo.clone(), number }).collect();
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
    fn every_pr_of_the_right_repo_is_collected() {
        let needle = "github.com/o/r/pull/";
        assert_eq!(numbers_in("see https://github.com/o/r/pull/12 please", needle), vec![12]);
        assert_eq!(numbers_in("https://github.com/o/r/pull/12 then /pull/34", needle), vec![12], "wrong repo ignored");
        assert_eq!(
            numbers_in("https://github.com/o/r/pull/12 and https://github.com/o/r/pull/34", needle),
            vec![12, 34],
            "a ticket can have several (#63)"
        );
        assert!(numbers_in("nothing here", needle).is_empty());
    }

    #[test]
    fn rollups_ignore_cancelled_and_skipped_runs() {
        let id = PrRef { repo: "o/r".into(), number: 7 };
        let v = serde_json::json!({
            "state": "OPEN", "isDraft": false, "title": "t",
            "statusCheckRollup": [
                {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SKIPPED"},
                {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "CANCELLED"},
                {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "FAILURE"},
                {"__typename": "CheckRun", "status": "IN_PROGRESS", "conclusion": null},
                {"__typename": "CheckRun", "status": "QUEUED", "conclusion": null},
                {"__typename": "StatusContext", "state": "SUCCESS"},
                {"__typename": "StatusContext", "state": "PENDING"},
            ],
        });
        let pr = parse_pr(&id, &v);
        assert_eq!((pr.passed, pr.failed, pr.pending), (2, 1, 3));
        assert_eq!(pr.look(), Look::Failing, "one red check colours the whole connector");
        assert!(pr.running(), "and the border still says CI is working");
    }

    #[test]
    fn state_and_checks_decide_the_look() {
        let id = PrRef { repo: "o/r".into(), number: 1 };
        let base =
            Pr { id, title: String::new(), state: State::Open, approved: false, passed: 1, failed: 0, pending: 0 };
        assert!(!base.running());
        assert!(Pr { pending: 1, ..base.clone() }.running());
        assert!(!Pr { pending: 1, state: State::Merged, ..base.clone() }.running(), "a landed PR is never busy");
        assert_eq!(base.look(), Look::Open);
        assert_eq!(Pr { approved: true, ..base.clone() }.look(), Look::Approved);
        assert_eq!(
            Pr { approved: true, failed: 1, ..base.clone() }.look(),
            Look::Failing,
            "a red check outranks a sign-off"
        );
        assert_eq!(Pr { state: State::Draft, ..base.clone() }.look(), Look::Draft);
        assert_eq!(Pr { failed: 1, ..base.clone() }.look(), Look::Failing);
        assert_eq!(Pr { state: State::Draft, failed: 1, ..base.clone() }.look(), Look::Failing, "red beats draft");
        assert_eq!(Pr { state: State::Merged, failed: 1, ..base.clone() }.look(), Look::Merged, "landed is landed");
        assert_eq!(Pr { state: State::Closed, ..base }.look(), Look::Closed);
    }
}
