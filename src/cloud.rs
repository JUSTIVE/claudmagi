//! Sessions that are not on this machine at all (#120).
//!
//! The board was built on `~/.claude/sessions/<pid>.json`, which is written
//! by the CLI entrypoint for a session running here. A cloud session has
//! neither: it runs in Anthropic's own container, started from the desktop
//! app, the iOS app or a schedule, and it leaves nothing on this disk — no
//! process, no registry record, not even a transcript.
//!
//! So it is read over the wire instead, from the same endpoint the desktop
//! app lists them with, using the token `usage.rs` already reads out of the
//! login keychain (#48). The answer is richer than the local registry: it
//! says whether the container is still connected, what the last turn made of
//! itself, which branch it is on and whether the person has looked yet.
//!
//! Everything here is reported rather than inferred, which is what makes it
//! worth putting on the board beside sessions this machine can see.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::model::{Phase, SessionInfo, SubagentInfo};
use crate::usage;

pub const URL: &str = "https://api.anthropic.com/v1/code/sessions";
/// How many to ask for. More than a board can show, so the recency filter
/// below is the one that decides, not the page size.
const LIMIT: usize = 60;
const CURL_TIMEOUT_S: u64 = 12;

/// How often to ask. A cloud session turns over in minutes, not seconds, and
/// this is a network round trip on every tick otherwise.
const TTL: Duration = Duration::from_secs(30);
/// How soon to try again after a failure, so a flaky minute does not empty
/// the board for the whole TTL.
const RETRY: Duration = Duration::from_secs(5);

/// What the endpoint calls a session the person has put away (#136). The
/// other value seen is `active`; anything else is left alone, so a value
/// nobody here has heard of cannot empty the board.
const ARCHIVED: &str = "archived";

/// A session whose container has let go still shows for this long after its
/// last event. Long enough that work finished over lunch is still there to
/// be looked at, short enough that last week's scheduled runs are not.
const RECENT: Duration = Duration::from_secs(12 * 60 * 60);

