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
//! State comes from `gh`. The fetching happens on a worker thread rather than
//! inside `snapshot()`: a `gh pr view` takes about a second, and a board with
//! ten PRs on it would otherwise turn the one-second session poll into a
//! twenty-second one. The board reads whatever the cache holds and fills in as
//! answers land. (#64)

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How soon `gh` is asked about a pull request again, by what it last said
/// (#247).
///
/// It was one number for all of them, and that was one number too few. A
/// board of thirteen refreshes in four seconds — three workers, a second a
/// call — so what you wait for is never the asking, it is the gap between
/// asks. And the same gap is wrong at both ends: a minute is an age to a
/// pull request whose checks are running right now, and a waste on one that
/// merged an hour ago and will never say anything else.
///
/// Checks running, or in the merge queue: this changes by the minute.
const MOVING: Duration = Duration::from_secs(12);
/// Open, with nothing of its own left to do: it changes when a person does
/// something — a review, a merge — so it is watched, not stared at.
const WAITING: Duration = Duration::from_secs(30);
/// Merged or closed. It is on the board to be seen, not to be followed:
/// GitHub has nothing left to tell us about it.
const SETTLED: Duration = Duration::from_secs(10 * 60);
/// One `gh` would not answer for (#112). Asking harder will not make it
/// answer, so it keeps the old pace.
const UNANSWERED: Duration = Duration::from_secs(60);
/// How long a worker idles when everything is fresh.
const IDLE: Duration = Duration::from_millis(500);

/// Where one pull request stands with the worker pool (#64, #247, #251).
///
/// It was three containers and a subtraction at the call site — `wanted`,
/// `seen`, `inflight` — and the four states a pull request can be in had no
/// names. The pool's whole job is to move each of them round this ring, so
/// the ring is written down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// On the board and never asked about: the board has nothing for it.
    Unasked,
    /// A worker is asking `gh` right now.
    Asking,
    /// `gh` answered and the answer is still good for its kind.
    Fresh,
    /// The answer has aged past what its own state allows.
    Stale,
}

/// Where a pull request stands, from what the pool holds about it.
fn ask(answer: Option<&(Option<Pr>, Instant)>, asking: bool, now: Instant) -> Ask {
    match (answer, asking) {
        // A worker has it: nobody else may take it, whatever its age.
        (_, true) => Ask::Asking,
        (None, false) => Ask::Unasked,
        (Some((pr, at)), false) => {
            if now.duration_since(*at) >= again_after(pr.as_ref()) { Ask::Stale } else { Ask::Fresh }
        }
    }
}

/// When the worker may ask about this pull request again, given the last
/// answer. `None` is a pull request `gh` had nothing to say about.
fn again_after(pr: Option<&Pr>) -> Duration {
    match pr {
        None => UNANSWERED,
        Some(pr) => match pr.state {
            State::Merged | State::Closed => SETTLED,
            _ if pr.pending > 0 || pr.queued => MOVING,
            _ => WAITING,
        },
    }
}
/// Workers fetching in parallel. A `gh pr view` is a second of waiting on the
/// network, so a board with ten PRs fills in three seconds instead of twelve.
const WORKERS: usize = 3;
/// PRs kept per session. A ticket can have several (#63), but a transcript
/// that has wandered past this many is no longer describing one lane's work.
pub const MAX_PER_SESSION: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrRef {
    /// `owner/repo`.
    pub repo: String,
    pub number: u32,
}

impl PrRef {
    /// What a node carrying this number is labelled with.
    pub fn label(&self) -> String {
        format!("#{}", self.number)
    }

    /// Where a click on it goes. Known from the number alone, which is why a
    /// pull request GitHub would not answer for can still be opened (#112).
    pub fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.number)
    }
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
    /// Somebody has been asked to look at it (`gh`'s `reviewRequests`). An
    /// open pull request nobody has asked anything of is not waiting on a
    /// reviewer — it is waiting on the person who opened it (#218).
    pub asked: bool,
    /// It is based on another branch of the repository rather than the one
    /// pull requests are opened against: it sits on top of another pull
    /// request, and waits for that one rather than for a reviewer (#220).
    pub stacked: bool,
    /// It is in the repository's merge queue: going in by itself, with
    /// nothing left for anybody to do (#231).
    pub queued: bool,
    /// Auto-merge is on (`gh`'s `autoMergeRequest`): it goes in on its own
    /// the moment it is allowed to, so it is not waiting on anybody (#239).
    pub auto: bool,
    /// Checks that concluded green, and ones that concluded red. Cancelled and
    /// skipped runs count as neither — a cancelled job is not a failure.
    pub passed: u32,
    pub failed: u32,
    /// Checks still queued or running (#62).
    pub pending: u32,
}

