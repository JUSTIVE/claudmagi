mod font;
mod geom;
mod mac;
mod model;
mod render;
mod settings;
mod sources;
mod theme;
mod ui;
mod pr;
mod ticket;
mod usage;
mod warp;

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};

use model::{BoardModel, Phase, SessionInfo};
use render::scene;
use sources::{ClaudeSource, FakeSource, SessionSource};
use ui::board::Board;

actions!(claudmagi, [Quit]);

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--svg") {
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "claudmagi.svg".into());
        let demo = args.iter().any(|a| a == "--demo");
        let user_zoom = args
            .iter()
            .position(|a| a == "--zoom")
            .and_then(|k| args.get(k + 1))
            .and_then(|z| z.parse().ok())
            .unwrap_or(1.0);
        let (w, h) = args
            .iter()
            .position(|a| a == "--size")
            .and_then(|k| args.get(k + 1))
            .and_then(|s| s.split_once('x'))
            .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
            .unwrap_or((scene::DESIGN_W, scene::DESIGN_H));
        let count = args
            .iter()
            .position(|a| a == "--count")
            .and_then(|k| args.get(k + 1))
            .and_then(|n| n.parse().ok())
            .unwrap_or(8usize);
        let sessions = if demo {
            let fake = FakeSource::new();
            fake.fill(count);
            let ids: Vec<String> = fake.snapshot().iter().map(|s| s.session_id.clone()).collect();
            for (i, running) in [(0, true), (0, false), (3, true), (3, true), (4, false)] {
                if let Some(id) = ids.get(i) {
                    fake.add_sub(id, running);
                }
            }
            // One connector per look, so a demo render shows the whole set (#56).
            for (i, look) in pr::Look::ALL.into_iter().enumerate() {
                if let Some(id) = ids.get(i) {
                    fake.set_pr(id, Some(look));
                }
            }
            // One ticket status per lane, so both edges show their whole
            // vocabulary in a demo render (#57, #58).
            for (i, st) in ticket::Status::ALL.into_iter().enumerate() {
                if let Some(id) = ids.get(i) {
                    fake.set_ticket(id, Some(Some(st)));
                }
            }
            // One PR with CI in flight, for the yellow border (#62).
            if let Some(id) = ids.get(1) {
                fake.set_pr_running(id, true);
            }
            // One lane finished end to end, so the bold trace shows up in a
            // demo render (#61).
            if let Some(id) = ids.get(4) {
                fake.set_pr(id, Some(pr::Look::Merged));
                fake.set_ticket(id, Some(Some(ticket::Status::Done)));
                fake.set_phase(id, Phase::Idle);
            }
            fake.snapshot()
        } else {
            ClaudeSource::default().snapshot()
        };
        let palette = match args.iter().position(|a| a == "--theme").and_then(|k| args.get(k + 1)).map(|s| s.as_str()) {
            Some("orange") => theme::ORANGE,
            Some("dark") => theme::DARK,
            _ => theme::PALETTE,
        };
        let svg = render_svg(&sessions, w, h, demo, user_zoom, palette);
        std::fs::write(&out, svg).expect("write svg");
        println!("wrote {out} ({} sessions)", sessions.len());
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--icon") {
        // App icon: a square board with a single chip and no badge.
        let out = args.get(i + 1).cloned().unwrap_or_else(|| "icon.svg".into());
        let mut chip = SessionInfo::synthetic(1, "MAGI", model::Phase::Working);
        chip.subagents.push(model::SubagentInfo::synthetic(1, "AGENT", "", true));
        // Bold version of the board on the reference orange: few lanes, thick
        // traces, the chip near the centre.
        let zoom = 3.0;
        let layout = scene::Layout { zoom, width: 176.0, height: 1024.0 / zoom, bands: 1, lanes: scene::LEAD_LANES + 10 };
        let mut palette = theme::PALETTE;
        palette.bg = theme::hex(0xF04A0E);
        palette.packet = palette.line;
        let svg = render_frame(&[chip], layout, false, String::new(), (0.0, -26.0), palette, 1024.0, 1024.0);
        std::fs::write(&out, svg).expect("write icon svg");
        println!("wrote {out}");
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--focus") {
        let key = args.get(i + 1).cloned().unwrap_or_default().to_ascii_uppercase();
        let list = ClaudeSource::default().snapshot();
        match list.iter().find(|s| s.label() == key || s.pid.to_string() == key) {
            Some(s) => println!("{:?}", warp::focus(s)),
            None => eprintln!("no session matching {key:?}; try --list"),
        }
        return;
    }
    // Same idea as `--usage` (#48): check the PR resolution from the terminal
    // without opening a window (#56).
    if args.iter().any(|a| a == "--prs") {
        let source = ClaudeSource::eager();
        for s in source.snapshot() {
            let ticket = match &s.ticket {
                Some(t) => match t.status {
                    Some(st) => format!("{} {}", t.label(), st.short()),
                    None => format!("{} ?", t.label()),
                },
                None => "-".into(),
            };
            match &s.pr {
                Some(p) => println!(
                    "{:<16} {:<20} {} {:<7} {}✓ {}✗ {}⟳  {}",
                    s.label(),
                    ticket,
                    p.label(),
                    p.look().short(),
                    p.passed,
                    p.failed,
                    p.pending,
                    p.title
                ),
                None => println!("{:<16} {:<20} -", s.label(), ticket),
            }
        }
        return;
    }

    if args.iter().any(|a| a == "--usage") {
        match usage::fetch() {
            Ok(u) => {
                println!("{}", u.label(usage::now_secs()));
                println!("five_hour: {:?}\nseven_day: {:?}", u.five_hour, u.seven_day);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--list") {
        for s in ClaudeSource::default().snapshot() {
            println!(
                "{:<14} {:<12} {:<40} {:?} {:<18} {}",
                s.label(),
                s.phase().label(),
                s.short_cwd(),
                s.tty,
                s.group_label(),
                s.warp_focus_url.as_deref().unwrap_or("-")
            );
        }
        return;
    }

    Application::new().run(|cx: &mut App| {
        ui::register_fonts(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "claudmagi".into(),
            items: vec![MenuItem::action("Quit claudmagi", Quit)],
        }]);

        // CLAUDMAGI_WINDOW=WxH opens at a given size (memory / layout checks).
        let (win_w, win_h) = std::env::var("CLAUDMAGI_WINDOW")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
            .unwrap_or((scene::DESIGN_W, scene::DESIGN_H));
        let bounds = Bounds::centered(None, size(px(win_w), px(win_h)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(520.), px(340.))),
                focus: true,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Board::new(window, cx)),
        )
        .expect("open window");
        mac::style_frameless();

        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// Headless still frame with every animation settled.
