//! The interactive board: owns the model + sources, handles input, and hands
//! each frame to the renderer.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, ClickEvent, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollWheelEvent, SharedString, Stateful, Window, canvas, div, prelude::*, px,
};

use crate::font;
use crate::geom::Pt;
use crate::model::{BoardModel, Target};
use crate::render::paint::PathCache;
use crate::render::scene::{self, ChipDraw, Frame, GAP, Lane, Layout};
use crate::settings::Settings;
use crate::sources::{self, ClaudeSource, FakeSource, SessionSource};
use crate::theme::{self, Palette};
use crate::ui::palette::PaletteState;
use crate::ui::panel::PanelState;
use crate::usage::{self, Usage};
use crate::{mac, warp};

pub const STATUS_H: f32 = 30.0;
/// The hover tooltip on a pull request connector (#77): how far it keeps from
/// the viewport's edges, from the connector itself, and how wide it may run.
const TIP_MARGIN: f32 = 8.0;
const TIP_GAP: f32 = 8.0;
const TIP_PAD: f32 = 8.0;
const TIP_H: f32 = 22.0;
const TIP_MIN: f32 = 90.0;
const TIP_MAX: f32 = 340.0;
/// Scale at which `font::measure` sizes the box: the board's own metric, at
/// roughly the cap height gpui gives an 11px D-DIN.
const TIP_SCALE: f32 = 1.32;
/// Session registry poll interval.
const POLL_MS: u64 = 1000;
/// Redraw cadence: the packets glide at 30fps, which halves the CPU of a
/// vsync-driven loop (#38).
const FRAME_MS: u64 = 33;

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
    pub(crate) settings_panel: PanelState,
    layout: Layout,
    lanes: Rc<Vec<Lane>>,
    /// True while the last built lanes were mid-slide (#49).
    lanes_moving: bool,
    draws: Vec<ChipDraw>,
    paths: Rc<RefCell<PathCache>>,
    scroll_y: f32,
    /// Mouse in design units / window pixels.
    mouse: Option<Pt>,
    pressed: Option<Target>,
    /// PR connectors of the last frame, kept so clicks can hit them (#57).
    prs: Vec<scene::PrDraw>,
    hovered_pr: Option<usize>,
    pressed_pr: Option<usize>,
    tickets: Vec<scene::TicketDraw>,
    hovered_ticket: Option<usize>,
    pressed_ticket: Option<usize>,
    title: String,
    drag_anchor: Option<(f64, f64)>,
    started: Instant,
    last_tick: Instant,
    focus_handle: FocusHandle,
    /// Claude plan usage for the status bar (#48): the last good value and
    /// when it was fetched, plus the latest error if the poll is failing.
    usage: Option<(Usage, Instant)>,
    usage_error: Option<usage::UsageError>,
    /// What the live source wants said in the status bar (#67).
    source_note: Option<&'static str>,
    /// How far down the last frame reached, so the scroll can get there even
    /// when a narrow window has pushed chips onto later stripes (#83).
    content_bottom: f32,
    /// The window in pixels, for the chrome that has to fit inside it (#83).
    pub(crate) window: (f32, f32),
    /// The ⌘K search over everything on the board (#71).
    pub(crate) palette: PaletteState,
}

