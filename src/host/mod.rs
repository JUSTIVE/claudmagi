//! Where a session is running, and how to get back to it (#118).
//!
//! The session registry at `~/.claude/sessions/<pid>.json` is Claude Code's,
//! not any terminal's: every session writes one wherever it runs. What a
//! terminal adds is a **seat** — which window of which app the session sits
//! in, so sessions that share one can be drawn together, and a way to bring
//! that seat to the front when its chip is clicked.
//!
//! That was Warp, hard-coded, in five places. It is a trait now, because the
//! two hosts worth adding work nothing like Warp does: Orca puts its tab id
//! straight into the environment, where Warp makes us read a database behind
//! Full Disk Access (#67, #69), and the desktop app leaves no mark at all.
//!
//! A host claims a session by what it put in that session's environment, and
//! the first one to claim it wins. Claude Code's own `CLAUDE_CODE_ENTRYPOINT`
//! says which kind of thing started it, but not which window, so it is no
//! help here.

use std::collections::HashMap;

use anyhow::Result;

use crate::model::SessionInfo;

pub mod cloud;
pub mod desktop;
pub mod orca;
pub mod warp;

/// A session's place in its host: what the host calls it, and what the host
/// told us for free in the environment.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Seat {
    /// Which host, as it appears in a group key and in the status bar.
    pub host: &'static str,
    /// What the host knows this session by: a Warp pane uuid, an Orca
    /// terminal handle. Empty where the host has no id for it.
    pub handle: String,
    /// The window several seats share, where the environment already said so.
    /// `None` means the host has to be asked (`Host::windows`).
    pub window: Option<String>,
    /// A direct way back, where the host put one in the environment.
    pub focus_url: Option<String>,
}

#[derive(Debug)]
pub enum Outcome {
    Focused(String),
    ActivatedApp(String),
}

pub trait Host: Send + Sync {
    /// The prefix this host's group keys carry.
    fn id(&self) -> &'static str;

    /// Claims a session from its process environment, or passes.
    fn seat(&self, env: &HashMap<String, String>) -> Option<Seat>;

    /// Which window each handle sits in, for the seats whose environment did
    /// not say. One call per snapshot, so a host that has to go and ask pays
    /// for it once. Empty when the host has nothing to add.
    fn windows(&self, _handles: &[&str]) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Brings the seat to the front.
    fn focus(&self, seat: &Seat) -> Result<Outcome>;

    /// Something about this host the board should say out loud.
    fn note(&self) -> Option<&'static str> {
        None
    }
}

/// Every host, in the order they get asked. The desktop one claims whatever
/// is left, so it goes last.
pub fn all() -> Vec<Box<dyn Host>> {
    vec![
        Box::new(warp::Warp::default()),
        Box::new(orca::Orca),
        // Claims nothing from an environment; it is here so a cloud seat has
        // somewhere to be focused from (#120).
        Box::new(cloud::Cloud),
        Box::new(desktop::Desktop),
    ]
}

/// The seat a session sits in, from its environment.
pub fn seat_for(hosts: &[Box<dyn Host>], env: &HashMap<String, String>) -> Option<Seat> {
    hosts.iter().find_map(|h| h.seat(env))
}

/// The group key a seat belongs to. Sessions sharing one are drawn together
/// (#41): a window where the host could name one, the seat itself otherwise,
/// so a session is never grouped with one it has nothing to do with.
pub fn group_key(seat: &Seat) -> String {
    match (&seat.window, seat.handle.as_str()) {
        (Some(w), _) => format!("{}-tab:{w}", seat.host),
        (None, "") => seat.host.to_string(),
        (None, handle) => format!("{}:{handle}", seat.host),
    }
}

