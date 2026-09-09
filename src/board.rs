//! The interactive circuit-board view: animation state, input, gpui painting.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Path, PathBuilder, PathStyle, Pixels, ScrollWheelEvent, StrokeOptions, Window,
    canvas, div, fill, point, prelude::*, px, size,
};
use lyon::path::{LineCap, LineJoin};

use crate::font;
use crate::geom::{Polyline, Pt, ease_out, smoothstep};
use crate::scene::{self, ChipDraw, Frame, Shape, CHIP_H, GAP, PULL};
use crate::sessions::{self, Phase, SessionInfo};
use crate::theme::{self, PALETTE};
use crate::{mac, warp};

const STATUS_H: f32 = 30.0;
const POLL_MS: u64 = 500;

struct Chip {
    info: SessionInfo,
    lane: usize,
    on_diag: bool,
    disconnect: f32,
    hover_t: f32,
    born: Instant,
    gone: bool,
    fade: f32,
}

pub struct Board {
    chips: Vec<Chip>,
    lanes: Rc<Vec<Polyline>>,
    lane_key: (i32, usize),
    draws: Vec<ChipDraw>,
    scroll_y: f32,
    mouse: Option<Pt>,
    hovered: Option<usize>,
    pressed: Option<usize>,
    drag_anchor: Option<(f64, f64)>,
    started: Instant,
    last_tick: Instant,
    focus_handle: FocusHandle,
    notice: Option<(String, Instant)>,
}

