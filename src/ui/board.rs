//! The interactive board: owns the model + sources, handles input, and hands
//! each frame to the renderer.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollWheelEvent, SharedString, Stateful, StyleRefinement, WeakEntity, Window, canvas,
    div, prelude::*, px,
};

use crate::geom::Pt;
use crate::model::{BoardModel, Target};
use crate::render::paint::PathCache;
use crate::render::scene::{self, GAP, Layout};

use crate::made;
use crate::render::comb;
use crate::settings::{BoardView, Settings};
use crate::sources::{ClaudeSource, FakeSource, SessionSource};
use crate::theme::{self, Palette};
use crate::ui::palette::PaletteState;
use crate::ui::panel::PanelState;
use crate::usage::{self, Usage};
use crate::host;

pub const STATUS_H: f32 = 30.0;
/// The strip along the top that drags the window, as a titlebar would: the
/// height of the macOS titlebar the traffic lights sit in (#171), which
/// AppKit drags from itself (#185) and which runs to 32 px on macOS 27.
const GRAB_H: f32 = 32.0;
/// Session registry poll interval.
const POLL_MS: u64 = 1000;
/// Redraw cadence while the honeycomb moves (cells arriving and gliding to
/// new seats): 30fps, which halves the CPU of a vsync-driven loop (#38).
const FRAME_MS: u64 = 33;
/// And while nothing is. A floor rather than a stop: anything the predicate
/// below fails to notice is a quarter second late instead of frozen, which
/// is the difference between a slow board and a broken one (#126).
const IDLE_MS: u64 = 250;
/// How far a press has to travel before it drags the window, in window px.
const DRAG_SLOP: f32 = 4.0;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Real Claude Code sessions from `~/.claude/sessions`.
    Live,
    /// Synthetic sessions edited from the test tools.
    Sandbox,
}

pub struct Board {
    pub(crate) model: BoardModel,
    pub(crate) live: Arc<ClaudeSource>,
    pub(crate) sandbox: Arc<FakeSource>,
    pub(crate) mode: Mode,
    pub(crate) settings: Settings,
    pub(crate) dev: PanelState,
    /// The start screen and the panels, each a view of its own (#181). The
    /// panels are cached: drawn from the last frame until `touch` says what
    /// they show has changed, so the frame clock and the pointer redraw the
    /// board, not them. The start screen is not (#182, see `render`).
    tiles_view: Entity<TilesView>,
    panels_view: Entity<PanelsView>,
    /// The sandbox sessions whose detail rows the test panel shows (#180).
    pub(crate) dev_expanded: std::collections::HashSet<String>,
    pub(crate) settings_panel: PanelState,
    /// The list down the right-hand side (#103).
    pub(crate) list_panel: PanelState,
    pub(crate) layout: Layout,
    paths: Rc<RefCell<PathCache>>,
    pub(crate) scroll_y: f32,
    /// Mouse in design units / window pixels.
    mouse: Option<Pt>,
    pressed: Option<Target>,
    /// The cell whose pull requests are dropped down, by session id (#134).
    /// The id rather than the index: sessions come and go while the list is
    /// open, and an index would quietly become somebody else's.
    pub(crate) pr_menu: Option<String>,
    /// A press on bare board that has not moved yet: the first drag hands the
    /// window to AppKit's own move loop.
    drag_pending: bool,
    /// Where the press that may become a drag went down, in window px.
    press_at: Option<gpui::Point<Pixels>>,
    /// Bumped after a tile's press so the start screen turns back in (#176).
    pub(crate) tiles_replay: usize,
    last_tick: Instant,
    focus_handle: FocusHandle,
    /// Claude plan usage for the status bar (#48): the last good value and
    /// when it was fetched, plus the latest error if the poll is failing.
    usage: Option<(Usage, Instant)>,
    usage_error: Option<usage::UsageError>,
    /// What the live source wants said in the status bar (#67).
    source_note: Option<&'static str>,
    /// How far down the honeycomb reaches, so the scroll can get there.
    content_bottom: f32,
    /// The window in pixels, for the chrome that has to fit inside it (#83).
    pub(crate) window: (f32, f32),
    /// The hex grid, when the comb view is the one showing (#86).
    pub(crate) comb: Option<comb::Comb>,
    /// The cells still on their way to seats they were given this frame
    /// (#150).
    slides: comb::Slides,
    /// When the comb's first frame was painted, which is what its cells
    /// arrive on. Started there rather than when the view was chosen: a
    /// window that opens straight into the comb spends its first moments on
    /// fonts and the first session poll, and the whole arrival would be over
    /// before anything reached the screen (#92, #93).
    pub(crate) comb_since: Option<Instant>,
    /// The ⌘K search over everything on the board (#71).
    pub(crate) palette: PaletteState,
}

