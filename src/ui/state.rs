//! The board's state, and the handle the views reach it through (#195).
//!
//! Under gpui the board was a view: every change called `cx.notify()` and the
//! whole window was built again from it. Native Shell builds a view once and
//! binds what changes, so the state lives here, behind [`Ui`], and every
//! change bumps one revision signal; the closures that read the board read
//! that signal first, and only they run again.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use nsh_core::prelude::*;

use crate::host;
use crate::model::{BoardModel, Target};
use crate::settings::Settings;
use crate::zoom::{self, Layout};
use crate::sources::{ClaudeSource, FakeSource, SessionSource};
use crate::theme::Palette;
use crate::ui::palette::PaletteState;
use crate::usage::{self, Usage};

pub const STATUS_H: f32 = 30.0;
/// The strip along the top that drags the window, as a titlebar would: the
/// height of the macOS titlebar the traffic lights sit in (#171, #185).
pub const GRAB_H: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Real Claude Code sessions from `~/.claude/sessions`.
    Live,
    /// Synthetic sessions edited from the test tools.
    Sandbox,
}

#[derive(Default)]
pub struct PanelState {
    pub open: bool,
}

/// A tile's own little menu (#194): where the pointer opened it, and the
/// pull request it is about.
#[derive(Clone)]
pub struct TileMenu {
    pub at: (f32, f32),
    pub url: String,
    pub label: String,
}

pub struct Board {
    pub model: BoardModel,
    pub live: Arc<ClaudeSource>,
    pub sandbox: Arc<FakeSource>,
    pub mode: Mode,
    pub settings: Settings,
    pub dev: PanelState,
    /// The sandbox sessions whose detail rows the test panel shows (#180).
    pub dev_expanded: std::collections::HashSet<String>,
    pub settings_panel: PanelState,
    /// The list down the right-hand side (#103).
    pub list_panel: PanelState,
    pub layout: Layout,
    /// The tile whose little menu is open (#194).
    pub tile_menu: Option<TileMenu>,
    /// Bumped after a tile's press so the start screen turns back in (#176).
    pub tiles_replay: usize,
    pub last_tick: Instant,
    /// Claude plan usage for the status bar (#48).
    pub usage: Option<(Usage, Instant)>,
    pub usage_error: Option<usage::UsageError>,
    /// What the live source wants said in the status bar (#67).
    pub source_note: Option<&'static str>,
    /// The window in points.
    pub window: (f32, f32),
    /// The ⌘K search over everything on the board (#71).
    pub palette: PaletteState,
    /// Bumped whenever the board is read in hand (`Ui::sync_sandbox`): a
    /// poll that started before then is holding an older snapshot, and
    /// laying it down would undo what was just done.
    pub generation: u64,
}

impl Board {
    pub fn new() -> Self {
        let settings = Settings::load();
        Self {
            model: BoardModel::new(),
            live: Arc::new(ClaudeSource::default()),
            sandbox: Arc::new(FakeSource::new()),
            mode: Mode::Live,
            layout: Layout::new(zoom::DESIGN_W, zoom::DESIGN_H, settings.zoom),
            settings,
            dev: PanelState::default(),
            dev_expanded: Default::default(),
            settings_panel: PanelState::default(),
            list_panel: PanelState::default(),
            tile_menu: None,
            tiles_replay: 0,
            last_tick: Instant::now(),
            usage: None,
            usage_error: None,
            source_note: None,
            window: (zoom::DESIGN_W, zoom::DESIGN_H),
            palette: PaletteState::default(),
            generation: 0,
        }
    }

    /// Plan usage for the status bar, and whether the reading is stale (#48).
    pub fn usage_now(&self, now: Instant) -> Option<(Usage, bool)> {
        let (u, at) = self.usage?;
        Some((u, now.duration_since(at) > usage::STALE_AFTER))
    }

    pub fn source(&self) -> Arc<dyn SessionSource> {
        match self.mode {
            Mode::Live => self.live.clone(),
            Mode::Sandbox => self.sandbox.clone(),
        }
    }

