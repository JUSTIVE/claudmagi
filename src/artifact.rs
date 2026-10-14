//! The claude.ai artifacts a session published (#137).
//!
//! An artifact is a page the session made and left where the person can open
//! it: a before/after report, a tracker, a deck. The board already reads a
//! session's pull requests and its Linear issue out of the same place, the
//! session's own transcript (#56, #57), and this is a third thing of that
//! kind — with one difference that decides the whole scan.
//!
//! A pull request is *named in passing*, inside a sentence somebody wrote, so
//! that scan hunts for a link anywhere in the text. An artifact is
//! *announced*, by the tool that made it, in a line of its own:
//!
//! ```text
//! Published <file> at https://claude.ai/artifact/<id> (Version 3, version id …)
//! Created a new Artifact at https://claude.ai/artifact/<id> (version …) from the Artifact type https://claude.ai/artifact/<other>
//! ```
//!
//! So this reads those two sentences and nothing else. That anchoring is what
//! keeps the list to pages this session *made*: a link the person pasted, one
//! the session merely read or opened, an asset uploaded into somebody else's
//! page, and the type the second line goes on to name are all somebody else's
//! work, and all of them carry the same URL shape.
//!
//! The name is the page's own `<title>`, which is what claude.ai calls it in
//! the gallery — read from the file that was published, since the announcing
//! line does not carry it. A page whose file has been cleared away falls back
//! to what it was called on disk, and then to its id: an artifact is worth
//! pointing at even when the board cannot say its name.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::pr::{in_context, tail};

/// How much of a transcript's tail is searched, matching the other two scans.
const TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// How far into a published file the `<title>` is looked for. It lives in the
/// head, and these files run to megabytes once a video is inlined.
const HEAD_BYTES: usize = 64 * 1024;

/// How many are kept per session, oldest dropped first. The same cap the pull
/// requests have and for the same reason: a cell can only draw so much, and
/// the newest are the ones still being looked at.
pub const MAX_PER_SESSION: usize = 4;

/// Where an artifact lives. The id is the whole address.
pub const HOME: &str = "https://claude.ai/artifact/";

/// What the tool is called in a transcript.
const TOOL: &str = "\"Artifact\"";
/// The two sentences that mean "this session made a page".
const PUBLISHED: &str = "Published ";
const CREATED: &str = "Created a new Artifact at ";
/// How a publish counts itself. A typed artifact's line says `version` in
/// lower case and follows it with a release stamp rather than a count, so the
/// capital is what tells the two apart.
const VERSION: &str = "(Version ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    /// The whole address, as `claude.ai/artifact/<id>`.
    pub id: String,
    /// What the page calls itself, or the best name the board could find.
    pub title: String,
    /// Which publish this is, where the line counted them.
    pub version: Option<u32>,
}

impl Artifact {
    pub fn url(&self) -> String {
        format!("{HOME}{}", self.id)
    }

    /// What a row of the board calls it.
    pub fn label(&self) -> String {
        match self.title.trim() {
            "" => self.id.clone(),
            title => title.to_string(),
        }
    }

    /// The short thing that stands where a pull request puts its number. A
    /// page has no number, so it offers the one count it has; nothing is not
    /// an error, since the title is the better name either way.
    pub fn version_label(&self) -> Option<String> {
        self.version.map(|v| format!("v{v}"))
    }
}

/// One announcement, before the name has been worked out.
struct Made {
    id: String,
    /// The file that was published, where the line named one.
    file: Option<String>,
    version: Option<u32>,
}

/// Every artifact the transcript's tail says this session published, in the
/// order it made them.
pub fn scan_transcript(path: &Path, since: u64) -> Vec<Artifact> {
    let Some(text) = tail(path, TAIL_BYTES) else { return Vec::new() };
    // Titles the call asked for, by the call's own id, and then by the id of
    // the page that call turned out to make. A typed artifact is named in the
    // asking rather than in a file, so this is the only place its name is
    // ever written down.
    let mut asked: HashMap<String, String> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut made: Vec<Made> = Vec::new();
    for line in text.lines() {
        // Cheap gate: parsing every record of a two-megabyte tail is the
        // expensive part, and almost no line is about an artifact.
        if !line.contains(HOME) && !line.contains(TOOL) {
            continue;
        }
        // A page made before the tab turned to new work is not this work's
        // (#74), the same way a pull request is not.
        if !in_context(line, since) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("tool_use") => {
                    if block.get("name").and_then(|n| n.as_str()) != Some("Artifact") {
                        continue;
                    }
                    let id = block.get("id").and_then(|i| i.as_str());
                    let title = block.get("input").and_then(|i| i.get("title")).and_then(|t| t.as_str());
                    if let (Some(id), Some(title)) = (id, title) {
                        asked.insert(id.to_string(), title.to_string());
                    }
                }
                Some("tool_result") => {
                    let Some(one) = announced(&said(block)) else { continue };
                    // The call's own title beats anything read off disk: it
                    // is what the person will see the page called.
                    let title =
                        block.get("tool_use_id").and_then(|i| i.as_str()).and_then(|i| asked.get(i)).cloned();
                    if let Some(title) = title {
                        names.insert(one.id.clone(), title);
                    }
                    made.push(one);
                }
                _ => {}
            }
        }
    }
    gather(made, &names)
}