impl Board {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);

        cx.spawn(async move |this, cx| {
            loop {
                let list = cx.background_executor().spawn(async { sessions::read_sessions() }).await;
                if this.update(cx, |board, cx| board.apply(list, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(POLL_MS)).await;
            }
        })
        .detach();

        let now = Instant::now();
        Self {
            chips: Vec::new(),
            lanes: Rc::new(Vec::new()),
            lane_key: (0, 0),
            draws: Vec::new(),
            scroll_y: 0.0,
            mouse: None,
            hovered: None,
            pressed: None,
            drag_anchor: None,
            started: now,
            last_tick: now,
            focus_handle,
            notice: None,
        }
    }

    fn apply(&mut self, list: Vec<SessionInfo>, cx: &mut Context<Self>) {
        let now = Instant::now();
        let mut seen = vec![false; self.chips.len()];
        for info in list {
            if let Some(i) = self
                .chips
                .iter()
                .position(|c| c.info.session_id == info.session_id && c.info.pid == info.pid)
            {
                self.chips[i].info = info;
                self.chips[i].gone = false;
                seen[i] = true;
            } else {
                self.chips.push(Chip {
                    info,
                    lane: 0,
                    on_diag: true,
                    disconnect: 0.0,
                    hover_t: 0.0,
                    born: now,
                    gone: false,
                    fade: 0.0,
                });
                seen.push(true);
            }
        }
        for (i, s) in seen.iter().enumerate() {
            if !s {
                self.chips[i].gone = true;
            }
        }
        self.assign_lanes();
        cx.notify();
    }

    fn assign_lanes(&mut self) {
        for (k, chip) in self.chips.iter_mut().enumerate() {
            chip.lane = 1 + k;
            chip.on_diag = k % 2 == 0;
        }
    }

    fn ensure_lanes(&mut self, width: f32) {
        let key = (width as i32, scene::lane_count(self.chips.len()));
        if key == self.lane_key && !self.lanes.is_empty() {
            return;
        }
        self.lane_key = key;
        self.lanes = Rc::new(scene::build_lanes(key.1, width));
    }

    fn clamp_scroll(&mut self, viewport_h: f32) {
        let max = (scene::content_height(self.chips.len()) - viewport_h).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    fn tick(&mut self, dt: f32, now: Instant) {
        let hovered = self.hovered;
        for (i, c) in self.chips.iter_mut().enumerate() {
            let age = now.duration_since(c.born).as_secs_f32();
            let target_p = if c.info.phase() == Phase::Working || age < 0.7 { 0.0 } else { 1.0 };
            approach(&mut c.disconnect, target_p, dt / 0.42);
            let target_h = if hovered == Some(i) { 1.0 } else { 0.0 };
            approach(&mut c.hover_t, target_h, dt / 0.12);
            let target_f = if c.gone { 0.0 } else { 1.0 };
            approach(&mut c.fade, target_f, dt / 0.35);
        }
        let before = self.chips.len();
        self.chips.retain(|c| !(c.gone && c.fade <= 0.001));
        if self.chips.len() != before {
            self.assign_lanes();
        }
        if let Some((_, at)) = &self.notice {
            if now.duration_since(*at) > Duration::from_secs(5) {
                self.notice = None;
            }
        }
    }

    fn compute_draws(&self, width: f32, now: Instant) -> Vec<ChipDraw> {
        let mut out = Vec::with_capacity(self.chips.len());
        for c in &self.chips {
            let Some(lane) = self.lanes.get(c.lane) else { continue };
            let label = c.info.label();
            let w = scene::chip_width(&label);
            let s_c = scene::chip_anchor(lane, c.lane, c.on_diag, width, w);
            let p = smoothstep(c.disconnect);
            let (pos, tan) = lane.point_at(s_c + p * PULL);
            let age = now.duration_since(c.born).as_secs_f32();
            let alpha = ease_out(age / 0.45) * c.fade;
            out.push(ChipDraw {
                lane: c.lane,
                s_c,
                width: w,
                label,
                p,
                hover: c.hover_t,
                alpha,
                phase: c.info.phase(),
                center: pos,
                tangent: tan,
            });
        }
        out
    }

    /// Hit test in window coordinates.
    fn hit_test(&self, m: Pt) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, d) in self.draws.iter().enumerate() {
            if d.alpha < 0.4 {
                continue;
            }
            let center = Pt::new(d.center.x, d.center.y - self.scroll_y);
            let rel = m - center;
            let lx = rel.dot(d.tangent);
            let ly = rel.dot(d.tangent.perp());
            if lx.abs() <= d.width / 2.0 + 3.0 && ly.abs() <= CHIP_H / 2.0 + 3.0 {
                let dist = lx.abs() + ly.abs();
                if best.is_none_or(|(bd, _)| dist < bd) {
                    best = Some((dist, i));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(chip) = self.chips.get(index) else { return };
        let info = chip.info.clone();
        let label = info.label();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { warp::focus(&info) }).await;
            let msg = match result {
                Ok(warp::Outcome::Focused(_)) => format!("→ {label} (Warp)"),
                Ok(warp::Outcome::ActivatedApp(app)) => {
                    format!("{label}: no Warp session URL, activated {app}")
                }
                Err(e) => format!("{label}: {e}"),
            };
            this.update(cx, |board, cx| {
                board.notice = Some((msg, Instant::now()));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn summary(&self) -> String {
        let live: Vec<&Chip> = self.chips.iter().filter(|c| !c.gone).collect();
        let count = |p: Phase| live.iter().filter(|c| c.info.phase() == p).count();
        if live.is_empty() {
            return "NO LIVE CLAUDE CODE SESSIONS".to_string();
        }
        format!(
            "{} SESSION{} · {} WORKING · {} NEEDS INPUT · {} IDLE",
            live.len(),
            if live.len() == 1 { "" } else { "S" },
            count(Phase::Working),
            count(Phase::NeedsUser),
            count(Phase::Idle)
        )
    }

    fn detail(&self) -> String {
        if let Some((msg, _)) = &self.notice {
            return msg.clone();
        }
        let Some(i) = self.hovered else { return String::new() };
        let Some(c) = self.chips.get(i) else { return String::new() };
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
        parts.push(format!("pid {}", info.pid));
        parts.join(" · ")
    }
}

impl Render for Board {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.05);
        self.last_tick = now;
        let vp = window.viewport_size();
        let (w, h) = (f32::from(vp.width), f32::from(vp.height));

        self.tick(dt, now);
        self.ensure_lanes(w);
        self.clamp_scroll(h);
        self.draws = self.compute_draws(w, now);
        self.hovered = self.mouse.and_then(|m| self.hit_test(m));

        let frame = Frame {
            lanes: self.lanes.clone(),
            chips: self.draws.clone(),
            scroll_y: self.scroll_y,
            t: now.duration_since(self.started).as_secs_f32(),
            width: w,
            height: h,
        };
        window.request_animation_frame();

        let ink = theme::hsla(PALETTE.ink);
        let summary = self.summary();
        let detail = self.detail();

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .when(self.hovered.is_some(), |d| d.cursor_pointer())
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" {
                    cx.quit();
                }
            }))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                let m = Pt::new(f32::from(ev.position.x), f32::from(ev.position.y));
                this.mouse = Some(m);
                this.hovered = this.hit_test(m);
                if ev.pressed_button == Some(MouseButton::Left) {
                    if let Some(anchor) = this.drag_anchor {
                        let cur = mac::mouse_location();
                        let (dx, dy) = (cur.0 - anchor.0, cur.1 - anchor.1);
                        this.drag_anchor = Some(cur);
                        // Move outside the event dispatch: AppKit calls back into
                        // gpui synchronously from `setFrameOrigin:`, which would
                        // otherwise hit an already-borrowed App.
                        cx.spawn(async move |_, _| mac::move_window_by(dx, dy)).detach();
                    }
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    let m = Pt::new(f32::from(ev.position.x), f32::from(ev.position.y));
                    this.mouse = Some(m);
                    this.hovered = this.hit_test(m);
                    if this.hovered.is_some() {
                        this.pressed = this.hovered;
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
                    let m = Pt::new(f32::from(ev.position.x), f32::from(ev.position.y));
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
                this.scroll_y -= f32::from(delta.y);
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
                    .child(div().flex_none().child(summary))
                    .child(div().flex_1().overflow_hidden().text_right().child(detail)),
            )
    }
}

