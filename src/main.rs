mod board;
mod font;
mod geom;
mod mac;
mod scene;
mod sessions;
mod theme;
mod warp;

use std::rc::Rc;

use gpui::{
    App, Application, Bounds, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};

use board::Board;

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
            .unwrap_or((980.0, 620.0));
        let sessions = if demo { demo_sessions() } else { sessions::read_sessions() };
        let svg = render_svg(&sessions, w, h);
        std::fs::write(&out, svg).expect("write svg");
        println!("wrote {out} ({} sessions)", sessions.len());
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--focus") {
        let key = args.get(i + 1).cloned().unwrap_or_default().to_ascii_uppercase();
        let list = sessions::read_sessions();
        match list.iter().find(|s| s.label() == key || s.pid.to_string() == key) {
            Some(s) => println!("{:?}", warp::focus(s)),
            None => eprintln!("no session matching {key:?}; try --list"),
        }
        return;
    }
    if args.iter().any(|a| a == "--list") {
        for s in sessions::read_sessions() {
            println!(
                "{:<14} {:<10} {:<40} {:?} {}",
                s.label(),
                format!("{:?}", s.phase()),
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

        let bounds = Bounds::centered(None, size(px(980.), px(620.)), cx);
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
fn render_svg(list: &[sessions::SessionInfo], width: f32, height: f32) -> String {
    let lanes = Rc::new(scene::build_lanes(scene::lane_count(list.len()), width));
    let chips = list
        .iter()
        .enumerate()
        .map(|(k, info)| {
            let lane = 1 + k;
            let on_diag = k % 2 == 0;
            let label = info.label();
            let w = scene::chip_width(&label);
            let s_c = scene::chip_anchor(&lanes[lane], lane, on_diag, width, w);
            let p = if info.phase() == sessions::Phase::Working { 0.0 } else { 1.0 };
            let (pos, tan) = lanes[lane].point_at(s_c + p * scene::PULL);
            scene::ChipDraw {
                lane,
                s_c,
                width: w,
                label,
                p,
                hover: if k == 1 { 1.0 } else { 0.0 },
                alpha: 1.0,
                phase: info.phase(),
                center: pos,
                tangent: tan,
            }
        })
        .collect();
    let frame = scene::Frame { lanes, chips, scroll_y: 0.0, t: 3.7, width, height };
    let shapes = scene::build_shapes(&frame, geom::Pt::new(0.0, 0.0));
    scene::to_svg(&shapes, width, height)
}

fn demo_sessions() -> Vec<sessions::SessionInfo> {
    let mk = |pid: i32, name: &str, cwd: &str, status: &str, waiting: Option<&str>| sessions::SessionInfo {
        pid,
        session_id: format!("demo-{pid}"),
        cwd: cwd.into(),
        name: name.into(),
        status: status.into(),
        waiting_for: waiting.map(Into::into),
        state: None,
        tempo: None,
        detail: None,
        started_at: pid as u64,
        status_updated_at: 0,
        kind: "interactive".into(),
        version: None,
        tty: Some("ttys001".into()),
        warp_focus_url: None,
        warp_session_uuid: None,
    };
    vec![
        mk(1, "personal-71", "/Users/ben/git/personal", "busy", None),
        mk(2, "yoshi-d3", "/Users/ben/git/cookieplace/crepe/yoshi", "idle", None),
        mk(3, "crepe-2f", "/Users/ben/git/cookieplace/crepe", "busy", Some("dialog open")),
        mk(4, "flxtra-08", "/Users/ben/git/personal/flxtra", "busy", None),
        mk(5, "trailblazer-a1", "/Users/ben/git/personal/trailblazer", "idle", None),
        mk(6, "morphing-sheet-4", "/Users/ben/git/personal/morphing-sheet", "busy", Some("input needed")),
        mk(7, "gql-9c", "/Users/ben/git/personal/GompassQL", "busy", None),
        mk(8, "djdeck-31", "/Users/ben/git/personal/djdeck", "idle", None),
    ]
}
