//! What Linear is asking of you (#241).
//!
//! The board is the place for work in flight; an inbox is the place for
//! people waiting on *you*, which is a different question and so gets a
//! different place to stand — the ⌘K list and a count in the status bar,
//! never a tile. Half of what arrives here is about work no session on this
//! machine is doing (a review someone asked for, a question on a ticket
//! nobody has picked up), and a board of live sessions has nowhere to put
//! that.
//!
//! **Only unread counts**, and Linear is what decides that: `inboxNotifications`
//! is the viewer's own inbox, archived and snoozed already taken out, one
//! entry per group of related activity, and `unreadOnly` on top of it.
//! Reading it in Linear clears it here on the next poll, so nothing is ever
//! put away by hand — #194 exists for tiles that outlive their work, and a
//! notification outlives nothing.
//!
//! **A row is composed here, not served.** `Notification` carries no title,
//! subtitle or link of its own: it carries the actor, the kind, and whatever
//! entity it is about. So the issue's own `identifier` is the key in the
//! chip (no parsing a key out of a URL), the comment's URL is where the row
//! leads, and the kind becomes words by taking the entity off the front and
//! splitting the rest — `issueCommentMention` → "comment mention". A table
//! of hand-written phrases would be wrong the first time Linear adds a kind;
//! this is only ever clumsy.
//!
//! The key is a Linear personal API key in the login keychain under
//! `claudmagi-linear`, read with `/usr/bin/security` and spent through
//! `/usr/bin/curl` — the same two programs `usage.rs` (#48) and `cloud.rs`
//! (#120) use, because no HTTP client is linked into this binary. With no
//! key the board says nothing about Linear rather than complaining, the way
//! a closed Orca leaves a ticket showing just its key (#58).

use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Deserialize;

/// The login-keychain item the key lives in.
pub const KEYCHAIN_SERVICE: &str = "claudmagi-linear";
/// Read first, for a one-off run from a terminal. A bundle launched from
/// Finder has no environment to speak of, so the keychain is the real path.
pub const KEY_ENV: &str = "LINEAR_API_KEY";
pub const URL: &str = "https://api.linear.app/graphql";
const CURL_TIMEOUT_S: u64 = 10;
/// How often the inbox is asked. Sixty an hour against a personal key's
/// 1500 is a rounding error.
pub const POLL: Duration = Duration::from_secs(60);
/// How many stacks the query asks for. Unread is a handful on any account
/// that is read at all; this is the ceiling, not the expectation.
const LIMIT: usize = 50;
/// How much of a comment a row carries. The row is one line and truncates
/// on screen anyway; this keeps the string itself sane.
const EXCERPT: usize = 120;

/// The one request. `$N` is the only thing substituted into it.
///
/// Newest first: the schema's own word on `first` is "results are newest
/// first", which is the order a list of what is still asking wants.
const QUERY: &str = "query { \
viewer { organization { urlKey } } \
inboxNotifications(first: $N, unreadOnly: true) { nodes { \
id type category actor { displayName } \
... on IssueNotification { issue { identifier title url } comment { url body } } \
... on ProjectNotification { project { name url } comment { url body } } \
} } }";

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// No key anywhere. Nothing here can fix it, but it is not fatal: the
    /// board goes on without the inbox.
    NoKey,
    /// `curl` failed, or the API answered with an error status.
    Request(String),
    /// The API answered with something unreadable, or with GraphQL errors.
    Response(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoKey => write!(
                f,
                "no Linear API key: security add-generic-password -U -A -s {KEYCHAIN_SERVICE} -a $USER -w"
            ),
            Error::Request(e) => write!(f, "Linear request failed: {e}"),
            Error::Response(e) => write!(f, "unreadable Linear response: {e}"),
        }
    }
}

