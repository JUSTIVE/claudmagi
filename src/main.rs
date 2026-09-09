mod font;
mod geom;
mod mac;
mod model;
mod render;
mod sources;
mod theme;
mod ui;
mod warp;

use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};

use model::{BoardModel, SessionInfo};
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
        let (w, h) = args
            .iter()
            .position(|a| a == "--size")
            .and_then(|k| args.get(k + 1))
            .and_then(|s| s.split_once('x'))
            .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
            .unwrap_or((scene::DESIGN_W, scene::DESIGN_H));
        let sessions = if demo {
            let fake = FakeSource::new();
            fake.fill(8);
            fake.snapshot()
        } else {
            ClaudeSource.snapshot()
        };
        let svg = render_svg(&sessions, w, h, demo);
        std::fs::write(&out, svg).expect("write svg");
        println!("wrote {out} ({} sessions)", sessions.len());
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--focus") {
        let key = args.get(i + 1).cloned().unwrap_or_default().to_ascii_uppercase();
        let list = ClaudeSource.snapshot();
        match list.iter().find(|s| s.label() == key || s.pid.to_string() == key) {
            Some(s) => println!("{:?}", warp::focus(s)),
            None => eprintln!("no session matching {key:?}; try --list"),
        }
        return;
    }
    if args.iter().any(|a| a == "--list") {
        for s in ClaudeSource.snapshot() {
            println!(
                "{:<14} {:<12} {:<40} {:?} {}",
                s.label(),
                s.phase().label(),
                s.short_cwd(),
                s.tty,
                s.warp_focus_url.as_deref().unwrap_or("-")
            );
        }
        return;
    }

    Application::new().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "claudmagi".into(),
            items: vec![MenuItem::action("Quit claudmagi", Quit)],
        }]);

        let bounds = Bounds::centered(None, size(px(scene::DESIGN_W), px(scene::DESIGN_H)), cx);
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
fn render_svg(list: &[SessionInfo], width: f32, height: f32, demo_hover: bool) -> String {
    let now = Instant::now();
    let mut model = BoardModel::new();
    model.apply(list.to_vec(), now);
    model.settle();
    if demo_hover && model.chips.len() > 1 {
        model.chips[1].hover_t = 1.0;
    }
    let layout = scene::Layout::new(width, height, model.chips.len());
    let lanes = Rc::new(layout.build_lanes());
    let chips = scene::chip_draws(&model, &layout, &lanes, now);
    let frame = scene::Frame { lanes, chips, scroll_y: 0.0, t: 3.7, layout };
    let shapes = scene::build_shapes(&frame, geom::Pt::new(0.0, 0.0));
    render::svg::to_svg(&shapes, width, height)
}
