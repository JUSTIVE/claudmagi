//! The interactive board: owns the model + sources, handles input, and hands
//! each frame to the renderer.

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, ScrollWheelEvent, Window, canvas, div, prelude::*, px,
};

use crate::geom::{Polyline, Pt};
use crate::model::{BoardModel, Target};
use crate::render::paint::paint_shapes;
use crate::render::scene::{self, ChipDraw, Frame, GAP, Layout};
use crate::sources::{self, ClaudeSource, FakeSource, SessionSource};
use crate::theme::{self, PALETTE};
use crate::ui::devtools::DevState;
use crate::{mac, warp};

pub const STATUS_H: f32 = 30.0;
const POLL_MS: u64 = 300;

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
    pub(crate) dev: DevState,
    layout: Layout,
    lanes: Rc<Vec<Polyline>>,
    draws: Vec<ChipDraw>,
    scroll_y: f32,
    /// Mouse in design units / window pixels.
    mouse: Option<Pt>,
    mouse_px: Option<Pt>,
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

        let now = Instant::now();
        Self {
            model: BoardModel::new(),
            live: Arc::new(ClaudeSource::default()),
            sandbox: Arc::new(FakeSource::new()),
            mode: Mode::Live,
            dev: DevState::default(),
            layout: Layout::new(scene::DESIGN_W, scene::DESIGN_H, 0),
            lanes: Rc::new(Vec::new()),
            draws: Vec::new(),
            scroll_y: 0.0,
            mouse: None,
            mouse_px: None,
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

    pub(crate) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            let msg = format!("source: {}", self.source().name());
            self.model.set_notice(msg, Instant::now());
            cx.notify();
        }
    }

    fn ensure_lanes(&mut self, win_w: f32, win_h: f32) {
        let layout = Layout::new(win_w, win_h, self.model.chips.len());
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
        self.draws = scene::chip_draws(&self.model, &self.layout, &self.lanes, now);
        self.model.hovered = self.mouse.and_then(|m| self.hit_test(m));

        let frame = Frame {
            lanes: self.lanes.clone(),
            chips: self.draws.clone(),
            scroll_y: self.scroll_y,
            t: now.duration_since(self.started).as_secs_f32(),
            layout: self.layout,
            title: self.title.clone(),
            pan_x: 0.0,
            palette: PALETTE,
        };
        window.request_animation_frame();

        let ink = theme::hsla(PALETTE.ink);
        let summary = self.model.summary();
        let detail = self.model.detail();
        let mode_tag = match self.mode {
            Mode::Live => None,
            Mode::Sandbox => Some("SANDBOX"),
        };
        let dev_open = self.dev.open;

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .when(self.model.hovered.is_some(), |d| d.cursor_pointer())
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" if this.dev.open => this.dev.open = false,
                    "escape" => cx.quit(),
                    "t" => this.toggle_devtools(f32::from(window.viewport_size().width)),
                    _ => return,
                }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                let m_px = Pt::new(f32::from(ev.position.x), f32::from(ev.position.y));
                this.mouse_px = Some(m_px);
                let m = this.to_design(ev.position);
                this.mouse = Some(m);
                this.model.hovered = this.hit_test(m);
                if ev.pressed_button == Some(MouseButton::Left) {
                    if let Some(offset) = this.dev.drag {
                        this.dev.pos = Some(m_px - offset);
                    } else if let Some(anchor) = this.drag_anchor {
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
                    this.dev.drag = None;
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let delta = ev.delta.pixel_delta(px(GAP));
                this.scroll_y -= f32::from(delta.y) / this.layout.zoom;
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let origin = Pt::new(f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        let shapes = scene::build_shapes(&frame, origin);
                        paint_shapes(&shapes, window);
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
                    .bg(theme::hsla(PALETTE.bg))
                    .border_t_1()
                    .border_color(theme::hsla(theme::with_alpha(PALETTE.ink, 0.35)))
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
                                        .bg(theme::hsla(PALETTE.ink))
                                        .text_color(theme::hsla(PALETTE.text_needs))
                                        .child(tag),
                                )
                            }),
                    )
                    .child(div().flex_1().overflow_hidden().text_right().child(detail))
                    .child(
                        div()
                            .id("dev-toggle")
                            .flex_none()
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .border_1()
                            .border_color(theme::hsla(theme::with_alpha(PALETTE.ink, 0.6)))
                            .cursor_pointer()
                            .when(dev_open, |d| d.bg(theme::hsla(PALETTE.ink)).text_color(theme::hsla(PALETTE.text_on)))
                            .hover(|s| s.bg(theme::hsla(theme::with_alpha(PALETTE.ink, 0.15))))
                            .child("TEST")
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_devtools(f32::from(window.viewport_size().width));
                                cx.notify();
                            })),
                    ),
            )
            .when(dev_open, |d| d.child(self.render_devtools(cx)))
    }
}