/// How the board draws a connector. Colour is the whole signal (#57), and
/// filled always means "further along" than outlined (#58).
///
/// What GitHub says the pull request is, and what its checks say about it.
/// Review is deliberately not in here (#116): approval is orthogonal to all
/// of this — a pull request can be approved and failing, approved and still
/// a draft, approved and landed — and an enum forces a choice where there is
/// none to make. It rides along as `Pr::signed_off()` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Ink outline: not asking for review yet.
    Draft,
    /// Green outline: open.
    Open,
    /// Filled orange: at least one check failed.
    Failing,
    /// In the merge queue: on its way in, and nobody's turn (#231).
    Merging,
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
            // In the queue is the news, whatever the checks say: the queue
            // runs its own and takes the pull request out if they fail.
            _ if self.queued => Look::Merging,
            _ if self.failed > 0 => Look::Failing,
            State::Draft => Look::Draft,
            State::Open => Look::Open,
        }
    }

    /// Review has signed off on this pull request.
    ///
    /// Orthogonal to everything `look()` reports (#116). It is drawn as a tick
    /// beside the number rather than as a colour, the way a running check is a
    /// border rather than a look of its own (#62): the colour is left to say
    /// what GitHub says the pull request is and what its checks say about it,
    /// and the tick says what review made of it. Nothing has to be given up to
    /// say the other — approved and failing, approved and still a draft,
    /// approved and landed all read as themselves.
    ///
    /// Merging counts. A pull request nobody signed off on does not land, so
    /// a merged one is an approved one whatever `reviewDecision` says about it
    /// afterwards, and the tick stays on it.
    pub fn signed_off(&self) -> bool {
        self.approved || matches!(self.state, State::Merged)
    }

    /// Review has been asked for and has not answered yet (#153).
    ///
    /// A draft has not asked (`Look::Draft` is exactly that), a merged one
    /// has its answer (#117), and a closed one will never get one. What is
    /// left — an open pull request nobody has signed off, red checks or not
    /// — is the only kind that is waiting on a person who is not here.
    pub fn awaiting_review(&self) -> bool {
        !self.signed_off() && matches!(self.state, State::Open)
    }

    /// Open, with nobody asked and nobody signed off: the pull request is
    /// waiting on the person who opened it, not on a reviewer (#218). A
    /// draft has not asked on purpose, and a landed or closed one is done
    /// asking.
    pub fn unasked(&self) -> bool {
        matches!(self.state, State::Open) && !self.approved && !self.asked && !self.stacked
    }

    /// CI is still working. Drawn as a border rather than a look of its own,
    /// so it can sit on top of whatever the PR already is (#62). A landed or
    /// abandoned PR is never busy, whatever a stale rollup says.
    pub fn running(&self) -> bool {
        self.pending > 0 && matches!(self.state, State::Open | State::Draft)
    }

    /// What the connector is labelled with.
    pub fn label(&self) -> String {
        self.id.label()
    }

    /// Where the connector points a click.
    pub fn url(&self) -> String {
        self.id.url()
    }

    /// A made-up PR for the sandbox, one per look. Review is set separately,
    /// since it is not one of these (#116).
    pub fn synthetic(number: u32, look: Look) -> Self {
        let (state, approved, passed, failed) = match look {
            Look::Draft => (State::Draft, false, 3, 0),
            Look::Open => (State::Open, false, 12, 0),
            Look::Failing => (State::Open, false, 8, 2),
            Look::Merging => (State::Open, true, 11, 0),
            Look::Merged => (State::Merged, false, 14, 0),
            Look::Closed => (State::Closed, false, 0, 0),
        };
        Self {
            id: PrRef { repo: "sandbox/demo".into(), number },
            title: format!("synthetic pull request {number}"),
            state,
            approved,
            asked: true,
            stacked: false,
            queued: look == Look::Merging,
            auto: false,
            passed,
            failed,
            pending: 0,
        }
    }
}

impl Look {
    pub const ALL: [Look; 6] =
        [Look::Draft, Look::Open, Look::Failing, Look::Merging, Look::Merged, Look::Closed];

    pub fn short(self) -> &'static str {
        match self {
            Look::Draft => "DRAFT",
            Look::Open => "OPEN",
            Look::Failing => "FAIL",
            Look::Merging => "MERGING",
            Look::Merged => "MERGED",
            Look::Closed => "CLOSED",
        }
    }

    /// Single letter for the test panel's state row (#58).
    pub fn letter(self) -> &'static str {
        match self {
            Look::Draft => "D",
            Look::Open => "O",
            Look::Failing => "F",
            Look::Merging => "Q",
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

/// Every PR of `repo` the session named from `from` bytes into its
/// transcript on, oldest number first, with the offset to pick up at next
/// time.
///
/// Read forward rather than from the tail back (#240): a session names a
/// pull request **in its own words** when it makes it, which is early, and
/// what keeps being printed afterwards is a command's output, which is
/// late. Reading the last two megabytes found a session's own work only
/// where it had done nothing else since.
pub fn scan_from(path: &Path, repo: &str, since: u64, from: u64) -> (Vec<u32>, u64) {
    let Some((text, offset)) = read_on(path, from) else { return (Vec::new(), from) };
    let needle = format!("github.com/{repo}/pull/");
    let mut found: Vec<u32> = Vec::new();
    for line in text.lines() {
        // Cheap gate: parsing every record is the expensive part, and almost
        // no line mentions a PR.
        if !line.contains(&needle) {
            continue;
        }
        // A PR from before the tab turned to new work is not this work's (#74).
        if !in_context(line, since) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        // And something the session itself said or did, rather than a copy of
        // what was handed to it (#152).
        if !spoken(&v) {
            continue;
        }
        let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) else {
            continue;
        };
        for block in blocks {
            if !own_words(block) {
                continue;
            }
            found.extend(numbers_in(&block.to_string(), &needle));
        }
    }
    found.sort_unstable();
    found.dedup();
    (found, offset)
}