/// The fields of a cloud session this board has any use for. The endpoint
/// sends a great deal more.
#[derive(Debug, Deserialize)]
struct Raw {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    config: RawConfig,
    #[serde(default)]
    external_metadata: RawMeta,
    /// `archived` once the person has put the session away; `active`
    /// otherwise (#136).
    #[serde(default)]
    status: Option<String>,
    /// `connected` while the container is up.
    #[serde(default)]
    connection_status: Option<String>,
    /// `idle` between turns; anything else is a turn in flight.
    #[serde(default)]
    worker_status: Option<String>,
    /// What the service itself makes of the session (#155): `working`,
    /// `blocked`, `failed`, `review_ready` or `completed`. This is the
    /// answer the board was guessing at out of `unread` and the summary.
    #[serde(default)]
    status_bucket: Option<String>,
    #[serde(default)]
    unread: bool,
    /// Set when the session was started by a schedule rather than by a
    /// person. The endpoint can leave these out and is asked to; this is
    /// here so that a renamed parameter cannot quietly put them back (#125).
    #[serde(default)]
    trigger_id: Option<String>,
    #[serde(default)]
    last_event_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    #[serde(default)]
    outcomes: Vec<RawOutcome>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOutcome {
    #[serde(default)]
    git_info: Option<RawGit>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGit {
    #[serde(default)]
    repo: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawMeta {
    #[serde(default)]
    post_turn_summary: Option<RawSummary>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSummary {
    /// Non-empty when the last turn ended on something for the person to do.
    #[serde(default)]
    needs_action: Option<String>,
    #[serde(default)]
    status_detail: Option<String>,
    /// The same five words as `status_bucket`, said by the last turn about
    /// itself. `status_bucket` is the live answer and this one is the last
    /// turn's, so this is read second (#155).
    #[serde(default)]
    status_category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Page {
    #[serde(default)]
    data: Vec<Raw>,
}

/// Where a cloud session's chip points, in the Claude desktop app (#127).
///
/// The app registers `claude://` and routes `/code/<id>` straight at a
/// session: its own handler takes the first path segment and accepts it
/// against `^(cse|session)_[A-Za-z0-9_-]+$`, which is the shape these ids
/// come in. A cloud session is the app's own work, so sending the click
/// there rather than to a browser tab is sending it where it came from.
pub fn app_url(id: &str) -> String {
    format!("claude://code/{id}")
}

/// The same session on the web, for a machine with no desktop app.
pub fn web_url(id: &str) -> String {
    format!("https://claude.ai/code/{id}")
}

/// A scheduled run names itself with a lightning bolt, which is already the
/// board's business to shorten rather than to decorate.
fn label_of(raw: &Raw) -> String {
    let title = raw.title.as_deref().unwrap_or("").trim();
    let title = title.trim_start_matches('\u{26a1}').trim();
    if title.is_empty() { raw.id.clone() } else { title.to_string() }
}

/// What a cloud session is doing, in the board's own vocabulary.
///
/// `worker_status` answers it outright while a turn is in flight. For the
/// rest, the service says what it makes of the session and the board says
/// it back (#155). It was working the answer out instead, from whether the
/// person had opened the session and whether the summary named an action,
/// and that is a different question with a different answer: of eighteen
/// live sessions it put eight of them wrong — three that had finished with
/// something to show were called idle, three that were merely unopened were
/// called waiting, and the two that had *failed* could not be told from
/// either, because the board had no way to say so.
///
/// Mapping the service's words onto three (#259, #260):
///
/// - `working` is a turn in flight.
/// - `needs_action`, when the last turn wrote one, is the session naming
///   what it wants from a person — in its own sentence, which beats any
///   bucket word, including one that says the run is over. A run that ends
///   with "paste this prompt into the routine" is not finished.
/// - `blocked` is the board's own meaning of waiting on a person: it has
///   asked something and stopped. It is the **only** bucket word that does.
/// - everything else has stopped and wants nothing, so it is idle.
///
/// This is where the board was lying. `unread` was being read as "wants
/// you": a `review_ready` run nobody had opened came out in the accent,
/// the same colour as one that had stopped to ask a question, and `failed`
/// came out there too. Watching one session two minutes apart showed what
/// that costs — `Relay 버전 확인`, worker idle both times and nobody
/// touching it, went `review_ready` + unread (accent) → `completed` +
/// unread (grey) because the *service* changed a word. Nothing about
/// whether it wanted a person had changed, and the board had said it twice
/// in two colours.
///
/// So `unread` says nothing here at all. It keeps the one job it already
/// had forty lines down — whether the tile is on the board — and that is
/// the whole of what it has to say: a run nobody has opened is **there**,
/// and it is grey like everything else that has stopped (#260). A phase
/// and a colour of its own said the same thing a second time.
///
/// A word nobody here has heard of is idle too, which is where the old
/// reckoning put it as well.
fn phase_of(raw: &Raw) -> Phase {
    let worker = raw.worker_status.as_deref().unwrap_or("idle");
    if worker != "idle" {
        return Phase::Working;
    }
    let summary = raw.external_metadata.post_turn_summary.as_ref();
    let needs = summary.and_then(|s| s.needs_action.as_deref()).unwrap_or("").trim();
    if !needs.is_empty() {
        return Phase::NeedsUser;
    }
    match said(raw) {
        "working" => Phase::Working,
        "blocked" => Phase::NeedsUser,
        _ => Phase::Idle,
    }
}

/// The service's own word for the session: the live one, or the last turn's
/// where there is no live one (#155).
fn said(raw: &Raw) -> &str {
    raw.status_bucket
        .as_deref()
        .or_else(|| raw.external_metadata.post_turn_summary.as_ref().and_then(|s| s.status_category.as_deref()))
        .unwrap_or("")
        .trim()
}

/// What the session stopped on, in its own words (#155).
///
/// The board prints this after "needs you:" in the hovered detail (#115).
/// Every cloud session was printing `dialog open` there — the placeholder
/// `set_phase` writes for a local session sitting on a prompt — which is
/// not only wrong but unanswerable: there is no dialog, and the session is
/// not on this machine. The summary says what it actually stopped on.
fn stopped_on(raw: &Raw) -> Option<String> {
    let summary = raw.external_metadata.post_turn_summary.as_ref();
    let pick = |s: Option<&str>| s.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    pick(summary.and_then(|s| s.status_detail.as_deref()))
        .or_else(|| pick(summary.and_then(|s| s.needs_action.as_deref())))
        .or_else(|| pick(raw.status_bucket.as_deref()))
}

/// The pull request the last turn named, if it named one.
///
/// A cloud session has no transcript here for `pr.rs` to scan (#56), but the
/// summary it sends back is written for a person and says the number out
/// loud: "PR #9312 E2E passed (194/194); awaiting reviewer approval". Paired
/// with the repository the session is working in, that is a whole `PrRef`,
/// and the lane gets its GitHub node like any other.
fn pr_of(raw: &Raw, repo: &str) -> Option<crate::pr::PrRef> {
    let detail = raw.external_metadata.post_turn_summary.as_ref()?.status_detail.as_deref()?;
    Some(crate::pr::PrRef { repo: repo.to_string(), number: numbered(detail)? })
}

/// The pull request a sentence names, if it names one (#154).
///
/// A number is not a pull request for having a hash in front of it. These
/// summaries are prose and count all sorts of things out loud: "poll #307
/// done; no new critical/high cards" is a watch on its three hundred and
/// seventh round, and reading that as a pull request opened number 307 of
/// whichever repository the session happened to be in — a real one, merged
/// years ago, by somebody else, on a cell nobody could explain.
///
/// So the sentence has to say what the number is. A link says it outright;
/// otherwise the word before the hash has to be the one that means this.
fn numbered(detail: &str) -> Option<u32> {
    let digits = |s: &str| {
        s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().ok().filter(|n| *n > 0)
    };
    const LINK: &str = "/pull/";
    if let Some(at) = detail.find(LINK) {
        if let Some(n) = digits(&detail[at + LINK.len()..]) {
            return Some(n);
        }
    }
    detail.match_indices('#').find_map(|(at, _)| {
        let said = detail[..at].trim_end().to_ascii_lowercase();
        let word: String = said.chars().rev().take_while(|c| c.is_ascii_alphabetic()).collect();
        let word: String = word.chars().rev().collect();
        matches!(word.as_str(), "pr" | "pull" | "request").then(|| digits(&detail[at + 1..]))?
    })
}

fn repo_of(raw: &Raw) -> Option<String> {
    raw.config
        .outcomes
        .iter()
        .find_map(|o| o.git_info.as_ref()?.repo.clone())
        .filter(|r| !r.trim().is_empty())
}

/// RFC3339 to unix millis, which is all the board wants from a timestamp.
fn millis_of(stamp: Option<&str>) -> u64 {
    stamp.and_then(usage::parse_iso8601).map(|s| s * 1000).unwrap_or(0)
}

/// Turns the endpoint's answer into sessions, dropping the ones whose work is
/// old enough that nobody is waiting on it any more.
///
/// `now` is unix seconds, so the filter can be tested without a clock.
pub fn parse(body: &str, now: u64) -> Vec<SessionInfo> {
    let Ok(page) = serde_json::from_str::<Page>(body) else { return Vec::new() };
    let mut out = Vec::new();
    for raw in page.data {
        // A scheduled run is a routine, not somebody's work. It is asked for
        // without them, and dropped here too in case it ever arrives anyway.
        if raw.trigger_id.is_some() {
            continue;
        }
        // Archived is the person saying they are done with it, which is a
        // better answer than anything the board could work out for itself:
        // it outranks the clock below and the container being up, and a
        // session archived while it is still running goes too (#136).
        if raw.status.as_deref() == Some(ARCHIVED) {
            continue;
        }
        // Finished, and the person has already looked: there is nothing left
        // for a tile to say (#186). The board went on showing these because
        // their container was still up, so a session closed in the app sat
        // there pointing at work that was over.
        //
        // What nobody has opened yet still waits out the clock below, which
        // is exactly what that window was for: cloud work finishes while
        // nobody is looking (#120). The service draws the line between a run
        // that left something to look at and one that did not (#155), so
        // this takes its word rather than the container's.
        //
        // Except one that named what it wants (#259): the bucket is the
        // service's summary of the run and the sentence is the run's own,
        // and a run that ends "paste this into the routine" is not over
        // however it was bucketed. Dropping those was the same mistake as
        // painting an unopened one orange, pointing the other way — the
        // board said nothing at all about the one session that wanted
        // something.
        let phase = phase_of(&raw);
        if said(&raw) == "completed" && !raw.unread && phase != Phase::NeedsUser {
            continue;
        }
        let connected = raw.connection_status.as_deref() == Some("connected");
        let last = usage::parse_iso8601(raw.last_event_at.as_deref().unwrap_or("")).unwrap_or(0);
        // A container that is still up is current whatever its clock says.
        // Everything else has to be recent enough to still be somebody's
        // business.
        if !connected && now.saturating_sub(last) > RECENT.as_secs() {
            continue;
        }
        let repo = repo_of(&raw);
        let pr = repo.as_deref().and_then(|r| pr_of(&raw, r));
        let mut info = SessionInfo {
            // There is no process here and never will be. The id is the key.
            pid: 0,
            session_id: raw.id.clone(),
            // The board shows a working directory; a cloud session has a
            // repository instead, which is the same answer to the same
            // question ("what is this about").
            cwd: repo.clone().unwrap_or_else(|| "cloud".into()),
            name: label_of(&raw),
            // A cloud session's title is the service's own, and the only
            // name it has: nothing to go looking for (#222).
            name_chosen: true,
            status: "idle".into(),
            waiting_for: None,
            state: None,
            tempo: None,
            started_at: millis_of(raw.created_at.as_deref()),
            context_since: 0,
            tty: None,
            seat: Some(crate::host::Seat {
                host: "cloud",
                handle: raw.id.clone(),
                // Sessions on one repository belong together, the way panes
                // of one tab do.
                window: repo.clone(),
                focus_url: Some(app_url(&raw.id)),
            }),
            synthetic: false,
            subagents: Vec::new(),
            group: match &repo {
                Some(r) => format!("cloud-tab:{r}"),
                None => "cloud".into(),
            },
            prs: Vec::new(),
            ticket: None,
            // Named but not looked up: nothing here has asked `gh` about it,
            // which is exactly what this field is for (#112).
            prs_unanswered: pr.into_iter().collect(),
            // Nothing of this session is on this disk, so there is no
            // transcript to read a published page out of (#120).
            artifacts: Vec::new(),
            job: None,
            parked_job: None,
        };
        info.set_phase(phase);
        // The service's own word for the session, kept where the board keeps
        // a session's own word for itself, and its own sentence where the
        // board has somewhere to print it (#155).
        info.state = raw.status_bucket.clone();
        if phase == Phase::NeedsUser {
            if let Some(said) = stopped_on(&raw) {
                info.waiting_for = Some(said);
            }
        }
        out.push(info);
    }
    out.sort_by(|a, b| a.started_at.cmp(&b.started_at).then_with(|| a.session_id.cmp(&b.session_id)));
    out
}

struct Cached {
    sessions: Vec<SessionInfo>,
    fetched: Instant,
    good: bool,
}

/// Holds the last answer so the board can ask every tick without the network
/// hearing about it.
#[derive(Default)]
pub struct Tracker {
    inner: Mutex<Option<Cached>>,
}

impl Tracker {
    /// The cloud sessions worth drawing, refetching when the cache is due.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = match guard.as_ref() {
            Some(c) if c.good => TTL,
            _ => RETRY,
        };
        if guard.as_ref().is_none_or(|c| c.fetched.elapsed() > ttl) {
            match fetch() {
                Some(sessions) => {
                    *guard = Some(Cached { sessions, fetched: Instant::now(), good: true });
                }
                // Keep whatever we last had: one refused request is not the
                // same as the cloud having nothing, and a board that empties
                // on a flaky minute is worse than one a little stale (#59).
                None => match guard.as_mut() {
                    Some(c) => {
                        c.fetched = Instant::now();
                        c.good = false;
                    }
                    None => {
                        *guard = Some(Cached { sessions: Vec::new(), fetched: Instant::now(), good: false })
                    }
                },
            }
        }
        guard.as_ref().map(|c| c.sessions.clone()).unwrap_or_default()
    }
}

/// One request, through `curl` for the same reason `usage.rs` does it: no
/// HTTP client is linked into this binary.
fn fetch() -> Option<Vec<SessionInfo>> {
    let token = match usage::read_token() {
        Ok(t) => t,
        Err(e) => {
            // An API-key user has no OAuth login and never will; saying so
            // once a poll would be noise.
            log::debug!("no cloud sessions: {e:?}");
            return None;
        }
    };
    // Routines are not work anyone is waiting on, and a fortnight of nightly
    // runs is most of what the cloud remembers: on this account they were
    // twelve of nineteen. The endpoint drops them on request (#125).
    let url = format!("{URL}?limit={LIMIT}&include_trigger_sessions=false");
    let out = Command::new("/usr/bin/curl")
        .args(["-sS", "-m", &CURL_TIMEOUT_S.to_string(), "-w", "\n%{http_code}"])
        .args(["-H", &format!("Authorization: Bearer {token}")])
        .args(["-H", "anthropic-beta: oauth-2025-04-20"])
        .args(["-H", "anthropic-version: 2023-06-01"])
        .args(["-H", "User-Agent: claudmagi"])
        .arg(&url)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        log::warn!("cloud sessions: curl failed");
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n')?;
    if code.trim() != "200" {
        log::warn!("cloud sessions: HTTP {}", code.trim());
        return None;
    }
    let mut sessions = parse(body, now_secs());
    // Only a session with a turn in flight is asked (#258). Subagents run
    // inside a turn, so a session between turns has nothing running, and a
    // round trip for a list of finished agents is a round trip for
    // nothing. It is also what keeps this to one extra call on a poll
    // rather than one per session.
    for s in sessions.iter_mut().filter(|s| s.phase() == Phase::Working) {
        s.subagents = subagents(&s.session_id, &token);
    }
    Some(sessions)
}

/// How many of a session's newest events to read when looking for what it
/// has running (#258). The endpoint hands them back newest first, so this
/// is a window on the present, not a page into the past.
const EVENTS: usize = 150;

/// The subagents a cloud session is running, read from its own
/// conversation.
///
/// Nothing in the sessions endpoint says: not the list, not the detail,
/// and not `task_summary`, which is one line about the tool running this
/// second. The conversation does. `/events` hands back the same assistant
/// and user messages a local transcript holds (#1), and a subagent is a
/// `Task` tool call in it — one with no result yet is still running.
fn subagents(id: &str, token: &str) -> Vec<SubagentInfo> {
    let url = format!("{URL}/{id}/events?limit={EVENTS}");
    let Some(body) = ask(&url, token) else { return Vec::new() };
    task_agents(&body)
}

/// One GET, through `curl` for the same reason the rest of this file does
/// it. `None` covers every way of not being told, which is never an error
/// worth emptying the board over (#58).
fn ask(url: &str, token: &str) -> Option<String> {
    let out = Command::new("/usr/bin/curl")
        .args(["-sS", "-m", &CURL_TIMEOUT_S.to_string(), "-w", "\n%{http_code}"])
        .args(["-H", &format!("Authorization: Bearer {token}")])
        .args(["-H", "anthropic-beta: oauth-2025-04-20"])
        .args(["-H", "anthropic-version: 2023-06-01"])
        .args(["-H", "User-Agent: claudmagi"])
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n')?;
    (code.trim() == "200").then(|| body.to_string())
}

/// The `Task` calls in a page of events, oldest first.
///
/// A call and its result are two events: the assistant asks, and the
/// result comes back addressed to the call's own id. So a call whose id no
/// result names is one still in flight — the same rule a local transcript
/// is read by, said in the shape the wire uses.
pub fn task_agents(body: &str) -> Vec<SubagentInfo> {
    let Ok(page) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    let Some(events) = page.get("data").and_then(|d| d.as_array()) else { return Vec::new() };
    let mut started: Vec<SubagentInfo> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    for event in events {
        let at = millis_of(event.get("created_at").and_then(|v| v.as_str()));
        let blocks = event
            .get("payload")
            .and_then(|p| p.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array());
        for block in blocks.into_iter().flatten() {
            let kind = block.get("type").and_then(|t| t.as_str()).unwrap_or_default();
            match kind {
                "tool_use" if block.get("name").and_then(|n| n.as_str()) == Some("Task") => {
                    let input = block.get("input");
                    let text = |key: &str| {
                        input.and_then(|i| i.get(key)).and_then(|v| v.as_str()).unwrap_or_default().to_string()
                    };
                    started.push(SubagentInfo {
                        agent_id: block.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        agent_type: text("subagent_type"),
                        description: text("description"),
                        // Filled in once every result has been seen.
                        running: true,
                        started_at: at,
                        updated_at: at,
                    });
                }
                "tool_result" => {
                    if let Some(id) = block.get("tool_use_id").and_then(|v| v.as_str()) {
                        done.push(id.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    started.retain(|a| !a.agent_id.is_empty());
    for agent in &mut started {
        agent.running = !done.contains(&agent.agent_id);
    }
    // The page comes newest first; the board draws them in the order they
    // were started (`sources::subagents_for`).
    started.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.agent_id.cmp(&b.agent_id)));
    started
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BoardModel;

    const NOW: u64 = 1_790_000_000;

    /// A cloud session's subagents come from its own conversation (#258):
    /// nothing the sessions endpoint says mentions them. A `Task` call and
    /// its result are two events, and a call no result names is one still
    /// in flight.
    #[test]
    fn a_task_nobody_answered_is_a_subagent_still_running() {
        // As the wire sends it: newest first.
        let body = r#"{"data":[
          {"created_at":"2026-11-27T02:51:23.190076Z","event_type":"assistant","payload":{"message":{"content":[
            {"type":"tool_use","id":"toolu_b","name":"Task",
             "input":{"subagent_type":"Explore","description":"Find the relevant files"}}]}}},
          {"created_at":"2026-11-27T02:50:00Z","event_type":"user","payload":{"message":{"content":[
            {"type":"tool_result","tool_use_id":"toolu_a","content":"done"}]}}},
          {"created_at":"2026-11-27T02:49:00Z","event_type":"assistant","payload":{"message":{"content":[
            {"type":"tool_use","id":"toolu_a","name":"Task",
             "input":{"subagent_type":"Plan","description":"Design the approach"}},
            {"type":"tool_use","id":"toolu_x","name":"Bash","input":{"command":"ls"}}]}}},
          {"created_at":"2026-11-27T02:48:00Z","event_type":"system","payload":{"agents":["Task","Plan"]}}
        ],"next_cursor":"12"}"#;

        let found = task_agents(body);
        assert_eq!(found.len(), 2, "a Bash call is not a subagent, and neither is a list of tool names");
        assert_eq!(found[0].agent_id, "toolu_a", "oldest first, the way the board draws them");
        assert_eq!(found[0].agent_type, "Plan");
        assert_eq!(found[0].description, "Design the approach");
        assert!(!found[0].running, "its result came back");
        assert_eq!(found[1].agent_type, "Explore");
        assert!(found[1].running, "nothing has answered this one");
        assert!(found[1].started_at > found[0].started_at, "and it started later");
        // The wire writes microseconds; a stamp that did not parse would
        // leave every agent at zero and the order with it.
        assert!(found[1].started_at > 1_700_000_000_000, "microseconds and all: {}", found[1].started_at);
    }

    /// Not being told is not an error, and nothing here is a shape worth
    /// trusting: an answer that is not a page of events is simply no
    /// subagents (#58).
    #[test]
    fn an_answer_that_is_not_events_is_no_subagents() {
        for body in ["", "null", "{}", "{\"data\":null}", "{\"data\":[]}", "not json at all"] {
            assert!(task_agents(body).is_empty(), "{body:?}");
        }
        // A Task call with no id of its own cannot be matched to a result,
        // so it is not one the board can speak for.
        let nameless = r#"{"data":[{"created_at":"2026-11-27T02:51:00Z","event_type":"assistant",
          "payload":{"message":{"content":[{"type":"tool_use","name":"Task","input":{}}]}}}]}"#;
        assert!(task_agents(nameless).is_empty());
    }


    fn body(rows: &str) -> String {
        format!(r#"{{"data":[{rows}],"resume_token":"x"}}"#)
    }

    fn at(secs_ago: u64) -> String {
        let t = NOW - secs_ago;
        // The endpoint sends RFC3339; build one the parser accepts.
        let days = t / 86_400;
        let _ = days;
        format!("{}", iso(t))
    }

    fn iso(unix: u64) -> String {
        // 1970-01-01 plus `unix` seconds, to the second, UTC.
        let (mut days, rem) = (unix / 86_400, unix % 86_400);
        let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        let (mut y, mut mo) = (1970u64, 1u64);
        loop {
            let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
            let len = if leap { 366 } else { 365 };
            if days < len {
                break;
            }
            days -= len;
            y += 1;
        }
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for len in months {
            if days < len {
                break;
            }
            days -= len;
            mo += 1;
        }
        format!("{y:04}-{mo:02}-{:02}T{h:02}:{m:02}:{s:02}Z", days + 1)
    }

    /// A cloud session has no process and no transcript, so everything the
    /// board shows has to come out of this one answer (#120).
    #[test]
    fn a_cloud_session_becomes_a_chip() {
        let rows = format!(
            r#"{{"id":"cse_1","title":"Yoshi E2E","connection_status":"connected","worker_status":"idle",
                 "status_bucket":"review_ready","unread":false,"last_event_at":"{}","created_at":"{}",
                 "config":{{"origin":"desktop_app","outcomes":[{{"git_info":{{"repo":"cookieplace/crepe"}}}}]}},
                 "external_metadata":{{"post_turn_summary":{{"needs_action":"","status_detail":"PR #9312"}}}}}}"#,
            at(60),
            at(3600)
        );
        let list = parse(&body(&rows), NOW);
        assert_eq!(list.len(), 1);
        let s = &list[0];
        assert_eq!(s.name, "Yoshi E2E");
        assert_eq!(s.cwd, "cookieplace/crepe");
        assert_eq!(s.group, "cloud-tab:cookieplace/crepe", "sessions on one repo cluster");
        assert_eq!(s.pid, 0, "there is no process to point at");
        assert!(!s.synthetic, "it is a real session, just not one of ours");
        let seat = s.seat.as_ref().expect("a cloud session is seated in the cloud");
        assert_eq!(seat.host, "cloud");
        assert_eq!(seat.focus_url.as_deref(), Some("claude://code/cse_1"));
    }

    /// The board's four phases, out of fields that do not look like them.
    #[test]
    fn the_phase_comes_from_the_worker_and_from_who_is_waiting() {
        let row = |extra: &str| {
            format!(
                r#"{{"id":"cse_1","title":"t","connection_status":"connected","last_event_at":"{}",{extra}}}"#,
                at(60)
            )
        };
        let phase = |extra: &str| parse(&body(&row(extra)), NOW)[0].phase();

        assert_eq!(phase(r#""worker_status":"running","unread":true"#), Phase::Working, "a turn in flight wins");
        // Cloud work finishes while nobody is looking. That a person has not
        // opened it yet is said by the tile being there at all, and by
        // nothing else: it is a finished session, so it is idle (#259, #260).
        assert_eq!(phase(r#""worker_status":"idle","unread":true"#), Phase::Idle);
        assert_eq!(
            phase(
                r#""worker_status":"idle","unread":false,
                   "external_metadata":{"post_turn_summary":{"needs_action":"pick a branch"}}"#
            ),
            Phase::NeedsUser,
            "a summary naming an action is the same thing said out loud"
        );
        assert_eq!(phase(r#""worker_status":"idle","unread":false"#), Phase::Idle);
    }

    /// The service says what it makes of each session, in five words, and
    /// the board says them back rather than working them out again (#155).
    #[test]
    fn the_service_s_own_word_decides_the_phase() {
        let row = |extra: &str| {
            format!(
                r#"{{"id":"cse_1","title":"t","connection_status":"connected","last_event_at":"{}",{extra}}}"#,
                at(60)
            )
        };
        let phase = |extra: &str| parse(&body(&row(extra)), NOW)[0].phase();

        assert_eq!(phase(r#""status_bucket":"working","unread":false"#), Phase::Working);
        assert_eq!(
            phase(r#""status_bucket":"blocked","unread":false"#),
            Phase::NeedsUser,
            "the one word that means a person is being waited on"
        );
        // #259: the accent is for a session that asked something. A run that
        // stopped badly asked nothing — it is over.
        assert_eq!(phase(r#""status_bucket":"failed","unread":true"#), Phase::Idle, "stopped badly, nobody looked");
        assert_eq!(phase(r#""status_bucket":"failed","unread":false"#), Phase::Idle, "stopped badly, and seen");
        assert_eq!(phase(r#""status_bucket":"completed","unread":true"#), Phase::Idle);
        // (`completed` once it has been seen is not on the board at all, so
        // it has no phase to ask about — see the keep test below.)
        assert_eq!(
            phase(r#""status_bucket":"review_ready","unread":true"#),
            Phase::Idle,
            "finished with something to show: on the board, and quiet about it"
        );
        assert_eq!(phase(r#""status_bucket":"review_ready","unread":false"#), Phase::Idle, "looked at: just done");

        // The fault, in one line. The two words the service uses for a
        // finished run differ in what it left behind, not in whether it
        // wants anybody — and `unread` says nothing about that either.
        // Watching one session flip from one word to the other with nobody
        // touching it is what caught this (#259).
        for word in ["review_ready", "completed", "failed", "hibernating"] {
            for seen in ["true", "false"] {
                // `completed` once it has been seen leaves the board
                // altogether (#186), so there is no phase to ask it for.
                if (word, seen) == ("completed", "false") {
                    continue;
                }
                assert_eq!(
                    phase(&format!(r#""status_bucket":"{word}","unread":{seen}"#)),
                    Phase::Idle,
                    "{word}/{seen} is a finished run and the board has nothing to ask about it"
                );
            }
        }

        // A turn in flight is the one thing the board can see for itself.
        assert_eq!(phase(r#""status_bucket":"blocked","worker_status":"running""#), Phase::Working);
        // The live word first, the last turn's word when there is no live one.
        assert_eq!(
            phase(r#""unread":true,"external_metadata":{"post_turn_summary":{"status_category":"failed"}}"#),
            Phase::Idle
        );

        // The session's own sentence outranks any bucket word, including
        // one that says the run is over: a turn that ends by naming what it
        // wants from a person has not finished (#259). Seen live —
        // `purple-haro 주간 트렌드 차트` sat at `completed` with
        // "프롬프트 교체" in `needs_action`, and the board called it idle.
        assert_eq!(
            phase(
                r#""status_bucket":"completed","unread":false,
                   "external_metadata":{"post_turn_summary":
                   {"status_category":"need_input","needs_action":"replace the prompt"}}"#
            ),
            Phase::NeedsUser,
            "a run that named what it wants is not finished, whatever the bucket says"
        );

        // What it stopped on, in its own words: the board prints this after
        // "needs you:" and was printing `dialog open` for every one of them.
        let said = &parse(
            &body(&row(
                r#""status_bucket":"blocked","external_metadata":{"post_turn_summary":{"status_detail":"need the report link"}}"#,
            )),
            NOW,
        )[0];
        assert_eq!(said.waiting_for.as_deref(), Some("need the report link"));
        assert_eq!(said.state.as_deref(), Some("blocked"), "the service's own word is kept");
        // With nothing said, the word itself is still better than a dialog
        // that does not exist.
        let bare = &parse(&body(&row(r#""status_bucket":"failed""#)), NOW)[0];
        assert_eq!(bare.state.as_deref(), Some("failed"));
        assert_eq!(bare.waiting_for, None, "nothing is waiting on a person here (#259)");

        // A session holding something nobody has read is on the board and
        // quiet: `waiting_for` is the one field that would put it in the
        // accent, and nothing fills it (#259, #260).
        let held = &parse(
            &body(&row(
                r#""status_bucket":"review_ready","unread":true,"external_metadata":{"post_turn_summary":
                   {"status_detail":"PR CI passed, awaiting human review"}}"#,
            )),
            NOW,
        )[0];
        assert_eq!(held.phase(), Phase::Idle);
        assert_eq!(held.waiting_for, None, "unread never reads as waiting on you");
        assert_eq!(held.state.as_deref(), Some("review_ready"), "its own word is what the detail line shows");

        // And a finished one says which kind of finished it is, where the
        // board would otherwise call a session that is not on this machine
        // and not at a prompt "idle".
        let done = parse(&body(&row(r#""status_bucket":"review_ready","unread":false"#)), NOW);
        let mut model = BoardModel::new();
        model.apply(done, std::time::Instant::now());
        model.settle();
        model.hovered = Some(crate::model::Target::Session(0));
        assert!(model.detail().contains("review_ready"), "the detail line said: {}", model.detail());
    }

    /// A session that is finished and has been looked at is not going on
    /// (#186). The board kept these because their container was still up, so
    /// a session closed in the app sat on the board pointing at work that
    /// was over.
    #[test]
    fn a_finished_session_somebody_has_looked_at_is_not_on_the_board() {
        let row = |extra: &str| {
            format!(
                r#"{{"id":"cse_1","title":"t","connection_status":"connected","last_event_at":"{}",{extra}}}"#,
                at(60)
            )
        };
        let kept = |extra: &str| parse(&body(&row(extra)), NOW).len();

        assert_eq!(kept(r#""status_bucket":"completed","unread":false"#), 0, "finished and seen");
        assert_eq!(
            kept(r#""status_bucket":"completed","unread":true"#),
            1,
            "finished while nobody was looking is the whole point of the window (#120)"
        );
        // Only the word `completed` means there is nothing to look at. The
        // service draws that line itself (#155).
        for busy in ["working", "blocked", "failed", "review_ready"] {
            assert_eq!(kept(&format!(r#""status_bucket":"{busy}","unread":false"#)), 1, "{busy} is not finished");
        }
        // And the last turn's word does just as well when there is no live
        // one, which is the order `said` already reads them in.
        assert_eq!(
            kept(r#""unread":false,"external_metadata":{"post_turn_summary":{"status_category":"completed"}}"#),
            0
        );
        // But a run that named what it wants from a person stays, whatever
        // bucket it was put in (#259). This is the fault pointing the other
        // way: `purple-haro 주간 트렌드 차트` sat at `completed`, seen,
        // with "프롬프트 교체" written in `needs_action`, and the board
        // threw away the one session that actually wanted something.
        assert_eq!(
            kept(
                r#""status_bucket":"completed","unread":false,
                   "external_metadata":{"post_turn_summary":{"needs_action":"replace the prompt"}}"#
            ),
            1,
            "a run that asked for something is not finished"
        );
    }

    /// A board is for what is going on now. The cloud remembers every
    /// scheduled run for weeks, and a fortnight of them would bury the work.
    #[test]
    fn only_work_somebody_could_still_be_waiting_on_shows() {
        let row = |id: &str, conn: &str, ago: u64| {
            format!(
                r#"{{"id":"{id}","title":"t","connection_status":"{conn}","worker_status":"idle",
                     "unread":false,"last_event_at":"{}"}}"#,
                at(ago)
            )
        };
        let rows = [
            row("fresh", "disconnected", 60),
            row("stale", "disconnected", RECENT.as_secs() + 60),
            // Still connected, however long since it last said anything.
            row("live", "connected", RECENT.as_secs() * 10),
        ]
        .join(",");
        let list = parse(&body(&rows), NOW);
        let ids: Vec<&str> = list.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"fresh"), "work from an hour ago is still somebody's business");
        assert!(ids.contains(&"live"), "a container that is still up is current whatever its clock says");
        assert!(!ids.contains(&"stale"), "last week's scheduled run is not");
    }

    /// Nothing about a bad answer may empty the board or panic it.
    #[test]
    fn a_body_that_makes_no_sense_is_no_sessions() {
        assert!(parse("", NOW).is_empty());
        assert!(parse("null", NOW).is_empty());
        assert!(parse(r#"{"error":{"type":"not_found"}}"#, NOW).is_empty());
        // A row with nothing but an id still has to produce a drawable chip.
        let list = parse(&body(r#"{"id":"cse_bare","connection_status":"connected"}"#), NOW);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "cse_bare", "a nameless session wears its id");
        assert_eq!(list[0].group, "cloud", "and groups with the other homeless ones");
    }

    /// A title's lightning bolt is decoration, and the board has its own way
    /// of saying what a thing is.
    #[test]
    fn a_title_loses_its_decoration() {
        let rows = format!(
            r#"{{"id":"cse_1","title":"⚡ nightly","connection_status":"connected","last_event_at":"{}"}}"#,
            at(10)
        );
        assert_eq!(parse(&body(&rows), NOW)[0].name, "nightly");
    }

    /// A routine is not work anyone is waiting on (#125). The endpoint is
    /// asked to leave them out, and this is the guard for the day that
    /// parameter is renamed: on this account they were two thirds of
    /// everything the cloud remembered.
    #[test]
    fn a_scheduled_run_never_reaches_the_board() {
        let row = |id: &str, trigger: &str| {
            format!(
                r#"{{"id":"{id}","title":"nightly","connection_status":"connected","last_event_at":"{}"{trigger}}}"#,
                at(10)
            )
        };
        let rows = [row("routine", r#","trigger_id":"trg_1""#), row("work", "")].join(",");
        let list = parse(&body(&rows), NOW);
        let ids: Vec<&str> = list.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, vec!["work"], "a session a schedule started is not on the board");
    }
    /// Archiving is the person saying they are done with it, and the board
    /// is for what is going on now (#136).
    #[test]
    fn an_archived_session_is_off_the_board() {
        let row = |id: &str, status: &str, conn: &str| {
            format!(
                r#"{{"id":"{id}","title":"t","status":"{status}","connection_status":"{conn}",
                     "worker_status":"running","unread":true,"last_event_at":"{}"}}"#,
                at(60)
            )
        };
        let rows = [
            row("put-away", "archived", "disconnected"),
            // Archived while the container is still up and a turn is in
            // flight: the person has still said they are done with it.
            row("put-away-busy", "archived", "connected"),
            row("working", "active", "connected"),
            // A status this board has never heard of is not a reason to drop
            // somebody's session.
            row("unknown", "something_new", "connected"),
        ]
        .join(",");
        let list = parse(&body(&rows), NOW);
        let ids: Vec<&str> = list.iter().map(|s| s.session_id.as_str()).collect();
        assert!(!ids.contains(&"put-away"), "an archived session is still on the board");
        assert!(!ids.contains(&"put-away-busy"), "archiving outranks the container being up");
        assert!(ids.contains(&"working"));
        assert!(ids.contains(&"unknown"), "a status nobody here knows is not a reason to drop a session");
    }

    /// A cloud session has no transcript here, so the only place a pull
    /// request can come from is the sentence the last turn wrote about
    /// itself (#120).
    #[test]
    fn the_summary_gives_up_its_pull_request() {
        let row = |detail: &str, outcomes: &str| {
            format!(
                r#"{{"id":"cse_1","title":"t","connection_status":"connected","last_event_at":"{}",
                     "config":{{"outcomes":[{outcomes}]}},
                     "external_metadata":{{"post_turn_summary":{{"status_detail":"{detail}"}}}}}}"#,
                at(60)
            )
        };
        let repo = r#"{"git_info":{"repo":"cookieplace/crepe"}}"#;
        let named = parse(&body(&row("PR #9312 E2E passed (194/194); awaiting approval", repo)), NOW);
        assert_eq!(
            named[0].prs_unanswered,
            vec![crate::pr::PrRef { repo: "cookieplace/crepe".into(), number: 9312 }],
            "the number is in the sentence and the repo is in the config"
        );

        // Nothing invented where there is nothing to read.
        assert!(parse(&body(&row("no pull request here", repo)), NOW)[0].prs_unanswered.is_empty());
        // A number is not a pull request for having a hash in front of it
        // (#154). This one is a watch counting its own rounds, and the repo
        // it is working in has a pull request 307 somebody merged years ago.
        assert!(
            parse(&body(&row("poll #307 done; no new critical/high cards", repo)), NOW)[0]
                .prs_unanswered
                .is_empty(),
            "a sentence counting something else was read as naming a pull request"
        );
        // A link says it outright, hash or no hash.
        assert_eq!(
            parse(&body(&row("merged https://github.com/cookieplace/crepe/pull/9312 just now", repo)), NOW)[0]
                .prs_unanswered,
            vec![crate::pr::PrRef { repo: "cookieplace/crepe".into(), number: 9312 }],
            "a link to one is the plainest naming there is"
        );
        assert!(
            parse(&body(&row("PR #9312 passed", "")), NOW)[0].prs_unanswered.is_empty(),
            "a number without a repository is not a pull request anyone can open"
        );
        // A hash that is not a number is not one either.
        assert!(parse(&body(&row("see #main for details", repo)), NOW)[0].prs_unanswered.is_empty());
    }

}

