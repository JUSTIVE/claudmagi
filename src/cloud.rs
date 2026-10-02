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

use crate::model::{Phase, SessionInfo};
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
    /// `connected` while the container is up.
    #[serde(default)]
    connection_status: Option<String>,
    /// `idle` between turns; anything else is a turn in flight.
    #[serde(default)]
    worker_status: Option<String>,
    #[serde(default)]
    unread: bool,
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
}

#[derive(Debug, Deserialize)]
struct Page {
    #[serde(default)]
    data: Vec<Raw>,
}

/// Where a cloud session's chip points. The web app is the one place every
/// origin can be opened from; the desktop app takes the same link.
pub fn session_url(id: &str) -> String {
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
/// `worker_status` answers it outright while a turn is in flight. Between
/// turns the question is whether anyone is being waited on, and two things
/// say so: the summary naming an action, or a finished session the person
/// has not opened. The second is the common one — cloud work finishes while
/// nobody is looking, which is exactly what a board is for.
fn phase_of(raw: &Raw) -> Phase {
    let worker = raw.worker_status.as_deref().unwrap_or("idle");
    if worker != "idle" {
        return Phase::Working;
    }
    let needs = raw
        .external_metadata
        .post_turn_summary
        .as_ref()
        .and_then(|s| s.needs_action.as_deref())
        .unwrap_or("")
        .trim();
    if !needs.is_empty() || raw.unread {
        return Phase::NeedsUser;
    }
    Phase::Idle
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
    let number = detail.split('#').skip(1).find_map(|rest| {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u32>().ok().filter(|n| *n > 0)
    })?;
    Some(crate::pr::PrRef { repo: repo.to_string(), number })
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
        let connected = raw.connection_status.as_deref() == Some("connected");
        let last = usage::parse_iso8601(raw.last_event_at.as_deref().unwrap_or("")).unwrap_or(0);
        // A container that is still up is current whatever its clock says.
        // Everything else has to be recent enough to still be somebody's
        // business.
        if !connected && now.saturating_sub(last) > RECENT.as_secs() {
            continue;
        }
        let phase = phase_of(&raw);
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
                focus_url: Some(session_url(&raw.id)),
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
            job: None,
            parked_job: None,
        };
        info.set_phase(phase);
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
    let url = format!("{URL}?limit={LIMIT}&include_trigger_sessions=true");
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
    Some(parse(body, now_secs()))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000;

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
        assert_eq!(seat.focus_url.as_deref(), Some("https://claude.ai/code/cse_1"));
    }

    /// The board's three phases, out of fields that do not look like them.
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
        // Cloud work finishes while nobody is looking, so an unread finished
        // session is a session waiting on a person.
        assert_eq!(phase(r#""worker_status":"idle","unread":true"#), Phase::NeedsUser);
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

    /// A scheduled run's bolt is decoration, and the board has its own way of
    /// saying what a thing is.
    #[test]
    fn a_scheduled_run_loses_its_decoration() {
        let rows = format!(
            r#"{{"id":"cse_1","title":"⚡ graphql-coverage-daily","connection_status":"connected","last_event_at":"{}"}}"#,
            at(10)
        );
        assert_eq!(parse(&body(&rows), NOW)[0].name, "graphql-coverage-daily");
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
        assert!(
            parse(&body(&row("PR #9312 passed", "")), NOW)[0].prs_unanswered.is_empty(),
            "a number without a repository is not a pull request anyone can open"
        );
        // A hash that is not a number is not one either.
        assert!(parse(&body(&row("see #main for details", repo)), NOW)[0].prs_unanswered.is_empty());
    }

}
