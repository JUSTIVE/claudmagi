//! User settings (theme, zoom), persisted as JSON under the app support dir.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardTheme {
    #[default]
    White,
    Orange,
    Dark,
}

impl BoardTheme {
    pub const ALL: [BoardTheme; 3] = [BoardTheme::White, BoardTheme::Orange, BoardTheme::Dark];

    pub fn label(self) -> &'static str {
        match self {
            BoardTheme::White => "WHITE",
            BoardTheme::Orange => "ORANGE",
            BoardTheme::Dark => "DARK",
        }
    }
}

/// Which way the sessions are drawn (#86).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardView {
    /// Chips riding traces: the circuit board this started as.
    #[default]
    Circuit,
    /// A honeycomb of cells, one per session, clustered by Warp tab.
    Comb,
}

impl BoardView {
    pub const ALL: [BoardView; 2] = [BoardView::Circuit, BoardView::Comb];

    pub fn label(self) -> &'static str {
        match self {
            BoardView::Circuit => "CIRCUIT",
            BoardView::Comb => "COMB",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: BoardTheme,
    /// Board zoom factor (0.5 – 3.0).
    pub zoom: f32,
    pub view: BoardView,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: BoardTheme::White, zoom: 1.0, view: BoardView::Circuit }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("claudmagi").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Best-effort write; the board keeps working if the disk says no.
    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_tolerate_missing_fields() {
        let s = Settings { theme: BoardTheme::Dark, zoom: 1.5, view: BoardView::Comb };
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), s);
        let partial: Settings = serde_json::from_str(r#"{"theme":"orange"}"#).unwrap();
        assert_eq!(partial.theme, BoardTheme::Orange);
        assert_eq!(partial.zoom, 1.0);
        assert_eq!(partial.view, BoardView::Circuit, "an old settings file still opens on the circuit");
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
    }
}