/// Whether a block of a turn is the session's own — what it said, what it
/// thought, a command it ran — rather than what came back from running it
/// (#240).
///
/// A command prints whatever it was pointed at. A cleanup plan lists the
/// pull requests of the worktree it is about to delete; a merge dashboard
/// lists a whole queue. None of that is the work of the session that ran
/// it, and reading it as such put two of somebody else's merged pull
/// requests under a session that was reading a list of skills.
fn own_words(block: &serde_json::Value) -> bool {
    block.get("type").and_then(|t| t.as_str()) != Some("tool_result")
}

/// A file from `from` bytes in, with the offset to start at next time: the
/// end of the last whole line, so a record still being written is read when
/// it is finished rather than cut in half.
fn read_on(path: &Path, from: u64) -> Option<(String, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    if from > 0 {
        f.seek(SeekFrom::Start(from)).ok()?;
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let end = buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    buf.truncate(end);
    Some((String::from_utf8_lossy(&buf).into_owned(), from + end as u64))
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

/// Unix millis of a transcript line, read straight out of its `timestamp`
/// field. Parsing the whole record to date it costs more than the scan it is
/// gating, and every line that carries a message carries this.
pub fn line_millis(line: &str) -> Option<u64> {
    const KEY: &str = "\"timestamp\":\"";
    let at = line.find(KEY)? + KEY.len();
    let end = line[at..].find('"')? + at;
    Some(crate::usage::parse_iso8601(&line[at..end])? * 1000)
}

/// Whether a line belongs to the session's current context. Lines with no
/// timestamp are kept: only a line that can be dated can be ruled out.
pub fn in_context(line: &str, since: u64) -> bool {
    since == 0 || line_millis(line).is_none_or(|at| at >= since)
}

/// Whether a transcript record is one of the session's own turns (#152).
///
/// A transcript holds a great deal more than the conversation. Among two
/// dozen kinds of record there are the files attached as instructions, the
/// title, the mode, the queue — and `api-request-blob`, which is every
/// request exactly as it was sent: system prompt, `CLAUDE.md`, `AGENTS.md`,
/// the memory index, all of it, a hundred times over in one transcript.
///
/// That record carries a `user` message with a content array like any turn,
/// so it walked straight through the shape gate here. A repo whose
/// instructions link a pull request was handing that pull request to **every
/// session in the repo**, and through its title the issue key with it (#56,
/// #57) — which is the loudest thing a cell can say about work nobody is
/// doing.
///
/// Only `user` and `assistant` are turns. Tool results are `user` records, so
/// what a command printed still counts: the session ran it.
pub fn spoken(v: &serde_json::Value) -> bool {
    matches!(v.get("type").and_then(|t| t.as_str()), Some("user") | Some("assistant"))
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
            "number,title,state,isDraft,reviewDecision,reviewRequests,baseRefName,autoMergeRequest,statusCheckRollup",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut pr = parse_pr(id, &v);
    pr.stacked = stacked(&id.repo, v.get("baseRefName").and_then(|b| b.as_str()).unwrap_or_default());
    pr.queued = matches!(pr.state, State::Open) && in_queue(&id.repo, id.number);
    Some(pr)
}

/// Whether the repository has this pull request in its merge queue (#231).
///
/// `gh pr view` does not carry it, and asking GitHub per pull request would
/// be a call each; the repository's queue is one call that answers for all
/// of them, kept for [`WAITING`] — a pull request sits in a queue for
/// minutes, so this is the one thing on a moving pull request that does not
/// need the moving pace (#247).
fn in_queue(repo: &str, number: u32) -> bool {
    static QUEUES: OnceLock<Mutex<HashMap<String, (Instant, HashSet<u32>)>>> = OnceLock::new();
    let cache = QUEUES.get_or_init(Mutex::default);
    let fresh = {
        let seen = cache.lock().unwrap_or_else(|e| e.into_inner());
        seen.get(repo).filter(|(at, _)| at.elapsed() < WAITING).map(|(_, q)| q.clone())
    };
    let queue = match fresh {
        Some(q) => q,
        None => {
            let q = merge_queue(repo).unwrap_or_default();
            cache.lock().unwrap_or_else(|e| e.into_inner()).insert(repo.to_string(), (Instant::now(), q.clone()));
            q
        }
    };
    queue.contains(&number)
}

/// The numbers in a repository's merge queue, or `None` when `gh` could not
/// say. A repository with no queue answers `null`, which is an empty one.
fn merge_queue(repo: &str) -> Option<HashSet<u32>> {
    let (owner, name) = repo.split_once('/')?;
    let out = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            "query=query($o:String!,$r:String!){repository(owner:$o,name:$r){mergeQueue{entries(first:100){nodes{pullRequest{number}}}}}}",
            "-f",
            &format!("o={owner}"),
            "-f",
            &format!("r={name}"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(queue_numbers(&v))
}

/// The pull request numbers in the answer to the merge-queue query.
pub fn queue_numbers(v: &serde_json::Value) -> HashSet<u32> {
    v.get("data")
        .and_then(|d| d.get("repository"))
        .and_then(|r| r.get("mergeQueue"))
        .and_then(|q| q.get("entries"))
        .and_then(|e| e.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|n| n.get("pullRequest")?.get("number")?.as_u64())
                .map(|n| n as u32)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a pull request based on `base` is stacked on another one: it is,
/// when `base` is not the branch this repository opens pull requests
/// against (#220). Asking GitHub for the parent itself would be a call per
/// pull request; the default branch is one call per repository, kept for
/// the life of the process, and in a stack the branch underneath is always
/// somebody else's pull request.
fn stacked(repo: &str, base: &str) -> bool {
    if base.is_empty() {
        return false;
    }
    static DEFAULTS: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = DEFAULTS.get_or_init(Mutex::default);
    let known = cache.lock().unwrap_or_else(|e| e.into_inner()).get(repo).cloned();
    let default = match known {
        Some(d) => d,
        None => {
            let found = default_branch(repo);
            cache.lock().unwrap_or_else(|e| e.into_inner()).insert(repo.to_string(), found.clone());
            found
        }
    };
    // Unknown is not stacked: a repository `gh` cannot answer for is better
    // read as an ordinary pull request than hidden from the person.
    default.is_some_and(|d| d != base)
}

fn default_branch(repo: &str) -> Option<String> {
    let out = Command::new("gh")
        .args(["repo", "view", repo, "--json", "defaultBranchRef", "-q", ".defaultBranchRef.name"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
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
    let asked = v.get("reviewRequests").and_then(|r| r.as_array()).is_some_and(|who| !who.is_empty());
    // `autoMergeRequest` is the request itself, or null where nobody set one.
    let auto = v.get("autoMergeRequest").is_some_and(|a| !a.is_null());
    Pr {
        id: id.clone(),
        title: v.get("title").and_then(|t| t.as_str()).unwrap_or_default().into(),
        state,
        approved,
        asked,
        stacked: false,
        queued: false,
        auto,
        passed,
        failed,
        pending,
    }
}

/// Hands a PR URL to the browser (#57).
pub fn open(url: &str) -> std::io::Result<bool> {
    Command::new("open").arg(url).status().map(|st| st.success())
}

/// The shared half of the tracker: what the board wants to know, and what
/// `gh` has said so far. Held behind an `Arc` so the worker thread can outlive
/// any one call. (#64)
#[derive(Default)]
struct Shared {
    /// Refs currently on the board, replaced wholesale each snapshot so a
    /// closed session stops being refreshed.
    wanted: Mutex<Vec<PrRef>>,
    seen: Mutex<HashMap<PrRef, (Option<Pr>, Instant)>>,
    /// Claimed by a worker right now, so the pool never fetches one twice.
    inflight: Mutex<HashSet<PrRef>>,
}

impl Shared {
    /// Claims the next ref that has never been answered, or whose answer went
    /// stale. Claiming and testing happen under the same lock so two workers
    /// cannot pick the same one.
    fn claim(&self) -> Option<PrRef> {
        let wanted = self.wanted.lock().ok()?;
        let seen = self.seen.lock().ok()?;
        let mut inflight = self.inflight.lock().ok()?;
        let now = Instant::now();
        let standing = |id: &PrRef| ask(seen.get(id), inflight.contains(id), now);
        // Never asked comes first: a pull request the board has nothing at
        // all for is one the board is quietly lying about (#112).
        let id = wanted
            .iter()
            .find(|id| standing(id) == Ask::Unasked)
            .or_else(|| wanted.iter().find(|id| standing(id) == Ask::Stale))
            .cloned()?;
        inflight.insert(id.clone());
        Some(id)
    }

    fn store(&self, id: PrRef, pr: Option<Pr>) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.insert(id.clone(), (pr, Instant::now()));
        }
        if let Ok(mut inflight) = self.inflight.lock() {
            inflight.remove(&id);
        }
    }
}

/// How much of one session's transcript has been read, and every pull
/// request it named along the way (#240).
#[derive(Clone, Default)]
struct Reading {
    /// The transcript's size when it was last looked at: unchanged means
    /// there is nothing new in it.
    size: u64,
    /// Bytes taken in — the end of the last whole line.
    offset: u64,
    /// The context boundary these were gathered under (#74).
    since: u64,
    numbers: Vec<u32>,
}

/// Per-session PR resolution plus the fetched state, all cached.
pub struct Tracker {
    /// cwd → `owner/repo`; `None` means "checked, not a GitHub checkout".
    repos: Mutex<HashMap<String, Option<String>>>,
    /// session id → how far its transcript has been read and what it named.
    refs: Mutex<HashMap<String, Reading>>,
    shared: Arc<Shared>,
    /// Fetch inline instead of on the worker, for the one-shot diagnostics.
    pub eager: bool,
    started: Mutex<bool>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            repos: Mutex::default(),
            refs: Mutex::default(),
            shared: Arc::default(),
            eager: false,
            started: Mutex::new(false),
        }
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

    /// The PRs a session has named, oldest first, read on from wherever its
    /// transcript was left off and kept (#240). A session that opened a
    /// pull request a day ago still has it: what it said once it has said.
    /// More than `MAX_PER_SESSION` and the newest are the ones shown, PR
    /// numbers only ever going up.
    pub fn resolve(&self, session_id: &str, cwd: &str, transcript: Option<&Path>, since: u64) -> Vec<PrRef> {
        let Some(path) = transcript else { return Vec::new() };
        let Ok(size) = std::fs::metadata(path).map(|m| m.len()) else { return Vec::new() };
        let Some(repo) = self.repo(cwd) else { return Vec::new() };
        let mut read = self.refs.lock().ok().and_then(|map| map.get(session_id).cloned()).unwrap_or_default();
        // A rename moves the boundary, and a transcript shorter than what was
        // read is not the one that was read: either way, start again (#74).
        if read.since != since || size < read.offset {
            read = Reading { since, ..Reading::default() };
        }
        if read.size != size {
            let (named, offset) = scan_from(path, &repo, since, read.offset);
            read.numbers.extend(named);
            read.numbers.sort_unstable();
            read.numbers.dedup();
            read.offset = offset;
            read.size = size;
            if let Ok(mut map) = self.refs.lock() {
                map.insert(session_id.to_string(), read.clone());
            }
        }
        let first = read.numbers.len().saturating_sub(MAX_PER_SESSION);
        read.numbers[first..].iter().map(|&number| PrRef { repo: repo.clone(), number }).collect()
    }

    /// Whatever `gh` last said, stale or not. Never blocks.
    pub fn cached(&self, id: &PrRef) -> Option<Pr> {
        self.shared.seen.lock().ok()?.get(id).and_then(|(pr, _)| pr.clone())
    }

    /// Whether a pull request the board knows about has gone unanswered:
    /// asked for and `gh` came back with nothing. A PR that has not been
    /// asked about yet is not this — that is the board still working — and
    /// neither is one that answered (#112).
    pub fn unanswered(&self, id: &PrRef) -> bool {
        self.shared.seen.lock().is_ok_and(|seen| matches!(seen.get(id), Some((None, _))))
    }

    /// Declares the refs the board is showing. In eager mode every missing or
    /// stale one is fetched before returning; otherwise the worker picks them
    /// up and the board sees them on a later poll.
    pub fn want(&self, refs: Vec<PrRef>) {
        if let Ok(mut wanted) = self.shared.wanted.lock() {
            *wanted = refs;
        }
        if self.eager {
            while let Some(id) = self.shared.claim() {
                let got = fetch(&id);
                self.shared.store(id, got);
            }
            return;
        }
        self.start_workers();
    }

    fn start_workers(&self) {
        let Ok(mut started) = self.started.lock() else { return };
        if *started {
            return;
        }
        *started = true;
        for _ in 0..WORKERS {
            let shared = Arc::clone(&self.shared);
            std::thread::spawn(move || {
                loop {
                    match shared.claim() {
                        Some(id) => {
                            let got = fetch(&id);
                            shared.store(id, got);
                        }
                        None => std::thread::sleep(IDLE),
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a transcript and scans it, the way the board does.
    fn scan(lines: &[String]) -> Vec<u32> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("claudmagi-pr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", NEXT.fetch_add(1, Ordering::Relaxed)));
        // A whole line ends in a newline: a record still being written is
        // read when it is finished (#240).
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        let out = scan_from(&path, "cookieplace/crepe", 0, 0).0;
        let _ = std::fs::remove_file(&path);
        out
    }

    /// A transcript is a great deal more than its conversation (#152). The
    /// instructions a repo loads into every session are in it, and so is a
    /// blob of every request exactly as it was sent — instructions and all.
    /// A pull request linked in a `CLAUDE.md` or a memory index was being
    /// read as the work of *every session in that repo*, which is how a
    /// dozen cells came to be carrying one stranger's pull request.
    #[test]
    fn a_pull_request_linked_in_the_instructions_is_nobody_s_work() {
        let link = |n: u32| format!("https://github.com/cookieplace/crepe/pull/{n}");
        let said = |role: &str, text: String| {
            let text = serde_json::to_string(&text).unwrap();
            format!(r#"{{"type":"{role}","message":{{"content":[{{"type":"text","text":{text}}}]}}}}"#)
        };
        // What the session itself did.
        let mine = said("assistant", format!("opened {}", link(10154)));
        // The whole request as it was sent, with the repo's instructions
        // inside it. A `user` message with a content array, like any turn.
        let blob = {
            let text = serde_json::to_string(&format!(
                "<system-reminder>\nContents of /x/CLAUDE.md:\n- [PJM-1544]({})\n</system-reminder>",
                link(7295)
            ))
            .unwrap();
            format!(r#"{{"type":"api-request-blob","sessionId":"s","message":{{"role":"user","content":[{{"type":"text","text":{text}}}]}}}}"#)
        };
        // And the record that names the files that were loaded.
        let attached = format!(
            r#"{{"type":"attachment","attachment":{{"type":"instructions","files":["/x/CLAUDE.md"]}},"rendered":"see {}"}}"#,
            link(7295)
        );

        assert_eq!(scan(&[mine.clone()]), vec![10154]);
        assert_eq!(scan(&[blob.clone()]), Vec::<u32>::new(), "the request blob handed the session somebody else's work");
        assert_eq!(scan(&[attached.clone()]), Vec::<u32>::new(), "the instructions handed the session somebody else's work");
        assert_eq!(scan(&[blob, attached, mine]), vec![10154], "one real turn among the furniture");
    }

    /// A command prints whatever it was pointed at, and none of it is the
    /// work of the session that ran it (#240). The cleanup plan below is
    /// the one that put two of somebody else's merged pull requests under
    /// a session that was reading a list of skills.
    #[test]
    fn a_pull_request_a_command_printed_is_not_the_session_s_work() {
        let link = |n: u32| format!("https://github.com/cookieplace/crepe/pull/{n}");
        let block = |body: String| format!(r#"{{"type":"user","message":{{"content":[{body}]}}}}"#);
        let printed = block(format!(
            r#"{{"type":"tool_result","content":{}}}"#,
            serde_json::to_string(&format!(
                "PR\t11383\tfeature/DEV-6166\tMERGED\t{}\nPR\t11379\tfeature/DEV-6166-be\tMERGED\t{}",
                link(11383),
                link(11379)
            ))
            .unwrap()
        ));
        let said = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":{}}}]}}}}"#,
            serde_json::to_string(&format!("PR 를 올렸습니다: {}", link(11500))).unwrap()
        );
        // Running the command is the session's own doing, even though what
        // came back is not.
        let ran = block(format!(
            r#"{{"type":"tool_use","name":"Bash","input":{{"command":{}}}}}"#,
            serde_json::to_string(&format!("gh pr view {}", link(11501))).unwrap()
        ));

        assert_eq!(scan(&[printed.clone()]), Vec::<u32>::new(), "a command listed somebody else's worktree");
        assert_eq!(scan(&[said.clone()]), vec![11500], "the session said what it made");
        assert_eq!(scan(&[ran.clone()]), vec![11501], "and a command it ran is something it did");
        assert_eq!(scan(&[printed, said, ran]), vec![11500, 11501], "the listing alone is dropped");
    }

    /// The scan reads forward and keeps what it read (#240). A session
    /// names a pull request in its own words when it makes it, and then
    /// works for hours: by the time anybody looks, that moment is far
    /// behind the end of the file, and reading only the tail lost it.
    #[test]
    fn a_pull_request_the_session_named_hours_ago_is_still_its_work() {
        let dir = std::env::temp_dir().join(format!("claudmagi-read-on-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let _ = std::fs::remove_file(&path);
        let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"opened https://github.com/cookieplace/crepe/pull/11499"}]}}"#;
        // Hours of work since, none of it about that pull request, and far
        // more of it than the two megabytes the tail used to read.
        let noise = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
            "x".repeat(4096)
        );
        let hours = |times: usize| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
            for _ in 0..times {
                writeln!(f, "{noise}").unwrap();
            }
        };
        std::fs::write(&path, format!("{said}\n")).unwrap();
        hours(800);
        assert!(std::fs::metadata(&path).unwrap().len() > 3 * 1024 * 1024, "past the tail that used to be read");

        // Cold: the board started just now and has read none of this file.
        let tracker = Tracker::default();
        tracker.repos.lock().unwrap().insert("/w".into(), Some("cookieplace/crepe".into()));
        let refs = |t: &Tracker| t.resolve("s", "/w", Some(path.as_path()), 0).iter().map(|r| r.number).collect::<Vec<_>>();
        assert_eq!(refs(&tracker), vec![11499], "the whole transcript is read the first time");

        // And warm: what it said once it has said, however long it works on.
        hours(800);
        assert_eq!(refs(&tracker), vec![11499], "read on from where it was left");
        let _ = std::fs::remove_file(&path);
    }

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
    fn a_line_is_dated_by_its_own_timestamp() {
        let line = r#"{"type":"pr-link","prNumber":8792,"timestamp":"2026-09-18T09:00:00.000Z"}"#;
        let at = line_millis(line).expect("a dated line");
        assert!(in_context(line, 0), "no boundary means the whole transcript counts");
        assert!(in_context(line, at), "the moment of the turn is still this context");
        assert!(in_context(line, at - 1));
        assert!(!in_context(line, at + 1), "written before the tab turned to new work");
        assert_eq!(line_millis(r#"{"type":"mode","mode":"normal"}"#), None);
        assert!(in_context(r#"{"type":"mode"}"#, at), "a line that cannot be dated is kept");
    }

    /// Asked and told nothing is not the same as never asked, nor as
    /// answered: it is GitHub being the problem, and the board says so by
    /// blinking (#112).
    #[test]
    fn a_pull_request_github_would_not_answer_for_is_known_to_be_unanswered() {
        let tracker = Tracker::default();
        let id = PrRef { repo: "o/r".into(), number: 7 };
        assert!(!tracker.unanswered(&id), "nobody has asked yet");

        tracker.shared.store(id.clone(), None);
        assert!(tracker.unanswered(&id), "asked, and gh came back with nothing");
        assert!(tracker.cached(&id).is_none());

        tracker.shared.store(id.clone(), Some(Pr::synthetic(7, Look::Open)));
        assert!(!tracker.unanswered(&id), "and once it answers it is answered");
    }

    /// An open pull request nobody has been asked about is waiting on the
    /// person who opened it, not on a reviewer (#218). `gh` says who was
    /// asked in `reviewRequests`; an empty list is nobody.
    #[test]
    fn a_pull_request_nobody_was_asked_about_is_waiting_on_me() {
        let id = PrRef { repo: "o/r".into(), number: 7 };
        let of = |requests: serde_json::Value, decision: serde_json::Value| {
            parse_pr(
                &id,
                &serde_json::json!({
                    "state": "OPEN", "isDraft": false, "title": "t",
                    "reviewRequests": requests, "reviewDecision": decision,
                }),
            )
        };
        let nobody = of(serde_json::json!([]), serde_json::Value::Null);
        assert!(!nobody.asked);
        assert!(nobody.unasked(), "open, nobody asked, nobody signed off");

        let asked = of(serde_json::json!([{"__typename": "User", "login": "someone"}]), serde_json::json!("REVIEW_REQUIRED"));
        assert!(asked.asked);
        assert!(!asked.unasked(), "it is out with a reviewer");
        assert!(asked.awaiting_review(), "and still waiting on an answer");

        // A sign-off is an answer, however it arrived: a review on a pull
        // request with no outstanding request still leaves nothing to ask.
        let signed = of(serde_json::json!([]), serde_json::json!("APPROVED"));
        assert!(!signed.unasked(), "somebody has looked at it");

        // Only an open one asks: a draft has not asked on purpose, and a
        // landed or closed one is done asking.
        for state in ["MERGED", "CLOSED"] {
            let done = parse_pr(
                &id,
                &serde_json::json!({ "state": state, "isDraft": false, "title": "t", "reviewRequests": [] }),
            );
            assert!(!done.unasked(), "{state} is not waiting on anybody");
        }
        let draft = parse_pr(
            &id,
            &serde_json::json!({ "state": "OPEN", "isDraft": true, "title": "t", "reviewRequests": [] }),
        );
        assert!(!draft.unasked(), "a draft has not asked on purpose");
    }

    /// A pull request in the merge queue is on its way in by itself: the
    /// board says `merging` and asks nothing of anybody (#231). One call
    /// for the repository answers for every pull request in it.
    #[test]
    fn the_merge_queue_answers_for_the_whole_repository() {
        let answer = serde_json::json!({
            "data": {"repository": {"mergeQueue": {"entries": {"nodes": [
                {"pullRequest": {"number": 11376}},
                {"pullRequest": {"number": 11463}},
            ]}}}}
        });
        let queue = queue_numbers(&answer);
        assert!(queue.contains(&11376) && queue.contains(&11463));
        assert!(!queue.contains(&11464));

        // A repository with no queue answers null, which is an empty queue
        // rather than an error.
        assert!(queue_numbers(&serde_json::json!({"data": {"repository": {"mergeQueue": null}}})).is_empty());
        assert!(queue_numbers(&serde_json::json!({})).is_empty());

        // The look it wears: in the queue beats the checks, because the
        // checks running are the queue's own and it takes the pull request
        // out itself if they fail. Landed still beats everything.
        let id = PrRef { repo: "o/r".into(), number: 7 };
        let base = Pr { id, title: String::new(), state: State::Open, approved: true, asked: true, stacked: false, queued: true, auto: false, passed: 2, failed: 0, pending: 1 };
        assert_eq!(base.look(), Look::Merging);
        assert_eq!(Pr { failed: 1, ..base.clone() }.look(), Look::Merging);
        assert_eq!(Pr { state: State::Merged, ..base.clone() }.look(), Look::Merged);
        assert_eq!(Pr { queued: false, ..base.clone() }.look(), Look::Open);
        assert_eq!(Look::Merging.short(), "MERGING");
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
            Pr { id, title: String::new(), state: State::Open, approved: false, asked: true, stacked: false, queued: false, auto: false, passed: 1, failed: 0, pending: 0 };
        assert!(!base.running());
        assert!(Pr { pending: 1, ..base.clone() }.running());
        assert!(!Pr { pending: 1, state: State::Merged, ..base.clone() }.running(), "a landed PR is never busy");
        assert_eq!(base.look(), Look::Open);
        // Review is not in here at all (#116): the look answers for GitHub's
        // state and the checks, and says nothing either way about approval.
        assert_eq!(Pr { approved: true, ..base.clone() }.look(), Look::Open);
        assert_eq!(Pr { approved: true, failed: 1, ..base.clone() }.look(), Look::Failing);
        assert_eq!(Pr { state: State::Draft, ..base.clone() }.look(), Look::Draft);
        assert_eq!(Pr { failed: 1, ..base.clone() }.look(), Look::Failing);
        assert_eq!(Pr { state: State::Draft, failed: 1, ..base.clone() }.look(), Look::Failing, "red beats draft");
        assert_eq!(Pr { state: State::Merged, failed: 1, ..base.clone() }.look(), Look::Merged, "landed is landed");
        assert_eq!(Pr { state: State::Closed, ..base.clone() }.look(), Look::Closed);

        // And approval rides on top of every one of them, merging included:
        // nothing lands without a sign-off, so a landed pull request keeps
        // the tick whatever `reviewDecision` says about it afterwards (#116).
        assert!(!base.signed_off(), "an open pull request nobody reviewed has nothing to tick");
        for state in [State::Open, State::Draft, State::Closed] {
            let pr = Pr { approved: true, state, ..base.clone() };
            assert!(pr.signed_off(), "{state:?}: a sign-off is not undone by the state");
            assert!(Pr { failed: 1, ..pr.clone() }.signed_off(), "{state:?}: nor by a red check");
        }
        let landed = Pr { state: State::Merged, approved: false, ..base };
        assert!(landed.signed_off(), "merging is a sign-off, whatever the review field says");
    }

    /// The four states a pull request can be in with the pool, and the one
    /// rule that outranks the clock (#251): a worker has it, so nobody else
    /// may take it, however old the answer is.
    #[test]
    fn a_pull_request_a_worker_is_holding_is_nobody_elses_to_take() {
        let now = Instant::now();
        let old = now.checked_sub(Duration::from_secs(3600)).expect("a clock with a past");
        let answer: (Option<Pr>, Instant) = (None, old);
        let young: (Option<Pr>, Instant) = (None, now);

        assert_eq!(ask(None, false, now), Ask::Unasked, "nothing at all is the loudest state");
        assert_eq!(ask(Some(&young), false, now), Ask::Fresh);
        assert_eq!(ask(Some(&answer), false, now), Ask::Stale);
        for held in [None, Some(&young), Some(&answer)] {
            assert_eq!(ask(held, true, now), Ask::Asking, "a worker has it, so it is not free");
        }
    }

    /// What `gh` last said decides when it is asked again (#247). Under one
    /// pace for all of them the board spent its calls on pull requests that
    /// had finished, and made the one whose checks were running wait out the
    /// same minute as the rest.
    #[test]
    fn the_board_asks_again_as_often_as_a_pull_request_moves() {
        let id = |n: u32| PrRef { repo: "o/r".into(), number: n };
        let pr = |n: u32, state: State, pending: u32| Pr {
            id: id(n),
            title: String::new(),
            state,
            approved: false,
            asked: true,
            stacked: false,
            queued: false,
            auto: false,
            passed: 1,
            failed: 0,
            pending,
        };
        assert_eq!(again_after(Some(&pr(1, State::Merged, 0))), SETTLED);
        assert_eq!(again_after(Some(&pr(1, State::Closed, 0))), SETTLED);
        assert_eq!(again_after(Some(&pr(1, State::Open, 2))), MOVING, "its checks are running");
        let going_in = Pr { queued: true, ..pr(1, State::Open, 0) };
        assert_eq!(again_after(Some(&going_in)), MOVING, "it is going in by itself");
        assert_eq!(again_after(Some(&pr(1, State::Open, 0))), WAITING);
        assert_eq!(again_after(None), UNANSWERED, "asking harder will not make it answer");
        assert!(MOVING < WAITING && WAITING < SETTLED, "the busier it is, the sooner it is asked about");

        // And the worker acts on it. Forty seconds after both were answered,
        // the one with checks running is due and the merged one is not.
        let shared = Shared::default();
        *shared.wanted.lock().expect("a fresh lock") = vec![id(1), id(2)];
        let then = Instant::now().checked_sub(Duration::from_secs(40)).expect("a clock with a past");
        {
            let mut seen = shared.seen.lock().expect("a fresh lock");
            seen.insert(id(1), (Some(pr(1, State::Merged, 0)), then));
            seen.insert(id(2), (Some(pr(2, State::Open, 3)), then));
        }
        assert_eq!(shared.claim(), Some(id(2)), "the one still working is the one to ask about");
        assert_eq!(shared.claim(), None, "and the one that landed is left alone");
    }
}