fn approach(v: &mut f32, target: f32, amount: f32) {
    if *v < target {
        *v = (*v + amount).min(target);
    } else if *v > target {
        *v = (*v - amount).max(target);
    }
}

fn gp(p: Pt) -> gpui::Point<Pixels> {
    point(px(p.x), px(p.y))
}

fn stroke_path(pieces: &[Vec<Pt>], width: f32) -> Option<Path<Pixels>> {
    let options = StrokeOptions::default()
        .with_line_width(width)
        .with_line_join(LineJoin::Round)
        .with_line_cap(LineCap::Round);
    let mut b = PathBuilder::stroke(px(width)).with_style(PathStyle::Stroke(options));
    let mut any = false;
    for piece in pieces {
        if piece.len() < 2 {
            continue;
        }
        for (k, p) in piece.iter().enumerate() {
            if k == 0 {
                b.move_to(gp(*p));
            } else {
                b.line_to(gp(*p));
            }
        }
        any = true;
    }
    if !any {
        return None;
    }
    b.build().ok()
}

fn rounded_rect_outline(w: f32, h: f32, r: f32, mut b: PathBuilder) -> PathBuilder {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let r = r.min(hw).min(hh);
    b.move_to(point(px(-hw + r), px(-hh)));
    b.line_to(point(px(hw - r), px(-hh)));
    b.curve_to(point(px(hw), px(-hh + r)), point(px(hw), px(-hh)));
    b.line_to(point(px(hw), px(hh - r)));
    b.curve_to(point(px(hw - r), px(hh)), point(px(hw), px(hh)));
    b.line_to(point(px(-hw + r), px(hh)));
    b.curve_to(point(px(-hw), px(hh - r)), point(px(-hw), px(hh)));
    b.line_to(point(px(-hw), px(-hh + r)));
    b.curve_to(point(px(-hw + r), px(-hh)), point(px(-hw), px(-hh)));
    b.close();
    b
}

fn rounded_rect(center: Pt, w: f32, h: f32, r: f32, angle: f32, stroke: Option<f32>) -> Option<Path<Pixels>> {
    let builder = match stroke {
        Some(sw) => {
            let options = StrokeOptions::default().with_line_width(sw).with_line_join(LineJoin::Round);
            PathBuilder::stroke(px(sw)).with_style(PathStyle::Stroke(options))
        }
        None => PathBuilder::fill(),
    };
    let mut b = rounded_rect_outline(w, h, r, builder);
    if angle != 0.0 {
        b.rotate(angle);
    }
    b.translate(gp(center));
    b.build().ok()
}

fn paint_shapes(shapes: &[Shape], window: &mut Window) {
    for shape in shapes {
        match shape {
            Shape::Rect { x, y, w, h, color } => {
                let bounds = Bounds { origin: point(px(*x), px(*y)), size: size(px(*w), px(*h)) };
                window.paint_quad(fill(bounds, theme::hsla(*color)));
            }
            Shape::Stroke { pieces, width, color } => {
                if let Some(path) = stroke_path(pieces, *width) {
                    window.paint_path(path, theme::hsla(*color));
                }
            }
            Shape::RoundedRect { center, w, h, r, angle, color, stroke } => {
                if let Some(path) = rounded_rect(*center, *w, *h, *r, *angle, *stroke) {
                    window.paint_path(path, theme::hsla(*color));
                }
            }
            Shape::Text { text, scale, stroke, angle, center, color } => {
                if let Some(path) = font::build(text, *scale, *stroke, *angle, (center.x, center.y)) {
                    window.paint_path(path, theme::hsla(*color));
                }
            }
        }
    }
}
