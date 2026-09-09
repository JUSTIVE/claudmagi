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
    /// Where the session runs: `warp:<pane uuid>`, `term:<program>` or
    /// `desktop`. Sessions sharing a group sit together on the board (#41).
    pub group: String,
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
            group: format!("sandbox:{}", (seq.max(1) - 1) / 3),
        };
        s.set_phase(phase);
        s
    }

    /// Human label for the group.
    pub fn group_label(&self) -> String {
        match self.group.split_once(':') {
            Some(("warp", uuid)) => format!("warp pane {}", &uuid[..uuid.len().min(8)]),
            Some(("term", prog)) => prog.to_string(),
            Some(("sandbox", n)) => format!("sandbox group {n}"),
            _ => self.group.clone(),
        }
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
/// Half of a relocation: fade out at the old spot, then fade in at the new one.
pub const RELOC_HALF_SECS: f32 = 0.22;
pub const NOTICE_SECS: u64 = 5;

/// A chip moving to a different spot on its lane (window resized, #28).
#[derive(Clone, Copy, Debug)]
pub struct Reloc {
    /// Arc length of the old spot.
    pub from_s: f32,
    pub started: Instant,
}

impl Reloc {
    /// `(use_old_spot, alpha_factor)` for `now`.
    pub fn stage(&self, now: Instant) -> (bool, f32) {
        let t = now.saturating_duration_since(self.started).as_secs_f32();
        if t < RELOC_HALF_SECS {
            (true, 1.0 - t / RELOC_HALF_SECS)
        } else {
            (false, ((t - RELOC_HALF_SECS) / RELOC_HALF_SECS).min(1.0))
        }
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started).as_secs_f32() >= 2.0 * RELOC_HALF_SECS
    }
}

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
    /// Which spot on the lane the chip was last placed at (see
    /// `scene::placements`), and its arc length; a change starts a `Reloc`.
    pub place_key: Option<u32>,
    pub last_s: f32,
    pub reloc: Option<Reloc>,
}

impl Anim {
    fn new(now: Instant) -> Self {
        Self {
            disconnect: 0.0,
            hover_t: 0.0,
            born: now,
            gone: false,
            fade: 0.0,
            place_key: None,
            last_s: 0.0,
            reloc: None,
        }
    }

