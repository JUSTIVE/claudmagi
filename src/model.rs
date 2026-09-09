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

/// A subagent spawned by a session (the `Agent` tool). Lives in the parent
/// process, so it is discovered from the session's transcript directory.
#[derive(Clone, Debug, PartialEq)]
pub struct SubagentInfo {
    pub agent_id: String,
    pub agent_type: String,
    pub description: String,
    pub running: bool,
    /// Unix millis of the transcript's creation / last write.
    pub started_at: u64,
    pub updated_at: u64,
}

impl SubagentInfo {
    pub fn synthetic(seq: u32, agent_type: &str, description: &str, running: bool) -> Self {
        Self {
            agent_id: format!("fake-agent-{seq:04}"),
            agent_type: agent_type.to_string(),
            description: description.to_string(),
            running,
            started_at: seq as u64,
            updated_at: seq as u64,
        }
    }

    pub fn phase(&self) -> Phase {
        if self.running { Phase::Working } else { Phase::Idle }
    }

    /// Short uppercase label for the chip: the last segment of the agent type.
    pub fn label(&self) -> String {
        let base = self.agent_type.rsplit(':').next().unwrap_or(&self.agent_type);
        let mut s: String = base.chars().take(12).collect::<String>().to_ascii_uppercase();
        if s.is_empty() {
            s = "AGENT".into();
        }
        s
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
    pub subagents: Vec<SubagentInfo>,
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
            subagents: Vec::new(),
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

/// Animated presentation state shared by session and subagent chips.
#[derive(Clone, Debug)]
pub struct Anim {
    /// 0 = plugged in, 1 = pulled out of the socket.
    pub disconnect: f32,
    pub hover_t: f32,
    pub born: Instant,
    /// The underlying thing disappeared; fade out then drop.
    pub gone: bool,
    pub fade: f32,
}

impl Anim {
    fn new(now: Instant) -> Self {
        Self { disconnect: 0.0, hover_t: 0.0, born: now, gone: false, fade: 0.0 }
    }

    pub fn age(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.born).as_secs_f32()
    }

    /// Combined appear/fade opacity.
    pub fn alpha(&self, now: Instant) -> f32 {
        crate::geom::ease_out(self.age(now) / APPEAR_SECS) * self.fade
    }

    fn step(&mut self, phase: Phase, hovered: bool, dt: f32, now: Instant) {
        let target_p = if phase == Phase::Working || self.age(now) < UNPLUG_DELAY_SECS { 0.0 } else { 1.0 };
        approach(&mut self.disconnect, target_p, dt / UNPLUG_SECS);
        approach(&mut self.hover_t, if hovered { 1.0 } else { 0.0 }, dt / HOVER_SECS);
        approach(&mut self.fade, if self.gone { 0.0 } else { 1.0 }, dt / FADE_SECS);
    }

    fn settle(&mut self, phase: Phase) {
        self.born = Instant::now() - Duration::from_secs(60);
        self.fade = 1.0;
        self.disconnect = if phase == Phase::Working { 0.0 } else { 1.0 };
        self.hover_t = 0.0;
    }

    fn dead(&self) -> bool {
        self.gone && self.fade <= 0.001
    }
}

#[derive(Clone, Debug)]
pub struct SubState {
    pub info: SubagentInfo,
    pub anim: Anim,
}

/// A session plus its animated presentation state and subagents.
#[derive(Clone, Debug)]
pub struct ChipState {
    pub info: SessionInfo,
    pub anim: Anim,
    pub subs: Vec<SubState>,
}

impl ChipState {
    fn new(info: SessionInfo, now: Instant) -> Self {
        let mut chip = Self { info: info.clone(), anim: Anim::new(now), subs: Vec::new() };
        chip.merge_subs(&info.subagents, now);
        chip
    }

    pub fn phase(&self) -> Phase {
        self.info.phase()
    }

