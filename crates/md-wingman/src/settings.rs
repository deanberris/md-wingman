//! User settings, kept as JSON in the platform's config directory
//! (`~/Library/Application Support/md-wingman/settings.json` on macOS).
//! Unknown or missing fields fall back to their defaults, so the file only
//! needs the values you want to change.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Column for the ruler and for reflowing paragraphs.
    pub ruler_column: usize,
    /// Draw the ruler in the raw editor.
    pub show_ruler: bool,
    /// Soft-wrap long lines at the ruler instead of the window edge.
    pub wrap_at_ruler: bool,
    /// The assistant pane's share of the window width. The default splits
    /// editor and assistant at the golden ratio.
    pub assistant_fraction: f32,
    /// Check paragraphs in the background as you write.
    pub continuous: bool,
    /// How long typing must pause before a background check, in milliseconds.
    pub continuous_delay_ms: u64,
    /// Also fact-check in the background, not just copy-edit. Fact-checks
    /// search the web, so they take longer and use more of your plan.
    pub continuous_fact_check: bool,
    /// Grade paragraphs with Jev first, and check only what scores above
    /// the threshold. The TypeSafe key is kept in the Keychain.
    pub jev_prefilter: bool,
    /// The score (0 to 1) a grade must be above to act on.
    pub jev_threshold: f32,
    /// The Jev model, pinned so grades stay consistent.
    pub jev_model: String,
    /// Light, dark, or follow the system.
    pub appearance: Appearance,
    /// The raw editor's font; the theme's monospace font when unset.
    pub editor_font: Option<String>,
    /// The raw editor's font size, in points.
    pub editor_font_size: f32,
    /// The preview's font; the system font when unset.
    pub preview_font: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ruler_column: 120,
            show_ruler: true,
            wrap_at_ruler: true,
            assistant_fraction: 1. - 1. / std::f32::consts::GOLDEN_RATIO,
            continuous: false,
            continuous_delay_ms: 3000,
            continuous_fact_check: true,
            jev_prefilter: false,
            jev_threshold: 0.66,
            jev_model: "jev-1.14.0".into(),
            appearance: Appearance::System,
            editor_font: None,
            editor_font_size: 14.,
            preview_font: None,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("WINGMAN_SETTINGS") {
            return Some(PathBuf::from(path));
        }
        Some(dirs::config_dir()?.join("md-wingman").join("settings.json"))
    }

    /// Reads the settings file, or returns the defaults if there isn't one.
    /// A file that can't be parsed is reported and otherwise ignored.
    pub fn load() -> (Settings, Option<String>) {
        let Some(path) = Self::path() else {
            return (Settings::default(), None);
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(s) => (s.sanitized(), None),
                Err(e) => (
                    Settings::default(),
                    Some(format!("Ignoring {}: {e}", path.display())),
                ),
            },
            Err(_) => (Settings::default(), None),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        text.push('\n');
        std::fs::write(path, text)
    }

    fn sanitized(mut self) -> Self {
        self.ruler_column = self.ruler_column.clamp(20, 400);
        if !(0.15..=0.7).contains(&self.assistant_fraction) {
            self.assistant_fraction = Settings::default().assistant_fraction;
        }
        self.continuous_delay_ms = self.continuous_delay_ms.clamp(500, 60_000);
        if !(0.0..=1.0).contains(&self.jev_threshold) {
            self.jev_threshold = Settings::default().jev_threshold;
        }
        self.editor_font_size = self.editor_font_size.clamp(9., 32.);
        if self.jev_model.trim().is_empty() {
            self.jev_model = Settings::default().jev_model;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_files_fill_in_defaults() {
        let s: Settings = serde_json::from_str(r#"{"ruler_column": 80}"#).unwrap();
        assert_eq!(s.ruler_column, 80);
        assert!(s.show_ruler);
        let s: Settings = serde_json::from_str(r#"{"ruler_column": 3}"#).unwrap();
        assert_eq!(s.sanitized().ruler_column, 20);
        let s: Settings =
            serde_json::from_str(r#"{"appearance": "dark", "editor_font": "Menlo"}"#).unwrap();
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.editor_font.as_deref(), Some("Menlo"));
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::env::set_var("WINGMAN_SETTINGS", &path);
        let s = Settings {
            ruler_column: 100,
            ..Settings::default()
        };
        s.save().unwrap();
        assert_eq!(Settings::load().0, s);
        std::env::remove_var("WINGMAN_SETTINGS");
    }
}