/// Brings the session's seat to the front.
pub fn focus(session: &SessionInfo) -> Result<Outcome> {
    let hosts = all();
    let seat = session.seat.clone().unwrap_or_default();
    match hosts.iter().find(|h| h.id() == seat.host) {
        Some(host) => host.focus(&seat),
        None => desktop::Desktop.focus(&seat),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// The first host to recognise its own marks takes the session, and the
    /// desktop one takes whatever is left — which is the only reason a
    /// session hosted by the Claude app, or by a terminal nobody wrote a host
    /// for, lands anywhere at all (#118).
    #[test]
    fn each_host_claims_what_it_put_in_the_environment() {
        let hosts = all();
        let seat = |e: &[(&str, &str)]| seat_for(&hosts, &env(e)).expect("something always claims");

        let warp = seat(&[
            ("WARP_TERMINAL_SESSION_UUID", "80602dab-5730-4503-b001-386bf769bcd9"),
            ("WARP_FOCUS_URL", "warp://session/80602dab"),
            ("TERM_PROGRAM", "WarpTerminal"),
        ]);
        assert_eq!(warp.host, "warp");
        assert_eq!(warp.focus_url.as_deref(), Some("warp://session/80602dab"));
        assert_eq!(warp.window, None, "Warp keeps the tab in a database, not in the environment");

        let orca = seat(&[
            ("ORCA_TERMINAL_HANDLE", "term_abc123"),
            ("ORCA_TAB_ID", "tab_9"),
            ("ORCA_WORKTREE_ID", "wt_1"),
        ]);
        assert_eq!(orca.host, "orca");
        assert_eq!(orca.handle, "term_abc123");
        assert_eq!(
            orca.window.as_deref(),
            Some("tab_9"),
            "Orca hands the tab over, so grouping costs nothing (#118)"
        );

        // A terminal nobody wrote a host for still names itself.
        assert_eq!(seat(&[("TERM_PROGRAM", "iTerm.app")]).host, "term");
        // And the desktop app leaves nothing behind at all.
        let desktop = seat(&[]);
        assert_eq!(desktop.host, "desktop");
        assert!(desktop.handle.is_empty());
    }

    /// Sessions sharing a window are drawn together, and sessions that only
    /// share a *host* are not: one Warp with four panes is four clusters
    /// until the tabs land, never one (#41, #59).
    #[test]
    fn a_group_key_never_lumps_seats_that_share_nothing_but_a_host() {
        let pane = |h: &str| Seat { host: "warp", handle: h.into(), window: None, focus_url: None };
        assert_ne!(group_key(&pane("aaaa")), group_key(&pane("bbbb")));

        let tabbed = |h: &str, w: &str| Seat {
            host: "warp",
            handle: h.into(),
            window: Some(w.into()),
            focus_url: None,
        };
        assert_eq!(group_key(&tabbed("aaaa", "1-1")), group_key(&tabbed("bbbb", "1-1")));
        assert_ne!(group_key(&tabbed("aaaa", "1-1")), group_key(&tabbed("bbbb", "1-2")));
        assert_eq!(group_key(&tabbed("aaaa", "1-1")), "warp-tab:1-1");

        // Orca's keys read the same way, which is what lets the board and the
        // comb stay written against the shape rather than against Warp.
        let orca = Seat { host: "orca", handle: "term_a".into(), window: Some("tab_9".into()), focus_url: None };
        assert_eq!(group_key(&orca), "orca-tab:tab_9");

        // The desktop app has no window to tell apart, so its sessions do
        // share one group, which is the honest answer rather than a guess.
        let desk = Seat { host: "desktop", handle: String::new(), window: None, focus_url: None };
        assert_eq!(group_key(&desk), "desktop");
    }

    /// Every key the hosts can make has to read as a sentence, since the
    /// status bar prints it (#118).
    #[test]
    fn every_group_key_has_a_label() {
        use crate::model::{Phase, SessionInfo};
        let label = |group: &str| {
            let mut s = SessionInfo::synthetic(1, "S-1", Phase::Working);
            s.group = group.to_string();
            s.group_label()
        };
        assert_eq!(label("warp-tab:1-2"), "warp tab 1-2");
        assert_eq!(label("orca-tab:tab_9"), "orca tab tab_9");
        assert_eq!(label("warp:80602dab57304503"), "warp pane 80602dab");
        assert_eq!(label("orca:term_abc123def"), "orca pane term_abc");
        assert_eq!(label("term:iTerm.app"), "iTerm.app");
        assert_eq!(label("sandbox:3"), "sandbox group 3");
        assert_eq!(label("desktop"), "desktop");
    }
}
