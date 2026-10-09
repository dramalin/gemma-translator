use serde::{Deserialize, Serialize};

use crate::lane::Lane;
use crate::language::RotateDirection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyboardMode {
    /// One active person at a time: Space switches, Z records, arrows rotate.
    #[default]
    Landscape,
    /// Each lane has its own keys: Z/X record, arrows and -/+ rotate.
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Space,
    Z,
    X,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Minus,
    Plus,
    Escape,
}

impl Key {
    /// Maps the text of a Slint key event. Slint encodes special keys as
    /// private-use code points (`slint::platform::Key`).
    pub fn from_event_text(text: &str) -> Option<Key> {
        match text {
            " " => Some(Key::Space),
            "z" | "Z" => Some(Key::Z),
            "x" | "X" => Some(Key::X),
            "-" | "_" => Some(Key::Minus),
            "+" | "=" => Some(Key::Plus),
            "\u{F702}" => Some(Key::ArrowLeft),
            "\u{F703}" => Some(Key::ArrowRight),
            "\u{F700}" => Some(Key::ArrowUp),
            "\u{F701}" => Some(Key::ArrowDown),
            "\u{1b}" => Some(Key::Escape),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleActiveLane,
    StartRecording(Lane),
    StopRecording(Lane),
    RotateLanguage(Lane, RotateDirection),
    ToggleSettings,
}

pub fn command_for_key_press(mode: KeyboardMode, key: Key, active_lane: Lane) -> Option<Command> {
    if key == Key::Escape {
        return Some(Command::ToggleSettings);
    }
    match mode {
        KeyboardMode::Landscape => match key {
            Key::Space => Some(Command::ToggleActiveLane),
            Key::Z => Some(Command::StartRecording(active_lane)),
            Key::ArrowLeft => Some(Command::RotateLanguage(
                active_lane,
                RotateDirection::Previous,
            )),
            Key::ArrowRight => Some(Command::RotateLanguage(active_lane, RotateDirection::Next)),
            _ => None,
        },
        KeyboardMode::Vertical => match key {
            Key::Z => Some(Command::StartRecording(Lane::One)),
            Key::X => Some(Command::StartRecording(Lane::Two)),
            Key::ArrowLeft => Some(Command::RotateLanguage(
                Lane::One,
                RotateDirection::Previous,
            )),
            Key::ArrowRight => Some(Command::RotateLanguage(Lane::One, RotateDirection::Next)),
            Key::Minus => Some(Command::RotateLanguage(
                Lane::Two,
                RotateDirection::Previous,
            )),
            Key::Plus => Some(Command::RotateLanguage(Lane::Two, RotateDirection::Next)),
            _ => None,
        },
    }
}

pub fn command_for_key_release(mode: KeyboardMode, key: Key, active_lane: Lane) -> Option<Command> {
    match (mode, key) {
        (KeyboardMode::Landscape, Key::Z) => Some(Command::StopRecording(active_lane)),
        (KeyboardMode::Vertical, Key::Z) => Some(Command::StopRecording(Lane::One)),
        (KeyboardMode::Vertical, Key::X) => Some(Command::StopRecording(Lane::Two)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::RotateDirection::{Next, Previous};

    #[test]
    fn event_text_maps_letters_case_insensitively_and_symbols() {
        assert_eq!(Key::from_event_text(" "), Some(Key::Space));
        assert_eq!(Key::from_event_text("Z"), Some(Key::Z));
        assert_eq!(Key::from_event_text("x"), Some(Key::X));
        assert_eq!(Key::from_event_text("_"), Some(Key::Minus));
        assert_eq!(Key::from_event_text("="), Some(Key::Plus));
        assert_eq!(Key::from_event_text("\u{F702}"), Some(Key::ArrowLeft));
        assert_eq!(Key::from_event_text("\u{F703}"), Some(Key::ArrowRight));
        assert_eq!(Key::from_event_text("\u{F700}"), Some(Key::ArrowUp));
        assert_eq!(Key::from_event_text("\u{F701}"), Some(Key::ArrowDown));
        assert_eq!(Key::from_event_text("\u{1b}"), Some(Key::Escape));
        assert_eq!(Key::from_event_text("q"), None);
    }

    #[test]
    fn landscape_drives_the_active_lane() {
        let mode = KeyboardMode::Landscape;
        assert_eq!(
            command_for_key_press(mode, Key::Space, Lane::One),
            Some(Command::ToggleActiveLane)
        );
        assert_eq!(
            command_for_key_press(mode, Key::Z, Lane::Two),
            Some(Command::StartRecording(Lane::Two))
        );
        assert_eq!(
            command_for_key_press(mode, Key::ArrowLeft, Lane::Two),
            Some(Command::RotateLanguage(Lane::Two, Previous))
        );
        assert_eq!(
            command_for_key_press(mode, Key::ArrowRight, Lane::One),
            Some(Command::RotateLanguage(Lane::One, Next))
        );
        assert_eq!(command_for_key_press(mode, Key::X, Lane::One), None);
        assert_eq!(
            command_for_key_release(mode, Key::Z, Lane::Two),
            Some(Command::StopRecording(Lane::Two))
        );
    }

    #[test]
    fn vertical_gives_each_lane_its_own_keys() {
        let mode = KeyboardMode::Vertical;
        assert_eq!(
            command_for_key_press(mode, Key::Z, Lane::Two),
            Some(Command::StartRecording(Lane::One))
        );
        assert_eq!(
            command_for_key_press(mode, Key::X, Lane::One),
            Some(Command::StartRecording(Lane::Two))
        );
        assert_eq!(
            command_for_key_press(mode, Key::ArrowLeft, Lane::Two),
            Some(Command::RotateLanguage(Lane::One, Previous))
        );
        assert_eq!(
            command_for_key_press(mode, Key::Minus, Lane::One),
            Some(Command::RotateLanguage(Lane::Two, Previous))
        );
        assert_eq!(
            command_for_key_press(mode, Key::Plus, Lane::One),
            Some(Command::RotateLanguage(Lane::Two, Next))
        );
        assert_eq!(command_for_key_press(mode, Key::Space, Lane::One), None);
        assert_eq!(
            command_for_key_release(mode, Key::Z, Lane::Two),
            Some(Command::StopRecording(Lane::One))
        );
        assert_eq!(
            command_for_key_release(mode, Key::X, Lane::One),
            Some(Command::StopRecording(Lane::Two))
        );
    }

    #[test]
    fn escape_toggles_settings_in_both_modes() {
        for mode in [KeyboardMode::Landscape, KeyboardMode::Vertical] {
            assert_eq!(
                command_for_key_press(mode, Key::Escape, Lane::One),
                Some(Command::ToggleSettings)
            );
            assert_eq!(command_for_key_release(mode, Key::Escape, Lane::One), None);
        }
    }
}