    pub fn palette(&self) -> Palette {
        Palette::for_theme(self.settings.theme)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            let msg = format!("source: {}", self.source().name());
            self.model.set_notice(msg, Instant::now());
        }
    }

    /// Multiplies the zoom by `factor` (or resets it when `factor` is 0).
    pub fn zoom_by(&mut self, factor: f32) {
        let next = if factor == 0.0 { 1.0 } else { self.settings.zoom * factor };
        self.set_zoom(next);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(zoom::USER_ZOOM_MIN, zoom::USER_ZOOM_MAX);
        if (zoom - self.settings.zoom).abs() > 1e-4 {
            self.settings.zoom = zoom;
            self.settings.save();
        }
        self.layout = Layout::new(self.window.0, self.window.1, self.settings.zoom);
        self.model.set_notice(format!("zoom {:.0}%", zoom * 100.0), Instant::now());
    }

    /// The window changed size.
    pub fn resize(&mut self, w: f32, h: f32) {
        self.window = (w, h);
        self.layout = Layout::new(w, h, self.settings.zoom);
    }

    /// Ages the model: arrivals settle, departures leave.
    pub fn frame(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        self.model.tick(dt, now);
    }

    /// A snapshot of the sessions, laid on the board: what a poll does with
    /// what it read.
    pub fn take(&mut self, mut list: Vec<crate::model::SessionInfo>, note: Option<&'static str>) {
        let now = Instant::now();
        self.sandbox.churn(now);
        self.drop_dismissed(&mut list);
        self.model.apply(list, now);
        self.source_note = note;
        crate::ui::palette::refresh(self);
        self.frame();
    }

    /// What a poll read, laid on the board unless the board was read in hand
    /// since the poll set out (`generation`): then it is the older snapshot,
    /// and laying it down would undo what was just done (#197).
    pub fn take_polled(&mut self, generation: u64, list: Vec<crate::model::SessionInfo>, note: Option<&'static str>) -> bool {
        if self.generation != generation {
            return false;
        }
        self.take(list, note);
        true
    }

    /// Pull requests put away by hand never reach the board again (#194).
    pub fn drop_dismissed(&self, list: &mut [crate::model::SessionInfo]) {
        crate::ui::tiles::without_dismissed(list, &self.settings.dismissed);
    }
}

/// The board, shared by every view, and its revision.
#[derive(Clone)]
pub struct Ui {
    board: Rc<RefCell<Board>>,
    /// Bumped by every change to the board.
    pub rev: Signal<u64>,
}

impl Ui {
    pub fn new(board: Board) -> Self {
        Self { board: Rc::new(RefCell::new(board)), rev: signal(0) }
    }

    /// Reads the board, tracked: a binding that reads it runs again on the
    /// next change.
    pub fn read<R>(&self, f: impl FnOnce(&Board) -> R) -> R {
        self.rev.get();
        f(&self.board.borrow())
    }

