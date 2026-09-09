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

use crate::geom::{Polyline, Pt};
use crate::model::{BoardModel, Target};
use crate::render::paint::PathCache;
use crate::render::scene::{self, ChipDraw, Frame, GAP, Layout};
use crate::settings::Settings;
use crate::sources::{self, ClaudeSource, FakeSource, SessionSource};
use crate::theme::{self, Palette};
use crate::ui::panel::PanelState;
use crate::{mac, warp};

pub const STATUS_H: f32 = 30.0;
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
    lanes: Rc<Vec<Polyline>>,
    draws: Vec<ChipDraw>,
    paths: Rc<RefCell<PathCache>>,
    scroll_y: f32,
    /// Mouse in design units / window pixels.
    mouse: Option<Pt>,
    pressed: Option<Target>,
    title: String,
    drag_anchor: Option<(f64, f64)>,
    started: Instant,
    last_tick: Instant,
    focus_handle: FocusHandle,
}

impl Board {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        cx.spawn(async move |this, cx| {
            loop {
                let Ok(source) = this.update(cx, |board, _| board.source()) else { break };
                let list = cx.background_executor().spawn(async move { source.snapshot() }).await;
                let now = Instant::now();
                let ok = this.update(cx, |board, cx| {
                    board.sandbox.churn(now);
                    board.model.apply(list, now);
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
            draws: Vec::new(),
            paths: Rc::new(RefCell::new(PathCache::default())),
            scroll_y: 0.0,
            mouse: None,
            pressed: None,
            title: sources::machine_user(),
            drag_anchor: None,
            started: now,
            last_tick: now,
            focus_handle,
        }
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

    fn ensure_lanes(&mut self, win_w: f32, win_h: f32) {
        let layout = Layout::new(win_w, win_h, self.model.slot_span(), self.settings.zoom);
        if layout == self.layout && !self.lanes.is_empty() {
            return;
        }
        self.layout = layout;
        self.lanes = Rc::new(layout.build_lanes());
    }

    fn clamp_scroll(&mut self) {
        let max = (self.layout.content_height() - self.layout.height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
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

        self.model.tick(dt, now);
        self.ensure_lanes(w, h);
        self.clamp_scroll();
        for (target, key, s) in scene::placements(&self.model, &self.layout, &self.lanes) {
            if let Some(anim) = self.model.anim_mut(target) {
                anim.track_placement(key, s, now);
            }
        }
        self.draws = scene::chip_draws(&self.model, &self.layout, &self.lanes, now);
        self.model.hovered = self.mouse.and_then(|m| self.hit_test(m));

        let palette = self.palette();
        let frame = Frame {
            lanes: self.lanes.clone(),
            chips: self.draws.clone(),
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
        let dev_open = self.dev.open;
        let settings_open = self.settings_panel.open;

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .when(self.model.hovered.is_some(), |d| d.cursor_pointer())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let cmd = ev.keystroke.modifiers.platform;
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
                    if this.model.hovered.is_some() {
                        this.pressed = this.model.hovered;
                        this.drag_anchor = None;
                    } else {
                        this.pressed = None;
                        this.drag_anchor = Some(mac::mouse_location());
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
                    this.pressed = None;
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
                    .font_family("Menlo")
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
                            }),
                    )
                    .child(div().flex_1().overflow_hidden().text_right().child(detail))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
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
    }
}

/// Small bordered button for the status bar.
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