impl Board {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        cx.spawn(async move |this, cx| {
            loop {
                let Ok(source) = this.update(cx, |board, _| board.source()) else { break };
                let (list, note) =
                    cx.background_executor().spawn(async move { (source.snapshot(), source.note()) }).await;
                let now = Instant::now();
                let ok = this.update(cx, |board, cx| {
                    board.sandbox.churn(now);
                    board.model.apply(list, now);
                    board.source_note = note;
                    board.palette_refresh();
                    board.touch(cx);
                });
                if ok.is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(POLL_MS)).await;
            }
        })
        .detach();

        // Frame clock: repaint on a fixed cadence instead of every vsync, and
        // slow right down when nothing on screen is moving (#126). A board
        // that sits still was redrawing itself thirty times a second —
        // rebuilding every shape, tessellating them and handing Metal a frame
        // — for no change at all, which was most of what this app cost.
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(ms) = this.update(cx, |board, cx| {
                    cx.notify();
                    if board.moving() { FRAME_MS } else { IDLE_MS }
                }) else {
                    break;
                };
                cx.background_executor().timer(Duration::from_millis(ms)).await;
            }
        })
        .detach();

        // Plan usage for the status bar (#48): keychain + curl off the UI
        // thread, once a minute. A missing login is final; anything else
        // keeps the last good value and retries.
        cx.spawn(async move |this, cx| {
            loop {
                let result = cx.background_executor().spawn(async move { usage::fetch() }).await;
                let now = Instant::now();
                let keep_polling = this.update(cx, |board, cx| {
                    let again = !matches!(result, Err(usage::UsageError::NoCredentials));
                    match result {
                        Ok(u) => {
                            board.usage = Some((u, now));
                            board.usage_error = None;
                        }
                        Err(e) => board.usage_error = Some(e),
                    }
                    cx.notify();
                    again
                });
                if !matches!(keep_polling, Ok(true)) {
                    break;
                }
                cx.background_executor().timer(usage::POLL).await;
            }
        })
        .detach();

        let settings = Settings::load();
        let now = Instant::now();
        let me = cx.entity().downgrade();
        Self {
            model: BoardModel::new(),
            live: Arc::new(ClaudeSource::default()),
            sandbox: Arc::new(FakeSource::new()),
            mode: Mode::Live,
            layout: Layout::new(scene::DESIGN_W, scene::DESIGN_H, settings.zoom),
            settings,
            dev: PanelState::default(),
            tiles_view: cx.new(|_| TilesView { board: me.clone() }),
            panels_view: cx.new(|_| PanelsView { board: me.clone() }),
            dev_expanded: Default::default(),
            settings_panel: PanelState::default(),
            list_panel: PanelState::default(),
            paths: Rc::new(RefCell::new(PathCache::default())),
            scroll_y: 0.0,
            mouse: None,
            pressed: None,
            pr_menu: None,
            drag_pending: false,
            press_at: None,
            tiles_replay: 0,
            last_tick: now,
            focus_handle,
            usage: None,
            usage_error: None,
            source_note: None,
            content_bottom: 0.0,
            window: (scene::DESIGN_W, scene::DESIGN_H),
            comb: None,
            slides: comb::Slides::default(),
            comb_since: None,
            palette: PaletteState::default(),
        }
    }

    /// Plan usage for the status bar, and whether the reading is stale (#48).
    /// Nothing until the first successful fetch.
    fn usage_now(&self, now: Instant) -> Option<(Usage, bool)> {
        let (u, at) = self.usage?;
        Some((u, now.duration_since(at) > usage::STALE_AFTER))
    }

    fn source(&self) -> Arc<dyn SessionSource> {
        match self.mode {
            Mode::Live => self.live.clone(),
            Mode::Sandbox => self.sandbox.clone(),
        }
    }

    pub(crate) fn palette(&self) -> Palette {
        Palette::for_theme(self.settings.theme)
    }

    pub(crate) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            let msg = format!("source: {}", self.source().name());
            self.model.set_notice(msg, Instant::now());
            self.touch(cx);
        }
    }

    /// Multiplies the zoom by `factor` (or resets it when `factor` is 0).
    pub(crate) fn zoom_by(&mut self, factor: f32, cx: &mut Context<Self>) {
        let next = if factor == 0.0 { 1.0 } else { self.settings.zoom * factor };
        self.set_zoom(next, cx);
    }

    pub(crate) fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(scene::USER_ZOOM_MIN, scene::USER_ZOOM_MAX);
        if (zoom - self.settings.zoom).abs() > 1e-4 {
            self.settings.zoom = zoom;
            self.settings.save();
        }
        self.model.set_notice(format!("zoom {:.0}%", zoom * 100.0), Instant::now());
        self.touch(cx);
    }

    /// How far down the board can be pushed.
    ///
    /// The status bar is drawn over the board's bottom edge, so what can be
    /// seen is shorter than the window by that much, and scrolling has to run
    /// that much further or the lowest lane never comes out from under it.
    /// The bar is a fixed number of window pixels, so in design units it
    /// grows as the board is zoomed out: past a zoom of about 0.88 it is
    /// taller than `BOTTOM_PAD` and the last lane's connectors were being
    /// clipped (#81).
    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll(&self.layout, self.content_bottom));
    }

    fn to_design(&self, p: gpui::Point<Pixels>) -> Pt {
        Pt::new(f32::from(p.x), f32::from(p.y)) * (1.0 / self.layout.zoom)
    }

    /// Whether a window point is on the window's chrome rather than the
    /// board: the status bar along the bottom or the titlebar strip along the
    /// top. The board's canvas runs under both, so a cell scrolled there would
    /// otherwise take the press (and the hover) meant for them, and neither
    /// would drag the window (#170, #171).
    fn on_chrome(p: gpui::Point<Pixels>, window: &Window) -> bool {
        let y = f32::from(p.y);
        y < GRAB_H || y >= f32::from(window.viewport_size().height) - STATUS_H
    }

    /// The cell under a board point — none at all when the pointer is on
    /// the chrome.
    fn hover_at(&mut self, m: Pt, bar: bool) {
        self.model.hovered = if bar { None } else { self.hit_test(m) };
    }

    /// The cell under a point in design units. The comb's cells live in
    /// content coordinates, so the pointer is pushed back down by however far
    /// the grid is scrolled (#86).
    fn hit_test(&self, m: Pt) -> Option<Target> {
        let c = self.comb.as_ref()?;
        comb::hit(c, m + Pt::new(0.0, self.scroll_y)).map(Target::Session)
    }

    /// The session whose output row is under the pointer, when that row has
    /// more than one thing to show (#134, #137).
    ///
    /// Only in the comb. On the board every pull request is already a node of
    /// its own, so there is no one place a list could drop from.
    fn pr_menu_at(&self, m: Pt) -> Option<String> {
        let comb = self.comb.as_ref()?;
        let (i, slot) = comb::hit_slot(comb, m + Pt::new(0.0, self.scroll_y))?;
        if slot != comb::Slot::Output {
            return None;
        }
        let info = &self.model.chips.get(i)?.info;
        matches!(made::press(info), made::Press::List).then(|| info.session_id.clone())
    }

    /// The list hangs off a cell, so it goes when the cell does: the comb put
    /// away, or the session ended (#134).
    fn prune_pr_menu(&mut self) {
        let Some(id) = self.pr_menu.as_deref() else { return };
        let standing = self.comb.is_some()
            && self.model.chips.iter().any(|c| c.info.session_id == id && !c.anim.gone);
        if !standing {
            self.pr_menu = None;
        }
    }

    /// Opens a link node, reporting what happened in the status bar the way a
    /// chip's Warp jump does (#57). `first` is tried before `then`, so a
    /// Linear node can prefer its desktop app and still land somewhere if the
    /// scheme is not registered (#60).
    pub(crate) fn open_link(
        &mut self,
        label: String,
        first: Option<String>,
        then: Option<String>,
        what: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(url) = first.or(then.clone()) else {
            self.model.set_notice(format!("{label}: no {what} to open"), Instant::now());
            cx.notify();
            return;
        };
        let fallback = then.filter(|f| *f != url);
        cx.spawn(async move |this, cx| {
            let opened = cx.background_executor().spawn({
                let url = url.clone();
                async move {
                    match crate::pr::open(&url) {
                        Ok(true) => (Ok(true), url),
                        other => match fallback {
                            Some(web) => (crate::pr::open(&web), web),
                            None => (other, url),
                        },
                    }
                }
            });
            let (result, url) = opened.await;
            let msg = match result {
                Ok(true) => format!("→ {label} ({url})"),
                Ok(false) => format!("{label}: `open` refused {url}"),
                Err(e) => format!("{label}: {e}"),
            };
            this.update(cx, |board, cx| {
                board.model.set_notice(msg, Instant::now());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Something on screen is *animating*, so the next frame will differ from
    /// this one without anybody doing anything (#126).
    ///
    /// Only animation. Everything else that changes the board — a poll
    /// answering, a key, the mouse — calls `cx.notify()` where it happens,
    /// and gpui repaints then whatever this says. The clock is here for the
    /// things nothing notifies about: the arrival wave, the blink, and a cell
    /// gliding to a new seat.
    fn moving(&self) -> bool {
        // The start screen is gpui-metro's, and asks for its own frames while
        // a tile turns or slides (#176).
        if self.settings.view == BoardView::Tiles {
            return false;
        }
        let now = Instant::now();
        if self.model.moving(now) || self.model.blinking() {
            return true;
        }
        // A cell on its way to a new seat (#150).
        if self.slides.moving() {
            return true;
        }
        // The comb is still once its cells have all arrived.
        match (&self.comb, self.comb_since) {
            (Some(comb), Some(since)) => !comb::wave_over(comb, now.duration_since(since).as_secs_f32()),
            _ => false,
        }
    }

    /// Opens the pane the `NO WARP TABS` tag asks for (#70). That tag is the
    /// only place the board can say the permission is missing, so it is also
    /// the way there.
    fn open_full_disk_access(&mut self, cx: &mut Context<Self>) {
        self.open_link("FULL DISK ACCESS".into(), Some(PRIVACY_ALL_FILES.into()), None, "settings pane", cx);
    }

    /// A click on the board, which in the comb view can land on one of a
    /// cell's three rows rather than on the cell as a whole (#89).
    fn activate_at(&mut self, target: Target, m: Pt, cx: &mut Context<Self>) {
        let Some(comb) = &self.comb else { return self.activate(target, cx) };
        let Some((i, slot)) = comb::hit_slot(comb, m + Pt::new(0.0, self.scroll_y)) else {
            return self.activate(target, cx);
        };
        let Some(info) = self.model.chips.get(i).map(|c| c.info.clone()) else { return };
        match slot {
            comb::Slot::Ticket => match &info.ticket {
                Some(t) => self.open_link(t.label(), t.app_url(), t.url(), "Linear issue", cx),
                // A row with nothing in it is still part of the cell.
                None => self.activate(Target::Session(i), cx),
            },
            comb::Slot::Output => match made::press(&info) {
                made::Press::Cell => self.activate(Target::Session(i), cx),
                made::Press::One { name, kind, url } => self.open_link(name, None, url, kind, cx),
                // The list dropped down on the press already, and the
                // release is nothing more (#134).
                made::Press::List => {}
            },
            comb::Slot::Session => self.activate(Target::Session(i), cx),
        }
    }

    /// Jumps to the session's terminal (chip click / test panel). Subagent
    /// chips jump to their parent session.
    pub(crate) fn activate(&mut self, target: Target, cx: &mut Context<Self>) {
        let Some(chip) = self.model.chips.get(target.session()) else { return };
        let info = chip.info.clone();
        let label = info.label();
        if info.synthetic {
            self.model.set_notice(format!("sandbox: {label} has no terminal to jump to"), Instant::now());
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { host::focus(&info) }).await;
            let msg = match result {
                Ok(host::Outcome::Focused(url)) => format!("→ {label} ({url})"),
                Ok(host::Outcome::ActivatedApp(app)) => {
                    format!("{label}: no Warp session URL, activated {app}")
                }
                Err(e) => format!("{label}: {e}"),
            };
            this.update(cx, |board, cx| {
                board.model.set_notice(msg, Instant::now());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for Board {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        let vp = window.viewport_size();
        let (w, h) = (f32::from(vp.width), f32::from(vp.height));
        self.window = (w, h);

        self.model.tick(dt, now);
        self.layout = Layout::new(w, h, self.settings.zoom);
        let board_mouse = self.mouse.filter(|_| self.settings.view != BoardView::Tiles);
        self.model.hovered = board_mouse.and_then(|m| self.hit_test(m));
        // The honeycomb lays the sessions out on a hex grid and scrolls by
        // its height (#86).
        if self.settings.view.seated() {
            // The clock starts on the first frame with something to spread
            // out from. At launch the first session poll has not answered
            // yet, and a wave out of an empty grid is every cell at once:
            // by the time the sessions land it has already run (#93, #101).
            if self.model.chips.iter().all(|c| c.anim.gone) {
                self.comb_since = None;
            } else if self.comb_since.is_none() {
                self.comb_since = Some(now);
            }
            // The visible height, not the window's: the status bar is drawn
            // over the board's bottom edge, and the middle of the screen is
            // the middle of what is left (#138).
            let seen = visible_height(&self.layout);
            let mut comb = comb::lay_out(&self.model, self.layout.width, seen);
            // A seat the packing has just handed over is somewhere to go,
            // not somewhere to be: the cells that changed seats are moved
            // back to where they were and let glide (#150). Before anything
            // reads the seating, so the press and the walk are on the cell
            // that is drawn.
            self.slides.settle(&mut comb, &self.model, now);
            self.comb = Some(comb);
        } else {
            self.comb = None;
            self.comb_since = None;
        }
        self.prune_pr_menu();
        self.content_bottom = self.comb.as_ref().map_or(0.0, |c| c.height);
        self.clamp_scroll();

        let palette = self.palette();
        let paths = self.paths.clone();
        let comb = self.comb.clone();
        // Seconds since the view opened: the cells spread out from the
        // clusters on this (#92).
        let comb_t = self.comb_since.map_or(0.0, |at| now.duration_since(at).as_secs_f32());
        let tiles_view = self.settings.view == BoardView::Tiles;
        let model = (comb.is_some()).then(|| self.model.clone());
        let scroll_y = self.scroll_y;
        let zoom = self.layout.zoom;

        let ink = theme::hsla(palette.ink);
        // The status bar stays on the board's ground and in its ink, so it
        // reads on all three boards; Metro gives it the type, the square
        // controls and the accent that fills whatever is switched on. On
        // the orange board an orange accent would vanish, so ink fills there.
        let metro = gpui_metro::Theme::palette(cx);
        let ui_font = gpui_metro::Theme::font(cx);
        let on_orange = self.settings.theme == crate::settings::BoardTheme::Orange;
        let fill = if on_orange { ink } else { metro.accent };
        let fill_ink = if on_orange { theme::hsla(palette.bg) } else { metro.accent_ink };
        let status_colors = (ink, fill, fill_ink);
        let summary = self.model.summary();
        let detail = self.model.detail();
        let mode_tag = match self.mode {
            Mode::Live => None,
            Mode::Sandbox => Some("SANDBOX"),
        };
        let source_note = self.source_note;
        let open_fda = cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.open_full_disk_access(cx));
        let dev_open = self.dev.open;
        let settings_open = self.settings_panel.open;
        let usage = self.usage_now(now);

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .when(self.model.hovered.is_some(), |d| d.cursor_pointer())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let cmd = ev.keystroke.modifiers.platform;
                // ⌘K opens and closes it; while it is open every other key
                // is typing, not a shortcut (#71).
                if cmd && ev.keystroke.key == "k" {
                    this.toggle_palette();
                    // The search is drawn with the panels, in a view of its
                    // own that only repaints when it is told to (#181, #193).
                    this.touch(cx);
                    return;
                }
                if this.palette.open {
                    this.palette_key(ev, cx);
                    this.touch(cx);
                    return;
                }
                match ev.keystroke.key.as_str() {
                    "l" if cmd => this.toggle_list(),
                    "escape" if this.pr_menu.is_some() => this.pr_menu = None,
                    "escape" if this.list_panel.open => this.list_panel.open = false,
                    "escape" if this.settings_panel.open => this.settings_panel.open = false,
                    "escape" if this.dev.open => this.dev.open = false,
                    "escape" => cx.quit(),
                    "t" => this.toggle_devtools(),
                    "," if cmd => this.toggle_settings(),
                    "=" | "+" => this.zoom_by(scene::USER_ZOOM_STEP, cx),
                    "-" | "_" => this.zoom_by(1.0 / scene::USER_ZOOM_STEP, cx),
                    "0" if cmd => this.zoom_by(0.0, cx),
                    _ => return,
                }
                this.touch(cx);
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                let m = this.to_design(ev.position);
                this.mouse = Some(m);
                // The start screen's tiles are elements of their own, which
                // take their own hover and press (#176).
                let off_board = Self::on_chrome(ev.position, window) || this.settings.view == BoardView::Tiles;
                let before = this.model.hovered;
                this.hover_at(m, off_board);
                let changed = this.model.hovered != before;
                // The window server moves the window itself, in step with the
                // cursor; moving it by hand from deltas lagged and jumped.
                // Only once the pointer has really moved: a press with a pixel
                // of wobble in it is a click, and on the start screen the
                // click is a tile's (#176).
                let travelled = this.press_at.is_some_and(|at| {
                    let (dx, dy) = (f32::from(ev.position.x - at.x), f32::from(ev.position.y - at.y));
                    dx * dx + dy * dy >= DRAG_SLOP * DRAG_SLOP
                });
                if this.drag_pending && travelled && ev.pressed_button == Some(MouseButton::Left) {
                    this.drag_pending = false;
                    window.start_window_move();
                }
                // Only when what is under the pointer changed: the pointer is
                // read nowhere else, and a redraw for every pixel it moved
                // rebuilt the whole window at the display's rate — half a core
                // with sixty sessions in the sandbox (#180).
                if changed {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    let m = this.to_design(ev.position);
                    let bar = Self::on_chrome(ev.position, window) || this.settings.view == BoardView::Tiles;
                    this.press_at = Some(ev.position);
                    this.mouse = Some(m);
                    // A cell with more than one pull request drops its list
                    // down on the press, the way a menu opens; the same row
                    // again puts it away, and so does a press anywhere else
                    // that reaches the board at all (#134).
                    let asked = if bar { None } else { this.pr_menu_at(m) };
                    this.pr_menu = match (asked, this.pr_menu.take()) {
                        (Some(id), Some(open)) if id == open => None,
                        (asked, _) => asked,
                    };
                    this.hover_at(m, bar);
                    this.pressed = this.model.hovered;
                    // AppKit moves the window from the titlebar strip itself
                    // (#185); a second move started from here would fight it.
                    this.drag_pending = this.pressed.is_none() && f32::from(ev.position.y) >= GRAB_H;
                    // A click on the board is a click past the palette: the
                    // card itself stops propagation, so anything reaching here
                    // was outside it (#71).
                    if this.palette.open {
                        this.palette.open = false;
                        this.palette.results.clear();
                        this.panels_view.update(cx, |_, cx| cx.notify());
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                    let m = this.to_design(ev.position);
                    let hit = this.hit_test(m);
                    if let (Some(p), Some(h)) = (this.pressed, hit) {
                        if p == h {
                            this.activate_at(h, m, cx);
                        }
                    }
                    this.pressed = None;
                    this.drag_pending = false;
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let delta = ev.delta.pixel_delta(px(GAP));
                if ev.modifiers.platform {
                    // ⌘ + wheel zooms, like a browser.
                    let factor = (1.0 - f32::from(delta.y) / 300.0).clamp(0.8, 1.25);
                    this.zoom_by(factor, cx);
                    return;
                }
                // The start screen scrolls itself (#176).
                if this.settings.view == BoardView::Tiles {
                    return;
                }
                this.scroll_y -= f32::from(delta.y) / this.layout.zoom;
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let origin = Pt::new(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        // The start screen is its own element over this (#176):
                        // the canvas draws only the honeycomb.
                        let shapes = match (&comb, &model) {
                            (Some(c), Some(m)) => {
                                let at = origin * (1.0 / zoom);
                                let mut out = comb::build_shapes(m, c, palette, scroll_y, at, comb_t);
                                for s in &mut out {
                                    s.scale(zoom);
                                }
                                out
                            }
                            _ => Vec::new(),
                        };
                        paths.borrow_mut().paint(&shapes, window);
                    },
                )
                .size_full(),
            )
            // Its own view, but not a cached one (#182): a live tile asks for
            // frames while it slides, which redraws every view above it, and
            // gpui redraws everything under a cached view it redraws — the
            // tiles' own cached bodies included. Uncached, a slide redraws
            // the one tile and puts the page together around the rest.
            .when(tiles_view, |d| d.child(div().absolute().top_0().left_0().right_0().bottom(px(STATUS_H)).child(self.tiles_view.clone())))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(STATUS_H))
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .bg(theme::hsla(palette.bg))
                    .font_family(ui_font)
                    .text_size(px(crate::ui::metro::BODY))
                    .text_color(ink)
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(summary)
                            .when_some(mode_tag, |d, tag| {
                                d.child(
                                    div().px_1p5().bg(fill).text_color(fill_ink).child(tag),
                                )
                            })
                            // The board looks right when Warp's tabs are
                            // unreadable — every pane just becomes its own
                            // group — so the only way to tell is to say it
                            // (#67).
                            .when_some(source_note, move |d, note| {
                                d.child(
                                    div()
                                        .id("source-note")
                                        .px_1p5()
                                        .bg(theme::hsla(palette.alarm))
                                        .text_color(theme::hsla(palette.on_alarm))
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(theme::hsla(theme::with_alpha(palette.alarm, 0.75))))
                                        .child(note)
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                        .on_click(open_fda),
                                )
                            }),
                    )
                    .child(div().flex_1().overflow_hidden().text_right().child(detail))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .when_some(usage, |d, (u, stale)| {
                                let secs = usage::now_secs();
                                d.child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .child(usage_meter("5H", u.five_hour, secs, palette, fill, stale))
                                        .child(usage_meter("7D", u.seven_day, secs, palette, fill, stale)),
                                )
                            })
                            .child(status_button(
                                "settings-toggle",
                                "settings",
                                settings_open,
                                status_colors,
                                cx.listener(|this, _, _, cx| {
                                    this.toggle_settings();
                                    this.touch(cx);
                                }),
                            ))
                            .child(status_button(
                                "dev-toggle",
                                "test",
                                dev_open,
                                status_colors,
                                cx.listener(|this, _, _, cx| {
                                    this.toggle_devtools();
                                    this.touch(cx);
                                }),
                            )),
                    ),
            )
            // The panels are settings flyouts down the right edge (#175), over
            // everything but the status bar, whose buttons open and close
            // them. Only one is ever open (#163); each is drawn every frame
            // so the one just closed can slide away while the next comes in.
            .child(self.panels_view.clone().cached(below_bar()))
            // The connectors these hang on are the board's, not the comb's:
            // in the comb view they would sit over the honeycomb at
            // positions nothing is drawn at (#115, #134).
            .children(self.render_pr_menu(cx))
    }
}

