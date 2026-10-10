//! Converts controller state into the plain view model both frontends draw.

use translator_core::config::Config;
use translator_core::controller::{Controller, Phase};
use translator_core::keys::KeyboardMode;
use translator_core::lane::Lane;
use translator_core::settings::{SettingsCursor, SettingsItem};

/// Fraction a visualizer bar keeps per frame once recording stops.
const LEVEL_DECAY: f32 = 0.8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneView {
    pub label: String,
    pub language: String,
    pub is_active: bool,
    pub is_recording: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRowView {
    pub label: String,
    pub value: String,
    pub is_selected: bool,
}

/// Everything a frontend needs to draw one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct AppView {
    pub theme_rgb: (u8, u8, u8),
    pub lanes: [LaneView; 2],
    pub keyboard_mode: KeyboardMode,
    pub phase: Phase,
    /// Landscape mode marks the active lane; vertical mode has none.
    pub show_active_marker: bool,
    pub source_label: String,
    pub transcription: String,
    pub target_label: String,
    pub translation: String,
    pub is_translation_right_to_left: bool,
    pub status: String,
    /// Why recordings are silent or audio is discarded (no microphone or
    /// speaker). The TUI shows it; the Slint UI only logs it.
    pub audio_warning: Option<String>,
    pub levels: Vec<f32>,
    pub is_settings_open: bool,
    pub settings_rows: Vec<SettingsRowView>,
}

pub(crate) fn lane_view(controller: &Controller, lane: Lane) -> LaneView {
    LaneView {
        label: match lane {
            Lane::One => "1",
            Lane::Two => "2",
        }
        .to_string(),
        language: controller.language(lane).display_name().to_uppercase(),
        is_active: controller.active_lane() == lane,
        is_recording: controller.phase() == Phase::Recording(lane),
    }
}

fn phase_text(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "READY",
        Phase::Recording(_) => "RECORDING",
        Phase::Transcribing(_) => "TRANSCRIBING",
        Phase::Translating(_) => "TRANSLATING",
        Phase::Speaking(_) => "SPEAKING",
    }
}

/// The last error if there is one, otherwise the current phase.
pub(crate) fn status_line(controller: &Controller) -> String {
    let status = &controller.transcript().status;
    if status.is_empty() {
        phase_text(controller.phase()).to_string()
    } else {
        status.clone()
    }
}

pub(crate) fn settings_rows(
    config: &Config,
    cursor: &SettingsCursor,
    volume_percent: Option<u16>,
) -> Vec<SettingsRowView> {
    SettingsItem::ALL
        .iter()
        .map(|&item| {
            let (label, value) = match item {
                SettingsItem::KeyboardMode => (
                    "KEYBOARD",
                    keyboard_mode_text(config.keyboard_mode).to_string(),
                ),
                SettingsItem::ThemeColor => ("THEME", config.theme_color.name().to_string()),
                SettingsItem::EnableTts => (
                    "VOICE OUTPUT",
                    if config.enable_tts { "ON" } else { "OFF" }.to_string(),
                ),
                SettingsItem::VisualizerBars => {
                    ("VISUALIZER BARS", config.visualizer_bars.to_string())
                }
                SettingsItem::Volume => (
                    "VOLUME",
                    volume_percent
                        .map_or_else(|| "N/A".to_string(), |percent| format!("{percent}%")),
                ),
            };
            SettingsRowView {
                label: label.to_string(),
                value,
                is_selected: cursor.selected() == item,
            }
        })
        .collect()
}

fn keyboard_mode_text(mode: KeyboardMode) -> &'static str {
    match mode {
        KeyboardMode::Landscape => "LANDSCAPE",
        KeyboardMode::Vertical => "VERTICAL",
    }
}

/// Scrolls in a new level while recording, decays otherwise; always `bar_count` long.
pub(crate) fn update_levels(levels: &mut Vec<f32>, bar_count: usize, new_level: Option<f32>) {
    match new_level {
        Some(level) => levels.push(level.clamp(0.0, 1.0)),
        None => levels.iter_mut().for_each(|level| *level *= LEVEL_DECAY),
    }
    if levels.len() > bar_count {
        levels.drain(..levels.len() - bar_count);
    } else {
        levels.splice(..0, std::iter::repeat_n(0.0, bar_count - levels.len()));
    }
}

#[cfg(test)]
mod tests {
    use translator_core::config::Config;
    use translator_core::controller::JobEvent;
    use translator_core::keys::{Command, Key};
    use translator_core::settings::SettingsCursor;

    use super::*;

    #[test]
    fn lane_views_reflect_active_and_recording_lanes() {
        let mut controller = Controller::new(Config::default());
        let lane1 = lane_view(&controller, Lane::One);
        assert_eq!(lane1.label, "1");
        assert_eq!(lane1.language, "ARABIC");
        assert!(lane1.is_active);
        assert!(!lane1.is_recording);
        controller.handle_command(Command::StartRecording(Lane::Two));
        let lane2 = lane_view(&controller, Lane::Two);
        assert!(lane2.is_active && lane2.is_recording);
        assert!(!lane_view(&controller, Lane::One).is_active);
    }

    #[test]
    fn settings_rows_show_values_and_selection() {
        let config = Config::default();
        let mut cursor = SettingsCursor::default();
        let rows = settings_rows(&config, &cursor, None);
        let values: Vec<&str> = rows.iter().map(|row| row.value.as_str()).collect();
        assert_eq!(values, ["LANDSCAPE", "ORANGE", "ON", "16", "N/A"]);
        assert!(rows[0].is_selected && !rows[1].is_selected);
        cursor.handle_key(Key::ArrowUp);
        let rows = settings_rows(&config, &cursor, Some(75));
        assert_eq!(rows[4].value, "75%");
        assert!(rows[4].is_selected);
    }

    #[test]
    fn levels_scroll_decay_and_pad_to_the_bar_count() {
        let mut levels = Vec::new();
        update_levels(&mut levels, 4, Some(0.5));
        assert_eq!(levels, [0.0, 0.0, 0.0, 0.5]);
        update_levels(&mut levels, 4, Some(2.0));
        assert_eq!(levels, [0.0, 0.0, 0.5, 1.0]);
        update_levels(&mut levels, 4, None);
        assert_eq!(levels, [0.0, 0.0, 0.4, 0.8]);
        update_levels(&mut levels, 2, None);
        assert_eq!(levels.len(), 2);
    }

    #[test]
    fn status_line_prefers_errors_over_the_phase() {
        let mut controller = Controller::new(Config::default());
        assert_eq!(status_line(&controller), "READY");
        controller.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(status_line(&controller), "RECORDING");
        controller.handle_command(Command::StopRecording(Lane::One));
        let effects = controller.on_capture_finished(std::time::Duration::from_secs(1), true);
        let translator_core::controller::Effect::StartJob(request) = &effects[0] else {
            panic!("expected a job");
        };
        controller.on_job_event(JobEvent::Failed {
            job: request.id,
            message: "down".to_string(),
        });
        assert_eq!(status_line(&controller), "Error: down");
    }
}
