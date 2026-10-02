//! The cloud, which is a host with no window (#120).
//!
//! A cloud session runs in Anthropic's own container, so there is nothing on
//! this machine to raise. What there is instead is a link: the web app opens
//! any of them, and the desktop app takes the same one, so a chip's click
//! lands in whichever the person already has.

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
        let url = match seat.focus_url.as_deref() {
            Some(url) => url.to_string(),
            None => crate::cloud::session_url(&seat.handle),
        };
        Command::new("open").arg(&url).status().context("opening the session")?;
        Ok(Outcome::Focused(url))
    }
}
