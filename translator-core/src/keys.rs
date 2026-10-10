use serde::{Deserialize, Serialize};

use crate::controller::Phase;
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

/// The lane a recording key records for, if `key` is one.
fn recording_lane(mode: KeyboardMode, key: Key, active_lane: Lane) -> Option<Lane> {
    match (mode, key) {
        (KeyboardMode::Landscape, Key::Z) => Some(active_lane),
        (KeyboardMode::Vertical, Key::Z) => Some(Lane::One),
        (KeyboardMode::Vertical, Key::X) => Some(Lane::Two),
        _ => None,
    }
}

pub fn command_for_key_press(mode: KeyboardMode, key: Key, active_lane: Lane) -> Option<Command> {
    if key == Key::Escape {
        return Some(Command::ToggleSettings);
    }
    if let Some(lane) = recording_lane(mode, key, active_lane) {
        return Some(Command::StartRecording(lane));
    }
    match mode {
        KeyboardMode::Landscape => match key {
            Key::Space => Some(Command::ToggleActiveLane),
            Key::ArrowLeft => Some(Command::RotateLanguage(
                active_lane,
                RotateDirection::Previous,
            )),
            Key::ArrowRight => Some(Command::RotateLanguage(active_lane, RotateDirection::Next)),
            _ => None,
        },
        KeyboardMode::Vertical => match key {
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
    recording_lane(mode, key, active_lane).map(Command::StopRecording)
}

/// Toggle-style recording for terminals, which do not report key releases:
/// a recording key starts recording on its lane, or stops it when that lane
/// is already recording. Every other key maps as in `command_for_key_press`.
pub fn command_for_key_toggle(
    mode: KeyboardMode,
    key: Key,
    active_lane: Lane,
    phase: Phase,
) -> Option<Command> {
    match recording_lane(mode, key, active_lane) {
        Some(lane) if phase == Phase::Recording(lane) => Some(Command::StopRecording(lane)),
        Some(lane) => Some(Command::StartRecording(lane)),
        None => command_for_key_press(mode, key, active_lane),
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

    #[test]
    fn toggle_starts_then_stops_the_landscape_recording() {
        let mode = KeyboardMode::Landscape;
        assert_eq!(
            command_for_key_toggle(mode, Key::Z, Lane::Two, Phase::Idle),
            Some(Command::StartRecording(Lane::Two))
        );
        assert_eq!(
            command_for_key_toggle(mode, Key::Z, Lane::Two, Phase::Recording(Lane::Two)),
            Some(Command::StopRecording(Lane::Two))
        );
        // A job in flight is interrupted by a new recording, as with hold style.
        assert_eq!(
            command_for_key_toggle(mode, Key::Z, Lane::One, Phase::Speaking(Lane::One)),
            Some(Command::StartRecording(Lane::One))
        );
    }

    #[test]
    fn toggle_uses_z_and_x_per_lane_in_vertical_mode() {
        let mode = KeyboardMode::Vertical;
        assert_eq!(
            command_for_key_toggle(mode, Key::X, Lane::One, Phase::Idle),
            Some(Command::StartRecording(Lane::Two))
        );
        assert_eq!(
            command_for_key_toggle(mode, Key::X, Lane::One, Phase::Recording(Lane::Two)),
            Some(Command::StopRecording(Lane::Two))
        );
        // The other lane's key while lane 1 records asks to start lane 2;
        // the controller ignores it because a recording is in progress.
        assert_eq!(
            command_for_key_toggle(mode, Key::X, Lane::One, Phase::Recording(Lane::One)),
            Some(Command::StartRecording(Lane::Two))
        );
        assert_eq!(
            command_for_key_toggle(mode, Key::Z, Lane::Two, Phase::Recording(Lane::One)),
            Some(Command::StopRecording(Lane::One))
        );
    }

    #[test]
    fn toggle_maps_other_keys_like_a_press() {
        for mode in [KeyboardMode::Landscape, KeyboardMode::Vertical] {
            for key in [
                Key::Space,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::Minus,
                Key::Plus,
                Key::Escape,
            ] {
                assert_eq!(
                    command_for_key_toggle(mode, key, Lane::One, Phase::Idle),
                    command_for_key_press(mode, key, Lane::One),
                    "{mode:?} {key:?}"
                );
            }
        }
    }
}
