//! The window: the start screen, the status
//! bar along its bottom, the panels down its right edge, and the clocks that
//! feed them (#195).

use std::time::{Duration, Instant};

use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::ImageSrc;
use nsh_core::element::desktop::{main_window, quit};
use nsh_core::prelude::*;
use nsh_metro::{label, palette as metro_palette};

use crate::settings::BoardTheme;
use crate::zoom;
use crate::theme;
use crate::ui::metro::BODY;
use crate::ui::panel::{self, Which};
use crate::ui::state::{Board, Mode, STATUS_H, Ui};
use crate::inbox;
use crate::usage;

/// Session registry poll interval.
const POLL_MS: u64 = 1000;
/// How often the model ages between polls: arrivals settle, departures go.
const AGE_MS: u64 = 250;

#[component]
pub fn App() -> impl IntoElement {
    // The embedded face the chrome is set in (#55), before anything is
    // built in it.
    let _ = register_font(crate::font::FONT_TTF.to_vec());
    let board = Board::new();
    crate::ui::metro::init(&board.settings);
    let ui = Ui::new(board);
    open_window();
    poll_sessions(ui.clone());
    poll_usage(ui.clone());
    poll_inbox(ui.clone());
    run_clock(ui.clone());

    let keys = ui.clone();
    let wheel = ui.clone();
    let sized = ui.clone();
    let bg = ui.clone();
    view()
        .flex_1()
        .relative()
        .overflow_hidden()
        .bg(move || bg.read(|b| theme::hsla(b.palette().bg)))
        .focusable(true)
        .on_layout(move |r: &Bounds<Points>| {
            let (w, h) = (f32::from(r.size.width), f32::from(r.size.height));
            if sized.peek(|b| b.window) != (w, h) {
                sized.update(|b| {
                    b.resize(w, h);
                    b.frame();
                });
            }
        })
        .on_key_down(move |ev: &KeyEvent| on_key(&keys, ev))
        .on_wheel(move |ev: &WheelEvent| {
            if ev.modifiers.cmd {
                // ⌘ + wheel zooms, like a browser.
                let dy = wheel_y(ev);
                let factor = (1.0 - dy / 300.0).clamp(0.8, 1.25);
                wheel.update(|b| b.zoom_by(factor));
            }
        })
        .child(board_view(ui.clone()))
        .child(status_bar(ui.clone()))
        // The titlebar strip moves the window as any Mac window's does
        // (#171, #185); the traffic lights sit in it.
        .child(view().absolute().top_0().left_0().right_0().h(px(crate::ui::state::GRAB_H)).bg(Hsla::transparent_black()).window_drag(true))
        // Drawn after the strip so the strip's drag does not eat its press.
        .child(inbox_badge(ui.clone()))
        .child(panel::flyouts(ui.clone()))
        .child(crate::ui::tiles::menu(ui))
}

/// What fills a switched-on control in the chrome, and what reads on it.
/// Metro's accent, except on the orange board where an orange accent would
/// vanish into the board and the ink fills instead.
fn chrome_fill(b: &Board) -> (Hsla, Hsla) {
    let metro = metro_palette();
    if b.settings.theme == BoardTheme::Orange {
        (theme::hsla(b.palette().ink), theme::hsla(b.palette().bg))
    } else {
        (metro.accent, metro.accent_ink)
    }
}

/// What Linear is asking of you, at the top of the window (#242).
///
/// Not the status bar: that bar counts **the board's own** sessions, and the
/// inbox is not the board's. Up here it sits opposite the traffic lights and
/// across from the `start` header, where no tile ever reaches. Nothing at
/// all when the inbox is clear — a zero would be one more number to read
/// past every time the board is glanced at.
///
/// A press opens **the inbox panel** (#243, #244): magi is what shows what
/// is being asked of you, and Linear is opened to go and do something about
/// one of them — which is what a press on a row does, straight to that
/// comment (#241).
fn inbox_badge(ui: Ui) -> AnyElement {
    dynamic(move || {
        let (inbox, fill, ink) = ui.read(|b| {
            let (fill, ink) = chrome_fill(b);
            (b.inbox.clone(), fill, ink)
        });
        let open = ui.clone();
        view()
            .absolute()
            .top(px(7.))
            .right(px(14.))
            .children((!inbox.is_empty()).then(|| {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .px(px(6.))
                    .py(px(2.))
                    .bg(fill)
                    .cursor_pointer()
                    .hover(move |s| s.opacity(0.8))
                    .on_press(move || open.update(|b| panel::toggle(b, Which::Inbox)))
                    .child(
                        svg(ImageSrc::svg_markup(crate::logos::markup(crate::logos::Mark::Linear)))
                            .size(px(11.))
                            .flex_shrink_0()
                            .text_color(ink),
                    )
                    .child(label(inbox.len().to_string()).text_size(px(BODY)).text_color(ink))
            }))
            .into_element()
    })
    .into_element()
}

