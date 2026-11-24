//! Herdr: terminals that outlive the window they are shown in (#189).
//!
//! Herdr runs every program in `herdr-core` (`hc`), which keeps the terminal
//! alive when the window closes, and the window (`herdr-app`) arranges those
//! terminals in spaces and tabs. Claude Code started there writes the same
//! registry record it writes anywhere (#118), so a Herdr session was already
//! on the board — as `term:ghostty`, because Herdr's terminal engine is
//! Ghostty's and `TERM_PROGRAM` is all a plain terminal leaves behind. Every
//! session Herdr ran landed in one lump named after a terminal library, and
//! clicking it had nowhere to go.
//!
//! What Herdr does leave in the environment is `HC_TERMINAL_ID` — `t4`, the
//! terminal's address on its core, and the handle both `hc` and `herdr-app`
//! take. The tab belongs to the window rather than to the terminal, so it is
//! asked for the way Warp's is (#42), but over the app's own socket instead
//! of a database behind Full Disk Access (#67): milliseconds, and nothing to
//! grant.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat, run_with_timeout};

/// What Herdr calls the terminal a program runs in. `HC_TERMINAL_SOCKET` is
/// beside it, but it names the core, not this terminal, so it says nothing
/// about which seat this is.
const TERMINAL: &str = "HC_TERMINAL_ID";

/// How long one answer about tabs is kept.
const TABS_TTL: Duration = Duration::from_secs(3);
/// How soon to ask again after a miss. A window that was closed for a moment
/// must not pass for "no tabs" for a whole TTL, or the board groups by
/// terminal, lays itself out wrong, and slides every row when the real answer
/// lands (#59).
const TABS_RETRY: Duration = Duration::from_millis(400);
/// A local socket answers in milliseconds; this is only here so a wedged
/// window cannot hold up a snapshot.
const ASK_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct Herdr {
    tabs: Mutex<Option<Tabs>>,
}

struct Tabs {
    map: HashMap<String, String>,
    fetched: Instant,
    /// Whether this came from an answer that actually listed terminals.
    good: bool,
}

impl Host for Herdr {
    fn id(&self) -> &'static str {
        "herdr"
    }

    fn seat(&self, env: &HashMap<String, String>) -> Option<Seat> {
        let id = env.get(TERMINAL).filter(|s| !s.is_empty())?;
        Some(Seat {
            host: "herdr",
            handle: id.clone(),
            // The window knows which tab this terminal is shown in; the
            // terminal does not. `windows` goes and asks.
            window: None,
            focus_url: None,
        })
    }

    fn windows(&self, handles: &[&str]) -> HashMap<String, String> {
        if handles.is_empty() {
            return HashMap::new();
        }
        let tabs = self.tabs();
        handles.iter().filter_map(|h| tabs.get(*h).map(|tab| ((*h).to_string(), tab.clone()))).collect()
    }

    // `gone` (#186) stays the default silence. A Herdr terminal holds its
    // program, so a session that is still running still has a terminal to go
    // back to — parked without a pane, or in a closed window, but there.
    // The seat that outlives its session's place is a Warp shape, not this
    // one.
    fn focus(&self, seat: &Seat) -> Result<Outcome> {
        if !seat.handle.is_empty() && self.show(&seat.handle) {
            // The window picks the pane; macOS still has to be told to bring
            // that window to the front.
            let _ = Command::new("open").args(["-a", "Herdr"]).status();
            return Ok(Outcome::Focused(format!("herdr terminal {}", seat.handle)));
        }
        // The command refuses when no window is open, and the honest thing
        // then is to open one rather than claim a jump that did not happen.
        Command::new("open").args(["-a", "Herdr"]).status().context("activating Herdr")?;
        Ok(Outcome::ActivatedApp("Herdr".into()))
    }
}

impl Herdr {
    /// Cached terminal → tab map; empty when no window is open to ask.
    fn tabs(&self) -> HashMap<String, String> {
        let mut guard = self.tabs.lock().unwrap_or_else(|e| e.into_inner());
        let ttl = match guard.as_ref() {
            Some(t) if t.good => TABS_TTL,
            _ => TABS_RETRY,
        };
        if guard.as_ref().is_none_or(|t| t.fetched.elapsed() > ttl) {
            match ask_tabs() {
                Some(map) if !map.is_empty() => {
                    *guard = Some(Tabs { map, fetched: Instant::now(), good: true })
                }
                // A window that is closed or busy is not the same as a window
                // with no tabs in it: keep the last good answer (#59).
                _ => match guard.as_mut() {
                    Some(t) => t.fetched = Instant::now(),
                    None => *guard = Some(Tabs { map: HashMap::new(), fetched: Instant::now(), good: false }),
                },
            }
        }
        guard.as_ref().map(|t| t.map.clone()).unwrap_or_default()
    }

    /// Asks the window to show this terminal. False when there is no window,
    /// no `herdr-app` on this machine, or the command refused.
    fn show(&self, handle: &str) -> bool {
        let Some(bin) = app_bin() else { return false };
        let mut cmd = Command::new(bin);
        cmd.args(["do", "pane.focus", "--terminal", handle]);
        // It answers `{"ok": true}`, or `{"error": ...}` and exits 1, so a
        // successful exit is the whole answer.
        run_with_timeout(cmd, ASK_TIMEOUT).is_some()
    }
}

/// `herdr-app ls`, as a terminal → tab map. `None` when there is nothing to
/// ask or the ask failed, which is not the same as an empty window.
fn ask_tabs() -> Option<HashMap<String, String>> {
    let bin = app_bin()?;
    let mut cmd = Command::new(bin);
    cmd.arg("ls");
    Some(parse_tabs(&run_with_timeout(cmd, ASK_TIMEOUT)?))
}

