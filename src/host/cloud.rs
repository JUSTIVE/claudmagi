//! The cloud, which is a host with no window (#120).
//!
//! A cloud session runs in Anthropic's own container, so there is nothing on
//! this machine to raise. What there is instead is a link, and the right one
//! is the Claude desktop app's own: a cloud session is the app's work, and
//! `claude://code/<id>` puts the click back where it came from rather than
//! in a browser tab. A machine without the app falls through to the web
//! (#127).
//!
//! The click is two steps rather than one (#246). A single `open` of that
//! link asks the app to do two things at the same moment — wake up, and go
//! to a session — and the app's own handler does the second one
//! asynchronously: it looks the id up in a store that may still be loading,
//! and then drops the link without a word if the window is not there yet, or
//! if anything else navigated while it looked. Waking a window *is* such a
//! navigation, so one click can race itself and leave the app in front of
//! you showing whatever it was showing before. Waking it first and asking a
//! beat later takes that race away.
//!
//! Measured, on the machine the report came from: the window comes back in
//! 25 ms from an app that is running with none, and seven links in a row
//! landed on the right session — four with the window up, three with it
//! hidden. So this is not a fault that shows on demand, and the two steps
//! are not a fix for a fault that was caught; they are the shape that has no
//! room for it.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};

use super::{Host, Outcome, Seat};

pub struct Cloud;

/// The app a cloud session is at home in.
const APP: &str = "Claude";

/// How long the link waits behind the wake: six times the 25 ms the window
/// took to come back, and still a tenth of a second on a click.
const SETTLE: Duration = Duration::from_millis(150);

/// What a click on a cloud session runs, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Bring the app up, so the link arrives at a window that already
    /// exists rather than at one being made.
    Wake(&'static str),
    /// Let that window finish arriving before asking it to go anywhere.
    Settle(Duration),
    /// Ask for the session itself. `web` is for a machine the app link is
    /// no good on (#127).
    Go { app: String, web: String },
}

/// The steps a click on `seat` runs.
pub fn steps(seat: &Seat) -> Vec<Step> {
    let app = match seat.focus_url.as_deref() {
        Some(url) => url.to_string(),
        None => crate::cloud::app_url(&seat.handle),
    };
    vec![Step::Wake(APP), Step::Settle(SETTLE), Step::Go { app, web: crate::cloud::web_url(&seat.handle) }]
}

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
        let mut went = None;
        for step in steps(seat) {
            match step {
                // A wake that fails is not a reason to stop: the link still
                // has its own way of starting the app.
                Step::Wake(app) => {
                    let _ = Command::new("open").args(["-a", app]).status();
                }
                Step::Settle(how_long) => std::thread::sleep(how_long),
                Step::Go { app, web } => {
                    if Command::new("open").arg(&app).status().is_ok_and(|s| s.success()) {
                        went = Some(app);
                    } else {
                        // No desktop app on this machine, or it would not
                        // take the link.
                        Command::new("open").arg(&web).status().context("opening the session")?;
                        went = Some(web);
                    }
                }
            }
        }
        Ok(Outcome::Focused(went.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app is woken before it is asked for a session, never in the same
    /// breath (#246): the handler on the other side looks the session up
    /// while the window arrives, and a link that arrives during that is one
    /// the app is free to drop.
    #[test]
    fn the_app_is_awake_before_it_is_asked_for_a_session() {
        let seat = Seat {
            host: "cloud",
            handle: "cse_1".into(),
            window: Some("o/r".into()),
            focus_url: Some("claude://code/cse_1".into()),
        };
        let click = steps(&seat);
        assert_eq!(
            click,
            vec![
                Step::Wake("Claude"),
                Step::Settle(SETTLE),
                Step::Go { app: "claude://code/cse_1".into(), web: "https://claude.ai/code/cse_1".into() },
            ]
        );
        let place = |want: fn(&Step) -> bool| click.iter().position(want).expect("every click has all three");
        assert!(
            place(|s| matches!(s, Step::Wake(_))) < place(|s| matches!(s, Step::Go { .. })),
            "the link arrives at a window that is already there"
        );
        assert!(
            place(|s| matches!(s, Step::Settle(_))) < place(|s| matches!(s, Step::Go { .. })),
            "and one that has stopped arriving"
        );

        // A seat with no link of its own still has one: the id is the
        // address (#120).
        let bare = Seat { focus_url: None, ..seat };
        assert!(matches!(
            steps(&bare).last(),
            Some(Step::Go { app, .. }) if app == "claude://code/cse_1"
        ));
    }
}
