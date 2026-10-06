//! The cloud, which is a host with no window (#120).
//!
//! A cloud session runs in Anthropic's own container, so there is nothing on
//! this machine to raise. What there is instead is a link, and the right one
//! is the Claude desktop app's own: a cloud session is the app's work, and
//! `claude://code/<id>` puts the click back where it came from rather than
//! in a browser tab. A machine without the app falls through to the web
//! (#127).

use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat};

pub struct Cloud;

impl Host for Cloud {
    fn id(&self) -> &'static str {
        "cloud"
    }

    /// Never claims anything. A cloud session has no process here, so it
    /// never passes through the environment the other hosts are offered;
    /// `cloud.rs` seats it when it reads it off the wire.
    fn seat(&self, _env: &HashMap<String, String>) -> Option<Seat> {
        None
    }

    fn focus(&self, seat: &Seat) -> Result<Outcome> {
        let app = match seat.focus_url.as_deref() {
            Some(url) => url.to_string(),
            None => crate::cloud::app_url(&seat.handle),
        };
        if Command::new("open").arg(&app).status().is_ok_and(|s| s.success()) {
            return Ok(Outcome::Focused(app));
        }
        // No desktop app on this machine, or it would not take the link.
        let web = crate::cloud::web_url(&seat.handle);
        Command::new("open").arg(&web).status().context("opening the session")?;
        Ok(Outcome::Focused(web))
    }
}
