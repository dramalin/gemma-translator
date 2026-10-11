use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keys::KeyboardMode;
use crate::language::{rotate_language, LanguageCode, RotateDirection};
use crate::models::{TtsSettings, DEFAULT_TTS_SPEED, TTS_SPEED_RANGE};

pub const VISUALIZER_BARS_MIN: u32 = 8;
pub const VISUALIZER_BARS_MAX: u32 = 128;
pub const VISUALIZER_BARS_STEP: u32 = 8;

/// The six UI accent colors.
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
        let index = ThemeColor::ALL
            .iter()
            .position(|color| *color == self)
            .expect("ALL lists every variant");
        ThemeColor::ALL[direction.wrap(index, ThemeColor::ALL.len())]
    }
}

impl TryFrom<String> for ThemeColor {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        ThemeColor::ALL
            .into_iter()
            .find(|color| String::from(*color).eq_ignore_ascii_case(&value))
            .ok_or_else(|| format!("unknown theme color {value}"))
    }
}

impl From<ThemeColor> for String {
    fn from(color: ThemeColor) -> String {
        let (red, green, blue) = color.rgb();
        format!("#{red:02x}{green:02x}{blue:02x}")
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
    pub model_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_input_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_output_device: Option<String>,
    /// Speaking rate of every voice; 1.0 is Moonshine's normal pace.
    pub tts_speed: f64,
    /// Moonshine voice ids by language (`[tts_voices]`, e.g.
    /// `zh = "kokoro_zm_yunxi"`); `translator-models` checks and fetches them.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tts_voices: BTreeMap<LanguageCode, String>,
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
            model_dir: PathBuf::from("/var/lib/gemma-translator/models"),
            audio_input_device: None,
            audio_output_device: None,
            tts_speed: DEFAULT_TTS_SPEED,
            tts_voices: BTreeMap::new(),
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

    /// Writes atomically (temp file + fsync + rename) so a power cut never
    /// leaves half a file. Values are normalized as `load` would.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(&self.clone().normalized())?;
        let write_error = |source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(write_error)?;
        }
        let temp_path = path.with_extension("toml.tmp");
        let mut file = fs::File::create(&temp_path).map_err(write_error)?;
        file.write_all(text.as_bytes()).map_err(write_error)?;
        file.sync_all().map_err(write_error)?;
        fs::rename(&temp_path, path).map_err(write_error)
    }

    pub fn tts_settings(&self) -> TtsSettings {
        TtsSettings {
            speed: self.tts_speed,
            voices: self.tts_voices.clone(),
        }
    }

    fn normalized(mut self) -> Config {
        let bars = self
            .visualizer_bars
            .clamp(VISUALIZER_BARS_MIN, VISUALIZER_BARS_MAX);
        self.visualizer_bars = bars / VISUALIZER_BARS_STEP * VISUALIZER_BARS_STEP;
        self.tts_speed = if self.tts_speed.is_finite() {
            self.tts_speed
                .clamp(*TTS_SPEED_RANGE.start(), *TTS_SPEED_RANGE.end())
        } else {
            DEFAULT_TTS_SPEED
        };
        self.tts_voices = std::mem::take(&mut self.tts_voices)
            .into_iter()
            .map(|(language, voice)| (language, voice.trim().to_string()))
            .filter(|(_, voice)| !voice.is_empty())
            .collect();
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
    fn save_writes_normalized_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            visualizer_bars: 3,
            lane1_language: LanguageCode::Zh,
            lane2_language: LanguageCode::Zh,
            ..Config::default()
        };
        config.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("visualizer_bars = 8"), "{text}");
        assert!(text.contains("lane2_language = \"ko\""), "{text}");
        assert!(!dir.path().join("config.toml.tmp").exists());
    }

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
    fn old_http_keys_are_ignored_and_dropped_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "inference_backend = \"http\"\n\
             speech_service_url = \"http://localhost:3000\"\n\
             llm_endpoint = \"http://localhost:9379/v1\"\n\
             llm_model = \"gemma4-e2b\"\n\
             keyboard_mode = \"vertical\"\n\
             model_dir = \"/home/pi/models\"\n",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.keyboard_mode, KeyboardMode::Vertical);
        assert_eq!(config.model_dir, PathBuf::from("/home/pi/models"));
        config.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for key in [
            "inference_backend",
            "speech_service_url",
            "llm_endpoint",
            "llm_model",
        ] {
            assert!(!text.contains(key), "{key} was written back:\n{text}");
        }
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "keyboard_mode = \"vertical\"\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.keyboard_mode, KeyboardMode::Vertical);
        assert_eq!(
            config.model_dir,
            PathBuf::from("/var/lib/gemma-translator/models")
        );
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

    #[test]
    fn tts_voices_and_speed_round_trip_and_are_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "tts_speed = 1.2\n[tts_voices]\nzh = \" kokoro_zm_yunxi \"\nen = \"\"\n",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.tts_speed, 1.2);
        assert_eq!(
            config.tts_voices,
            BTreeMap::from([(LanguageCode::Zh, "kokoro_zm_yunxi".to_string())])
        );
        config.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("tts_speed = 1.2\n"), "{text}");
        assert!(
            text.contains("[tts_voices]\nzh = \"kokoro_zm_yunxi\""),
            "{text}"
        );
        assert_eq!(Config::load(&path).unwrap(), config);

        for (written, expected) in [("0.1", 0.5), ("9", 2.0), ("nan", 1.0)] {
            std::fs::write(&path, format!("tts_speed = {written}\n")).unwrap();
            assert_eq!(
                Config::load(&path).unwrap().tts_speed,
                expected,
                "{written}"
            );
        }
    }

    #[test]
    fn tts_voices_default_to_none_and_are_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::default().save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("tts_voices"), "{text}");
        assert!(text.contains("tts_speed = 1.0"), "{text}");
    }

    #[test]
    fn unknown_tts_voice_language_is_a_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tts_voices]\nfr = \"kokoro_ff_siwis\"\n").unwrap();
        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn model_dir_has_a_default_and_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(
            Config::load(&path).unwrap().model_dir,
            PathBuf::from("/var/lib/gemma-translator/models")
        );
        std::fs::write(&path, "model_dir = \"/home/pi/models\"\n").unwrap();
        assert_eq!(
            Config::load(&path).unwrap().model_dir,
            PathBuf::from("/home/pi/models")
        );
    }
}
