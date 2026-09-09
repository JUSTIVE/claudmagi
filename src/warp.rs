//! Jumps to the Warp tab/pane that hosts a session.
//!
//! Warp exports `WARP_FOCUS_URL=warp://session/<uuid>` into every shell it
//! spawns; opening that URL brings the matching pane to the front. We read it
//! from the Claude process environment (see `sessions::proc_env`). If the
//! session was not started from Warp we just activate the terminal app.

use std::process::Command;

use anyhow::{Context, Result};

use crate::model::SessionInfo;

#[derive(Debug)]
pub enum Outcome {
    Focused(String),
    ActivatedApp(String),
}

pub fn focus(session: &SessionInfo) -> Result<Outcome> {
    if let Some(url) = session.warp_focus_url.as_deref() {
        let status = Command::new("open").arg(url).status().context("spawning `open`")?;
        if status.success() {
            return Ok(Outcome::Focused(url.to_string()));
        }
    }
    if let Some(uuid) = session.warp_session_uuid.as_deref() {
        let url = format!("warp://session/{uuid}");
        let status = Command::new("open").arg(&url).status().context("spawning `open`")?;
        if status.success() {
            return Ok(Outcome::Focused(url));
        }
    }
    let app = "Warp";
    Command::new("open").args(["-a", app]).status().context("activating Warp")?;
    Ok(Outcome::ActivatedApp(app.to_string()))
}
