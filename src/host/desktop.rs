//! Everything that is not a terminal with a seat (#118).
//!
//! Two kinds of session land here. One is a plain terminal — iTerm, Terminal,
//! whatever `TERM_PROGRAM` says — which has a window but no way to name or
//! reach it. The other is the Claude desktop app, which leaves nothing in the
//! environment at all.
//!
//! The desktop app needs no reading of ours: the registry those sessions
//! write is the same one every session writes, and the app is a reader of it
//! just as this board is. So they already arrive. What they have never had is
//! a seat, which put them all in one bucket called `desktop` and sent a click
//! to Warp. Now they group by the app they are in and a click opens it.
//!
//! Claude.app does register deep links (`claude://code/continue?session=`,
//! `claude://resume?session=`), but resuming *imports* a CLI session into the
//! app rather than revealing one that is already running, which is not what a
//! click on a chip means. Until a session hosted by the app turns up here to
//! be looked at, activating the app is the honest action.

use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat};

pub struct Desktop;

/// The app that hosts a session with no terminal of its own.
const APP: &str = "Claude";

impl Host for Desktop {
    fn id(&self) -> &'static str {
        "desktop"
    }

    /// Claims whatever the other hosts passed on, which is why it is asked
    /// last. A terminal names itself and gets its own group; anything else is
    /// the desktop app.
    fn seat(&self, env: &HashMap<String, String>) -> Option<Seat> {
        Some(match env.get("TERM_PROGRAM") {
            Some(prog) => Seat { host: "term", handle: prog.clone(), window: None, focus_url: None },
            None => Seat { host: "desktop", handle: String::new(), window: None, focus_url: None },
        })
    }

    fn focus(&self, seat: &Seat) -> Result<Outcome> {
        // A terminal we cannot address is still a terminal: bring up the one
        // the session named rather than the desktop app.
        let app = if seat.host == "term" && !seat.handle.is_empty() { seat.handle.as_str() } else { APP };
        Command::new("open").args(["-a", app]).status().with_context(|| format!("activating {app}"))?;
        Ok(Outcome::ActivatedApp(app.to_string()))
    }
}
