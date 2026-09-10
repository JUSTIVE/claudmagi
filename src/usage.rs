//! Claude plan usage for the status bar (#48): the same numbers Claude Code
//! shows under `/usage`, read from Anthropic's OAuth usage endpoint with the
//! token Claude Code keeps in the login keychain.
//!
//! No HTTP client is linked in: the request goes through `/usr/bin/curl`,
//! the token through `/usr/bin/security` (with `~/.claude/.credentials.json`
//! as the fallback Claude Code uses on other platforms). Both run on the
//! background executor; the board polls once a minute.

use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::sources::config_dir;

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const CURL_TIMEOUT_S: u64 = 10;
/// How often the status bar refreshes.
pub const POLL: Duration = Duration::from_secs(60);
/// A value older than this is shown dimmed: the last fetches failed.
pub const STALE_AFTER: Duration = Duration::from_secs(15 * 60);

/// One rate-limit window as the API reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Percent of the window used, 0–100.
    pub utilization: f32,
    /// Unix seconds when the window resets, if known.
    pub resets_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Usage {
    pub five_hour: Window,
    pub seven_day: Window,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageError {
    /// No Claude Code OAuth login on this machine (API-key users, or never
    /// logged in): nothing to show, not worth retrying often.
    NoCredentials,
    /// The stored token is past its expiry; Claude Code refreshes it on its
    /// next request, so a later poll will pick the new one up.
    Expired,
    /// `curl` failed or the API answered with an error status.
    Request(String),
    /// The API answered with something we could not read.
    Response,
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsageError::NoCredentials => write!(f, "no Claude Code login found"),
            UsageError::Expired => write!(f, "Claude Code login token expired"),
            UsageError::Request(e) => write!(f, "usage request failed: {e}"),
            UsageError::Response => write!(f, "unreadable usage response"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOauth {
    access_token: Option<String>,
    /// Milliseconds since the epoch.
    expires_at: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCredentials {
    claude_ai_oauth: Option<RawOauth>,
}

#[derive(Deserialize, Default)]
struct RawWindow {
    utilization: Option<f32>,
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct RawUsage {
    five_hour: Option<RawWindow>,
    seven_day: Option<RawWindow>,
}

/// The OAuth access token out of Claude Code's credential JSON, plus its
/// expiry in unix seconds. Accepts both the `claudeAiOauth` wrapper the CLI
/// writes and a bare object.
pub fn parse_credentials(text: &str) -> Option<(String, Option<u64>)> {
    let oauth = match serde_json::from_str::<RawCredentials>(text) {
        Ok(RawCredentials {
            claude_ai_oauth: Some(o),
        }) => o,
        _ => serde_json::from_str::<RawOauth>(text).ok()?,
    };
    let token = oauth.access_token.filter(|t| !t.trim().is_empty())?;
    Some((token, oauth.expires_at.map(|ms| ms / 1000)))
}

fn keychain_credentials() -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

fn file_credentials() -> Option<String> {
    std::fs::read_to_string(config_dir()?.join(".credentials.json")).ok()
}

/// Claude Code's current access token, keychain first, then the file.
pub fn read_token() -> Result<String, UsageError> {
    let text = keychain_credentials()
        .or_else(file_credentials)
        .ok_or(UsageError::NoCredentials)?;
    let (token, expires_at) = parse_credentials(&text).ok_or(UsageError::NoCredentials)?;
    if expires_at.is_some_and(|exp| exp <= now_secs()) {
        return Err(UsageError::Expired);
    }
    Ok(token)
}

/// Turns the API body into `Usage`; both windows must be present.
pub fn parse_usage(text: &str) -> Option<Usage> {
    let raw: RawUsage = serde_json::from_str(text).ok()?;
    let window = |w: Option<RawWindow>| {
        let w = w?;
        Some(Window {
            utilization: w.utilization?,
            resets_at: w.resets_at.as_deref().and_then(parse_iso8601),
        })
    };
    Some(Usage {
        five_hour: window(raw.five_hour)?,
        seven_day: window(raw.seven_day)?,
    })
}

/// Fetches the usage with the given token through `curl`.
pub fn fetch_with(token: &str) -> Result<Usage, UsageError> {
    let out = Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "-m",
            &CURL_TIMEOUT_S.to_string(),
            "-w",
            "\n%{http_code}",
        ])
        .args(["-H", &format!("Authorization: Bearer {token}")])
        .args(["-H", "anthropic-beta: oauth-2025-04-20"])
        .args(["-H", "User-Agent: claudmagi"])
        .arg(USAGE_URL)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| UsageError::Request(e.to_string()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(UsageError::Request(if err.is_empty() {
            "curl failed".into()
        } else {
            err
        }));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim_end()));
    match code.trim() {
        "200" => parse_usage(body).ok_or(UsageError::Response),
        "401" | "403" => Err(UsageError::Expired),
        other => Err(UsageError::Request(format!("HTTP {other}"))),
    }
}

/// Token + request in one go; what the board polls.
pub fn fetch() -> Result<Usage, UsageError> {
    fetch_with(&read_token()?)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `2026-09-10T06:10:00.227254+00:00` (or `Z`) → unix seconds. Fractional
/// seconds are dropped; the offset is applied.
pub fn parse_iso8601(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let (time, offset) = match rest.find(['Z', 'z', '+', '-']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "Z"),
    };
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next().unwrap_or("0").split('.').next()?.parse().ok()?;
    let off_secs: i64 = match offset {
        "Z" | "z" => 0,
        o => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let (oh, om) = o[1..].split_once(':').unwrap_or((&o[1..], "0"));
            sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60)
        }
    };
    let secs = days_from_civil(y, m, day) * 86_400 + h * 3600 + min * 60 + sec - off_secs;
    u64::try_from(secs).ok()
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2H10M`, `35M`, `3D`: how long until `resets_at`, for the status bar.
pub fn countdown(resets_at: u64, now: u64) -> String {
    let left = resets_at.saturating_sub(now);
    let (d, h, m) = (left / 86_400, (left / 3600) % 24, (left / 60) % 60);
    if d > 0 {
        if h > 0 {
            format!("{d}D{h}H")
        } else {
            format!("{d}D")
        }
    } else if h > 0 {
        format!("{h}H{m:02}M")
    } else {
        format!("{}M", m.max(1))
    }
}

impl Usage {
    /// Status-bar text: `5H 22% ↻2H10M · 7D 16% ↻3D`.
    pub fn label(&self, now: u64) -> String {
        let part = |name: &str, w: &Window| {
            let mut s = format!("{name} {}%", w.utilization.round() as i64);
            if let Some(r) = w.resets_at {
                s.push_str(&format!(" ↻{}", countdown(r, now)));
            }
            s
        };
        format!(
            "{} · {}",
            part("5H", &self.five_hour),
            part("7D", &self.seven_day)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{
        "five_hour": {"utilization": 22.0, "resets_at": "2026-09-10T06:10:00.227254+00:00", "limit_dollars": null},
        "seven_day": {"utilization": 16.4, "resets_at": "2026-09-13T12:00:00.227279+00:00"},
        "seven_day_opus": null, "extra_usage": {"is_enabled": true}
    }"#;

    #[test]
    fn parses_the_usage_body_and_ignores_the_rest() {
        let u = parse_usage(BODY).unwrap();
        assert_eq!(u.five_hour.utilization, 22.0);
        assert_eq!(u.seven_day.utilization, 16.4);
        assert_eq!(u.five_hour.resets_at, parse_iso8601("2026-09-10T06:10:00Z"));
        assert!(
            parse_usage(r#"{"five_hour": {"utilization": 1}}"#).is_none(),
            "both windows required"
        );
        assert!(parse_usage("<html>").is_none());
    }

    #[test]
    fn iso8601_handles_offsets_and_fractions() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("1970-01-02T00:00:00+00:00"), Some(86_400));
        assert_eq!(
            parse_iso8601("2026-09-10T06:10:00.227254+00:00"),
            Some(1_789_020_600)
        );
        assert_eq!(
            parse_iso8601("2026-09-10T15:10:00+09:00"),
            Some(1_789_020_600),
            "KST is UTC+9"
        );
        assert_eq!(
            parse_iso8601("2026-09-10T01:10:00-05:00"),
            Some(1_789_020_600)
        );
        assert_eq!(
            parse_iso8601("2000-03-01T00:00:00Z"),
            Some(951_868_800),
            "leap day boundary"
        );
        assert!(parse_iso8601("soon").is_none());
    }

    #[test]
    fn credentials_come_wrapped_or_bare() {
        let wrapped = r#"{"mcpOAuth": {}, "claudeAiOauth": {"accessToken": "sk-ant-x", "expiresAt": 1789005650000, "subscriptionType": "max"}}"#;
        assert_eq!(
            parse_credentials(wrapped),
            Some(("sk-ant-x".into(), Some(1_789_005_650)))
        );
        let bare = r#"{"accessToken": "t", "refreshToken": "r"}"#;
        assert_eq!(parse_credentials(bare), Some(("t".into(), None)));
        assert!(parse_credentials(r#"{"claudeAiOauth": {"accessToken": ""}}"#).is_none());
        assert!(parse_credentials("nope").is_none());
    }

    #[test]
    fn label_rounds_and_counts_down() {
        let u = Usage {
            five_hour: Window {
                utilization: 22.4,
                resets_at: Some(1_000 + 2 * 3600 + 10 * 60),
            },
            seven_day: Window {
                utilization: 16.0,
                resets_at: Some(1_000 + 3 * 86_400 + 5 * 3600),
            },
        };
        assert_eq!(u.label(1_000), "5H 22% ↻2H10M · 7D 16% ↻3D5H");
        assert_eq!(countdown(1_000 + 30, 1_000), "1M", "never shows zero");
        assert_eq!(
            countdown(500, 1_000),
            "1M",
            "a reset in the past is imminent"
        );
        assert_eq!(countdown(1_000 + 86_400, 1_000), "1D");
        let bare = Usage {
            five_hour: Window {
                utilization: 0.0,
                resets_at: None,
            },
            seven_day: u.seven_day,
        };
        assert!(bare.label(1_000).starts_with("5H 0% · 7D"));
    }
}