/// How far down the board can be pushed.
///
/// The status bar is drawn over the board's bottom edge, so what can be seen
/// is shorter than the window by that much, and the scroll has to run that
/// much further or the lowest lane never comes out from under it. The bar is
/// a fixed number of window pixels, so in the board's own units it grows as
/// the board is zoomed out: past a zoom of about 0.88 it is taller than
/// `BOTTOM_PAD`, and the last lane's connectors were being clipped (#81).
fn max_scroll(layout: &Layout, bottom: f32) -> f32 {
    (bottom - visible_height(layout)).max(0.0)
}

/// Board height the user can actually see, in design units.
fn visible_height(layout: &Layout) -> f32 {
    (layout.height - STATUS_H / layout.zoom).max(1.0)
}

/// Small bordered button for the status bar.
/// One plan-usage window as a bar: `5H \u{25ac}\u{25ac}\u{2591}\u{2591} 22% \u{21bb}2H10M`. The fill is the
/// board's packet colour so it reads in every theme; a stale reading fades the
/// whole meter, bar included. (#52)
fn usage_meter(
    name: &'static str,
    w: usage::Window,
    now: u64,
    palette: Palette,
    fill: gpui::Hsla,
    stale: bool,
) -> gpui::Div {
    const TRACK_W: f32 = 44.0;
    let frac = (w.utilization / 100.0).clamp(0.0, 1.0);
    let fade = if stale { 0.45 } else { 1.0 };
    let ink = |a: f32| theme::hsla(theme::with_alpha(palette.ink, a * fade));
    div()
        .flex()
        .items_center()
        .gap_1()
        .text_color(ink(0.9))
        .child(div().flex_none().text_color(ink(0.55)).child(name))
        .child(
            div()
                .flex_none()
                .w(px(TRACK_W))
                .h(px(4.))
                .bg(ink(0.18))
                // A Metro progress bar: square, the accent filling it.
                .child(div().w(px(TRACK_W * frac)).h_full().bg(fill.opacity(fade))),
        )
        .child(
            div()
                .flex_none()
                .child(SharedString::from(format!("{}%", w.utilization.round() as i64))),
        )
        .when_some(w.resets_at, |d, r| {
            d.child(
                div()
                    .flex_none()
                    .text_color(ink(0.55))
                    .child(SharedString::from(format!("\u{21bb}{}", usage::countdown(r, now)))),
            )
        })
}

