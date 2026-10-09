//! Converts controller state into the structs the Slint UI renders.

use slint::Color;
use translator_core::config::{Config, ThemeColor};
use translator_core::controller::{Controller, Phase};
use translator_core::keys::KeyboardMode;
use translator_core::lane::Lane;
use translator_core::settings::{SettingsCursor, SettingsItem};

use crate::{LaneView, SettingsRow};

/// Fraction a visualizer bar keeps per frame once recording stops.
const LEVEL_DECAY: f32 = 0.8;

pub fn theme_color(theme: ThemeColor) -> Color {
    let (red, green, blue) = theme.rgb();
    Color::from_rgb_u8(red, green, blue)
}

pub fn lane_view(controller: &Controller, lane: Lane) -> LaneView {
    LaneView {
        label: match lane {
            Lane::One => "1".into(),
            Lane::Two => "2".into(),
        },
        language: controller
            .language(lane)
            .display_name()
            .to_uppercase()
            .into(),
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
pub fn status_line(controller: &Controller) -> String {
    let status = &controller.transcript().status;
    if status.is_empty() {
        phase_text(controller.phase()).to_string()
    } else {
        status.clone()
    }
}

pub fn settings_rows(
    config: &Config,
    cursor: &SettingsCursor,
    volume_percent: Option<u16>,
) -> Vec<SettingsRow> {
    SettingsItem::ALL
        .iter()
        .map(|&item| {
            let (label, value) = match item {
                SettingsItem::KeyboardMode => (
                    "KEYBOARD",
                    match config.keyboard_mode {
                        KeyboardMode::Landscape => "LANDSCAPE".to_string(),
                        KeyboardMode::Vertical => "VERTICAL".to_string(),
                    },
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
            SettingsRow {
                label: label.into(),
                value: value.into(),
                is_selected: cursor.selected() == item,
            }
        })
        .collect()
}

/// Scrolls in a new level while recording, decays otherwise; always `bar_count` long.
pub fn update_levels(levels: &mut Vec<f32>, bar_count: usize, new_level: Option<f32>) {
    match new_level {
        Some(level) => levels.push(level.clamp(0.0, 1.0)),
        None => levels.iter_mut().for_each(|level| *level *= LEVEL_DECAY),
    }
    if levels.len() > bar_count {
        levels.drain(..levels.len() - bar_count);
    }
    while levels.len() < bar_count {
        levels.insert(0, 0.0);
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

    #[test]
    fn slint_special_keys_match_core_key_codes() {
        use slint::platform::Key as SlintKey;
        let cases = [
            (SlintKey::LeftArrow, Key::ArrowLeft),
            (SlintKey::RightArrow, Key::ArrowRight),
            (SlintKey::UpArrow, Key::ArrowUp),
            (SlintKey::DownArrow, Key::ArrowDown),
            (SlintKey::Escape, Key::Escape),
        ];
        for (slint_key, key) in cases {
            let text = slint::SharedString::from(slint_key);
            assert_eq!(Key::from_event_text(&text), Some(key));
        }
    }

    #[test]
    fn theme_color_converts_to_slint() {
        let color = theme_color(translator_core::config::ThemeColor::Blue);
        assert_eq!(
            (color.red(), color.green(), color.blue()),
            (0x21, 0x96, 0xf3)
        );
    }
}
