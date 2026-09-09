//! Data layer: session facts and the board's animated state. No gpui here,
//! so everything in this module can be unit-tested and driven headlessly.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// What a session is doing, as far as the board cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Claude is working; the chip stays plugged in.
    Working,
    /// A permission dialog / question is waiting on the user.
    NeedsUser,
    /// The turn finished; the session is idle at the prompt.
    Idle,
}

impl Phase {
    pub const ALL: [Phase; 3] = [Phase::Working, Phase::NeedsUser, Phase::Idle];

    pub fn label(self) -> &'static str {
        match self {
            Phase::Working => "working",
            Phase::NeedsUser => "needs input",
            Phase::Idle => "idle",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Phase::Working => "W",
            Phase::NeedsUser => "N",
            Phase::Idle => "I",
        }
    }

    pub fn next(self) -> Phase {
        match self {
            Phase::Working => Phase::NeedsUser,
            Phase::NeedsUser => Phase::Idle,
            Phase::Idle => Phase::Working,
        }
    }
}

/// One Claude Code session as reported by a `SessionSource`.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub pid: i32,
    pub session_id: String,
    pub cwd: String,
    pub name: String,
    pub status: String,
    pub waiting_for: Option<String>,
    pub state: Option<String>,
    pub tempo: Option<String>,
    pub started_at: u64,
    pub tty: Option<String>,
    pub warp_focus_url: Option<String>,
    pub warp_session_uuid: Option<String>,
    /// True for sessions invented by the test tools; never focused for real.
    pub synthetic: bool,
}

impl SessionInfo {
    /// A made-up session for the sandbox.
    pub fn synthetic(seq: u32, name: &str, phase: Phase) -> Self {
        let mut s = Self {
            pid: 90_000 + seq as i32,
            session_id: format!("fake-{seq:04}"),
            cwd: format!("/tmp/sandbox/{}", name.to_ascii_lowercase()),
            name: name.to_string(),
            status: "busy".into(),
            waiting_for: None,
            state: None,
            tempo: None,
            started_at: seq as u64,
            tty: Some(format!("ttys{:03}", 900 + seq % 100)),
            warp_focus_url: None,
            warp_session_uuid: None,
            synthetic: true,
        };
        s.set_phase(phase);
        s
    }

    pub fn key(&self) -> (i32, &str) {
        (self.pid, &self.session_id)
    }

    pub fn phase(&self) -> Phase {
        if self.waiting_for.is_some() {
            return Phase::NeedsUser;
        }
        if self.status == "busy" {
            return Phase::Working;
        }
        let blocked = |v: &Option<String>| v.as_deref() == Some("blocked");
        if blocked(&self.tempo) || blocked(&self.state) { Phase::NeedsUser } else { Phase::Idle }
    }

    /// Rewrites the raw fields so that `phase()` reports `phase`.
    pub fn set_phase(&mut self, phase: Phase) {
        self.tempo = None;
        self.state = None;
        match phase {
            Phase::Working => {
                self.status = "busy".into();
                self.waiting_for = None;
            }
            Phase::NeedsUser => {
                self.status = "busy".into();
                self.waiting_for = Some("dialog open".into());
            }
            Phase::Idle => {
                self.status = "idle".into();
                self.waiting_for = None;
            }
        }
    }

    pub fn label(&self) -> String {
        let raw = if self.name.trim().is_empty() {
            PathBuf::from(&self.cwd)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("PID{}", self.pid))
        } else {
            self.name.clone()
        };
        raw.to_ascii_uppercase()
    }

    pub fn short_cwd(&self) -> String {
        if let Some(home) = dirs::home_dir() {
            if let Ok(rest) = PathBuf::from(&self.cwd).strip_prefix(&home) {
                return format!("~/{}", rest.display());
            }
        }
        self.cwd.clone()
    }
}

/// Animation timings, in seconds.
pub const APPEAR_SECS: f32 = 0.45;
pub const UNPLUG_DELAY_SECS: f32 = 0.7;
pub const UNPLUG_SECS: f32 = 0.42;
pub const HOVER_SECS: f32 = 0.12;
pub const FADE_SECS: f32 = 0.35;
pub const NOTICE_SECS: u64 = 5;