/// The main window as the board wants it: no system title bar, the board
/// drawn up to the top edge under the traffic lights (#167), and no smaller
/// than the status bar and a column of tiles need.
fn open_window() {
    let mut title_bar = TitleBarOptions::default();
    title_bar.style = TitleBarStyle::Hidden;
    let mut attributes = WindowAttributes::default();
    attributes.title_bar = Some(title_bar);
    attributes.min_size = Some(Some(size(px(520.), px(340.))));
    main_window().set(attributes);
}

/// The board itself: the start screen, above the status bar.
fn board_view(ui: Ui) -> AnyElement {
    view()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom(px(STATUS_H))
        .child(crate::ui::tiles::start_screen(ui))
        .into_element()
}

fn wheel_y(ev: &WheelEvent) -> f32 {
    match ev.delta {
        WheelDelta::Points(p) => f32::from(p.y),
        WheelDelta::Lines(l) => l.y * zoom::GAP,
    }
}

/// The board's keys (#71, #103, #163).
fn on_key(ui: &Ui, ev: &KeyEvent) {
    let cmd = ev.modifiers.cmd;
    let key = ev.key.as_ref();
    // ⌘K opens and closes the search; while it is open its field has the
    // keys (#71).
    if cmd && key.eq_ignore_ascii_case("k") {
        ui.update(|b| crate::ui::palette::toggle(b));
        return;
    }
    if ui.peek(|b| b.palette.open) {
        crate::ui::palette::key(ui, ev);
        return;
    }
    let Some(command) = command(key, cmd) else { return };
    let quit = ui.update(|b| {
        match command {
            Command::Panel(which) => panel::toggle(b, which),
            Command::Zoom(factor) => b.zoom_by(factor),
            // Esc puts away whatever is up, the topmost first, and quits
            // when nothing is.
            Command::Back if b.tile_menu.is_some() => b.tile_menu = None,
            Command::Back if b.inbox_panel.open => b.inbox_panel.open = false,
            Command::Back if b.list_panel.open => b.list_panel.open = false,
            Command::Back if b.settings_panel.open => b.settings_panel.open = false,
            Command::Back if b.dev.open => b.dev.open = false,
            Command::Back => return true,
        }
        false
    });
    if quit {
        quit_app();
    }
}

/// What a key does on the board, when the search is not open.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Panel(Which),
    /// Multiply the zoom by this, or reset it at 0.
    Zoom(f32),
    /// Esc.
    Back,
}

fn command(key: &str, cmd: bool) -> Option<Command> {
    Some(match key {
        "l" if cmd => Command::Panel(Which::List),
        // ⌘A: what Linear is asking of you (#244).
        "a" if cmd => Command::Panel(Which::Inbox),
        // ⌘T, or T alone as it always was (#202).
        "t" | "T" => Command::Panel(Which::Dev),
        "," if cmd => Command::Panel(Which::Settings),
        "=" | "+" => Command::Zoom(zoom::USER_ZOOM_STEP),
        "-" | "_" => Command::Zoom(1.0 / zoom::USER_ZOOM_STEP),
        "0" if cmd => Command::Zoom(0.0),
        "Escape" => Command::Back,
        _ => return None,
    })
}

fn quit_app() {
    quit();
}

/// The session registry, once a second, read off the engine thread.
fn poll_sessions(ui: Ui) {
    spawn(async move {
        loop {
            let (source, generation) = ui.peek(|b| (b.source(), b.generation));
            let read = background(async move { (source.snapshot(), source.note()) }).await;
            if let Ok((list, note)) = read {
                ui.update(|b| b.take_polled(generation, list, note));
            }
            timer(Duration::from_millis(POLL_MS)).await;
        }
    });
}

