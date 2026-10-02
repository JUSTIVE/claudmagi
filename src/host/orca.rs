//! Orca: a host that answers in the environment (#118).
//!
//! Orca puts the whole seat into every PTY it spawns — the terminal handle,
//! the tab, the pane, the worktree — and its own source says why: those ids
//! are per-PTY and stable for the life of the process, unlike anything tied
//! to a running Orca instance. So the tab a session sits in, which for Warp
//! means reading a database behind Full Disk Access (#67), is simply there in
//! `proc_env` alongside everything else we already read.
//!
//! Getting back to a seat goes through the CLI the ticket lookup already uses
//! (#57): `orca terminal switch --terminal <handle>`.

use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat};

/// What Orca calls a terminal, a tab and a worktree in a spawned environment.
const HANDLE: &str = "ORCA_TERMINAL_HANDLE";
const TAB: &str = "ORCA_TAB_ID";
const PANE: &str = "ORCA_PANE_KEY";
const WORKTREE: &str = "ORCA_WORKTREE_ID";

pub struct Orca;

impl Host for Orca {
    fn id(&self) -> &'static str {
        "orca"
    }

    fn seat(&self, env: &HashMap<String, String>) -> Option<Seat> {
        // The handle is what the CLI takes, so it is the one worth having;
        // a pane with no handle still seats, it just cannot be jumped to.
        let handle = env.get(HANDLE).or_else(|| env.get(PANE))?;
        Some(Seat {
            host: "orca",
            handle: handle.clone(),
            // The tab makes the cluster, and the worktree stands in where a
            // terminal was opened outside one.
            window: env.get(TAB).or_else(|| env.get(WORKTREE)).cloned(),
            focus_url: None,
        })
    }

    fn focus(&self, seat: &Seat) -> Result<Outcome> {
        if !seat.handle.is_empty() {
            let ok = Command::new("orca")
                .args(["terminal", "switch", "--terminal", &seat.handle])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return Ok(Outcome::Focused(format!("orca terminal {}", seat.handle)));
            }
        }
        // The CLI refuses when Orca's runtime is down, and the honest thing
        // then is to open the app rather than to claim a jump that did not
        // happen.
        Command::new("open").args(["-a", "Orca"]).status().context("activating Orca")?;
        Ok(Outcome::ActivatedApp("Orca".into()))
    }
}