/// A session plus its animated presentation state.
#[derive(Clone, Debug)]
pub struct ChipState {
    pub info: SessionInfo,
    /// 0 = plugged in, 1 = pulled out of the socket.
    pub disconnect: f32,
    pub hover_t: f32,
    pub born: Instant,
    /// The session disappeared; fade out then drop.
    pub gone: bool,
    pub fade: f32,
}

impl ChipState {
    fn new(info: SessionInfo, now: Instant) -> Self {
        Self { info, disconnect: 0.0, hover_t: 0.0, born: now, gone: false, fade: 0.0 }
    }

    pub fn phase(&self) -> Phase {
        self.info.phase()
    }

    pub fn age(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.born).as_secs_f32()
    }

    /// Combined appear/fade opacity.
    pub fn alpha(&self, now: Instant) -> f32 {
        crate::geom::ease_out(self.age(now) / APPEAR_SECS) * self.fade
    }

    fn target_disconnect(&self, now: Instant) -> f32 {
        if self.phase() == Phase::Working || self.age(now) < UNPLUG_DELAY_SECS { 0.0 } else { 1.0 }
    }
}

/// Everything the renderer needs, updated by `apply` (data) and `tick` (time).
#[derive(Default)]
pub struct BoardModel {
    pub chips: Vec<ChipState>,
    pub hovered: Option<usize>,
    pub notice: Option<(String, Instant)>,
}

impl BoardModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merges a fresh snapshot: updates known sessions, adds new ones at the
    /// end (so lanes stay stable), and marks missing ones as gone.
    pub fn apply(&mut self, list: Vec<SessionInfo>, now: Instant) {
        let mut seen = vec![false; self.chips.len()];
        for info in list {
            if let Some(i) = self.chips.iter().position(|c| c.info.key() == info.key()) {
                self.chips[i].info = info;
                self.chips[i].gone = false;
                seen[i] = true;
            } else {
                self.chips.push(ChipState::new(info, now));
                seen.push(true);
            }
        }
        for (i, s) in seen.iter().enumerate() {
            if !s {
                self.chips[i].gone = true;
            }
        }
    }

    /// Advances animations by `dt` seconds. Returns true when chips were
    /// dropped (callers may want to relayout).
    pub fn tick(&mut self, dt: f32, now: Instant) -> bool {
        let hovered = self.hovered;
        for (i, c) in self.chips.iter_mut().enumerate() {
            let target_p = c.target_disconnect(now);
            approach(&mut c.disconnect, target_p, dt / UNPLUG_SECS);
            let target_h = if hovered == Some(i) { 1.0 } else { 0.0 };
            approach(&mut c.hover_t, target_h, dt / HOVER_SECS);
            let target_f = if c.gone { 0.0 } else { 1.0 };
            approach(&mut c.fade, target_f, dt / FADE_SECS);
        }
        let before = self.chips.len();
        self.chips.retain(|c| !(c.gone && c.fade <= 0.001));
        if let Some((_, at)) = &self.notice {
            if now.saturating_duration_since(*at) > Duration::from_secs(NOTICE_SECS) {
                self.notice = None;
            }
        }
        if self.chips.len() != before {
            self.hovered = None;
            true
        } else {
            false
        }
    }

    /// Jumps every animation to its resting state (headless renders, tests).
    pub fn settle(&mut self) {
        let far = Instant::now() - Duration::from_secs(60);
        self.chips.retain(|c| !c.gone);
        for c in &mut self.chips {
            c.born = far;
            c.fade = 1.0;
            c.disconnect = if c.phase() == Phase::Working { 0.0 } else { 1.0 };
            c.hover_t = 0.0;
        }
    }

    pub fn live(&self) -> impl Iterator<Item = &ChipState> {
        self.chips.iter().filter(|c| !c.gone)
    }

    pub fn count(&self, phase: Phase) -> usize {
        self.live().filter(|c| c.phase() == phase).count()
    }

    pub fn summary(&self) -> String {
        let n = self.live().count();
        if n == 0 {
            return "NO LIVE CLAUDE CODE SESSIONS".to_string();
        }
        format!(
            "{} SESSION{} · {} WORKING · {} NEEDS INPUT · {} IDLE",
            n,
            if n == 1 { "" } else { "S" },
            self.count(Phase::Working),
            self.count(Phase::NeedsUser),
            self.count(Phase::Idle)
        )
    }

    pub fn detail(&self) -> String {
        if let Some((msg, _)) = &self.notice {
            return msg.clone();
        }
        let Some(c) = self.hovered.and_then(|i| self.chips.get(i)) else { return String::new() };
        let info = &c.info;
        let mut parts = vec![info.label(), info.short_cwd()];
        parts.push(match info.phase() {
            Phase::Working => "working".into(),
            Phase::NeedsUser => match &info.waiting_for {
                Some(w) => format!("needs you: {w}"),
                None => "waiting for your answer".into(),
            },
            Phase::Idle => "idle".into(),
        });
        if let Some(tty) = &info.tty {
            parts.push(tty.clone());
        }
        parts.push(if info.synthetic { "sandbox".into() } else { format!("pid {}", info.pid) });
        parts.join(" · ")
    }

    pub fn set_notice(&mut self, msg: impl Into<String>, now: Instant) {
        self.notice = Some((msg.into(), now));
    }
}