    /// What `f` reads off the board, as a memo: it notifies only when that
    /// changes, not on every change to the board. For anything that moves
    /// when its value changes (a flyout, a radio button, a slider), which a
    /// plain read would set moving on every poll.
    pub fn memo<T: PartialEq + 'static>(&self, f: impl Fn(&Board) -> T + 'static) -> Memo<T> {
        let ui = self.clone();
        computed(move || ui.read(&f))
    }

    /// Reads the board without subscribing.
    pub fn peek<R>(&self, f: impl FnOnce(&Board) -> R) -> R {
        f(&self.board.borrow())
    }

    /// Changes the board, then lets every binding that reads it know. The
    /// borrow ends before the signal fires, so a binding can read the board.
    pub fn update<R>(&self, f: impl FnOnce(&mut Board) -> R) -> R {
        let r = f(&mut self.board.borrow_mut());
        self.rev.update(|n| *n = n.wrapping_add(1));
        r
    }

    /// Changes the board quietly: for what no binding shows.
    pub fn quietly<R>(&self, f: impl FnOnce(&mut Board) -> R) -> R {
        f(&mut self.board.borrow_mut())
    }

    /// Reads the sandbox now rather than at the next poll, so what the test
    /// tools just did is on the board — and on their own buttons — at once.
    /// The sandbox is in memory; reading it costs nothing.
    pub fn sync_sandbox(&self) {
        let sandbox = self.peek(|b| (b.mode == Mode::Sandbox).then(|| b.sandbox.clone()));
        match sandbox {
            Some(sb) => {
                let (list, note) = (sb.snapshot(), sb.note());
                self.update(|b| {
                    b.generation += 1;
                    b.take(list, note)
                });
            }
            // Out of the sandbox: whatever a poll is holding was read from
            // the source just left.
            None => self.update(|b| b.generation += 1),
        }
    }

    /// A line in the status bar.
    pub fn notice(&self, msg: String) {
        self.update(|b| b.model.set_notice(msg, Instant::now()));
    }

    /// Opens a link node, reporting what happened in the status bar the way a
    /// chip's Warp jump does (#57). `first` is tried before `then` (#60).
    pub fn open_link(&self, label: String, first: Option<String>, then: Option<String>, what: &'static str) {
        let Some(url) = first.or(then.clone()) else {
            self.notice(format!("{label}: no {what} to open"));
            return;
        };
        let fallback = then.filter(|f| *f != url);
        let ui = self.clone();
        spawn(async move {
            let opened = background(async move {
                match crate::pr::open(&url) {
                    Ok(true) => (Ok(true), url),
                    other => match fallback {
                        Some(web) => (crate::pr::open(&web), web),
                        None => (other, url),
                    },
                }
            })
            .await;
            let msg = match opened {
                Ok((Ok(true), url)) => format!("→ {label} ({url})"),
                Ok((Ok(false), url)) => format!("{label}: `open` refused {url}"),
                Ok((Err(e), _)) => format!("{label}: {e}"),
                Err(e) => format!("{label}: {e}"),
            };
            ui.notice(msg);
        })
        .detach_quietly();
    }

    /// Jumps to the session's terminal. Subagent chips jump to their parent
    /// session.
    pub fn activate(&self, target: Target) {
        let Some(info) = self.peek(|b| b.model.chips.get(target.session()).map(|c| c.info.clone())) else {
            return;
        };
        let label = info.label();
        if info.synthetic {
            self.notice(format!("sandbox: {label} has no terminal to jump to"));
            return;
        }
        let ui = self.clone();
        spawn(async move {
            let result = background(async move { host::focus(&info) }).await;
            let msg = match result {
                Ok(Ok(host::Outcome::Focused(url))) => format!("→ {label} ({url})"),
                Ok(Ok(host::Outcome::ActivatedApp(app))) => format!("{label}: no Warp session URL, activated {app}"),
                Ok(Err(e)) => format!("{label}: {e}"),
                Err(e) => format!("{label}: {e}"),
            };
            ui.notice(msg);
        })
        .detach_quietly();
    }

    /// Jumps to a session by its id, wherever it sits on the board now.
    pub fn activate_session(&self, session_id: &str) {
        if let Some(at) = self.peek(|b| b.model.chips.iter().position(|c| c.info.session_id == session_id)) {
            self.activate(Target::Session(at));
        }
    }

    /// Opens the pane the `NO WARP TABS` tag asks for (#70).
    pub fn open_full_disk_access(&self) {
        self.open_link("FULL DISK ACCESS".into(), Some(PRIVACY_ALL_FILES.into()), None, "settings pane");
    }
}

/// System Settings → Privacy & Security → Full Disk Access.
const PRIVACY_ALL_FILES: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// A task left to run to its end.
trait DetachQuietly {
    fn detach_quietly(self);
}

impl<T> DetachQuietly for Spawned<T> {
    fn detach_quietly(self) {
        drop(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Phase, SessionInfo};

    /// A test-tools button reads the sandbox in hand and bumps the
    /// generation; a poll that set out before then holds the older snapshot
    /// and is dropped, or it would undo what was just done (#197).
    #[test]
    fn a_poll_that_set_out_before_a_change_in_hand_is_dropped() {
        let mut b = Board::new();
        let s = SessionInfo::synthetic(1, "S-1", Phase::Working);
        assert!(b.take_polled(0, vec![s.clone()], None), "a poll of the same generation lands");
        assert_eq!(b.model.live().count(), 1);

        b.generation += 1;
        assert!(!b.take_polled(0, Vec::new(), None), "an older one does not");
        assert_eq!(b.model.live().count(), 1, "and leaves the board as it was");

        let mut idle = s;
        idle.status = "idle".into();
        assert!(b.take_polled(1, vec![idle], None), "the next one lands again");
    }
}