/// One unread notification: who wants something, what kind of something, and
/// what it is about.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Note {
    pub id: String,
    /// Linear's own name for the kind (`issueCommentMention`, …).
    pub kind: String,
    /// Linear's own grouping (`reviews`, `mentions`, …), for the
    /// diagnostics line.
    pub category: String,
    /// Who caused it. Empty when an integration or Linear itself did.
    pub actor: String,
    /// The issue's key, straight from Linear rather than parsed out of a
    /// URL. `None` for anything that is not about an issue.
    pub key: Option<String>,
    /// What it is about: the issue's title, or the project's name.
    pub title: String,
    /// The first line of the comment, when there is one. This is usually the
    /// part that says what is actually being asked.
    pub body: String,
    pub url: String,
}

impl Note {
    /// What goes in the chip: the key when there is one, so the row is found
    /// by typing it the way every other row on the list is (#71).
    pub fn label(&self) -> String {
        self.key.clone().unwrap_or_else(|| "LINEAR".to_string())
    }

    /// The kind as words: the entity off the front, the rest split where it
    /// changes case. `pullRequestReviewRequested` → "review requested".
    pub fn verb(&self) -> String {
        const ENTITIES: [&str; 6] = ["issue", "pullRequest", "project", "document", "initiative", "agentSession"];
        let mut rest = self.kind.as_str();
        for e in ENTITIES {
            if let Some(tail) = rest.strip_prefix(e) {
                if !tail.is_empty() {
                    rest = tail;
                }
                break;
            }
        }
        let mut out = String::new();
        for c in rest.chars() {
            if c.is_ascii_uppercase() && !out.is_empty() {
                out.push(' ');
            }
            out.extend(c.to_lowercase());
        }
        out
    }

