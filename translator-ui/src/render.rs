//! Draws an `AppView` into the Slint window.

use std::rc::Rc;

use slint::{Color, Model, ModelRc, VecModel};
use translator_app::{AppView, LaneView as AppLaneView, SettingsRowView};

use crate::{AppWindow, LaneView, SettingsRow};

/// The window's list models, bound once and updated in place.
pub struct Models {
    levels: Rc<VecModel<f32>>,
    settings_rows: Rc<VecModel<SettingsRow>>,
}

impl Models {
    pub fn bind(window: &AppWindow) -> Models {
        let models = Models {
            levels: Rc::new(VecModel::default()),
            settings_rows: Rc::new(VecModel::default()),
        };
        window.set_visualizer_levels(ModelRc::from(models.levels.clone()));
        window.set_settings_rows(ModelRc::from(models.settings_rows.clone()));
        models
    }
}

pub fn render(view: &AppView, window: &AppWindow, models: &Models) {
    window.set_theme_color(theme_color(view.theme_rgb));
    window.set_lane1(lane(&view.lanes[0]));
    window.set_lane2(lane(&view.lanes[1]));
    window.set_show_active_brackets(view.show_active_marker);
    window.set_source_label(view.source_label.as_str().into());
    window.set_transcription(view.transcription.as_str().into());
    window.set_target_label(view.target_label.as_str().into());
    window.set_translation(view.translation.as_str().into());
    window.set_is_translation_right_to_left(view.is_translation_right_to_left);
    window.set_status_text(view.status.as_str().into());
    window.set_is_settings_open(view.is_settings_open);
    set_if_changed(&models.levels, &view.levels);
    // The rows are only drawn while the settings page is open.
    if view.is_settings_open {
        let settings_rows: Vec<SettingsRow> = view.settings_rows.iter().map(settings_row).collect();
        set_if_changed(&models.settings_rows, &settings_rows);
    }
}

fn theme_color((red, green, blue): (u8, u8, u8)) -> Color {
    Color::from_rgb_u8(red, green, blue)
}

fn lane(view: &AppLaneView) -> LaneView {
    LaneView {
        label: view.label.as_str().into(),
        language: view.language.as_str().into(),
        is_active: view.is_active,
        is_recording: view.is_recording,
    }
}

fn settings_row(row: &SettingsRowView) -> SettingsRow {
    SettingsRow {
        label: row.label.as_str().into(),
        value: row.value.as_str().into(),
        is_selected: row.is_selected,
    }
}

/// Brings the model in line with `rows`, touching only what differs: changed
/// rows when the length is the same (so Slint keeps the existing items instead
/// of rebuilding the list), otherwise the whole list.
fn set_if_changed<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: &[T]) {
    if model.row_count() != rows.len() {
        model.set_vec(rows.to_vec());
        return;
    }
    for (index, row) in rows.iter().enumerate() {
        if model.row_data(index).as_ref() != Some(row) {
            model.set_row_data(index, row.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use translator_core::config::ThemeColor;
    use translator_core::keys::Key;

    use super::*;

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
        let color = theme_color(ThemeColor::Blue.rgb());
        assert_eq!(
            (color.red(), color.green(), color.blue()),
            (0x21, 0x96, 0xf3)
        );
    }
}