/// Where Herdr's command lives. Its installer puts it in `~/.local/bin`,
/// which is not on the PATH an app bundle is launched with, so the path is
/// worked out rather than looked up (the same reason Warp's `sqlite3` is
/// spelled out in full).
fn app_bin() -> Option<PathBuf> {
    let mut tries = Vec::new();
    if let Some(home) = dirs::home_dir() {
        tries.push(home.join(".local/bin/herdr-app"));
    }
    tries.push(PathBuf::from("/opt/homebrew/bin/herdr-app"));
    tries.push(PathBuf::from("/usr/local/bin/herdr-app"));
    tries.into_iter().find(|p| p.is_file())
}

/// Spaces, their tabs, and the terminals shown in each, into
/// `terminal → tab`.
///
/// The tab's own id is what tells two tabs apart — Herdr names a tab after
/// whatever runs in it, so a person with two Claude tabs in one space has two
/// tabs called "Claude Code", and grouping by name would seat strangers
/// together (#59). The id is long and random, so the key carries its head the
/// way a Warp pane's does: enough to be unique among a window's tabs, short
/// enough to read in a status line.
fn parse_tabs(text: &str) -> HashMap<String, String> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else { return HashMap::new() };
    let mut out = HashMap::new();
    let array = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for space in array(doc.get("spaces")) {
        for tab in array(space.get("tabs")) {
            let Some(id) = tab.get("id").and_then(|i| i.as_str()) else { continue };
            let tab_key = short_tab(id);
            for term in array(tab.get("terminals")) {
                if let Some(addr) = term.get("terminal").and_then(|a| a.as_str()) {
                    out.insert(addr.to_string(), tab_key.clone());
                }
                // A pane can carry the sub-agents its program started, each
                // its own terminal on the core. They sit in the same tab,
                // and one running Claude Code writes its own record.
                for sub in array(term.get("sub_agents")) {
                    let addr = sub.as_str().or_else(|| sub.get("terminal").and_then(|a| a.as_str()));
                    if let Some(addr) = addr {
                        out.insert(addr.to_string(), tab_key.clone());
                    }
                }
            }
        }
    }
    out
}

/// The head of a tab id, without the `tb` every one of them starts with.
fn short_tab(id: &str) -> String {
    let body = id.strip_prefix("tb").unwrap_or(id);
    body.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// Herdr's own mark in the environment is the terminal's address, and
    /// `TERM_PROGRAM` is not it: Herdr draws with Ghostty's engine and says
    /// so, which is why these sessions used to pile up in one `term:ghostty`
    /// group (#189).
    #[test]
    fn a_herdr_session_is_claimed_by_its_terminal_address() {
        let host = Herdr::default();
        let seat = host
            .seat(&env(&[
                ("HC_TERMINAL_ID", "t4"),
                ("HC_TERMINAL_SOCKET", "/Users/x/.local/state/herdr-core/default/default.sock"),
                ("TERM_PROGRAM", "ghostty"),
            ]))
            .expect("Herdr claims it");
        assert_eq!((seat.host, seat.handle.as_str()), ("herdr", "t4"));
        assert_eq!(seat.window, None, "the tab belongs to the window, and is asked for");
        assert_eq!(seat.focus_url, None, "there is no URL to open; the window is told instead");

        assert!(host.seat(&env(&[("TERM_PROGRAM", "ghostty")])).is_none(), "Ghostty alone is not Herdr");
        assert!(host.seat(&env(&[("HC_TERMINAL_ID", "")])).is_none(), "an empty address seats nobody");
    }

    /// One answer from the window seats every terminal in it, sub-agents
    /// included, and two tabs that share a name stay two tabs (#59).
    #[test]
    fn the_window_says_which_tab_each_terminal_is_in() {
        let answer = r#"{
          "spaces": [
            {
              "id": "spb5fa2ca146d5c7ee71e834fa85cce637",
              "name": "crepe",
              "tabs": [
                {
                  "id": "tb93362db445a9534ca648354aeaf7748b",
                  "name": "Claude Code",
                  "terminals": [
                    {"terminal": "t4", "status": "working", "app": "claude-code", "sub_agents": ["t9"]}
                  ]
                },
                {
                  "id": "tb56df0cfe40d94d7f6504570efdb9f343",
                  "name": "Claude Code",
                  "terminals": [{"terminal": "t7", "status": null, "sub_agents": []}]
                }
              ]
            }
          ],
          "recent": []
        }"#;
        let map = parse_tabs(answer);
        assert_eq!(map.get("t4").map(String::as_str), Some("93362db4"));
        assert_eq!(map.get("t9").map(String::as_str), Some("93362db4"), "a sub-agent sits in its parent's tab");
        assert_eq!(map.get("t7").map(String::as_str), Some("56df0cfe"));
        assert_ne!(map.get("t4"), map.get("t7"), "two tabs called Claude Code are two tabs");

        // The group key reads like every other host's (#118), and a terminal
        // the window does not show keeps a group of its own rather than
        // joining a tab it is not in.
        let seated = Seat { host: "herdr", handle: "t4".into(), window: map.get("t4").cloned(), focus_url: None };
        assert_eq!(super::super::group_key(&seated), "herdr-tab:93362db4");
        let parked = Seat { host: "herdr", handle: "t8".into(), window: None, focus_url: None };
        assert_eq!(super::super::group_key(&parked), "herdr:t8");

        assert!(parse_tabs("not json").is_empty(), "a window that answers nonsense seats nobody");
        assert!(parse_tabs(r#"{"spaces":[]}"#).is_empty());
    }
}