/// Plan usage for the status bar (#48): keychain + curl off the engine
/// thread, once a minute. A missing login is final.
fn poll_usage(ui: Ui) {
    spawn(async move {
        loop {
            let Ok(result) = background(async move { usage::fetch() }).await else { break };
            let now = Instant::now();
            let again = !matches!(result, Err(usage::UsageError::NoCredentials));
            ui.update(|b| match result {
                Ok(u) => {
                    b.usage = Some((u, now));
                    b.usage_error = None;
                }
                Err(e) => b.usage_error = Some(e),
            });
            if !again {
                break;
            }
            timer(usage::POLL).await;
        }
    });
}

/// What Linear is asking of you (#241): keychain + curl off the engine
/// thread, once a minute. A missing key is not final the way a missing
/// Claude login is — the key is usually put in while the board is already
/// up, and the next poll is sixty seconds away.
fn poll_inbox(ui: Ui) {
    spawn(async move {
        loop {
            let Ok(result) = background(async move { inbox::fetch() }).await else { break };
            ui.update(|b| {
                match result {
                    Ok(notes) => {
                        b.inbox = notes;
                        b.inbox_error = None;
                    }
                    // One shaky minute does not empty the inbox (#59). No
                    // key at all is different: there is nothing to keep.
                    Err(e) => {
                        if e == inbox::Error::NoKey {
                            b.inbox = Default::default();
                        }
                        b.inbox_error = Some(e);
                    }
                }
                crate::ui::palette::refresh(b);
            });
            timer(inbox::POLL).await;
        }
    });
}

/// Ages the model between polls.
fn run_clock(ui: Ui) {
    spawn(async move {
        loop {
            ui.quietly(|b| b.frame());
            timer(Duration::from_millis(AGE_MS)).await;
        }
    });
}

/// The status bar: the counts, the mode, the source's complaint, the latest
/// notice, the plan usage and the two panel buttons.
fn status_bar(ui: Ui) -> AnyElement {
    // The status bar stays on the board's ground and in its ink, so it reads
    // on all three boards; Metro gives it the type, the square controls and
    // the accent that fills whatever is switched on. On the orange board an
    // orange accent would vanish, so ink fills there.
    let colors = {
        let ui = ui.clone();
        move || {
            ui.read(|b| {
                let pal = b.palette();
                let ink = theme::hsla(pal.ink);
                let on_orange = b.settings.theme == BoardTheme::Orange;
                let metro = metro_palette();
                let fill = if on_orange { ink } else { metro.accent };
                let fill_ink = if on_orange { theme::hsla(pal.bg) } else { metro.accent_ink };
                (pal, ink, fill, fill_ink)
            })
        }
    };
    let left = {
        let (ui, colors) = (ui.clone(), colors.clone());
        dynamic(move || {
            let (pal, ink, fill, fill_ink) = colors();
            let (summary, mode, note) = ui.read(|b| (b.model.summary(), b.mode, b.source_note));
            let fda = ui.clone();
            view()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(px(8.))
                .child(label(summary).text_size(px(BODY)).text_color(ink))
                .children((mode == Mode::Sandbox).then(|| {
                    view().px(px(6.)).bg(fill).child(label("SANDBOX").text_size(px(BODY)).text_color(fill_ink))
                }))

                // The board looks right when Warp's tabs are unreadable, so
                // the only way to tell is to say it (#67).
                .children(note.map(|note| {
                    view()
                        .px(px(6.))
                        .bg(theme::hsla(pal.alarm))
                        .cursor_pointer()
                        .hover(move |s| s.opacity(0.75))
                        .on_press(move || fda.open_full_disk_access())
                        .child(label(note).text_size(px(BODY)).text_color(theme::hsla(pal.on_alarm)))
                }))
                .into_element()
        })
    };
    let detail = {
        let (ui, colors) = (ui.clone(), colors.clone());
        label(move || SharedString::from(ui.read(|b| b.model.detail())))
            .flex_1()
            .text_size(px(BODY))
            .text_right()
            .truncate()
            .text_color(move || colors().1)
    };
    let right = {
        let (ui, colors) = (ui.clone(), colors.clone());
        dynamic(move || {
            let (pal, ink, fill, fill_ink) = colors();
            let (usage, settings_open, dev_open) =
                ui.read(|b| (b.usage_now(Instant::now()), b.settings_panel.open, b.dev.open));
            let (s, d) = (ui.clone(), ui.clone());
            view()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(px(4.))
                .children(usage.map(|(u, stale)| {
                    let secs = usage::now_secs();
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .child(usage_meter("5H", u.five_hour, secs, pal, fill, stale))
                        .child(usage_meter("7D", u.seven_day, secs, pal, fill, stale))
                }))
                .child(status_button("settings", settings_open, (ink, fill, fill_ink), move || {
                    s.update(|b| panel::toggle(b, Which::Settings))
                }))
                .child(status_button("test", dev_open, (ink, fill, fill_ink), move || {
                    d.update(|b| panel::toggle(b, Which::Dev))
                }))
                .into_element()
        })
    };
    let bg = colors.clone();
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(px(STATUS_H))
        .px(px(18.))
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .bg(move || theme::hsla(bg().0.bg))
        // A press on the bar's bare stretch moves the window (#170).
        .window_drag(true)
        .child(left)
        .child(detail)
        .child(right)
        .into_element()
}