impl Board {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

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
                    cx.notify();
                });
                if ok.is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(POLL_MS)).await;
            }
        })
        .detach();

        // Frame clock: repaint on a fixed cadence instead of every vsync.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(FRAME_MS)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
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
        Self {
            model: BoardModel::new(),
            live: Arc::new(ClaudeSource::default()),
            sandbox: Arc::new(FakeSource::new()),
            mode: Mode::Live,
            layout: Layout::new(scene::DESIGN_W, scene::DESIGN_H, 0, settings.zoom),
            settings,
            dev: PanelState::default(),
            settings_panel: PanelState::default(),
            lanes: Rc::new(Vec::new()),
            lanes_moving: false,
            draws: Vec::new(),
            paths: Rc::new(RefCell::new(PathCache::default())),
            scroll_y: 0.0,
            mouse: None,
            pressed: None,
            prs: Vec::new(),
            hovered_pr: None,
            pressed_pr: None,
            tickets: Vec::new(),
            hovered_ticket: None,
            pressed_ticket: None,
            title: sources::machine_user(),
            drag_anchor: None,
            started: now,
            last_tick: now,
            focus_handle,
            usage: None,
            usage_error: None,
            source_note: None,
            content_bottom: 0.0,
            window: (scene::DESIGN_W, scene::DESIGN_H),
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
            cx.notify();
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
        cx.notify();
    }

    /// Rebuilds the lane geometry when the layout changes and on every
    /// frame while rows are sliding into a new arrangement (#49), plus one
    /// more frame afterwards so the lanes settle on their exact spots.
    fn ensure_lanes(&mut self, win_w: f32, win_h: f32, now: Instant) {
        let layout = Layout::new(win_w, win_h, self.model.slot_span(), self.settings.zoom);
        let moving = self.model.rows.active(now);
        if layout == self.layout && !self.lanes.is_empty() && !moving && !self.lanes_moving {
            return;
        }
        self.layout = layout;
        self.lanes = Rc::new(layout.build_lanes_for(&self.model.rows, now));
        self.lanes_moving = moving;
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

    /// Hit test in design units.
    fn hit_test(&self, m: Pt) -> Option<Target> {
        let mut best: Option<(f32, Target)> = None;
        for d in &self.draws {
            if d.alpha < 0.4 {
                continue;
            }
            let center = Pt::new(d.center.x, d.center.y - self.scroll_y);
            let rel = m - center;
            let lx = rel.dot(d.tangent);
            let ly = rel.dot(d.tangent.perp());
            if lx.abs() <= d.width / 2.0 + 3.0 && ly.abs() <= d.style.h / 2.0 + 3.0 {
                let dist = lx.abs() + ly.abs();
                if best.is_none_or(|(bd, _)| dist < bd) {
                    best = Some((dist, d.target));
                }
            }
        }
        best.map(|(_, t)| t)
    }

    /// Index of the PR connector under the pointer. They never rotate, so the
    /// test is a plain box (#57).
    fn hit_pr(&self, m: Pt) -> Option<usize> {
        self.prs.iter().position(|p| {
            p.alpha >= 0.4
                && (m.x - p.center.x).abs() <= p.width / 2.0 + 3.0
                && (m.y - (p.center.y - self.scroll_y)).abs() <= scene::PR_H / 2.0 + 3.0
        })
    }

    /// Index of the Linear node under the pointer (#57).
    fn hit_ticket(&self, m: Pt) -> Option<usize> {
        self.tickets.iter().position(|t| {
            t.alpha >= 0.4
                && (m.x - t.center.x).abs() <= t.width / 2.0 + 3.0
                && (m.y - (t.center.y - self.scroll_y)).abs() <= scene::PR_H / 2.0 + 3.0
        })
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

    /// Opens a PR connector's pull request in the browser (#57).
    fn open_pr(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(p) = self.prs.get(i) else { return };
        let (label, url) = (p.label.clone(), p.url.clone());
        self.open_link(label, None, url, "pull request", cx);
    }

    /// Opens a Linear node's issue in the browser (#57).
    fn open_ticket(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(t) = self.tickets.get(i) else { return };
        let (label, app, web) = (t.label.clone(), t.app_url.clone(), t.url.clone());
        self.open_link(label, app, web, "Linear issue", cx);
    }

    /// The hovered pull request's title, parked next to its connector (#77).
    ///
    /// The connector is drawn in design units and scrolls with the board, so
    /// its place on screen is `(x, y - scroll) * zoom`. The box takes a width
    /// of its own rather than letting the text decide one, which is what lets
    /// the clamp be exact: whatever the estimate, the box is that wide, so
    /// keeping it inside the viewport is arithmetic and not a guess. It sits
    /// above the connector, and flips below when there is no room up there.
    fn pr_tooltip(&self, w: f32, h: f32) -> Option<gpui::Div> {
        let draw = self.prs.get(self.hovered_pr?)?;
        let title = draw.title.trim();
        if title.is_empty() {
            return None;
        }
        let zoom = self.layout.zoom;
        let (cx, cy) = (draw.center.x * zoom, (draw.center.y - self.scroll_y) * zoom);
        let half = scene::PR_H / 2.0 * zoom;

        let want = font::measure(title, TIP_SCALE) + 2.0 * TIP_PAD;
        let (left, top, tip_w) = tip_box((cx, cy), half, want, (w, h));

        let theme = self.palette();
        let ink = |a: f32| theme::hsla(theme::with_alpha(theme.ink, a));
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(tip_w))
                .h(px(TIP_H))
                .flex()
                .items_center()
                .px(px(TIP_PAD))
                .rounded_sm()
                .border_1()
                .border_color(ink(0.35))
                .bg(theme::hsla(crate::ui::palette::card_bg(theme)))
                .shadow_lg()
                .font_family(theme::UI_FONT)
                .text_size(px(11.))
                .text_color(ink(0.85))
                .overflow_hidden()
                .child(SharedString::from(title.to_string())),
        )
    }

    /// Opens the pane the `NO WARP TABS` tag asks for (#70). That tag is the
    /// only place the board can say the permission is missing, so it is also
    /// the way there.
    fn open_full_disk_access(&mut self, cx: &mut Context<Self>) {
        self.open_link("FULL DISK ACCESS".into(), Some(PRIVACY_ALL_FILES.into()), None, "settings pane", cx);
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
            let result = cx.background_executor().spawn(async move { warp::focus(&info) }).await;
            let msg = match result {
                Ok(warp::Outcome::Focused(url)) => format!("→ {label} ({url})"),
                Ok(warp::Outcome::ActivatedApp(app)) => {
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
        self.ensure_lanes(w, h, now);
        self.clamp_scroll();
        for (target, key, s) in scene::placements(&self.model, &self.layout, &self.lanes) {
            if let Some(anim) = self.model.anim_mut(target) {
                anim.track_placement(key, s, now);
            }
        }
        self.draws = scene::chip_draws(&self.model, &self.layout, &self.lanes, now);
        self.prs = scene::pr_draws(&self.model, &self.layout, &self.lanes, &self.draws);
        self.tickets = scene::ticket_draws(&self.model, &self.lanes, &self.draws);
        self.model.hovered = self.mouse.and_then(|m| self.hit_test(m));
        let (hp, ht) = self.mouse.map(|m| (self.hit_pr(m), self.hit_ticket(m))).unwrap_or((None, None));
        self.hovered_pr = hp;
        self.hovered_ticket = ht;
        if let Some(i) = self.hovered_pr {
            self.prs[i].hover = 1.0;
        }
        if let Some(i) = self.hovered_ticket {
            self.tickets[i].hover = 1.0;
        }
        self.content_bottom = self.layout.board_bottom(&self.draws, &self.prs, &self.tickets);
        self.clamp_scroll();
        let prs = self.prs.clone();
        let tickets = self.tickets.clone();
        let done_lanes = scene::done_lanes(&self.model, &self.draws);

        let palette = self.palette();
        let frame = Frame {
            lanes: self.lanes.clone(),
            chips: self.draws.clone(),
            prs,
            tickets,
            done_lanes,
            scroll_y: self.scroll_y,
            t: now.duration_since(self.started).as_secs_f32(),
            layout: self.layout,
            title: self.title.clone(),
            pan_x: 0.0,
            palette,
        };
        let paths = self.paths.clone();

        let ink = theme::hsla(palette.ink);
        let summary = self.model.summary();
        let detail = self.model.detail();
        let mode_tag = match self.mode {
            Mode::Live => None,
            Mode::Sandbox => Some("SANDBOX"),
        };
        let source_note = self.source_note;
        let open_fda = cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.open_full_disk_access(cx));
        let dev_open = self.dev.open;
        let palette_open = self.palette.open;
        let pr_tip = self.pr_tooltip(w, h);
        let settings_open = self.settings_panel.open;
        let usage = self.usage_now(now);

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .when(self.model.hovered.is_some() || self.hovered_pr.is_some() || self.hovered_ticket.is_some(), |d| d.cursor_pointer())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let cmd = ev.keystroke.modifiers.platform;
                // ⌘K opens and closes it; while it is open every other key
                // is typing, not a shortcut (#71).
                if cmd && ev.keystroke.key == "k" {
                    this.toggle_palette();
                    cx.notify();
                    return;
                }
                if this.palette.open {
                    this.palette_key(ev, cx);
                    cx.notify();
                    return;
                }
                match ev.keystroke.key.as_str() {
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
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                let m = this.to_design(ev.position);
                this.mouse = Some(m);
                this.model.hovered = this.hit_test(m);
                this.hovered_pr = this.hit_pr(m);
                this.hovered_ticket = this.hit_ticket(m);
                if ev.pressed_button == Some(MouseButton::Left) {
                    if let Some(anchor) = this.drag_anchor {
                        let cur = mac::mouse_location();
                        let (dx, dy) = (cur.0 - anchor.0, cur.1 - anchor.1);
                        this.drag_anchor = Some(cur);
                        // Move outside the event dispatch: AppKit calls back into
                        // gpui synchronously from `setFrameOrigin:`.
                        cx.spawn(async move |_, _| mac::move_window_by(dx, dy)).detach();
                    }
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let m = this.to_design(ev.position);
                    this.mouse = Some(m);
                    this.model.hovered = this.hit_test(m);
                    this.hovered_pr = this.hit_pr(m);
                    this.hovered_ticket = this.hit_ticket(m);
                    if this.model.hovered.is_some() || this.hovered_pr.is_some() || this.hovered_ticket.is_some() {
                        this.pressed = this.model.hovered;
                        this.pressed_pr = this.hovered_pr;
                        this.pressed_ticket = this.hovered_ticket;
                        this.drag_anchor = None;
                    } else {
                        this.pressed = None;
                        this.pressed_pr = None;
                        this.pressed_ticket = None;
                        this.drag_anchor = Some(mac::mouse_location());
                    }
                    // A click on the board is a click past the palette: the
                    // card itself stops propagation, so anything reaching here
                    // was outside it (#71).
                    if this.palette.open {
                        this.palette.open = false;
                        this.palette.results.clear();
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
                            this.activate(h, cx);
                        }
                    }
                    if let (Some(p), Some(h)) = (this.pressed_pr, this.hit_pr(m)) {
                        if p == h {
                            this.open_pr(h, cx);
                        }
                    }
                    if let (Some(p), Some(h)) = (this.pressed_ticket, this.hit_ticket(m)) {
                        if p == h {
                            this.open_ticket(h, cx);
                        }
                    }
                    this.pressed = None;
                    this.pressed_pr = None;
                    this.pressed_ticket = None;
                    this.drag_anchor = None;
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
                this.scroll_y -= f32::from(delta.y) / this.layout.zoom;
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let origin = Pt::new(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        let shapes = scene::build_shapes(&frame, origin);
                        paths.borrow_mut().paint(&shapes, window);
                    },
                )
                .size_full(),
            )
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
                    .border_t_1()
                    .border_color(theme::hsla(theme::with_alpha(palette.ink, 0.35)))
                    .font_family(theme::UI_FONT)
                    .text_size(px(11.))
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
                                    div()
                                        .px_1p5()
                                        .rounded_sm()
                                        .bg(theme::hsla(palette.ink))
                                        .text_color(theme::hsla(palette.bg))
                                        .child(tag),
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
                                        .rounded_sm()
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
                                        .py_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(theme::hsla(theme::with_alpha(palette.ink, 0.35)))
                                        .child(usage_meter("5H", u.five_hour, secs, palette, stale))
                                        .child(usage_meter("7D", u.seven_day, secs, palette, stale)),
                                )
                            })
                            .child(status_button(
                                "settings-toggle",
                                "SETTINGS",
                                settings_open,
                                palette,
                                cx.listener(|this, _, _, cx| {
                                    this.toggle_settings();
                                    cx.notify();
                                }),
                            ))
                            .child(status_button(
                                "dev-toggle",
                                "TEST",
                                dev_open,
                                palette,
                                cx.listener(|this, _, _, cx| {
                                    this.toggle_devtools();
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            // Panels dock in the bottom-right corner, side by side (#25).
            .when(dev_open || settings_open, |d| {
                let mut dock = div()
                    .absolute()
                    .right(px(18.))
                    .bottom(px(STATUS_H + 12.))
                    .flex()
                    .flex_row()
                    .items_end()
                    .gap_3();
                if settings_open {
                    dock = dock.child(self.render_settings(cx));
                }
                if dev_open {
                    dock = dock.child(self.render_devtools(cx));
                }
                d.child(dock)
            })
            // The search sits over the board, near the top, where a palette
            // is looked for (#71).
            .when(palette_open, |d| d.child(self.render_palette(w, h, cx)))
            .when_some(pr_tip, |d, tip| d.child(tip))
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

/// Where a connector's tooltip goes: `(left, top, width)` in window pixels,
/// from the connector's centre, its half height, the width the text would
/// like, and the viewport. It sits above the connector and flips below when
/// there is no room up there, and it never crosses the margin on any side,
/// the status bar included.
///
/// The box takes a width of its own rather than letting the text decide one.
/// That is what makes the clamp exact: however far off the estimate is, the
/// box is that wide, so staying inside the viewport is arithmetic rather than
/// a guess. (#77)
fn tip_box(centre: (f32, f32), half: f32, want: f32, vp: (f32, f32)) -> (f32, f32, f32) {
    let ((cx, cy), (w, h)) = (centre, vp);
    let room = (w - 2.0 * TIP_MARGIN).max(TIP_MIN);
    let tip_w = want.clamp(TIP_MIN, TIP_MAX.min(room));
    let left = (cx - tip_w / 2.0).clamp(TIP_MARGIN, (w - TIP_MARGIN - tip_w).max(TIP_MARGIN));
    let floor = (h - STATUS_H - TIP_MARGIN - TIP_H).max(TIP_MARGIN);
    let above = cy - half - TIP_GAP - TIP_H;
    let top = if above >= TIP_MARGIN { above } else { cy + half + TIP_GAP };
    (left, top.clamp(TIP_MARGIN, floor), tip_w)
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
                .rounded_full()
                .bg(ink(0.18))
                .child(
                    div()
                        .w(px(TRACK_W * frac))
                        .h_full()
                        .rounded_full()
                        .bg(theme::hsla(theme::with_alpha(palette.packet, fade))),
                ),
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

fn status_button(
    id: &'static str,
    label: impl Into<SharedString>,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(theme::hsla(theme::with_alpha(palette.ink, 0.6)))
        .cursor_pointer()
        .when(active, |d| d.bg(theme::hsla(palette.ink)).text_color(theme::hsla(palette.bg)))
        .hover(move |s| s.bg(theme::hsla(theme::with_alpha(palette.ink, 0.15))))
        .child(label.into())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_click)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: (f32, f32) = (980.0, 620.0);

    /// Pushed all the way down, the lowest lane has to be clear of the status
    /// bar. The bar is a fixed number of window pixels, so zooming out makes
    /// it taller in the board's own units, and it used to swallow the last
    /// lane's connectors (#81).
    #[test]
    fn the_lowest_lane_clears_the_status_bar_at_every_zoom() {
        for zoom in [0.5, 0.8, 1.0, 1.5, 3.0] {
            for sessions in [0, 1, 6, 14, 30] {
                let layout = Layout::new(VP.0, VP.1, sessions, zoom);
                let visible = visible_height(&layout);
                // With nothing on it the board reaches exactly as far as its
                // lanes do, and the lowest thing sits `BOTTOM_PAD` above that.
                let bottom = layout.content_height();
                let lowest = bottom - scene::BOTTOM_PAD - max_scroll(&layout, bottom);
                assert!(
                    lowest + scene::PR_H / 2.0 <= visible,
                    "zoom {zoom}, {sessions} sessions: the last lane reaches {lowest} of a visible {visible}"
                );
            }
        }
    }

    /// Half the height of a connector, at zoom 1.
    const HALF: f32 = scene::PR_H / 2.0;

    #[test]
    fn a_tooltip_sits_above_the_connector_it_belongs_to() {
        let (left, top, w) = tip_box((500.0, 300.0), HALF, 200.0, VP);
        assert_eq!(w, 200.0, "a width that fits is kept");
        assert_eq!(left, 500.0 - 100.0, "centred on the connector");
        assert_eq!(top, 300.0 - HALF - TIP_GAP - TIP_H, "clear of it, above");
    }

    #[test]
    fn a_tooltip_flips_below_a_connector_near_the_top() {
        let (_, top, _) = tip_box((500.0, 12.0), HALF, 200.0, VP);
        assert_eq!(top, 12.0 + HALF + TIP_GAP, "there is no room above, so it goes under");
    }

    /// The whole point of the ask: wherever the connector is, the box stays
    /// on screen (#77).
    #[test]
    fn a_tooltip_never_leaves_the_viewport() {
        let (w, h) = VP;
        let mut xs = vec![];
        let mut x = -60.0;
        while x <= w + 60.0 {
            xs.push(x);
            x += 20.0;
        }
        for cx in xs {
            let mut cy = -60.0;
            while cy <= h + 60.0 {
                for want in [40.0, 200.0, 900.0] {
                    let (left, top, tip_w) = tip_box((cx, cy), HALF, want, VP);
                    assert!(left >= TIP_MARGIN, "off the left at {cx},{cy}");
                    assert!(left + tip_w <= w - TIP_MARGIN + 0.01, "off the right at {cx},{cy}");
                    assert!(top >= TIP_MARGIN, "off the top at {cx},{cy}");
                    assert!(
                        top + TIP_H <= h - STATUS_H - TIP_MARGIN + 0.01,
                        "under the status bar at {cx},{cy}"
                    );
                }
                cy += 20.0;
            }
        }
    }

    #[test]
    fn a_narrow_window_shrinks_the_tooltip_rather_than_overflowing() {
        let narrow = (160.0, 620.0);
        let (left, _, tip_w) = tip_box((80.0, 300.0), HALF, 900.0, narrow);
        assert!(tip_w <= narrow.0 - 2.0 * TIP_MARGIN, "it gives up width before it gives up the edge");
        assert!(left >= TIP_MARGIN && left + tip_w <= narrow.0 - TIP_MARGIN);
    }

    #[test]
    fn a_long_title_stops_at_the_maximum() {
        let (_, _, tip_w) = tip_box((500.0, 300.0), HALF, 4000.0, VP);
        assert_eq!(tip_w, TIP_MAX, "a title does not get to run the width of the board");
    }
}