/// System Settings → Privacy & Security → Full Disk Access. The pane ids are
/// what `x-apple.systempreferences` has taken since the days of the old
/// Preferences app, and Settings still answers to them.
const PRIVACY_ALL_FILES: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// A flat Metro button for the status bar: a 2 px ink border, filled while
/// its panel is open and while it is pressed. `colors` is (ink, fill, the ink
/// that reads on the fill).
fn status_button(
    id: &'static str,
    label: impl Into<SharedString>,
    active: bool,
    colors: (gpui::Hsla, gpui::Hsla, gpui::Hsla),
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    let (ink, fill, fill_ink) = colors;
    div()
        .id(id)
        .flex_none()
        .px_2()
        .border_2()
        .border_color(if active { fill } else { ink })
        .cursor_pointer()
        .when(active, |d| d.bg(fill).text_color(fill_ink))
        .active(move |s| s.bg(fill).border_color(fill).text_color(fill_ink))
        .child(label.into())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_click)
}

impl Board {
    /// Redraws the board and the views that cache what it shows — the start
    /// screen and the panels (#181). For a change to the sessions, the
    /// settings or the panels; the frame clock, the pointer and the status
    /// bar's notices redraw the board alone.
    pub(crate) fn touch(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.tiles_view.update(cx, |_, cx| cx.notify());
        self.panels_view.update(cx, |_, cx| cx.notify());
    }
}

