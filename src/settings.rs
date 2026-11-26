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

/// Which way the sessions are drawn (#86). The circuit board this started
/// as and the metaballs went in #183: the start screen and the honeycomb are
/// what is left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardView {
    /// A honeycomb of cells, one per session, clustered by the window
    /// their host seats them in — a Warp tab, an Orca tab (#118). This build
    /// draws only the start screen, but the choice is still read and written
    /// back: settings.json is shared with the build that has the comb (#186).
    Comb,
    /// The Metro start screen: a live tile per session, grouped by the work
    /// it is on (#157, #176).
    #[default]
    Tiles,
}

/// A view that is gone (`circuit`, `blob`) or unknown opens on the default
/// rather than failing the whole file: a settings file that does not parse
/// is thrown away, theme and accent with it.
impl<'de> Deserialize<'de> for BoardView {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "comb" => BoardView::Comb,
            _ => BoardView::Tiles,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: BoardTheme,
    /// Board zoom factor (0.5 – 3.0).
    pub zoom: f32,
    pub view: BoardView,
    /// Metro accent for the panels and the status bar, by name ("orange",
    /// "cobalt", …). An unknown name falls back to the default.
    pub accent: String,
    /// Pull requests put away by hand (#194), by url.
    ///
    /// The url rather than the number: a number belongs to a repository, and
    /// two repositories have a #1. It is kept rather than forgotten on quit
    /// because what gets put away is work that has landed — a merged pull
    /// request is not going to change its mind and want the board back.
    pub dismissed: Vec<String>,
    /// How many subagent tiles a session shows in its group before the last
    /// one stands for the rest as `+N` (#253, #254). Read through
    /// [`Settings::agents`], which keeps a hand-edited file in range.
    pub agents: usize,
}

pub const DEFAULT_ACCENT: &str = "orange";

/// The subagent tiles a session may show: at least one (which, past it,
/// counts them all), at most four blocks of four.
pub const AGENTS_MIN: usize = 1;
pub const AGENTS_MAX: usize = 16;
/// Four small tiles fill one medium tile's room, so eight are two of them.
pub const DEFAULT_AGENTS: usize = 8;

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: BoardTheme::White,
            zoom: 1.0,
            view: BoardView::Tiles,
            accent: DEFAULT_ACCENT.into(),
            dismissed: Vec::new(),
            agents: DEFAULT_AGENTS,
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("claudmagi").join("settings.json"))
}

impl Settings {
    /// [`Settings::agents`] the field, held to [`AGENTS_MIN`]–[`AGENTS_MAX`].
    pub fn agents(&self) -> usize {
        self.agents.clamp(AGENTS_MIN, AGENTS_MAX)
    }

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
        let s = Settings {
            theme: BoardTheme::Dark,
            zoom: 1.5,
            view: BoardView::Comb,
            accent: "lime".into(),
            dismissed: vec!["https://github.com/o/r/pull/7".into()],
            agents: 12,
        };
        let text = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), s);
        let tiles = Settings { view: BoardView::Tiles, ..s.clone() };
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&tiles).unwrap()).unwrap(), tiles);
        let partial: Settings = serde_json::from_str(r#"{"theme":"orange"}"#).unwrap();
        assert_eq!(partial.theme, BoardTheme::Orange);
        assert_eq!(partial.zoom, 1.0);
        assert_eq!(partial.accent, DEFAULT_ACCENT, "an old settings file keeps the default accent");
        assert_eq!(partial.view, BoardView::Tiles, "an old settings file opens on the start screen");
        assert!(partial.dismissed.is_empty(), "a file written before #194 has put nothing away");
        assert_eq!(partial.agents(), DEFAULT_AGENTS, "a file written before #254 shows eight subagents");
        let wild: Settings = serde_json::from_str(r#"{"agents":0}"#).unwrap();
        assert_eq!(wild.agents(), AGENTS_MIN, "a hand-edited file is held in range");
        let wild: Settings = serde_json::from_str(r#"{"agents":99}"#).unwrap();
        assert_eq!(wild.agents(), AGENTS_MAX);
        // The views that went (#183) open on the default, and the rest of
        // the file still counts.
        for gone in ["circuit", "blob", "nonsense"] {
            let old: Settings = serde_json::from_str(&format!(r#"{{"theme":"dark","view":"{gone}","accent":"lime"}}"#)).unwrap();
            assert_eq!((old.view, old.theme, old.accent.as_str()), (BoardView::Tiles, BoardTheme::Dark, "lime"), "{gone}");
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
    }
}