/// One plan-usage window as a bar (#52): a Metro progress bar, square, the
/// accent filling it; a stale reading fades the whole meter.
fn usage_meter(name: &'static str, w: usage::Window, now: u64, palette: theme::Palette, fill: Hsla, stale: bool) -> View {
    const TRACK_W: f32 = 44.0;
    let frac = (w.utilization / 100.0).clamp(0.0, 1.0);
    let fade = if stale { 0.45 } else { 1.0 };
    let ink = move |a: f32| theme::hsla(theme::with_alpha(palette.ink, a * fade));
    view()
        .flex_row()
        .items_center()
        .gap(px(4.))
        .child(label(name).text_size(px(BODY)).text_color(ink(0.55)))
        .child(
            view()
                .flex_shrink_0()
                .w(px(TRACK_W))
                .h(px(4.))
                .bg(ink(0.18))
                .child(view().w(px(TRACK_W * frac)).h(px(4.)).bg(fill.opacity(fade))),
        )
        .child(label(format!("{}%", w.utilization.round() as i64)).text_size(px(BODY)).text_color(ink(0.9)))
        .children(w.resets_at.map(|r| {
            label(format!("\u{21bb}{}", usage::countdown(r, now))).text_size(px(BODY)).text_color(ink(0.55))
        }))
}

/// A flat Metro button for the status bar: a 2 px ink border, filled while
/// its panel is open and while it is pressed. `colors` is (ink, fill, the ink
/// that reads on the fill).
fn status_button(text: &'static str, active: bool, colors: (Hsla, Hsla, Hsla), on_press: impl Fn() + 'static) -> View {
    let (ink, fill, fill_ink) = colors;
    view()
        .flex_shrink_0()
        .px(px(8.))
        .border(px(2.))
        .border_color(if active { fill } else { ink })
        .bg(if active { fill } else { Hsla::transparent_black() })
        .cursor_pointer()
        .active(move |s| s.bg(fill))
        .on_press(move || on_press())
        .child(label(text).text_size(px(BODY)).text_color(if active { fill_ink } else { ink }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⌘T opens the test tools, as T alone always did (#202); the other
    /// panels keep their keys, and a bare letter that is not one does
    /// nothing.
    #[test]
    fn the_board_keys() {
        assert_eq!(command("t", true), Some(Command::Panel(Which::Dev)), "⌘T");
        assert_eq!(command("t", false), Some(Command::Panel(Which::Dev)), "T alone");
        assert_eq!(command("T", false), Some(Command::Panel(Which::Dev)), "with Shift held");
        assert_eq!(command("l", true), Some(Command::Panel(Which::List)), "⌘L");
        assert_eq!(command(",", true), Some(Command::Panel(Which::Settings)), "⌘,");
        assert_eq!(command("0", true), Some(Command::Zoom(0.0)), "⌘0 resets the zoom");
        assert_eq!(command("Escape", false), Some(Command::Back));
        assert_eq!(command("l", false), None, "L alone is not ⌘L");
        assert_eq!(command("0", false), None, "nor 0 ⌘0");
        assert_eq!(command("x", false), None);
    }
}