    /// The row's line: who wants what, then what it is about, then the words
    /// they used.
    pub fn detail(&self) -> String {
        let who = match (self.actor.trim(), self.verb()) {
            ("", v) => v,
            (a, v) if v.is_empty() => a.to_string(),
            (a, v) => format!("{a} {v}"),
        };
        [who.as_str(), self.title.trim(), self.body.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// What one look at the inbox found: what is in it, and where it is.
///
/// The page matters as much as the rows: the badge at the top of the window
/// leads to the inbox itself (#242), and a notification about a pull request
/// has nowhere else to go — Linear gives one no page of its own.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reading {
    /// The workspace's inbox page.
    pub url: String,
    pub notes: Vec<Note>,
}

impl Reading {
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }
}

/// The `linear://` twin of a `linear.app` link.
pub fn app_url(web: &str) -> Option<String> {
    web.strip_prefix("https://linear.app/").map(|rest| format!("linear://{rest}"))
}

/// The first line of a comment, short enough to live on one row.
fn excerpt(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    if line.chars().count() <= EXCERPT {
        return line.to_string();
    }
    line.chars().take(EXCERPT).collect::<String>().trim_end().to_string() + "…"
}

#[derive(Deserialize)]
struct Resp {
    data: Option<Data>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Data {
    #[serde(default)]
    viewer: Option<Viewer>,
    #[serde(rename = "inboxNotifications")]
    inbox: Nodes,
}

#[derive(Deserialize)]
struct Viewer {
    #[serde(default)]
    organization: Option<Org>,
}

#[derive(Deserialize)]
struct Org {
    #[serde(rename = "urlKey", default)]
    url_key: Option<String>,
}

#[derive(Deserialize)]
struct Nodes {
    nodes: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    actor: Option<Named>,
    #[serde(default)]
    issue: Option<IssueRaw>,
    #[serde(default)]
    project: Option<Named>,
    #[serde(default)]
    comment: Option<CommentRaw>,
}

#[derive(Deserialize)]
struct Named {
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct IssueRaw {
    #[serde(default)]
    identifier: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct CommentRaw {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

/// The unread notifications in an API answer, in the order it gave them.
///
/// A GraphQL error comes back with HTTP 200, so the body is the only thing
/// that says whether the answer is any good.
pub fn parse(body: &str) -> Result<Reading, Error> {
    let resp: Resp = serde_json::from_str(body).map_err(|e| Error::Response(e.to_string()))?;
    if !resp.errors.is_empty() {
        let first = resp.errors.into_iter().map(|e| e.message).find(|m| !m.is_empty());
        return Err(Error::Response(first.unwrap_or_else(|| "GraphQL error".into())));
    }
    let Some(data) = resp.data else {
        return Err(Error::Response("no data".into()));
    };
    // Where a row goes when the thing it is about has no page of its own —
    // a review notification carries an id and nothing else.
    let inbox_url = data
        .viewer
        .and_then(|v| v.organization)
        .and_then(|o| o.url_key)
        .map(|k| format!("https://linear.app/{k}/inbox"))
        .unwrap_or_default();
    let notes = data
        .inbox
        .nodes
        .into_iter()
        .map(|n| {
            let issue = n.issue.unwrap_or(IssueRaw { identifier: None, title: None, url: None });
            let project = n.project;
            let comment = n.comment;
            Note {
                id: n.id,
                kind: n.kind,
                category: n.category,
                actor: n.actor.and_then(|a| a.display_name.or(a.name)).unwrap_or_default(),
                key: issue.identifier,
                title: issue.title.or_else(|| project.as_ref().and_then(|p| p.name.clone())).unwrap_or_default(),
                body: comment.as_ref().and_then(|c| c.body.as_deref()).map(excerpt).unwrap_or_default(),
                // The comment is the thing being pointed at when there is
                // one; the issue is where it lives; the inbox is the last
                // resort.
                url: comment
                    .and_then(|c| c.url)
                    .or(issue.url)
                    .or_else(|| project.and_then(|p| p.url))
                    .unwrap_or_else(|| inbox_url.clone()),
            }
        })
        .collect();
    Ok(Reading { url: inbox_url, notes })
}

/// The key, from the environment first and the login keychain second.
pub fn read_key() -> Option<String> {
    let from_env = std::env::var(KEY_ENV).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    from_env.or_else(keychain_key)
}

fn keychain_key() -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let key = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!key.is_empty()).then_some(key)
}

/// The body of the one request, as JSON.
fn request_body() -> String {
    let query = QUERY.replace("$N", &LIMIT.to_string());
    serde_json::json!({ "query": query }).to_string()
}

/// One round trip. Blocks; call it off the engine thread.
pub fn fetch() -> Result<Reading, Error> {
    parse(&ask(&request_body())?)
}

/// Sends one GraphQL body and hands back what came out, for `fetch` and for
/// asking the live schema what a kind of notification actually carries.
pub fn ask(body: &str) -> Result<String, Error> {
    let key = read_key().ok_or(Error::NoKey)?;
    let out = Command::new("/usr/bin/curl")
        .args(["-sS", "-m", &CURL_TIMEOUT_S.to_string(), "-w", "\n%{http_code}"])
        .args(["-H", "Content-Type: application/json"])
        // Linear takes a personal key raw, with no `Bearer` in front of it.
        .args(["-H", &format!("Authorization: {key}")])
        .args(["-H", "User-Agent: claudmagi"])
        .args(["-d", body])
        .arg(URL)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Request(e.to_string()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(Error::Request(if err.is_empty() { "curl failed".into() } else { err }));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').ok_or_else(|| Error::Response("empty".into()))?;
    if code.trim() != "200" {
        return Err(Error::Request(format!("HTTP {}", code.trim())));
    }
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(nodes: &str) -> String {
        format!(
            "{{\"data\":{{\"viewer\":{{\"organization\":{{\"urlKey\":\"cookieplace\"}}}},\
\"inboxNotifications\":{{\"nodes\":[{nodes}]}}}}}}"
        )
    }

    /// Linear decides what is still asking, not this. The inbox query is
    /// the viewer's own, which has archived and snoozed taken out already,
    /// and `unreadOnly` takes out what has been looked at.
    #[test]
    fn the_query_asks_the_inbox_for_what_is_unread() {
        let body = request_body();
        for wanted in ["inboxNotifications", "unreadOnly: true", "first: 50", "identifier", "displayName", "urlKey"] {
            assert!(body.contains(wanted), "{wanted} missing from {body}");
        }
        assert!(!body.contains("$N"), "limit not substituted: {body}");
    }

    /// A row says who wants what, about which ticket, in their own words —
    /// and the key comes from the issue itself, so nothing has to be read
    /// out of a URL.
    #[test]
    fn a_row_says_who_wants_what_and_about_what() {
        let text = body(
            "{\"id\":\"n1\",\"type\":\"issueCommentMention\",\"category\":\"mentions\",\
\"actor\":{\"displayName\":\"Z\"},\
\"issue\":{\"identifier\":\"DEV-6350\",\"title\":\"[YO] [FE] 구해요·샘플 삭제 updater 를 걷어냅니다\",\
\"url\":\"https://linear.app/cookieplace/issue/DEV-6350/yo-fe\"},\
\"comment\":{\"url\":\"https://linear.app/cookieplace/issue/DEV-6350/yo-fe#comment-41b0\",\
\"body\":\"@biscuit 의견 부탁드립니다.\\n\\n상태는 바꾸지 않았습니다.\"}}",
        );
        let got = parse(&text).expect("parses");
        assert_eq!(got.len(), 1);
        assert_eq!(got.url, "https://linear.app/cookieplace/inbox", "the badge leads to the inbox itself");
        assert_eq!(app_url(&got.url).as_deref(), Some("linear://cookieplace/inbox"));
        let n = &got.notes[0];
        assert_eq!(n.label(), "DEV-6350", "the key is what the chip carries");
        assert_eq!(n.verb(), "comment mention");
        assert_eq!(n.detail(), "Z comment mention · [YO] [FE] 구해요·샘플 삭제 updater 를 걷어냅니다 · @biscuit 의견 부탁드립니다.");
        assert!(n.url.ends_with("#comment-41b0"), "a comment is what the row points at: {}", n.url);
    }

    /// A review notification is about a pull request, and Linear's
    /// `Notification` carries no title or link for one — only who and what
    /// kind. The row still reads, and still leads somewhere.
    #[test]
    fn a_review_has_no_ticket_and_still_reads() {
        let text = body("{\"id\":\"n2\",\"type\":\"pullRequestReviewRequested\",\"category\":\"reviews\",\"actor\":{\"displayName\":\"Mars\"}}");
        let n = &parse(&text).expect("parses").notes[0];
        assert_eq!(n.label(), "LINEAR");
        assert_eq!(n.detail(), "Mars review requested");
        assert_eq!(n.url, "https://linear.app/cookieplace/inbox", "the inbox is the last resort");
    }

    /// Nobody caused some of them, and some are about nothing this knows
    /// how to read. Neither is a broken answer.
    #[test]
    fn a_notification_with_no_actor_and_no_entity_still_reads() {
        let text = body("{\"id\":\"n3\",\"type\":\"triageResponsibilityIssueAddedToTriage\",\"category\":\"triage\"}");
        let n = &parse(&text).expect("parses").notes[0];
        assert_eq!(n.detail(), "triage responsibility issue added to triage");
    }

    /// A GraphQL error arrives with HTTP 200, so the body is what says
    /// whether the answer is any good. Said out loud, because a field this
    /// query is wrong about is the difference between an empty inbox and a
    /// broken one.
    #[test]
    fn an_error_body_is_an_error_not_an_empty_inbox() {
        let text = "{\"errors\":[{\"message\":\"Cannot query field 'subtitle' on type 'Notification'\"}]}";
        match parse(text) {
            Err(Error::Response(m)) => assert!(m.contains("subtitle"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    /// A row is one line, so a long comment gives up its tail rather than
    /// the row giving up its shape. Korean counts in characters.
    #[test]
    fn a_long_comment_keeps_its_first_line_and_no_more() {
        let long = "가".repeat(400);
        let cut = excerpt(&format!("{long}\n두 번째 줄"));
        assert_eq!(cut.chars().count(), EXCERPT + 1, "{cut}");
        assert!(cut.ends_with('…'));
        assert_eq!(excerpt("\n\n  첫 줄  \n둘째"), "첫 줄");
    }
}
