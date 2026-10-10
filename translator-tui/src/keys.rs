//! Terminal key events → translator keys, and the auto-repeat filter that
//! makes toggle-style recording usable when a recording key is held.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use translator_core::keys::Key;

/// A recording key pressed again this soon after its previous press is OS
/// auto-repeat.
const REPEAT_WINDOW: Duration = Duration::from_secs(1);

/// Ctrl and Alt combinations are never translator keys (Ctrl-Z would
/// otherwise start a recording); Shift is allowed.
pub fn key_from_event(event: &KeyEvent) -> Option<Key> {
    if event
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    match event.code {
        KeyCode::Char(' ') => Some(Key::Space),
        KeyCode::Char('z' | 'Z') => Some(Key::Z),
        KeyCode::Char('x' | 'X') => Some(Key::X),
        KeyCode::Char('-' | '_') => Some(Key::Minus),
        KeyCode::Char('+' | '=') => Some(Key::Plus),
        KeyCode::Left => Some(Key::ArrowLeft),
        KeyCode::Right => Some(Key::ArrowRight),
        KeyCode::Up => Some(Key::ArrowUp),
        KeyCode::Down => Some(Key::ArrowDown),
        KeyCode::Esc => Some(Key::Escape),
        _ => None,
    }
}

pub fn is_quit(event: &KeyEvent) -> bool {
    event.code == KeyCode::Char('c') && event.modifiers.contains(KeyModifiers::CONTROL)
}

/// Lets through one press per physical press of a recording key.
pub struct RepeatFilter {
    last_record_press: Option<(Key, Instant)>,
}

impl RepeatFilter {
    pub fn new() -> Self {
        RepeatFilter {
            last_record_press: None,
        }
    }

    /// Terminals report a held key as a stream of presses and no release.
    /// For the recording keys (Z, X), a press within the window of the
    /// previous press of the same key is auto-repeat; each one extends the
    /// window, so a whole held stream counts once. Other keys pass: quick
    /// arrow taps all count, and holding an arrow keeps rotating.
    pub fn accept(&mut self, key: Key, kind: KeyEventKind, now: Instant) -> bool {
        if kind != KeyEventKind::Press {
            return false;
        }
        if !matches!(key, Key::Z | Key::X) {
            return true;
        }
        let is_repeat = self.last_record_press.is_some_and(|(last_key, at)| {
            last_key == key && now.duration_since(at) < REPEAT_WINDOW
        });
        self.last_record_press = Some((key, now));
        !is_repeat
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn terminal_keys_map_to_translator_keys() {
        let cases = [
            (KeyCode::Char(' '), Some(Key::Space)),
            (KeyCode::Char('z'), Some(Key::Z)),
            (KeyCode::Char('Z'), Some(Key::Z)),
            (KeyCode::Char('x'), Some(Key::X)),
            (KeyCode::Char('-'), Some(Key::Minus)),
            (KeyCode::Char('_'), Some(Key::Minus)),
            (KeyCode::Char('+'), Some(Key::Plus)),
            (KeyCode::Char('='), Some(Key::Plus)),
            (KeyCode::Left, Some(Key::ArrowLeft)),
            (KeyCode::Right, Some(Key::ArrowRight)),
            (KeyCode::Up, Some(Key::ArrowUp)),
            (KeyCode::Down, Some(Key::ArrowDown)),
            (KeyCode::Esc, Some(Key::Escape)),
            (KeyCode::Char('q'), None),
            (KeyCode::Enter, None),
        ];
        for (code, expected) in cases {
            assert_eq!(key_from_event(&press(code)), expected, "{code:?}");
        }
    }

    #[test]
    fn ctrl_and_alt_combinations_are_not_translator_keys() {
        let shifted_z = KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT);
        assert_eq!(key_from_event(&shifted_z), Some(Key::Z));
        let ctrl_z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(key_from_event(&ctrl_z), None);
        let alt_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(key_from_event(&alt_x), None);
    }

    #[test]
    fn only_ctrl_c_quits() {
        assert!(is_quit(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_quit(&press(KeyCode::Char('c'))));
        assert!(!is_quit(&press(KeyCode::Char('q'))));
        assert!(!is_quit(&press(KeyCode::Esc)));
    }

    #[test]
    fn held_key_stream_is_accepted_once() {
        let start = Instant::now();
        let mut filter = RepeatFilter::new();
        assert!(filter.accept(Key::Z, KeyEventKind::Press, start));
        // OS auto-repeat: first repeat after ~500 ms, then every ~33 ms.
        let mut now = start + Duration::from_millis(500);
        for _ in 0..30 {
            assert!(!filter.accept(Key::Z, KeyEventKind::Press, now));
            now += Duration::from_millis(33);
        }
        // Released, then tapped again well after the last repeat.
        assert!(filter.accept(Key::Z, KeyEventKind::Press, now + Duration::from_secs(1)));
    }

    #[test]
    fn non_recording_keys_are_never_filtered() {
        let start = Instant::now();
        let mut filter = RepeatFilter::new();
        for step in 0..5 {
            let now = start + Duration::from_millis(33 * step);
            assert!(filter.accept(Key::ArrowRight, KeyEventKind::Press, now));
        }
        assert!(filter.accept(Key::Z, KeyEventKind::Press, start));
        assert!(filter.accept(
            Key::Space,
            KeyEventKind::Press,
            start + Duration::from_millis(10)
        ));
    }

    #[test]
    fn releases_and_repeats_are_dropped() {
        let start = Instant::now();
        let mut filter = RepeatFilter::new();
        assert!(!filter.accept(Key::Z, KeyEventKind::Release, start));
        assert!(!filter.accept(Key::ArrowLeft, KeyEventKind::Repeat, start));
    }
}