    fn merge_subs(&mut self, list: &[SubagentInfo], now: Instant) {
        let mut seen = vec![false; self.subs.len()];
        for info in list {
            if let Some(i) = self.subs.iter().position(|s| s.info.agent_id == info.agent_id) {
                self.subs[i].info = info.clone();
                self.subs[i].anim.gone = false;
                seen[i] = true;
            } else {
                self.subs.push(SubState { info: info.clone(), anim: Anim::new(now) });
                seen.push(true);
            }
        }
        for (i, s) in seen.iter().enumerate() {
            if !s {
                self.subs[i].anim.gone = true;
            }
        }
    }
}

/// Something on the board that can be hovered or clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Session(usize),
    Sub(usize, usize),
}

impl Target {
    pub fn session(self) -> usize {
        match self {
            Target::Session(i) | Target::Sub(i, _) => i,
        }
    }
}

/// Everything the renderer needs, updated by `apply` (data) and `tick` (time).
#[derive(Default)]
pub struct BoardModel {
    pub chips: Vec<ChipState>,
    pub hovered: Option<Target>,
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
                let chip = &mut self.chips[i];
                chip.merge_subs(&info.subagents, now);
                chip.info = info;
                chip.anim.gone = false;
                seen[i] = true;
            } else {
                self.chips.push(ChipState::new(info, now));
                seen.push(true);
            }
        }
        for (i, s) in seen.iter().enumerate() {
            if !s {
                self.chips[i].anim.gone = true;
                for sub in &mut self.chips[i].subs {
                    sub.anim.gone = true;
                }
            }
        }
    }

    /// Advances animations by `dt` seconds. Returns true when chips were
    /// dropped (callers may want to relayout).
    pub fn tick(&mut self, dt: f32, now: Instant) -> bool {
        let hovered = self.hovered;
        let mut dropped = false;
        for (i, c) in self.chips.iter_mut().enumerate() {
            c.anim.step(c.info.phase(), hovered == Some(Target::Session(i)), dt, now);
            for (j, s) in c.subs.iter_mut().enumerate() {
                s.anim.step(s.info.phase(), hovered == Some(Target::Sub(i, j)), dt, now);
            }
            let before = c.subs.len();
            c.subs.retain(|s| !s.anim.dead());
            dropped |= c.subs.len() != before;
        }
        let before = self.chips.len();
        self.chips.retain(|c| !c.anim.dead());
        dropped |= self.chips.len() != before;
        if let Some((_, at)) = &self.notice {
            if now.saturating_duration_since(*at) > Duration::from_secs(NOTICE_SECS) {
                self.notice = None;
            }
        }
        if dropped {
            self.hovered = None;
        }
        dropped
    }

    /// Jumps every animation to its resting state (headless renders, tests).
    pub fn settle(&mut self) {
        self.chips.retain(|c| !c.anim.gone);
        for c in &mut self.chips {
            c.anim.settle(c.info.phase());
            c.subs.retain(|s| !s.anim.gone);
            for s in &mut c.subs {
                s.anim.settle(s.info.phase());
            }
        }
    }

    pub fn live(&self) -> impl Iterator<Item = &ChipState> {
        self.chips.iter().filter(|c| !c.anim.gone)
    }

    pub fn count(&self, phase: Phase) -> usize {
        self.live().filter(|c| c.phase() == phase).count()
    }

    pub fn live_subagents(&self) -> usize {
        self.live().map(|c| c.subs.iter().filter(|s| !s.anim.gone).count()).sum()
    }

    pub fn summary(&self) -> String {
        let n = self.live().count();
        if n == 0 {
            return "NO LIVE CLAUDE CODE SESSIONS".to_string();
        }
        let mut s = format!(
            "{} SESSION{} · {} WORKING · {} NEEDS INPUT · {} IDLE",
            n,
            if n == 1 { "" } else { "S" },
            self.count(Phase::Working),
            self.count(Phase::NeedsUser),
            self.count(Phase::Idle)
        );
        let subs = self.live_subagents();
        if subs > 0 {
            s.push_str(&format!(" · {subs} SUBAGENT{}", if subs == 1 { "" } else { "S" }));
        }
        s
    }

    pub fn detail(&self) -> String {
        if let Some((msg, _)) = &self.notice {
            return msg.clone();
        }
        let Some(target) = self.hovered else { return String::new() };
        let Some(c) = self.chips.get(target.session()) else { return String::new() };
        let info = &c.info;
        match target {
            Target::Session(_) => {
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
            Target::Sub(_, j) => {
                let Some(sub) = c.subs.get(j) else { return String::new() };
                let s = &sub.info;
                format!(
                    "{} ⟵ {} · {} · {}",
                    s.agent_type,
                    info.label(),
                    if s.description.is_empty() { "(no description)" } else { &s.description },
                    if s.running { "running" } else { "done" }
                )
            }
        }
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
        assert!(m.chips[1].anim.gone);
        assert_eq!(m.live().count(), 2);
    }

    #[test]
    fn tick_fades_gone_chips_out_and_drops_them() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        m.apply(vec![SessionInfo::synthetic(1, "a", Phase::Working)], t0);
        m.tick(1.0, t0 + secs(1.0));
        assert_eq!(m.chips[0].anim.fade, 1.0);
        m.apply(vec![], t0 + secs(1.0));
        assert!(m.chips[0].anim.gone);
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
        assert_eq!(m.chips[0].anim.disconnect, 0.0, "still plugged in while appearing");
        m.tick(1.0, t0 + secs(2.0));
        assert_eq!(m.chips[0].anim.disconnect, 1.0);
        // Back to work: plugs in again.
        let mut a = m.chips[0].info.clone();
        a.set_phase(Phase::Working);
        m.apply(vec![a], t0 + secs(2.0));
        m.tick(1.0, t0 + secs(3.0));
        assert_eq!(m.chips[0].anim.disconnect, 0.0);
    }

    #[test]
    fn subagents_follow_their_session_and_unplug_when_done() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        let mut a = SessionInfo::synthetic(1, "a", Phase::Working);
        a.subagents.push(SubagentInfo::synthetic(1, "Explore", "look around", true));
        a.subagents.push(SubagentInfo::synthetic(2, "oh-my-claudecode:executor", "build it", true));
        m.apply(vec![a.clone()], t0);
        assert_eq!(m.chips[0].subs.len(), 2);
        assert_eq!(m.chips[0].subs[1].info.label(), "EXECUTOR");
        assert_eq!(m.live_subagents(), 2);

        // First agent finishes, second disappears entirely.
        a.subagents[0].running = false;
        a.subagents.truncate(1);
        m.apply(vec![a.clone()], t0 + secs(1.0));
        m.tick(2.0, t0 + secs(3.0));
        assert_eq!(m.chips[0].subs.len(), 1, "vanished subagent was faded out and dropped");
        assert_eq!(m.chips[0].subs[0].anim.disconnect, 1.0, "finished subagent is unplugged");

        // Session goes away: its subagents go with it.
        m.apply(vec![], t0 + secs(3.0));
        assert!(m.chips[0].subs[0].anim.gone);
        m.tick(1.0, t0 + secs(4.0));
        assert!(m.chips.is_empty());
    }

    #[test]
    fn settle_resolves_every_animation() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        let mut b = SessionInfo::synthetic(2, "b", Phase::Working);
        b.subagents.push(SubagentInfo::synthetic(9, "Explore", "", false));
        m.apply(vec![SessionInfo::synthetic(1, "a", Phase::Idle), b], t0);
        m.settle();
        assert_eq!(m.chips[0].anim.disconnect, 1.0);
        assert_eq!(m.chips[1].anim.disconnect, 0.0);
        assert_eq!(m.chips[1].subs[0].anim.disconnect, 1.0);
        assert!(m.chips.iter().all(|c| c.anim.alpha(Instant::now()) > 0.99));
    }
}
