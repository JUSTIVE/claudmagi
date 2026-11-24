mod artifact;
mod cloud;
mod font;
mod host;
mod logos;
mod made;
mod mac;
mod model;
mod pr;
mod ink;
mod settings;
mod sources;
mod theme;
mod ticket;
mod ui;
mod usage;
mod zoom;


use std::process::ExitCode;

use nsh_platform_macos as platform;

use sources::{ClaudeSource, SessionSource};

fn main() -> ExitCode {
    // Before anything else, and before any thread exists to race it: a bundle
    // launched from Finder has no `gh` on its `PATH` (#66).
    unsafe { mac::widen_path() };
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--focus") {
        let key = args.get(i + 1).cloned().unwrap_or_default().to_ascii_uppercase();
        let list = ClaudeSource::default().snapshot();
        match list.iter().find(|s| s.label() == key || s.pid.to_string() == key) {
            Some(s) => println!("{:?}", host::focus(s)),
            None => eprintln!("no session matching {key:?}; try --list"),
        }
        return ExitCode::SUCCESS;
    }
    // Same idea as `--usage` (#48): check the PR resolution from the terminal
    // without opening a window (#56).
    if args.iter().any(|a| a == "--prs") {
        let source = ClaudeSource::eager();
        for s in source.snapshot() {
            let prs: Vec<String> = s
                .prs
                .iter()
                .map(|p| format!("{} {} {}✓{}✗{}⟳", p.label(), p.look().short(), p.passed, p.failed, p.pending))
                .collect();
            let ticket = match &s.ticket {
                Some(t) => match t.status {
                    Some(st) => format!("{} {}", t.label(), st.short()),
                    None => format!("{} ?", t.label()),
                },
                None => "-".into(),
            };
            // The pages it published share the cell's output line with the
            // pull requests (#137), so they share this line too.
            let pages: Vec<String> = s
                .artifacts
                .iter()
                .map(|a| format!("{} {}", a.version_label().unwrap_or_else(|| "·".into()), a.label()))
                .collect();
            println!(
                "{:<16} {:<20} {:<40} {}",
                s.label(),
                ticket,
                if prs.is_empty() { "-".to_string() } else { prs.join("  ·  ") },
                if pages.is_empty() { "-".to_string() } else { pages.join("  ·  ") }
            );
        }
        return ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--usage") {
        match usage::fetch() {
            Ok(u) => {
                println!("{}", u.label(usage::now_secs()));
                println!("five_hour: {:?}\nseven_day: {:?}", u.five_hour, u.seven_day);
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--list") {
        let source = ClaudeSource::default();
        let list = source.snapshot();
        // A terminal runs under its own terminal app's Full Disk Access, so
        // this line is the bundle's symptom showing up where it can be read
        // (#67).
        if let Some(note) = source.note() {
            eprintln!("{note}");
        }
        for s in list {
            println!(
                "{:<14} {:<12} {:<40} {:?} {:<18} {}",
                s.label(),
                s.phase().label(),
                s.short_cwd(),
                s.tty,
                s.group_label(),
                // What the host gave us to get back there, which is the one
                // thing `--list` can show about a seat that the group key
                // does not already say (#118).
                s.seat
                    .as_ref()
                    .and_then(|seat| seat.focus_url.clone().or_else(|| (!seat.handle.is_empty()).then(|| seat.handle.clone())))
                    .unwrap_or_else(|| "-".into())
            );
        }
        return ExitCode::SUCCESS;
    }

    // CLAUDMAGI_WINDOW=WxH opens at a given size (memory / layout checks).
    let (win_w, win_h) = std::env::var("CLAUDMAGI_WINDOW")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
        .unwrap_or((zoom::DESIGN_W as i32, zoom::DESIGN_H as i32));
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<ui::app::App>()),
        platform::AppOptions {
            title: "claudmagi".to_owned(),
            app_id: "io.github.justive.claudmagi".to_owned(),
            size: (win_w, win_h),
            ..Default::default()
        },
    )
}
