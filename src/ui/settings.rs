//! Settings panel (⌘, or the settings button): board background, Metro
//! accent, UI size and how many subagent tiles a session shows.

use nsh_core::prelude::*;
use nsh_metro::{Accent, label, palette, radio_button, slider};

use crate::settings::{AGENTS_MAX, AGENTS_MIN, BoardTheme};
use crate::ui::metro;
use crate::ui::panel::{Which, button, flyout, hint, section};
use crate::ui::state::{Board, Ui};
use crate::zoom;

const ZOOM_PRESETS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
/// One block of small tiles, two, three, four.
const AGENT_PRESETS: [usize; 4] = [4, 8, 12, 16];
/// The twenty accents as a 10 × 2 grid of swatches, the way the phone's
/// accent picker laid them out as tiles.
const SWATCH: f32 = 26.0;
const SWATCH_GAP: f32 = 2.0;

/// Zoom ↔ slider position (0–1), linear over the allowed range.
fn zoom_to_unit(zoom: f32) -> f32 {
    ((zoom - zoom::USER_ZOOM_MIN) / (zoom::USER_ZOOM_MAX - zoom::USER_ZOOM_MIN)).clamp(0.0, 1.0)
}

fn unit_to_zoom(v: f32) -> f32 {
    zoom::USER_ZOOM_MIN + v.clamp(0.0, 1.0) * (zoom::USER_ZOOM_MAX - zoom::USER_ZOOM_MIN)
}

pub fn set_theme(b: &mut Board, theme: BoardTheme) {
    if b.settings.theme != theme {
        b.settings.theme = theme;
        b.settings.save();
        // The panels follow the board: dark board, dark Metro ground.
        metro::sync(&b.settings);
    }
}

/// Subagent count ↔ slider position (0–1), one step per tile.
fn agents_to_unit(n: usize) -> f32 {
    (n.clamp(AGENTS_MIN, AGENTS_MAX) - AGENTS_MIN) as f32 / (AGENTS_MAX - AGENTS_MIN) as f32
}

fn unit_to_agents(v: f32) -> usize {
    AGENTS_MIN + (v.clamp(0.0, 1.0) * (AGENTS_MAX - AGENTS_MIN) as f32).round() as usize
}

pub fn set_agents(b: &mut Board, n: usize) {
    let n = n.clamp(AGENTS_MIN, AGENTS_MAX);
    if b.settings.agents != n {
        b.settings.agents = n;
        b.settings.save();
    }
}

pub fn set_accent(b: &mut Board, accent: Accent) {
    if b.settings.accent != accent.name() {
        b.settings.accent = accent.name().into();
        b.settings.save();
        metro::sync(&b.settings);
    }
}

pub fn panel(ui: &Ui) -> AnyElement {
    flyout(ui, Which::Settings, nsh_metro::FlyoutWidth::Narrow, |_| "settings".into(), body)
}

fn body(ui: &Ui) -> AnyElement {
    let mut themes = view();
    for t in BoardTheme::ALL {
        let (on, set) = (ui.memo(move |b| b.settings.theme == t), ui.clone());
        themes = themes.child(
            radio_button(move || on.get())
                .label(t.label().to_lowercase())
                .on_select(move || set.update(|b| set_theme(b, t))),
        );
    }

    let mut swatches = view().flex_row().flex_wrap().gap(px(SWATCH_GAP)).w(px(10.0 * SWATCH + 9.0 * SWATCH_GAP));
    for a in Accent::ALL {
        let (on, set) = (ui.memo(move |b| metro::accent_of(&b.settings.accent) == a), ui.clone());
        let chosen = move || on.get();
        swatches = swatches.child(
            view()
                .size(px(SWATCH))
                .bg(a.color())
                .cursor_pointer()
                .border(px(2.))
                .border_color(move || if chosen() { palette().foreground } else { Hsla::transparent_black() })
                .hover(|s| s.opacity(0.8))
                .on_press(move || set.update(|b| set_accent(b, a))),
        );
    }
    let current = {
        let name = ui.memo(|b| metro::accent_of(&b.settings.accent).name());
        move || SharedString::from(name.get())
    };

    let mut presets = view().flex_row().flex_wrap().gap(px(4.));
    for z in ZOOM_PRESETS {
        let (on, set) = (ui.memo(move |b| (b.settings.zoom - z).abs() < 0.01), ui.clone());
        presets = presets.child(button(
            format!("{:.0}%", z * 100.0),
            move || on.get(),
            move || set.update(|b| b.set_zoom(z)),
        ));
    }
    let (zoom_now, set) = (ui.memo(|b| (b.settings.zoom * 1000.0).round() as i32), ui.clone());
    let zoom_slider = slider(move || zoom_to_unit(zoom_now.get() as f32 / 1000.0))
        .header(move || SharedString::from(format!("{:.0}%", zoom_now.get() as f32 / 10.0)))
        .on_change(move |v| set.update(|b| b.set_zoom(unit_to_zoom(v))));

    let mut agent_presets = view().flex_row().flex_wrap().gap(px(4.));
    for n in AGENT_PRESETS {
        let (on, set) = (ui.memo(move |b| b.settings.agents() == n), ui.clone());
        agent_presets = agent_presets.child(button(n.to_string(), move || on.get(), move || set.update(|b| set_agents(b, n))));
    }
    let (agents_now, set) = (ui.memo(|b| b.settings.agents()), ui.clone());
    let agents_slider = slider(move || agents_to_unit(agents_now.get()))
        .header(move || SharedString::from(format!("{} per session", agents_now.get())))
        .on_change(move |v| set.update(|b| set_agents(b, unit_to_agents(v))));

    // The way back for anything put away by hand (#194). Only when there is
    // something to bring back.
    let put_away = {
        let ui = ui.clone();
        dynamic(move || {
            let n = ui.read(|b| b.settings.dismissed.len());
            if n == 0 {
                return view().into_element();
            }
            let back = ui.clone();
            section("put away")
                .child(hint(format!("{n} merged pull request(s) hidden")))
                .child(button("bring back", || false, move || back.update(crate::ui::tiles::restore_dismissed)))
                .into_element()
        })
    };

    view()
        .child(section("background").child(themes))
        .child(
            section("accent")
                .child(swatches)
                .child(label(current).text_size(px(metro::BODY)).text_color(move || palette().accent)),
        )
        .child(section("ui size").child(zoom_slider).child(presets))
        .child(section("subagent tiles").child(agents_slider).child(agent_presets))
        .child(put_away)
        .child(hint("⌘+ ⌘− ⌘0 · ⌘+wheel zoom · ⌘, toggles this panel · settings persist"))
        .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_slider_maps_the_whole_range() {
        assert_eq!(zoom_to_unit(zoom::USER_ZOOM_MIN), 0.0);
        assert_eq!(zoom_to_unit(zoom::USER_ZOOM_MAX), 1.0);
        for z in ZOOM_PRESETS {
            assert!((unit_to_zoom(zoom_to_unit(z)) - z).abs() < 1e-5);
        }
    }

    #[test]
    fn agents_slider_steps_through_every_count() {
        assert_eq!(unit_to_agents(0.0), AGENTS_MIN);
        assert_eq!(unit_to_agents(1.0), AGENTS_MAX);
        for n in AGENTS_MIN..=AGENTS_MAX {
            assert_eq!(unit_to_agents(agents_to_unit(n)), n);
        }
    }
}