fn render_svg(
    list: &[SessionInfo],
    width: f32,
    height: f32,
    demo_hover: bool,
    user_zoom: f32,
    palette: theme::Palette,
) -> String {
    let mut model = BoardModel::new();
    model.apply(list.to_vec(), Instant::now());
    let layout = scene::Layout::new(width, height, model.slot_span(), user_zoom);
    render_frame(list, layout, demo_hover, sources::machine_user(), (0.0, 0.0), palette, width, height)
}

#[allow(dead_code)]
fn render_svg_titled(list: &[SessionInfo], width: f32, height: f32, demo_hover: bool, title: String) -> String {
    let mut model = BoardModel::new();
    model.apply(list.to_vec(), Instant::now());
    let layout = scene::Layout::new(width, height, model.slot_span(), 1.0);
    render_frame(list, layout, demo_hover, title, (0.0, 0.0), theme::PALETTE, width, height)
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    list: &[SessionInfo],
    layout: scene::Layout,
    demo_hover: bool,
    title: String,
    pan: (f32, f32),
    palette: theme::Palette,
    width: f32,
    height: f32,
) -> String {
    let now = Instant::now();
    let mut model = BoardModel::new();
    model.apply(list.to_vec(), now);
    model.settle();
    if demo_hover && model.chips.len() > 1 {
        model.chips[1].anim.hover_t = 1.0;
    }
    let lanes = Rc::new(layout.build_lanes_for(&model.rows, now));
    let chips = scene::chip_draws(&model, &layout, &lanes, now);
    let prs = scene::pr_draws(&model, &layout, &lanes, &chips);
    let tickets = scene::ticket_draws(&model, &lanes, &chips);
    let done_lanes = scene::done_lanes(&model, &chips);
    let frame = scene::Frame {
        lanes,
        chips,
        prs,
        tickets,
        done_lanes,
        scroll_y: pan.1,
        t: 3.7,
        layout,
        title,
        pan_x: pan.0,
        palette,
    };
    let shapes = scene::build_shapes(&frame, geom::Pt::new(0.0, 0.0));
    render::svg::to_svg(&shapes, width, height)
}
