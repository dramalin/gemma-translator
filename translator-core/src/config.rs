use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keys::KeyboardMode;
use crate::language::{rotate_language, LanguageCode, RotateDirection};

pub const VISUALIZER_BARS_MIN: u32 = 8;
pub const VISUALIZER_BARS_MAX: u32 = 128;
pub const VISUALIZER_BARS_STEP: u32 = 8;

/// The six UI accent colors offered by the old settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ThemeColor {
    Red,
    White,
    Yellow,
    Blue,
    Green,
    #[default]
    Orange,
}

impl ThemeColor {
    pub const ALL: [ThemeColor; 6] = [
        ThemeColor::Red,
        ThemeColor::White,
        ThemeColor::Yellow,
        ThemeColor::Blue,
        ThemeColor::Green,
        ThemeColor::Orange,
    ];

    pub fn hex(self) -> &'static str {
        match self {
            ThemeColor::Red => "#ff4444",
            ThemeColor::White => "#ffffff",
            ThemeColor::Yellow => "#ffeb3b",
            ThemeColor::Blue => "#2196f3",
            ThemeColor::Green => "#4caf50",
            ThemeColor::Orange => "#ffa500",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeColor::Red => "RED",
            ThemeColor::White => "WHITE",
            ThemeColor::Yellow => "YELLOW",
            ThemeColor::Blue => "BLUE",
            ThemeColor::Green => "GREEN",
            ThemeColor::Orange => "ORANGE",
        }
    }

    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            ThemeColor::Red => (0xff, 0x44, 0x44),
            ThemeColor::White => (0xff, 0xff, 0xff),
            ThemeColor::Yellow => (0xff, 0xeb, 0x3b),
            ThemeColor::Blue => (0x21, 0x96, 0xf3),
            ThemeColor::Green => (0x4c, 0xaf, 0x50),
            ThemeColor::Orange => (0xff, 0xa5, 0x00),
        }
    }

    pub fn cycled(self, direction: RotateDirection) -> ThemeColor {
        let count = ThemeColor::ALL.len();
        let index = ThemeColor::ALL
            .iter()
            .position(|color| *color == self)
            .expect("ALL lists every variant");
        let next = match direction {
            RotateDirection::Next => (index + 1) % count,
            RotateDirection::Previous => (index + count - 1) % count,
        };
        ThemeColor::ALL[next]
    }
}

impl TryFrom<String> for ThemeColor {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        ThemeColor::ALL
            .into_iter()
            .find(|color| color.hex().eq_ignore_ascii_case(&value))
            .ok_or_else(|| format!("unknown theme color {value}"))
    }
}

impl From<ThemeColor> for String {
    fn from(color: ThemeColor) -> String {
        color.hex().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keyboard_mode: KeyboardMode,
    pub theme_color: ThemeColor,
    pub enable_tts: bool,
    pub visualizer_bars: u32,
    pub lane1_language: LanguageCode,
    pub lane2_language: LanguageCode,
    pub speech_service_url: String,
    pub llm_endpoint: String,
    pub llm_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_input_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_output_device: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            keyboard_mode: KeyboardMode::Landscape,
            theme_color: ThemeColor::Orange,
            enable_tts: true,
            visualizer_bars: 16,
            lane1_language: LanguageCode::Ar,
            lane2_language: LanguageCode::En,
            speech_service_url: "http://localhost:3000".to_string(),
            llm_endpoint: "http://localhost:9379/v1".to_string(),
            llm_model: "gemma4-e2b".to_string(),
            audio_input_device: None,
            audio_output_device: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot write config {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl Config {
    /// `$XDG_CONFIG_HOME/gemma-translator/config.toml`, falling back to `~/.config`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(base.join("gemma-translator").join("config.toml"))
    }

    /// Loads the config; a missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        let config: Config = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(config.normalized())
    }

    /// Writes atomically (temp file + rename) so a power cut never leaves half a file.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self)?;
        let write_error = |source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(write_error)?;
        }
        let temp_path = path.with_extension("toml.tmp");
        fs::write(&temp_path, text).map_err(write_error)?;
        fs::rename(&temp_path, path).map_err(write_error)
    }

    fn normalized(mut self) -> Config {
        let bars = self
            .visualizer_bars
            .clamp(VISUALIZER_BARS_MIN, VISUALIZER_BARS_MAX);
        self.visualizer_bars = bars / VISUALIZER_BARS_STEP * VISUALIZER_BARS_STEP;
        if self.lane1_language == self.lane2_language {
            self.lane2_language = rotate_language(
                self.lane2_language,
                self.lane1_language,
                RotateDirection::Next,
            );
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.lane1_language, LanguageCode::Ar);
        assert_eq!(config.lane2_language, LanguageCode::En);
        assert_eq!(config.theme_color, ThemeColor::Orange);
        assert_eq!(config.visualizer_bars, 16);
    }

    #[test]
    fn save_then_load_round_trips_and_creates_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let config = Config {
            keyboard_mode: KeyboardMode::Vertical,
            theme_color: ThemeColor::Blue,
            enable_tts: false,
            visualizer_bars: 32,
            lane1_language: LanguageCode::Ja,
            lane2_language: LanguageCode::Ko,
            audio_input_device: Some("USB Mic".to_string()),
            ..Config::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("theme_color = \"#2196f3\""));
        assert!(text.contains("keyboard_mode = \"vertical\""));
        assert!(!text.contains("audio_output_device"));
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "keyboard_mode = \"vertical\"\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.keyboard_mode, KeyboardMode::Vertical);
        assert_eq!(config.llm_model, "gemma4-e2b");
    }

    #[test]
    fn unknown_theme_color_is_a_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme_color = \"#123456\"\n").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn equal_lane_languages_are_separated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "lane1_language = \"zh\"\nlane2_language = \"zh\"\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.lane1_language, LanguageCode::Zh);
        assert_eq!(config.lane2_language, LanguageCode::Ko);
    }

    #[test]
    fn visualizer_bars_are_clamped_to_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for (written, expected) in [(3, 8), (200, 128), (20, 16), (64, 64)] {
            std::fs::write(&path, format!("visualizer_bars = {written}\n")).unwrap();
            assert_eq!(Config::load(&path).unwrap().visualizer_bars, expected);
        }
    }

    #[test]
    fn saving_to_an_unwritable_location_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let result = Config::default().save(&blocker.join("config.toml"));
        assert!(matches!(result, Err(ConfigError::Write { .. })));
    }

    #[test]
    fn theme_colors_cycle_with_wrap() {
        assert_eq!(
            ThemeColor::Orange.cycled(RotateDirection::Next),
            ThemeColor::Red
        );
        assert_eq!(
            ThemeColor::Red.cycled(RotateDirection::Previous),
            ThemeColor::Orange
        );
        assert_eq!(ThemeColor::Orange.rgb(), (0xff, 0xa5, 0x00));
        assert_eq!(ThemeColor::Blue.name(), "BLUE");
    }
}
