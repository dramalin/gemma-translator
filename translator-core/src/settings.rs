use crate::keys::Key;
use crate::language::RotateDirection;

/// Rows of the settings page, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsItem {
    KeyboardMode,
    ThemeColor,
    EnableTts,
    VisualizerBars,
    Volume,
}

impl SettingsItem {
    pub const ALL: [SettingsItem; 5] = [
        SettingsItem::KeyboardMode,
        SettingsItem::ThemeColor,
        SettingsItem::EnableTts,
        SettingsItem::VisualizerBars,
        SettingsItem::Volume,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    ToggleKeyboardMode,
    CycleThemeColor(RotateDirection),
    ToggleTts,
    AdjustVisualizerBars(RotateDirection),
    AdjustVolume(RotateDirection),
}

/// Keyboard navigation on the settings page: Up/Down select a row,
/// Left/Right change its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsCursor {
    index: usize,
}

impl SettingsCursor {
    pub fn selected(&self) -> SettingsItem {
        SettingsItem::ALL[self.index]
    }

    pub fn handle_key(&mut self, key: Key) -> Option<SettingsAction> {
        let count = SettingsItem::ALL.len();
        match key {
            Key::ArrowUp => {
                self.index = (self.index + count - 1) % count;
                None
            }
            Key::ArrowDown => {
                self.index = (self.index + 1) % count;
                None
            }
            Key::ArrowLeft => Some(self.action(RotateDirection::Previous)),
            Key::ArrowRight => Some(self.action(RotateDirection::Next)),
            _ => None,
        }
    }

    fn action(&self, direction: RotateDirection) -> SettingsAction {
        match self.selected() {
            SettingsItem::KeyboardMode => SettingsAction::ToggleKeyboardMode,
            SettingsItem::ThemeColor => SettingsAction::CycleThemeColor(direction),
            SettingsItem::EnableTts => SettingsAction::ToggleTts,
            SettingsItem::VisualizerBars => SettingsAction::AdjustVisualizerBars(direction),
            SettingsItem::Volume => SettingsAction::AdjustVolume(direction),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_and_down_move_selection_with_wrap() {
        let mut cursor = SettingsCursor::default();
        assert_eq!(cursor.selected(), SettingsItem::KeyboardMode);
        assert_eq!(cursor.handle_key(Key::ArrowUp), None);
        assert_eq!(cursor.selected(), SettingsItem::Volume);
        assert_eq!(cursor.handle_key(Key::ArrowDown), None);
        assert_eq!(cursor.selected(), SettingsItem::KeyboardMode);
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(cursor.selected(), SettingsItem::ThemeColor);
    }

    #[test]
    fn left_and_right_change_the_selected_item() {
        let mut cursor = SettingsCursor::default();
        assert_eq!(
            cursor.handle_key(Key::ArrowRight),
            Some(SettingsAction::ToggleKeyboardMode)
        );
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(
            cursor.handle_key(Key::ArrowLeft),
            Some(SettingsAction::CycleThemeColor(RotateDirection::Previous))
        );
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(
            cursor.handle_key(Key::ArrowLeft),
            Some(SettingsAction::ToggleTts)
        );
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(
            cursor.handle_key(Key::ArrowRight),
            Some(SettingsAction::AdjustVisualizerBars(RotateDirection::Next))
        );
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(
            cursor.handle_key(Key::ArrowRight),
            Some(SettingsAction::AdjustVolume(RotateDirection::Next))
        );
    }

    #[test]
    fn other_keys_do_nothing() {
        let mut cursor = SettingsCursor::default();
        assert_eq!(cursor.handle_key(Key::Z), None);
        assert_eq!(cursor.selected(), SettingsItem::KeyboardMode);
    }
}