/// Where the cached views stand: the window above the status bar.
fn below_bar() -> StyleRefinement {
    StyleRefinement::default().absolute().top_0().left_0().right_0().bottom(px(STATUS_H))
}

/// The start screen as a view of its own (#181). Not cached (#182): its
/// live tiles cache their own bodies, which a cached parent would defeat.
pub(crate) struct TilesView {
    board: WeakEntity<Board>,
}

impl Render for TilesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.board.upgrade() {
            Some(board) => board.update(cx, |board, cx| board.render_tiles(cx)),
            None => div().into_any_element(),
        }
    }
}

/// The three panels as a view of its own (#181): a pointer moving over a
/// panel redraws the panel, not the start screen beside it, and the board's
/// clock redraws neither.
pub(crate) struct PanelsView {
    board: WeakEntity<Board>,
}

impl Render for PanelsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(board) = self.board.upgrade() else { return div().into_any_element() };
        board.update(cx, |board, cx| {
            let settings = board.render_settings(cx).into_any_element();
            let dev = board.render_devtools(cx).into_any_element();
            let list = board.render_list(cx).into_any_element();
            // The search is a panel like the rest since #193.
            let search = board.render_palette(cx).into_any_element();
            div().relative().size_full().child(settings).child(dev).child(list).child(search).into_any_element()
        })
    }
}