/// What a tool result said, however the transcript wrapped it.
fn said(block: &serde_json::Value) -> String {
    match block.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(parts)) => {
            parts.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str())).collect()
        }
        _ => String::new(),
    }
}

/// The page a tool result says was made, if it says one was.
///
/// Anchored at the front, because the same URL shape turns up in results that
/// mean something else entirely — a page that was read, one that was opened,
/// a listing of every page there is. Only the first address on the line is
/// taken: the `Created` sentence goes on to name the type it was made from,
/// which is a page somebody else published.
fn announced(said: &str) -> Option<Made> {
    let said = said.trim_start();
    let at = said.find(HOME)?;
    let id: String = said[at + HOME.len()..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if id.is_empty() {
        return None;
    }
    let file = if said.starts_with(PUBLISHED) && at > PUBLISHED.len() {
        // `Published <file> at <url>`: the file is everything in between. Cut
        // from the URL backwards rather than forwards, since a path is free
        // to contain the word this splits on.
        Some(said[PUBLISHED.len()..at].trim_end().strip_suffix(" at")?.trim().to_string())
    } else if said.starts_with(CREATED) && at == CREATED.len() {
        None
    } else {
        return None;
    };
    Some(Made { id, file, version: version_in(&said[at..]) })
}

fn version_in(rest: &str) -> Option<u32> {
    let at = rest.find(VERSION)? + VERSION.len();
    let digits: String = rest[at..].chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// One row per page, in the order the session made them, each wearing the
/// best name anyone could give it. A page published again is the same page:
/// the last publish is the one that counts, since that is the version the
/// link now opens.
fn gather(made: Vec<Made>, names: &HashMap<String, String>) -> Vec<Artifact> {
    let mut order: Vec<String> = Vec::new();
    let mut found: HashMap<String, Artifact> = HashMap::new();
    for one in made {
        let title = names
            .get(&one.id)
            .cloned()
            .or_else(|| one.file.as_deref().map(Path::new).and_then(page_title))
            .or_else(|| one.file.as_deref().map(Path::new).and_then(named_after))
            .unwrap_or_default();
        match found.get_mut(&one.id) {
            Some(prev) => {
                prev.version = one.version.or(prev.version);
                if !title.is_empty() {
                    prev.title = title;
                }
            }
            None => {
                order.push(one.id.clone());
                found.insert(one.id.clone(), Artifact { id: one.id, title, version: one.version });
            }
        }
    }
    let mut out: Vec<Artifact> = order.into_iter().filter_map(|id| found.remove(&id)).collect();
    if out.len() > MAX_PER_SESSION {
        out.drain(..out.len() - MAX_PER_SESSION);
    }
    out
}

/// The page's own `<title>`, which is the name claude.ai gives it.
fn page_title(file: &Path) -> Option<String> {
    let head = head_of(file)?;
    let at = head.find("<title")?;
    let open = head[at..].find('>')? + at + 1;
    let end = head[open..].find("</title>")? + open;
    let title = head[open..end].split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// What the file was called, for a page whose file is still named something
/// and whose title is not to be had. `index` names a directory rather than a
/// page, so the directory is the answer there.
fn named_after(file: &Path) -> Option<String> {
    let stem = file.file_stem()?.to_str()?;
    if stem == "index" {
        return file.parent()?.file_name()?.to_str().map(str::to_string);
    }
    (!stem.is_empty()).then(|| stem.to_string())
}

fn head_of(file: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(file).ok()?;
    let mut buf = vec![0u8; HEAD_BYTES];
    let read = f.read(&mut buf).ok()?;
    buf.truncate(read);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Per-session artifacts, rescanned only when a transcript grows.
///
/// Nothing here goes near the network, unlike the pull request and issue
/// trackers: the transcript already holds everything the board shows, so
/// there is no worker and no cache to go stale.
#[derive(Default)]
pub struct Tracker {
    /// session id → (transcript size when scanned, context boundary, found).
    seen: Mutex<HashMap<String, (u64, u64, Vec<Artifact>)>>,
}

impl Tracker {
    pub fn resolve(&self, session_id: &str, transcript: Option<&Path>, since: u64) -> Vec<Artifact> {
        let Some(path) = transcript else { return Vec::new() };
        let size = std::fs::metadata(path).ok().map(|m| m.len()).unwrap_or(0);
        {
            let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            // A rename is a new answer even when the file has not grown (#74).
            if let Some((scanned, from, found)) = seen.get(session_id) {
                if *scanned == size && *from == since {
                    return found.clone();
                }
            }
        }
        let found = scan_transcript(path, since);
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.insert(session_id.to_string(), (size, since, found.clone()));
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str) -> String {
        let text = serde_json::to_string(text).unwrap();
        format!(r#"{{"message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":{text}}}]}}}}"#)
    }

    fn scan(lines: &[String]) -> Vec<Artifact> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("claudmagi-artifact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, lines.join("\n")).unwrap();
        let out = scan_transcript(&path, 0);
        let _ = std::fs::remove_file(&path);
        out
    }

    /// The announcing line is the whole source, so the shape of that line is
    /// the thing worth pinning down.
    #[test]
    fn a_published_page_is_read_out_of_the_line_that_announced_it() {
        let found = scan(&[result(
            "Published /tmp/box/report/index.html at https://claude.ai/artifact/Abc123 (Version 3, version id 1-2) Icon: \"report\".\n\nLive subscription: arming.",
        )]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "Abc123");
        assert_eq!(found[0].version, Some(3));
        // No file on disk: the page is still worth pointing at, under the
        // name the folder gave it.
        assert_eq!(found[0].label(), "report");
        assert_eq!(found[0].url(), "https://claude.ai/artifact/Abc123");
    }

    /// A typed artifact names the page it was made *from*, and that page
    /// belongs to whoever published the type (#137).
    #[test]
    fn the_type_a_page_was_made_from_is_not_the_session_s_own() {
        let found = scan(&[result(
            "Created a new Artifact at https://claude.ai/artifact/Mine1 (version 1790576622-7951) from the Artifact type https://claude.ai/artifact/Theirs1 (release 1-2).",
        )]);
        assert_eq!(found.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["Mine1"]);
        // That line counts versions with a release stamp, not a number.
        assert_eq!(found[0].version, None);
    }

    /// Everything else the tool answers with carries the same URL, and none
    /// of it means the session made a page.
    #[test]
    fn a_page_read_opened_or_listed_was_not_made_here() {
        let found = scan(&[
            result("Opened the Artifact at https://claude.ai/artifact/Opened1 for the user. Nothing was published or changed."),
            result("[Artifact 99cb6f26 (version 1791)] https://claude.ai/artifact/Read1"),
            result("Showing 2 of 29 published artifacts:\n- (mine) something — https://claude.ai/artifact/Listed1 — updated 2026-10-13"),
            result("Asset uploaded: \"clip.webm\" (356353 bytes) into https://claude.ai/artifact/Asset1"),
        ]);
        assert!(found.is_empty(), "nothing here was made by this session, got {found:?}");
    }

    /// The same page published again is the same page, and the last publish
    /// is the one the link now opens.
    #[test]
    fn publishing_a_page_again_does_not_make_a_second_one() {
        let found = scan(&[
            result("Published /tmp/box/a.html at https://claude.ai/artifact/Same1 (Version 1, version id 1-2)"),
            result("Published /tmp/box/a.html at https://claude.ai/artifact/Same1 (Version 2, version id 3-4)"),
            result("Published /tmp/box/b.html at https://claude.ai/artifact/Other1 (Version 1, version id 5-6)"),
        ]);
        assert_eq!(found.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["Same1", "Other1"]);
        assert_eq!(found[0].version, Some(2));
    }

    /// A page the call named is named by the call: a typed artifact has no
    /// file to read a title out of.
    #[test]
    fn a_title_the_call_asked_for_is_the_page_s_name() {
        let use_line = r#"{"message":{"content":[{"type":"tool_use","id":"t1","name":"Artifact","input":{"title":"주간 리포트","type_url":"https://claude.ai/artifact/Type1"}}]}}"#;
        let found = scan(&[
            use_line.to_string(),
            result("Created a new Artifact at https://claude.ai/artifact/Typed1 (version 1-2) from the Artifact type https://claude.ai/artifact/Type1 (release 3-4)."),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label(), "주간 리포트");
    }

    /// The name on the board is the name on claude.ai, which is the page's
    /// own `<title>` and nothing the board invented.
    #[test]
    fn the_page_s_own_title_is_its_name() {
        let dir = std::env::temp_dir().join(format!("claudmagi-title-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("index.html");
        // The folder is what `index.html` would otherwise be named after, so
        // it has to be something the title cannot be mistaken for.
        std::fs::write(&page, "<!doctype html><html><head>\n<title>PJM-2217 로딩 캐릭터</title>\n</head></html>").unwrap();
        let found = scan(&[result(&format!(
            "Published {} at https://claude.ai/artifact/Titled1 (Version 1, version id 1-2)",
            page.display()
        ))]);
        let _ = std::fs::remove_file(&page);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label(), "PJM-2217 로딩 캐릭터");
    }

    /// A board can only draw so many, and the ones still being looked at are
    /// the newest.
    #[test]
    fn only_the_newest_pages_are_kept() {
        let lines: Vec<String> = (1..=MAX_PER_SESSION + 2)
            .map(|n| result(&format!("Published /tmp/box/p{n}.html at https://claude.ai/artifact/P{n} (Version 1, version id 1-2)")))
            .collect();
        let found = scan(&lines);
        assert_eq!(found.len(), MAX_PER_SESSION);
        assert_eq!(found[0].id, "P3", "the oldest go, not the newest");
        assert_eq!(found[found.len() - 1].id, format!("P{}", MAX_PER_SESSION + 2));
    }
}