    /// Records where the chip is about to be drawn; a new placement key
    /// (other than the very first) triggers the fade-out / fade-in.
    pub fn track_placement(&mut self, key: u32, s: f32, now: Instant) {
        match self.place_key {
            Some(prev) if prev != key => {
                let from_s = match self.reloc {
                    // Mid-relocation: keep fading from where we were heading.
                    Some(r) if !r.stage(now).0 => self.last_s,
                    Some(r) => r.from_s,
                    None => self.last_s,
                };
                self.reloc = Some(Reloc { from_s, started: now });
            }
            _ => {}
        }
        if self.reloc.is_some_and(|r| r.done(now)) {
            self.reloc = None;
        }
        self.place_key = Some(key);
        self.last_s = s;
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
        self.reloc = None;
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
    /// Lane slot, derived from the group blocks (see `assign_slots`).
    pub slot: usize,
    /// Position inside the group's block, fixed for the life of the chip:
    /// sessions never shift up when an earlier one disappears (#23).
    pub gslot: usize,
}

impl ChipState {
    fn new(info: SessionInfo, now: Instant, gslot: usize) -> Self {
        let mut chip = Self { info: info.clone(), anim: Anim::new(now), subs: Vec::new(), slot: gslot, gslot };
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

/// Empty lanes between session groups (#41).
pub const GROUP_GAP: usize = 5;

/// Everything the renderer needs, updated by `apply` (data) and `tick` (time).
#[derive(Default)]
pub struct BoardModel {
    pub chips: Vec<ChipState>,
    pub hovered: Option<Target>,
    pub notice: Option<(String, Instant)>,
    /// Groups in order of first appearance; each owns a block of lanes.
    pub groups: Vec<String>,
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
                let gslot = self.free_gslot(&info.group);
                if !self.groups.contains(&info.group) {
                    self.groups.push(info.group.clone());
                }
                self.chips.push(ChipState::new(info, now, gslot));
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
        self.assign_slots();
    }

    /// Lowest in-group position not held by any chip of `group` (fading ones
    /// included).
    fn free_gslot(&self, group: &str) -> usize {
        (0..).find(|g| !self.chips.iter().any(|c| c.info.group == group && c.gslot == *g)).unwrap_or(0)
    }

    /// Lays the groups out as consecutive blocks separated by `GROUP_GAP`
    /// empty lanes. Inside a block chips keep their `gslot`; a block only
    /// moves when an earlier group grows or vanishes entirely.
    pub fn assign_slots(&mut self) {
        self.groups.retain(|g| self.chips.iter().any(|c| &c.info.group == g));
        let mut offset = 0;
        for group in &self.groups {
            let mut span = 0;
            for c in self.chips.iter_mut().filter(|c| &c.info.group == group) {
                c.slot = offset + c.gslot;
                span = span.max(c.gslot + 1);
            }
            offset += span + GROUP_GAP;
        }
    }

    /// One past the highest occupied slot: how many lanes the board needs.
    pub fn slot_span(&self) -> usize {
        self.chips.iter().map(|c| c.slot + 1).max().unwrap_or(0)
    }

    pub fn anim_mut(&mut self, target: Target) -> Option<&mut Anim> {
        match target {
            Target::Session(i) => self.chips.get_mut(i).map(|c| &mut c.anim),
            Target::Sub(i, j) => self.chips.get_mut(i).and_then(|c| c.subs.get_mut(j)).map(|s| &mut s.anim),
        }
    }

    pub fn anim(&self, target: Target) -> Option<&Anim> {
        match target {
            Target::Session(i) => self.chips.get(i).map(|c| &c.anim),
            Target::Sub(i, j) => self.chips.get(i).and_then(|c| c.subs.get(j)).map(|s| &s.anim),
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
        if self.chips.len() != before {
            dropped = true;
            self.assign_slots();
        }
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
                parts.push(info.group_label());
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
    fn slots_stay_put_when_an_earlier_session_disappears() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        let mut a = SessionInfo::synthetic(1, "a", Phase::Working);
        let mut b = SessionInfo::synthetic(2, "b", Phase::Working);
        let mut c = SessionInfo::synthetic(3, "c", Phase::Working);
        for s in [&mut a, &mut b, &mut c] {
            s.group = "sandbox:0".into();
        }
        m.apply(vec![a.clone(), b.clone(), c.clone()], t0);
        assert_eq!(m.chips.iter().map(|c| c.slot).collect::<Vec<_>>(), vec![0, 1, 2]);
        // a leaves and is dropped; b and c keep their slots.
        m.apply(vec![b.clone(), c.clone()], t0);
        m.tick(FADE_SECS + 0.1, t0 + secs(1.0));
        assert_eq!(m.chips.iter().map(|c| (c.info.name.clone(), c.slot)).collect::<Vec<_>>(),
                   vec![("b".to_string(), 1), ("c".to_string(), 2)]);
        assert_eq!(m.slot_span(), 3);
        // A newcomer takes the freed slot instead of pushing everyone around.
        let mut d = SessionInfo::synthetic(4, "d", Phase::Working);
        d.group = "sandbox:0".into();
        m.apply(vec![b, c, d], t0 + secs(1.0));
        assert_eq!(m.chips[2].slot, 0);
    }

    #[test]
    fn groups_form_blocks_with_a_big_gap_between_them() {
        let t0 = Instant::now();
        let mut m = BoardModel::new();
        let mut warp1 = SessionInfo::synthetic(1, "w1", Phase::Working);
        warp1.group = "warp:aaaa".into();
        let mut warp2 = SessionInfo::synthetic(2, "w2", Phase::Working);
        warp2.group = "warp:aaaa".into();
        let mut desk = SessionInfo::synthetic(3, "d1", Phase::Working);
        desk.group = "desktop".into();
        m.apply(vec![warp1.clone(), desk.clone(), warp2.clone()], t0);
        let slots: Vec<(String, usize)> = m.chips.iter().map(|c| (c.info.name.clone(), c.slot)).collect();
        assert_eq!(slots, vec![("w1".into(), 0), ("d1".into(), 2 + GROUP_GAP), ("w2".into(), 1)]);
        // The warp group emptying collapses the gap; the desktop block moves up.
        m.apply(vec![desk], t0);
        m.tick(FADE_SECS + 0.1, t0 + secs(1.0));
        assert_eq!(m.chips[0].slot, 0);
        assert_eq!(m.groups, vec!["desktop".to_string()]);
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
    fn a_new_placement_key_starts_a_fade_out_then_in() {
        let t0 = Instant::now();
        let mut a = Anim::new(t0);
        a.track_placement(1, 100.0, t0);
        assert!(a.reloc.is_none(), "first placement never animates");
        a.track_placement(1, 120.0, t0 + secs(0.1));
        assert!(a.reloc.is_none(), "sliding within the same spot is not a relocation");
        a.track_placement(2, 400.0, t0 + secs(0.2));
        let r = a.reloc.expect("key change relocates");
        assert_eq!(r.from_s, 120.0);
        let (old, alpha) = r.stage(t0 + secs(0.2));
        assert!(old && (alpha - 1.0).abs() < 1e-6);
        let (old, _) = r.stage(t0 + secs(0.2 + RELOC_HALF_SECS + 0.01));
        assert!(!old, "second half draws at the new spot");
        a.track_placement(2, 400.0, t0 + secs(2.0));
        assert!(a.reloc.is_none(), "finished relocations are cleared");
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