fn approach(v: &mut f32, target: f32, amount: f32) {
    if *v < target {
        *v = (*v + amount).min(target);
    } else if *v > target {
        *v = (*v - amount).max(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: f32) -> Duration {
        Duration::from_secs_f32(s)
    }

    #[test]
    fn phase_round_trips_through_raw_fields() {
        for phase in Phase::ALL {
            let s = SessionInfo::synthetic(1, "t", phase);
            assert_eq!(s.phase(), phase);
        }
        let mut s = SessionInfo::synthetic(2, "t", Phase::Idle);
        s.tempo = Some("blocked".into());
        assert_eq!(s.phase(), Phase::NeedsUser, "idle + blocked tempo counts as needs-input");
    }

    #[test]
    fn apply_keeps_lane_order_and_marks_missing_as_gone() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        let a = SessionInfo::synthetic(1, "a", Phase::Working);
        let b = SessionInfo::synthetic(2, "b", Phase::Idle);
        m.apply(vec![a.clone(), b.clone()], t0);
        assert_eq!(m.chips.len(), 2);

        // b disappears, c appears: a stays first, c is appended, b is gone.
        let c = SessionInfo::synthetic(3, "c", Phase::Working);
        m.apply(vec![c.clone(), a.clone()], t0);
        let names: Vec<_> = m.chips.iter().map(|c| c.info.name.clone()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert!(m.chips[1].gone);
        assert_eq!(m.live().count(), 2);
    }

    #[test]
    fn tick_fades_gone_chips_out_and_drops_them() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        m.apply(vec![SessionInfo::synthetic(1, "a", Phase::Working)], t0);
        m.tick(1.0, t0 + secs(1.0));
        assert_eq!(m.chips[0].fade, 1.0);
        m.apply(vec![], t0 + secs(1.0));
        assert!(m.chips[0].gone);
        let dropped = m.tick(FADE_SECS + 0.1, t0 + secs(2.0));
        assert!(dropped);
        assert!(m.chips.is_empty());
    }

    #[test]
    fn unplug_waits_for_the_appear_animation_then_completes() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        m.apply(vec![SessionInfo::synthetic(1, "a", Phase::Idle)], t0);
        m.tick(0.3, t0 + secs(0.3));
        assert_eq!(m.chips[0].disconnect, 0.0, "still plugged in while appearing");
        m.tick(1.0, t0 + secs(2.0));
        assert_eq!(m.chips[0].disconnect, 1.0);
        // Back to work: plugs in again.
        let mut a = m.chips[0].info.clone();
        a.set_phase(Phase::Working);
        m.apply(vec![a], t0 + secs(2.0));
        m.tick(1.0, t0 + secs(3.0));
        assert_eq!(m.chips[0].disconnect, 0.0);
    }

    #[test]
    fn settle_resolves_every_animation() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        m.apply(
            vec![SessionInfo::synthetic(1, "a", Phase::Idle), SessionInfo::synthetic(2, "b", Phase::Working)],
            t0,
        );
        m.settle();
        assert_eq!(m.chips[0].disconnect, 1.0);
        assert_eq!(m.chips[1].disconnect, 0.0);
        assert!(m.chips.iter().all(|c| c.alpha(Instant::now()) > 0.99));
    }
}
