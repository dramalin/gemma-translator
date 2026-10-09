# Rust UI Phase A (desktop-buildable part) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust + Slint replacement for the Chromium/React kiosk UI that records and plays audio natively and talks to the existing Python speech service and `litert-lm serve` over HTTP, runnable on a desktop with `--mock`.

**Architecture:** A Cargo workspace at the repo root with three crates. `translator-core` holds all pure logic (languages, key mapping, sentence splitting, config, the per-lane state machine) and is unit-tested anywhere. `translator-io` holds everything that touches devices, processes or the network (cpal audio, HTTP clients, volume commands, the job runner thread). `translator-ui` is the Slint binary `translator` that wires them together, with mock services for desktop work. The Python backend gains two raw-PCM endpoints (`/v2/stt`, `/v2/tts`); the old endpoints and `frontend/` stay untouched.

**Tech Stack:** Rust stable (edition 2021), Slint 1.17 (winit + femtovg on desktop, linuxkms on the Pi), cpal 0.15, ureq 2 (no TLS), serde/serde_json/toml 0.8, thiserror 2; Python 3 + numpy for the backend.

**Spec:** `docs/superpowers/specs/2026-10-09-rust-frontend-design.md` (sections 1–7, 9–11 phase A steps 2–5). Phase A step 1 and step 6 (Pi hardware checks, deployment switch) and all of phase B are out of scope.

## Global Constraints

- Workspace members: `translator-core`, `translator-io`, `translator-ui`; release profile `lto = "thin"`, `codegen-units = 1`.
- `translator-core` must not depend on `cpal`, `ureq`, `slint` or any HTTP crate.
- Languages and revolver order: `ar, en, es, ja, zh, ko`; the two lanes never hold the same language. Defaults: lane 1 = `ar`, lane 2 = `en`.
- Key mapping identical to README: landscape `Space` / `Z` / `←` `→`; vertical `Z` / `X` / `←` `→` (lane 1) / `−` `+` (lane 2, also `_` and `=`). `Esc` opens/closes settings.
- Language rotation and active-lane switching are blocked while recording.
- Recordings shorter than 300 ms are discarded (new behaviour). Empty transcription shows `(No speech detected)`.
- Speech chunks are at most 180 characters.
- Config: TOML at `$XDG_CONFIG_HOME/gemma-translator/config.toml` (fallback `~/.config/...`), snake_case keys.
- Wire JSON keys camelCase; env vars SCREAMING_SNAKE_CASE; booleans prefixed `is`/`has`/`should`/`can`; event handlers prefixed `handle`/`on`.
- No TODOs or placeholders; only add functions the change actually calls.
- Commit messages in imperative mood. Do not touch `frontend/`, `start.sh`, `deploy-pi.sh` or `deploy/`.
- Visualizer bars: 8–128 in steps of 8, default 16. Volume steps: 5 %.

### Rulings that refine the spec (recorded so reviewers do not flag them)

1. Spec §6.4 names `rubato`; this plan uses a linear-interpolation resampler (`audio_convert::resample_linear`), a port of the current `audioHelpers.js::resample`, to avoid an unverified dependency API. Speech quality is unchanged from today.
2. Spec §3.1 model/voice table belongs to phase B (only FFI uses it). In phase A the Python service keeps `TTS_LANG_MAP`/`TTS_VOICE_MAP`; `LanguageCode` only carries what phase A uses.
3. Spec §10 bundled fonts: phase A uses system fonts (`DejaVu Sans Mono` as the default family). Font packaging is decided during the Pi checks.
4. The old response drawer becomes an always-visible transcript panel (keyboard-only UI has no way to dismiss a drawer).

## Review Focus

1. Speech service or LLM not running → each job ends with `Error: …` in the status line and the lane returns to ready; nothing hangs. (Task 8 connection-refused test, Task 5 failure test.)
2. Config file location not writable (read-only root on the Pi) → saving logs an error and the app keeps running with in-memory settings. (Task 4 unwritable-path test.)
3. No microphone or speaker on the machine → app still starts; capture yields silence, playback is discarded, jobs still finish. (Task 7 `DiscardingSink` test, Task 12 fallback.)
4. A long reply with no punctuation (common for CJK) → never a speech chunk over 180 characters. (Task 3 CJK and English long-text tests.)
5. No `wpctl`, `pactl` or `amixer` installed → volume shows `N/A`, no panic. (Task 9 all-missing test.)

---

## File Structure

```
Cargo.toml                         workspace + release profile
rust-toolchain.toml                stable channel
Makefile                           test / run-mock / run / fmt / clippy / build-pi
translator-core/
  Cargo.toml
  src/lib.rs                       module list
  src/lane.rs                      Lane
  src/language.rs                  LanguageCode, RotateDirection, rotate_language
  src/keys.rs                      KeyboardMode, Key, Command, key → command mapping
  src/settings.rs                  SettingsItem, SettingsAction, SettingsCursor
  src/text.rs                      system prompt, reply cleaning, SentenceSplitter
  src/config.rs                    ThemeColor, Config load/save
  src/pipeline.rs                  AudioClip, PipelineError, SpeechToText/Translator/TextToSpeech
  src/controller.rs                state machine: Phase, Effect, JobRequest, JobEvent, Controller
translator-io/
  Cargo.toml
  src/lib.rs
  src/audio_convert.rs             downmix, resample, PCM16 conversion, rms
  src/blip.rs                      synthesized UI sounds
  src/playback_queue.rs            PlaybackSink, PlaybackQueue, DiscardingSink
  src/speech_client.rs             HTTP client for /v2/stt and /v2/tts
  src/llm_client.rs                streaming chat-completions client
  src/volume.rs                    wpctl → pactl → amixer
  src/runner.rs                    JobRunner: STT → streaming translation → TTS pipeline
  src/audio_device.rs              cpal AudioRecorder / AudioPlayer
  tests/support/mod.rs             one-shot HTTP test server
  tests/speech_client.rs
  tests/llm_client.rs
  tests/runner.rs
translator-ui/
  Cargo.toml
  build.rs
  ui/app.slint
  src/main.rs                      CLI parsing, window + timer wiring
  src/app.rs                       AppState: keys → controller → effects → devices
  src/view.rs                      controller state → Slint structs
  src/mock.rs                      mock services for --mock
backend/pcm.py                     PCM/JSON helpers for /v2
backend/test_pcm.py
backend/server.py                  + /v2/stt, /v2/tts routes
```

---

### Task 1: Workspace scaffold, lanes and languages

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `translator-core/Cargo.toml`, `translator-core/src/lib.rs`, `translator-core/src/lane.rs`, `translator-core/src/language.rs`
- Modify: `.gitignore` (insert after line 2)

**Interfaces:**
- Produces: `translator_core::lane::Lane { One, Two }` with `fn other(self) -> Lane`; `translator_core::language::{LanguageCode, RotateDirection, rotate_language}`; `LanguageCode::{ALL, code, from_code, display_name, is_right_to_left}`.

- [ ] **Step 1: Create the workspace files**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["translator-core"]

[workspace.package]
edition = "2021"
version = "0.1.0"
license = "Apache-2.0"

[profile.release]
lto = "thin"
codegen-units = 1
```

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
```

In `.gitignore`, insert `target/` as a new line 3 (directly after `dist/`), leaving the rest of the file unchanged.

`translator-core/Cargo.toml`:
```toml
[package]
name = "translator-core"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
toml = "0.8"

[dev-dependencies]
tempfile = "3"
```

`translator-core/src/lib.rs`:
```rust
//! Portable logic for the Gemma translator: no audio, network or UI dependencies,
//! so everything here is unit-testable on any machine.

pub mod lane;
pub mod language;
```

`translator-core/src/lane.rs`:
```rust
/// One of the two people facing each other across the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    One,
    Two,
}

impl Lane {
    pub fn other(self) -> Lane {
        match self {
            Lane::One => Lane::Two,
            Lane::Two => Lane::One,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_swaps_lanes() {
        assert_eq!(Lane::One.other(), Lane::Two);
        assert_eq!(Lane::Two.other(), Lane::One);
    }
}
```

- [ ] **Step 2: Write the failing language tests**

`translator-core/src/language.rs` (tests first; the implementation follows in Step 4):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for language in LanguageCode::ALL {
            assert_eq!(LanguageCode::from_code(language.code()), Some(language));
        }
        assert_eq!(LanguageCode::from_code("fr"), None);
    }

    #[test]
    fn revolver_order_matches_old_ui() {
        let codes: Vec<&str> = LanguageCode::ALL.iter().map(|l| l.code()).collect();
        assert_eq!(codes, ["ar", "en", "es", "ja", "zh", "ko"]);
    }

    #[test]
    fn rotate_next_skips_language_of_other_lane() {
        let rotated = rotate_language(LanguageCode::Ar, LanguageCode::En, RotateDirection::Next);
        assert_eq!(rotated, LanguageCode::Es);
    }

    #[test]
    fn rotate_previous_wraps_and_skips() {
        let rotated = rotate_language(LanguageCode::Ar, LanguageCode::Ko, RotateDirection::Previous);
        assert_eq!(rotated, LanguageCode::Zh);
        let rotated = rotate_language(LanguageCode::Ar, LanguageCode::En, RotateDirection::Previous);
        assert_eq!(rotated, LanguageCode::Ko);
    }

    #[test]
    fn only_arabic_is_right_to_left() {
        let rtl: Vec<LanguageCode> =
            LanguageCode::ALL.into_iter().filter(|l| l.is_right_to_left()).collect();
        assert_eq!(rtl, [LanguageCode::Ar]);
    }

    #[test]
    fn display_names_are_english() {
        assert_eq!(LanguageCode::Zh.display_name(), "Chinese");
        assert_eq!(LanguageCode::Ar.display_name(), "Arabic");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p translator-core`
Expected: compile errors (`LanguageCode` not found).

- [ ] **Step 4: Implement `language.rs`** (place above the test module)

```rust
use serde::{Deserialize, Serialize};

/// A language offered on the lane revolvers. Serialized as its lowercase code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguageCode {
    Ar,
    En,
    Es,
    Ja,
    Zh,
    Ko,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotateDirection {
    Previous,
    Next,
}

impl LanguageCode {
    /// Revolver order, identical to `AVAILABLE_LANGUAGES` in the old React UI.
    pub const ALL: [LanguageCode; 6] = [
        LanguageCode::Ar,
        LanguageCode::En,
        LanguageCode::Es,
        LanguageCode::Ja,
        LanguageCode::Zh,
        LanguageCode::Ko,
    ];

    pub fn code(self) -> &'static str {
        match self {
            LanguageCode::Ar => "ar",
            LanguageCode::En => "en",
            LanguageCode::Es => "es",
            LanguageCode::Ja => "ja",
            LanguageCode::Zh => "zh",
            LanguageCode::Ko => "ko",
        }
    }

    pub fn from_code(code: &str) -> Option<LanguageCode> {
        LanguageCode::ALL.into_iter().find(|language| language.code() == code)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            LanguageCode::Ar => "Arabic",
            LanguageCode::En => "English",
            LanguageCode::Es => "Spanish",
            LanguageCode::Ja => "Japanese",
            LanguageCode::Zh => "Chinese",
            LanguageCode::Ko => "Korean",
        }
    }

    pub fn is_right_to_left(self) -> bool {
        matches!(self, LanguageCode::Ar)
    }

    fn index(self) -> usize {
        LanguageCode::ALL
            .iter()
            .position(|language| *language == self)
            .expect("ALL lists every variant")
    }
}

/// Next language on a lane's revolver, skipping the slot held by the other lane
/// (the two lanes may never show the same language).
pub fn rotate_language(
    current: LanguageCode,
    other: LanguageCode,
    direction: RotateDirection,
) -> LanguageCode {
    let count = LanguageCode::ALL.len();
    let step = match direction {
        RotateDirection::Previous => count - 1,
        RotateDirection::Next => 1,
    };
    let mut index = (current.index() + step) % count;
    if LanguageCode::ALL[index] == other {
        index = (index + step) % count;
    }
    LanguageCode::ALL[index]
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p translator-core && cargo fmt --all -- --check && cargo clippy -p translator-core --all-targets -- -D warnings`
Expected: all tests pass, no format diff, no clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore translator-core
git commit -m "Add Rust workspace with lane and language model"
```

---

### Task 2: Key mapping and settings cursor

**Files:**
- Create: `translator-core/src/keys.rs`, `translator-core/src/settings.rs`
- Modify: `translator-core/src/lib.rs`

**Interfaces:**
- Consumes: `Lane`, `RotateDirection` (Task 1).
- Produces:
  - `keys::KeyboardMode { Landscape (default), Vertical }` (serde lowercase)
  - `keys::Key { Space, Z, X, ArrowLeft, ArrowRight, ArrowUp, ArrowDown, Minus, Plus, Escape }`, `Key::from_event_text(&str) -> Option<Key>`
  - `keys::Command { ToggleActiveLane, StartRecording(Lane), StopRecording(Lane), RotateLanguage(Lane, RotateDirection), ToggleSettings }`
  - `keys::command_for_key_press(KeyboardMode, Key, Lane) -> Option<Command>`, `keys::command_for_key_release(KeyboardMode, Key, Lane) -> Option<Command>`
  - `settings::SettingsItem { KeyboardMode, ThemeColor, EnableTts, VisualizerBars, Volume }` with `ALL`
  - `settings::SettingsAction { ToggleKeyboardMode, CycleThemeColor(RotateDirection), ToggleTts, AdjustVisualizerBars(RotateDirection), AdjustVolume(RotateDirection) }`
  - `settings::SettingsCursor` (Default) with `selected() -> SettingsItem`, `handle_key(&mut self, Key) -> Option<SettingsAction>`

- [ ] **Step 1: Register the modules**

`translator-core/src/lib.rs` module list becomes:
```rust
pub mod keys;
pub mod lane;
pub mod language;
pub mod settings;
```

- [ ] **Step 2: Write the failing tests**

`translator-core/src/keys.rs` test module:
```rust
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
        assert_eq!(command_for_key_press(mode, Key::Space, Lane::One), Some(Command::ToggleActiveLane));
        assert_eq!(command_for_key_press(mode, Key::Z, Lane::Two), Some(Command::StartRecording(Lane::Two)));
        assert_eq!(
            command_for_key_press(mode, Key::ArrowLeft, Lane::Two),
            Some(Command::RotateLanguage(Lane::Two, Previous))
        );
        assert_eq!(
            command_for_key_press(mode, Key::ArrowRight, Lane::One),
            Some(Command::RotateLanguage(Lane::One, Next))
        );
        assert_eq!(command_for_key_press(mode, Key::X, Lane::One), None);
        assert_eq!(command_for_key_release(mode, Key::Z, Lane::Two), Some(Command::StopRecording(Lane::Two)));
    }

    #[test]
    fn vertical_gives_each_lane_its_own_keys() {
        let mode = KeyboardMode::Vertical;
        assert_eq!(command_for_key_press(mode, Key::Z, Lane::Two), Some(Command::StartRecording(Lane::One)));
        assert_eq!(command_for_key_press(mode, Key::X, Lane::One), Some(Command::StartRecording(Lane::Two)));
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
        assert_eq!(command_for_key_release(mode, Key::Z, Lane::Two), Some(Command::StopRecording(Lane::One)));
        assert_eq!(command_for_key_release(mode, Key::X, Lane::One), Some(Command::StopRecording(Lane::Two)));
    }

    #[test]
    fn escape_toggles_settings_in_both_modes() {
        for mode in [KeyboardMode::Landscape, KeyboardMode::Vertical] {
            assert_eq!(command_for_key_press(mode, Key::Escape, Lane::One), Some(Command::ToggleSettings));
            assert_eq!(command_for_key_release(mode, Key::Escape, Lane::One), None);
        }
    }
}
```

`translator-core/src/settings.rs` test module:
```rust
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
        assert_eq!(cursor.handle_key(Key::ArrowRight), Some(SettingsAction::ToggleKeyboardMode));
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(
            cursor.handle_key(Key::ArrowLeft),
            Some(SettingsAction::CycleThemeColor(RotateDirection::Previous))
        );
        cursor.handle_key(Key::ArrowDown);
        assert_eq!(cursor.handle_key(Key::ArrowLeft), Some(SettingsAction::ToggleTts));
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p translator-core`
Expected: compile errors (types not defined).

- [ ] **Step 4: Implement `keys.rs`** (above its tests)

```rust
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
            Key::ArrowLeft => Some(Command::RotateLanguage(active_lane, RotateDirection::Previous)),
            Key::ArrowRight => Some(Command::RotateLanguage(active_lane, RotateDirection::Next)),
            _ => None,
        },
        KeyboardMode::Vertical => match key {
            Key::Z => Some(Command::StartRecording(Lane::One)),
            Key::X => Some(Command::StartRecording(Lane::Two)),
            Key::ArrowLeft => Some(Command::RotateLanguage(Lane::One, RotateDirection::Previous)),
            Key::ArrowRight => Some(Command::RotateLanguage(Lane::One, RotateDirection::Next)),
            Key::Minus => Some(Command::RotateLanguage(Lane::Two, RotateDirection::Previous)),
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
```

- [ ] **Step 5: Implement `settings.rs`** (above its tests)

```rust
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
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p translator-core && cargo fmt --all -- --check && cargo clippy -p translator-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add translator-core
git commit -m "Add keyboard mapping and settings navigation"
```

---

### Task 3: Translation prompt, reply cleaning and sentence splitting

**Files:**
- Create: `translator-core/src/text.rs`
- Modify: `translator-core/src/lib.rs` (add `pub mod text;`)

**Interfaces:**
- Consumes: `LanguageCode` (Task 1).
- Produces: `text::MAX_SPEECH_CHUNK_CHARS: usize = 180`; `text::translation_system_prompt(LanguageCode, LanguageCode) -> String`; `text::clean_model_reply(&str) -> String`; `text::SentenceSplitter` (Default) with `new()`, `push(&mut self, &str) -> Vec<String>`, `finish(&mut self) -> Vec<String>`.

- [ ] **Step 1: Write the failing tests**

`translator-core/src/text.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_names_both_languages_and_forbids_extras() {
        let prompt = translation_system_prompt(LanguageCode::En, LanguageCode::Ja);
        assert!(prompt.contains("from English into Japanese"));
        assert!(prompt.contains("Output only the translation"));
    }

    #[test]
    fn cleaning_strips_fences_labels_quotes_and_markdown() {
        assert_eq!(clean_model_reply("```\nHola\n```"), "Hola");
        assert_eq!(clean_model_reply("Translation: Bonjour"), "Bonjour");
        assert_eq!(clean_model_reply("TRANSLATED TEXT:  Hallo"), "Hallo");
        assert_eq!(clean_model_reply("\"Hello there.\""), "Hello there.");
        assert_eq!(clean_model_reply("「こんにちは」"), "こんにちは");
        assert_eq!(clean_model_reply("**Bold** move"), "Bold move");
        assert_eq!(clean_model_reply("# Title"), "Title");
        assert_eq!(clean_model_reply("It's fine."), "It's fine.");
        assert_eq!(clean_model_reply("   "), "");
    }

    #[test]
    fn splits_english_sentences_while_streaming() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("Hello world. How"), ["Hello world."]);
        assert!(splitter.push(" are you?").is_empty());
        assert_eq!(splitter.finish(), ["How are you?"]);
    }

    #[test]
    fn decimal_points_do_not_split() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("Pi is 3.14 today. "), ["Pi is 3.14 today."]);
    }

    #[test]
    fn ellipsis_and_mixed_marks_stay_in_one_sentence() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("Wait... Really?! Yes"), ["Wait...", "Really?!"]);
        assert_eq!(splitter.finish(), ["Yes"]);
    }

    #[test]
    fn cjk_terminators_split_without_spaces() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("你好。今天"), ["你好。"]);
        assert_eq!(splitter.finish(), ["今天"]);
    }

    #[test]
    fn closing_quote_after_terminator_waits_for_more_text() {
        let mut splitter = SentenceSplitter::new();
        assert!(splitter.push("「你好。").is_empty());
        assert_eq!(splitter.push("」再见"), ["「你好。」"]);
        assert_eq!(splitter.finish(), ["再见"]);
    }

    #[test]
    fn arabic_question_mark_splits() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("كيف حالك؟ "), ["كيف حالك؟"]);
    }

    #[test]
    fn long_english_without_punctuation_breaks_at_spaces() {
        let words: Vec<String> = (0..80).map(|i| format!("word{i}")).collect();
        let text = words.join(" ");
        let mut splitter = SentenceSplitter::new();
        let mut chunks = splitter.push(&text);
        chunks.extend(splitter.finish());
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert_eq!(chunks.join(" "), text);
    }

    #[test]
    fn long_cjk_without_punctuation_breaks_at_the_limit() {
        let text = "字".repeat(400);
        let mut splitter = SentenceSplitter::new();
        let mut chunks = splitter.push(&text);
        chunks.extend(splitter.finish());
        let lengths: Vec<usize> = chunks.iter().map(|c| c.chars().count()).collect();
        assert_eq!(lengths, [180, 180, 40]);
    }

    #[test]
    fn long_sentence_with_terminator_is_still_capped() {
        let text = format!("{}。", "字".repeat(200));
        let mut splitter = SentenceSplitter::new();
        let mut chunks = splitter.push(&text);
        chunks.extend(splitter.finish());
        assert!(chunks.iter().all(|c| c.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn finish_on_empty_input_returns_nothing() {
        assert!(SentenceSplitter::new().finish().is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-core text`
Expected: compile errors.

- [ ] **Step 3: Implement `text.rs`** (above its tests)

```rust
use crate::language::LanguageCode;

/// Upper bound for one synthesized speech chunk, as in the old UI.
pub const MAX_SPEECH_CHUNK_CHARS: usize = 180;

const ALWAYS_TERMINATES: [char; 5] = ['。', '！', '？', '؟', '\n'];
const TERMINATES_BEFORE_SPACE: [char; 3] = ['.', '!', '?'];
const CLOSING_MARKS: [char; 8] = ['"', '\'', '”', '’', '」', '』', ')', '）'];
const QUOTE_PAIRS: [(char, char); 5] =
    [('"', '"'), ('\'', '\''), ('“', '”'), ('「', '」'), ('『', '』')];
const REPLY_LABELS: [&str; 2] = ["translation:", "translated text:"];

pub fn translation_system_prompt(source: LanguageCode, target: LanguageCode) -> String {
    format!(
        "You are a translator. Translate the user's text from {} into {}. \
         Output only the translation, with no explanations, notes, quotes or formatting.",
        source.display_name(),
        target.display_name()
    )
}

/// Removes wrappers a small model tends to add around a translation:
/// code fences, "Translation:" labels, markdown emphasis and surrounding quotes.
pub fn clean_model_reply(reply: &str) -> String {
    let without_fences: Vec<&str> = reply
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect();
    let mut text = without_fences.join("\n").replace("**", "").replace("__", "");
    text = text.trim().trim_start_matches('#').trim().to_string();
    for label in REPLY_LABELS {
        let has_label = text
            .get(..label.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(label));
        if has_label {
            text = text[label.len()..].trim().to_string();
        }
    }
    strip_quote_pair(&text).trim().to_string()
}

fn strip_quote_pair(text: &str) -> &str {
    for (open, close) in QUOTE_PAIRS {
        if let Some(inner) = text.strip_prefix(open).and_then(|rest| rest.strip_suffix(close)) {
            return inner;
        }
    }
    text
}

/// Turns a stream of text fragments into speakable sentences of at most
/// `MAX_SPEECH_CHUNK_CHARS` characters.
#[derive(Debug, Default)]
pub struct SentenceSplitter {
    pending: String,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a fragment and returns every sentence completed by it.
    pub fn push(&mut self, fragment: &str) -> Vec<String> {
        self.pending.push_str(fragment);
        self.drain(false)
    }

    /// Returns everything still buffered, split to the length limit.
    pub fn finish(&mut self) -> Vec<String> {
        self.drain(true)
    }

    fn drain(&mut self, is_final: bool) -> Vec<String> {
        let mut sentences = Vec::new();
        loop {
            let found = find_sentence_end(&self.pending, is_final)
                .filter(|&end| self.pending[..end].chars().count() <= MAX_SPEECH_CHUNK_CHARS);
            let end = match found {
                Some(end) => end,
                None if self.pending.chars().count() > MAX_SPEECH_CHUNK_CHARS => {
                    forced_break(&self.pending)
                }
                None => break,
            };
            let rest = self.pending.split_off(end);
            let sentence = std::mem::replace(&mut self.pending, rest);
            push_trimmed(&mut sentences, &sentence);
        }
        if is_final {
            let rest = std::mem::take(&mut self.pending);
            push_trimmed(&mut sentences, &rest);
        }
        sentences
    }
}

/// Byte offset just past the first complete sentence, if any. A terminator
/// at the very end of a non-final buffer waits, because a closing quote or a
/// digit may still follow.
fn find_sentence_end(text: &str, is_final: bool) -> Option<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (index, &(_, c)) in chars.iter().enumerate() {
        let is_always = ALWAYS_TERMINATES.contains(&c);
        if !is_always && !TERMINATES_BEFORE_SPACE.contains(&c) {
            continue;
        }
        let mut after = index + 1;
        while after < chars.len() && is_trailing_mark(chars[after].1) {
            after += 1;
        }
        match chars.get(after) {
            Some(&(byte, next)) if is_always || next.is_whitespace() => return Some(byte),
            Some(_) => continue,
            None if is_final => return Some(text.len()),
            None => return None,
        }
    }
    None
}

fn is_trailing_mark(c: char) -> bool {
    CLOSING_MARKS.contains(&c) || ALWAYS_TERMINATES.contains(&c) || TERMINATES_BEFORE_SPACE.contains(&c)
}

/// Break point for text over the limit: the last whitespace within the
/// limit, or exactly at the limit when there is none (CJK).
fn forced_break(text: &str) -> usize {
    let limit = text
        .char_indices()
        .nth(MAX_SPEECH_CHUNK_CHARS)
        .map_or(text.len(), |(byte, _)| byte);
    match text[..limit].rfind(char::is_whitespace) {
        Some(byte) if byte > 0 => byte,
        _ => limit,
    }
}

fn push_trimmed(sentences: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
}
```

Note: `is_trailing_mark` includes `\n`; a newline right after a terminator is consumed with it, which is harmless because sentences are trimmed.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p translator-core && cargo fmt --all -- --check && cargo clippy -p translator-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add translator-core
git commit -m "Add translation prompt, reply cleaning and sentence splitting"
```

---

### Task 4: Config file

**Files:**
- Create: `translator-core/src/config.rs`
- Modify: `translator-core/src/lib.rs` (add `pub mod config;`)

**Interfaces:**
- Consumes: `KeyboardMode` (Task 2), `LanguageCode`, `RotateDirection`, `rotate_language` (Task 1).
- Produces:
  - `config::ThemeColor { Red, White, Yellow, Blue, Green, Orange (default) }` with `ALL`, `hex()`, `name()`, `rgb() -> (u8, u8, u8)`, `cycled(RotateDirection) -> ThemeColor`; serialized as its hex string.
  - `config::VISUALIZER_BARS_MIN = 8`, `VISUALIZER_BARS_MAX = 128`, `VISUALIZER_BARS_STEP = 8`.
  - `config::Config` (pub fields: `keyboard_mode`, `theme_color`, `enable_tts`, `visualizer_bars: u32`, `lane1_language`, `lane2_language`, `speech_service_url: String`, `llm_endpoint: String`, `llm_model: String`, `audio_input_device: Option<String>`, `audio_output_device: Option<String>`), `Default`, `Config::default_path() -> Option<PathBuf>`, `Config::load(&Path) -> Result<Config, ConfigError>`, `Config::save(&self, &Path) -> Result<(), ConfigError>`.
  - `config::ConfigError`.

- [ ] **Step 1: Write the failing tests**

`translator-core/src/config.rs` test module:
```rust
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
        assert!(matches!(Config::load(&path), Err(ConfigError::Parse { .. })));
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
        assert_eq!(ThemeColor::Orange.cycled(RotateDirection::Next), ThemeColor::Red);
        assert_eq!(ThemeColor::Red.cycled(RotateDirection::Previous), ThemeColor::Orange);
        assert_eq!(ThemeColor::Orange.rgb(), (0xff, 0xa5, 0x00));
        assert_eq!(ThemeColor::Blue.name(), "BLUE");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-core config`
Expected: compile errors.

- [ ] **Step 3: Implement `config.rs`** (above its tests)

```rust
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
    Parse { path: PathBuf, source: toml::de::Error },
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
            Err(source) => return Err(ConfigError::Read { path: path.to_path_buf(), source }),
        };
        let config: Config = toml::from_str(&text)
            .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?;
        Ok(config.normalized())
    }

    /// Writes atomically (temp file + rename) so a power cut never leaves half a file.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self)?;
        let write_error = |source| ConfigError::Write { path: path.to_path_buf(), source };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(write_error)?;
        }
        let temp_path = path.with_extension("toml.tmp");
        fs::write(&temp_path, text).map_err(write_error)?;
        fs::rename(&temp_path, path).map_err(write_error)
    }

    fn normalized(mut self) -> Config {
        let bars = self.visualizer_bars.clamp(VISUALIZER_BARS_MIN, VISUALIZER_BARS_MAX);
        self.visualizer_bars = bars / VISUALIZER_BARS_STEP * VISUALIZER_BARS_STEP;
        if self.lane1_language == self.lane2_language {
            self.lane2_language =
                rotate_language(self.lane2_language, self.lane1_language, RotateDirection::Next);
        }
        self
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p translator-core && cargo fmt --all -- --check && cargo clippy -p translator-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add translator-core Cargo.lock
git commit -m "Add TOML config with theme colors and validation"
```

---

### Task 5: Pipeline traits and the lane state machine

**Files:**
- Create: `translator-core/src/pipeline.rs`, `translator-core/src/controller.rs`
- Modify: `translator-core/src/lib.rs` (add `pub mod controller;` and `pub mod pipeline;`)

**Interfaces:**
- Consumes: everything from Tasks 1–4.
- Produces:
  - `pipeline::STT_SAMPLE_RATE: u32 = 16_000`; `pipeline::AudioClip { samples: Vec<f32>, sample_rate: u32 }`; `pipeline::PipelineError { Unreachable { service, message }, Service { service, status: u16, message }, InvalidResponse { service, message } }` (`service: &'static str`, derives Clone, PartialEq); traits `SpeechToText`, `Translator`, `TextToSpeech` (all `Send + Sync`) with the signatures below.
  - `controller::{MIN_RECORDING_DURATION, NO_SPEECH_TEXT, TRANSCRIPTION_FAILED_TEXT, JobId(pub u64), JobRequest { id, source, target, enable_tts }, JobEvent, Blip { Language, Ping, Speaker }, Effect, Phase, Transcript, Controller}`.
  - `Controller::new(Config)`, `config()`, `phase()`, `active_lane()`, `is_settings_open()`, `transcript()`, `language(Lane)`, `handle_command(Command) -> Vec<Effect>`, `on_capture_finished(Duration) -> Vec<Effect>`, `on_job_event(JobEvent) -> Vec<Effect>`, `apply_settings_action(SettingsAction) -> Vec<Effect>`.

- [ ] **Step 1: Write `pipeline.rs`** (types only; exercised by the controller and runner tests)

```rust
use std::sync::atomic::AtomicBool;

use crate::language::LanguageCode;

/// Sample rate the speech recognizer expects.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Mono audio with its own sample rate (synthesis rates vary per engine).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PipelineError {
    #[error("cannot reach {service}: {message}")]
    Unreachable { service: &'static str, message: String },
    #[error("{service} returned HTTP {status}: {message}")]
    Service { service: &'static str, status: u16, message: String },
    #[error("invalid response from {service}: {message}")]
    InvalidResponse { service: &'static str, message: String },
}

pub trait SpeechToText: Send + Sync {
    /// `samples` must be 16 kHz mono.
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError>;
}

pub trait Translator: Send + Sync {
    /// Calls `on_sentence` once per completed sentence. Returns `Ok(())` early
    /// once `cancel` is set.
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError>;
}

pub trait TextToSpeech: Send + Sync {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError>;
}
```

- [ ] **Step 2: Write the failing controller tests**

`translator-core/src/controller.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeColor;
    use crate::language::RotateDirection::{Next, Previous};

    fn controller() -> Controller {
        Controller::new(Config::default())
    }

    /// Records on `lane` for one second and returns the started job.
    fn record(controller: &mut Controller, lane: Lane) -> JobRequest {
        controller.handle_command(Command::StartRecording(lane));
        controller.handle_command(Command::StopRecording(lane));
        match controller.on_capture_finished(Duration::from_secs(1)).as_slice() {
            [Effect::StartJob(request)] => request.clone(),
            other => panic!("expected StartJob, got {other:?}"),
        }
    }

    #[test]
    fn start_recording_on_active_lane() {
        let mut c = controller();
        let effects = c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(effects, [Effect::StopPlayback, Effect::PlayBlip(Blip::Ping), Effect::StartCapture]);
        assert_eq!(c.phase(), Phase::Recording(Lane::One));
    }

    #[test]
    fn recording_other_lane_makes_it_active() {
        let mut c = controller();
        let effects = c.handle_command(Command::StartRecording(Lane::Two));
        assert_eq!(
            effects,
            [
                Effect::StopPlayback,
                Effect::PlayBlip(Blip::Speaker),
                Effect::PlayBlip(Blip::Ping),
                Effect::StartCapture
            ]
        );
        assert_eq!(c.active_lane(), Lane::Two);
    }

    #[test]
    fn repeated_start_and_foreign_stop_are_ignored() {
        let mut c = controller();
        c.handle_command(Command::StartRecording(Lane::One));
        assert!(c.handle_command(Command::StartRecording(Lane::One)).is_empty());
        assert!(c.handle_command(Command::StopRecording(Lane::Two)).is_empty());
        assert_eq!(c.phase(), Phase::Recording(Lane::One));
    }

    #[test]
    fn short_recording_is_discarded() {
        let mut c = controller();
        c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(c.handle_command(Command::StopRecording(Lane::One)), [Effect::StopCapture]);
        assert_eq!(c.phase(), Phase::Transcribing(Lane::One));
        assert!(c.on_capture_finished(Duration::from_millis(200)).is_empty());
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn long_recording_translates_into_the_other_lane_language() {
        let mut c = controller();
        let request = record(&mut c, Lane::One);
        assert_eq!(
            request,
            JobRequest { id: JobId(1), source: LanguageCode::Ar, target: LanguageCode::En, enable_tts: true }
        );
        assert_eq!(c.transcript().source_label, "Arabic (Source)");
        assert_eq!(c.transcript().target_label, "English (Translation)");
        assert!(!c.transcript().is_translation_right_to_left);
    }

    #[test]
    fn full_job_with_speech() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed { job, text: "مرحبا".to_string() });
        assert_eq!(c.phase(), Phase::Translating(Lane::One));
        assert_eq!(c.transcript().transcription, "مرحبا");
        c.on_job_event(JobEvent::SentenceTranslated { job, sentence: "Hi.".to_string() });
        c.on_job_event(JobEvent::SentenceTranslated { job, sentence: "Bye.".to_string() });
        assert_eq!(c.transcript().translation, "Hi. Bye.");
        c.on_job_event(JobEvent::TranslationFinished { job });
        assert_eq!(c.phase(), Phase::Speaking(Lane::One));
        c.on_job_event(JobEvent::PlaybackFinished { job });
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn chinese_target_joins_sentences_without_spaces() {
        let mut c = Controller::new(Config { lane2_language: LanguageCode::Zh, ..Config::default() });
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed { job, text: "hello".to_string() });
        c.on_job_event(JobEvent::SentenceTranslated { job, sentence: "你好。".to_string() });
        c.on_job_event(JobEvent::SentenceTranslated { job, sentence: "再见。".to_string() });
        assert_eq!(c.transcript().translation, "你好。再见。");
    }

    #[test]
    fn arabic_target_is_right_to_left() {
        let mut c = controller();
        let request = record(&mut c, Lane::Two);
        assert_eq!(request.target, LanguageCode::Ar);
        assert!(c.transcript().is_translation_right_to_left);
    }

    #[test]
    fn without_speech_output_the_job_ends_after_translation() {
        let mut c = Controller::new(Config { enable_tts: false, ..Config::default() });
        let request = record(&mut c, Lane::One);
        assert!(!request.enable_tts);
        let job = request.id;
        c.on_job_event(JobEvent::Transcribed { job, text: "hi".to_string() });
        c.on_job_event(JobEvent::TranslationFinished { job });
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn empty_transcription_shows_no_speech() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed { job, text: "  ".to_string() });
        assert_eq!(c.transcript().translation, NO_SPEECH_TEXT);
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn events_from_other_jobs_are_ignored() {
        let mut c = controller();
        record(&mut c, Lane::One);
        let before = c.transcript().clone();
        assert!(c.on_job_event(JobEvent::Transcribed { job: JobId(99), text: "x".to_string() }).is_empty());
        assert_eq!(c.transcript(), &before);
        assert_eq!(c.phase(), Phase::Transcribing(Lane::One));
    }

    #[test]
    fn recording_during_a_job_cancels_it() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed { job, text: "hi".to_string() });
        let effects = c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(
            effects,
            [Effect::CancelJob(job), Effect::StopPlayback, Effect::PlayBlip(Blip::Ping), Effect::StartCapture]
        );
        c.on_job_event(JobEvent::SentenceTranslated { job, sentence: "late".to_string() });
        assert_ne!(c.transcript().translation, "late");
        assert_eq!(c.phase(), Phase::Recording(Lane::One));
    }

    #[test]
    fn transcription_failure_is_reported() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        let effects = c.on_job_event(JobEvent::Failed { job, message: "boom".to_string() });
        assert_eq!(effects, [Effect::StopPlayback]);
        assert_eq!(c.transcript().transcription, TRANSCRIPTION_FAILED_TEXT);
        assert_eq!(c.transcript().status, "Error: boom");
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn failure_after_translation_keeps_the_text() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed { job, text: "hi".to_string() });
        c.on_job_event(JobEvent::SentenceTranslated { job, sentence: "Hello.".to_string() });
        c.on_job_event(JobEvent::TranslationFinished { job });
        c.on_job_event(JobEvent::Failed { job, message: "tts down".to_string() });
        assert_eq!(c.transcript().translation, "Hello.");
        assert_eq!(c.transcript().status, "Error: tts down");
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn a_new_job_clears_the_previous_error() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Failed { job, message: "boom".to_string() });
        record(&mut c, Lane::One);
        assert_eq!(c.transcript().status, "");
    }

    #[test]
    fn rotation_skips_the_other_lane_and_saves() {
        let mut c = controller();
        let effects = c.handle_command(Command::RotateLanguage(Lane::One, Next));
        assert_eq!(effects, [Effect::PlayBlip(Blip::Language), Effect::SaveConfig]);
        assert_eq!(c.language(Lane::One), LanguageCode::Es);
        c.handle_command(Command::RotateLanguage(Lane::Two, Previous));
        assert_eq!(c.config().lane2_language, LanguageCode::Ar);
    }

    #[test]
    fn rotation_and_lane_switch_are_blocked_while_recording() {
        let mut c = controller();
        c.handle_command(Command::StartRecording(Lane::One));
        assert!(c.handle_command(Command::RotateLanguage(Lane::One, Next)).is_empty());
        assert!(c.handle_command(Command::ToggleActiveLane).is_empty());
        assert!(c.handle_command(Command::ToggleSettings).is_empty());
        assert_eq!(c.language(Lane::One), LanguageCode::Ar);
    }

    #[test]
    fn toggling_the_active_lane_blips() {
        let mut c = controller();
        assert_eq!(c.handle_command(Command::ToggleActiveLane), [Effect::PlayBlip(Blip::Speaker)]);
        assert_eq!(c.active_lane(), Lane::Two);
    }

    #[test]
    fn open_settings_block_other_commands() {
        let mut c = controller();
        c.handle_command(Command::ToggleSettings);
        assert!(c.is_settings_open());
        assert!(c.handle_command(Command::StartRecording(Lane::One)).is_empty());
        assert_eq!(c.phase(), Phase::Idle);
        c.handle_command(Command::ToggleSettings);
        assert!(!c.is_settings_open());
    }

    #[test]
    fn settings_actions_update_config_only_while_open() {
        let mut c = controller();
        assert!(c.apply_settings_action(SettingsAction::ToggleTts).is_empty());
        assert!(c.config().enable_tts);
        c.handle_command(Command::ToggleSettings);
        assert_eq!(c.apply_settings_action(SettingsAction::ToggleKeyboardMode), [Effect::SaveConfig]);
        assert_eq!(c.config().keyboard_mode, KeyboardMode::Vertical);
        c.apply_settings_action(SettingsAction::CycleThemeColor(Next));
        assert_eq!(c.config().theme_color, ThemeColor::Red);
        c.apply_settings_action(SettingsAction::ToggleTts);
        assert!(!c.config().enable_tts);
        c.apply_settings_action(SettingsAction::AdjustVisualizerBars(Next));
        assert_eq!(c.config().visualizer_bars, 24);
        for _ in 0..20 {
            c.apply_settings_action(SettingsAction::AdjustVisualizerBars(Previous));
        }
        assert_eq!(c.config().visualizer_bars, 8);
        assert_eq!(
            c.apply_settings_action(SettingsAction::AdjustVolume(Next)),
            [Effect::AdjustVolume(Next)]
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p translator-core controller`
Expected: compile errors.

- [ ] **Step 4: Implement `controller.rs`** (above its tests)

```rust
//! Per-lane state machine. Pure: it receives commands, capture results and job
//! events, updates its state and returns the side effects for the caller to run.

use std::time::Duration;

use crate::config::{Config, VISUALIZER_BARS_MAX, VISUALIZER_BARS_MIN, VISUALIZER_BARS_STEP};
use crate::keys::{Command, KeyboardMode};
use crate::lane::Lane;
use crate::language::{rotate_language, LanguageCode, RotateDirection};
use crate::settings::SettingsAction;

/// Shorter recordings are treated as accidental key presses.
pub const MIN_RECORDING_DURATION: Duration = Duration::from_millis(300);
pub const NO_SPEECH_TEXT: &str = "(No speech detected)";
pub const TRANSCRIPTION_FAILED_TEXT: &str = "(Transcription failed)";
const IDLE_TRANSCRIPTION_TEXT: &str = "— listening —";
const IDLE_TRANSLATION_TEXT: &str = "— waiting —";
const ANALYZING_TEXT: &str = "Analyzing voice input...";
const TRANSLATING_TEXT: &str = "Translating...";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRequest {
    pub id: JobId,
    pub source: LanguageCode,
    pub target: LanguageCode,
    pub enable_tts: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    Transcribed { job: JobId, text: String },
    SentenceTranslated { job: JobId, sentence: String },
    TranslationFinished { job: JobId },
    PlaybackFinished { job: JobId },
    Failed { job: JobId, message: String },
}

impl JobEvent {
    pub fn job(&self) -> JobId {
        match self {
            JobEvent::Transcribed { job, .. }
            | JobEvent::SentenceTranslated { job, .. }
            | JobEvent::TranslationFinished { job }
            | JobEvent::PlaybackFinished { job }
            | JobEvent::Failed { job, .. } => *job,
        }
    }
}

/// UI sounds: language rotation click, recording-start ping, person switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blip {
    Language,
    Ping,
    Speaker,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    PlayBlip(Blip),
    StopPlayback,
    StartCapture,
    StopCapture,
    CancelJob(JobId),
    StartJob(JobRequest),
    AdjustVolume(RotateDirection),
    SaveConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording(Lane),
    Transcribing(Lane),
    Translating(Lane),
    Speaking(Lane),
}

/// Text shown in the transcript panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub source_label: String,
    pub transcription: String,
    pub target_label: String,
    pub translation: String,
    pub is_translation_right_to_left: bool,
    pub status: String,
}

pub struct Controller {
    config: Config,
    active_lane: Lane,
    phase: Phase,
    is_settings_open: bool,
    current_job: Option<JobRequest>,
    next_job_id: u64,
    has_translation: bool,
    transcript: Transcript,
}

impl Controller {
    pub fn new(config: Config) -> Self {
        Controller {
            config,
            active_lane: Lane::One,
            phase: Phase::Idle,
            is_settings_open: false,
            current_job: None,
            next_job_id: 0,
            has_translation: false,
            transcript: Transcript {
                source_label: String::new(),
                transcription: IDLE_TRANSCRIPTION_TEXT.to_string(),
                target_label: String::new(),
                translation: IDLE_TRANSLATION_TEXT.to_string(),
                is_translation_right_to_left: false,
                status: String::new(),
            },
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn active_lane(&self) -> Lane {
        self.active_lane
    }

    pub fn is_settings_open(&self) -> bool {
        self.is_settings_open
    }

    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    pub fn language(&self, lane: Lane) -> LanguageCode {
        match lane {
            Lane::One => self.config.lane1_language,
            Lane::Two => self.config.lane2_language,
        }
    }

    pub fn handle_command(&mut self, command: Command) -> Vec<Effect> {
        if self.is_settings_open && command != Command::ToggleSettings {
            return Vec::new();
        }
        match command {
            Command::ToggleActiveLane => self.toggle_active_lane(),
            Command::StartRecording(lane) => self.start_recording(lane),
            Command::StopRecording(lane) => self.stop_recording(lane),
            Command::RotateLanguage(lane, direction) => self.rotate_language_of(lane, direction),
            Command::ToggleSettings => self.toggle_settings(),
        }
    }

    /// Called after `Effect::StopCapture` with the length of the recording.
    pub fn on_capture_finished(&mut self, duration: Duration) -> Vec<Effect> {
        let Phase::Transcribing(lane) = self.phase else {
            return Vec::new();
        };
        if self.current_job.is_some() {
            return Vec::new();
        }
        if duration < MIN_RECORDING_DURATION {
            self.phase = Phase::Idle;
            return Vec::new();
        }
        self.next_job_id += 1;
        let request = JobRequest {
            id: JobId(self.next_job_id),
            source: self.language(lane),
            target: self.language(lane.other()),
            enable_tts: self.config.enable_tts,
        };
        self.transcript = Transcript {
            source_label: format!("{} (Source)", request.source.display_name()),
            transcription: ANALYZING_TEXT.to_string(),
            target_label: format!("{} (Translation)", request.target.display_name()),
            translation: TRANSLATING_TEXT.to_string(),
            is_translation_right_to_left: request.target.is_right_to_left(),
            status: String::new(),
        };
        self.has_translation = false;
        self.current_job = Some(request.clone());
        vec![Effect::StartJob(request)]
    }

    pub fn on_job_event(&mut self, event: JobEvent) -> Vec<Effect> {
        let Some(job) = self.current_job.clone() else {
            return Vec::new();
        };
        if event.job() != job.id {
            return Vec::new();
        }
        let lane = match self.phase {
            Phase::Transcribing(lane) | Phase::Translating(lane) | Phase::Speaking(lane) => lane,
            Phase::Idle | Phase::Recording(_) => return Vec::new(),
        };
        match event {
            JobEvent::Transcribed { text, .. } => {
                if text.trim().is_empty() {
                    self.transcript.transcription = String::new();
                    self.transcript.translation = NO_SPEECH_TEXT.to_string();
                    self.finish_job();
                } else {
                    self.transcript.transcription = text;
                    self.phase = Phase::Translating(lane);
                }
            }
            JobEvent::SentenceTranslated { sentence, .. } => {
                if self.has_translation {
                    if !joins_without_space(job.target) {
                        self.transcript.translation.push(' ');
                    }
                    self.transcript.translation.push_str(&sentence);
                } else {
                    self.transcript.translation = sentence;
                    self.has_translation = true;
                }
            }
            JobEvent::TranslationFinished { .. } => {
                if !self.has_translation {
                    self.transcript.translation = String::new();
                }
                if job.enable_tts {
                    self.phase = Phase::Speaking(lane);
                } else {
                    self.finish_job();
                }
            }
            JobEvent::PlaybackFinished { .. } => self.finish_job(),
            JobEvent::Failed { message, .. } => {
                if matches!(self.phase, Phase::Transcribing(_)) {
                    self.transcript.transcription = TRANSCRIPTION_FAILED_TEXT.to_string();
                }
                if !self.has_translation {
                    self.transcript.translation = String::new();
                }
                self.transcript.status = format!("Error: {message}");
                self.finish_job();
                return vec![Effect::StopPlayback];
            }
        }
        Vec::new()
    }

    pub fn apply_settings_action(&mut self, action: SettingsAction) -> Vec<Effect> {
        if !self.is_settings_open {
            return Vec::new();
        }
        match action {
            SettingsAction::ToggleKeyboardMode => {
                self.config.keyboard_mode = match self.config.keyboard_mode {
                    KeyboardMode::Landscape => KeyboardMode::Vertical,
                    KeyboardMode::Vertical => KeyboardMode::Landscape,
                };
            }
            SettingsAction::CycleThemeColor(direction) => {
                self.config.theme_color = self.config.theme_color.cycled(direction);
            }
            SettingsAction::ToggleTts => self.config.enable_tts = !self.config.enable_tts,
            SettingsAction::AdjustVisualizerBars(direction) => {
                let bars = self.config.visualizer_bars;
                self.config.visualizer_bars = match direction {
                    RotateDirection::Next => (bars + VISUALIZER_BARS_STEP).min(VISUALIZER_BARS_MAX),
                    RotateDirection::Previous => {
                        bars.saturating_sub(VISUALIZER_BARS_STEP).max(VISUALIZER_BARS_MIN)
                    }
                };
            }
            SettingsAction::AdjustVolume(direction) => return vec![Effect::AdjustVolume(direction)],
        }
        vec![Effect::SaveConfig]
    }

    fn is_recording(&self) -> bool {
        matches!(self.phase, Phase::Recording(_))
    }

    fn toggle_active_lane(&mut self) -> Vec<Effect> {
        if self.is_recording() {
            return Vec::new();
        }
        self.active_lane = self.active_lane.other();
        vec![Effect::PlayBlip(Blip::Speaker)]
    }

    fn start_recording(&mut self, lane: Lane) -> Vec<Effect> {
        if self.is_recording() {
            return Vec::new();
        }
        let mut effects = Vec::new();
        if let Some(job) = self.current_job.take() {
            effects.push(Effect::CancelJob(job.id));
        }
        effects.push(Effect::StopPlayback);
        if self.active_lane != lane {
            self.active_lane = lane;
            effects.push(Effect::PlayBlip(Blip::Speaker));
        }
        effects.push(Effect::PlayBlip(Blip::Ping));
        effects.push(Effect::StartCapture);
        self.phase = Phase::Recording(lane);
        effects
    }

    fn stop_recording(&mut self, lane: Lane) -> Vec<Effect> {
        if self.phase != Phase::Recording(lane) {
            return Vec::new();
        }
        self.phase = Phase::Transcribing(lane);
        vec![Effect::StopCapture]
    }

    fn rotate_language_of(&mut self, lane: Lane, direction: RotateDirection) -> Vec<Effect> {
        if self.is_recording() {
            return Vec::new();
        }
        let rotated = rotate_language(self.language(lane), self.language(lane.other()), direction);
        match lane {
            Lane::One => self.config.lane1_language = rotated,
            Lane::Two => self.config.lane2_language = rotated,
        }
        vec![Effect::PlayBlip(Blip::Language), Effect::SaveConfig]
    }

    fn toggle_settings(&mut self) -> Vec<Effect> {
        if self.is_recording() {
            return Vec::new();
        }
        self.is_settings_open = !self.is_settings_open;
        Vec::new()
    }

    fn finish_job(&mut self) {
        self.current_job = None;
        self.phase = Phase::Idle;
    }
}

/// Chinese and Japanese sentences are concatenated without spaces.
fn joins_without_space(language: LanguageCode) -> bool {
    matches!(language, LanguageCode::Zh | LanguageCode::Ja)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p translator-core && cargo fmt --all -- --check && cargo clippy -p translator-core --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add translator-core
git commit -m "Add pipeline traits and lane state machine"
```

---

### Task 6: Raw-PCM speech endpoints in the Python service

**Files:**
- Create: `backend/pcm.py`, `backend/test_pcm.py`
- Modify: `backend/server.py` (imports near line 26; new handler methods inside `ProxyHTTPRequestHandler`; routing at the top of `do_POST`)

**Interfaces:**
- Produces (HTTP, consumed by Task 8):
  - `POST /v2/stt?language=<code>` — body `application/octet-stream`, 16 kHz mono 16-bit little-endian PCM → `200 {"text": "..."}`; `400 {"error": ...}` for unsupported language / empty or odd-length body; `500 {"error": ...}` on inference failure.
  - `POST /v2/tts` — body JSON `{"text": "...", "language": "<code>"}` → `200 application/octet-stream` 16-bit LE PCM with `X-Sample-Rate` header; `400 {"error": ...}` for bad JSON, missing text or unsupported language; `500` on failure.

- [ ] **Step 1: Write the failing tests**

`backend/test_pcm.py`:
```python
# Copyright 2026 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import json
import unittest

import numpy as np

from pcm import STT_SAMPLE_RATE, decode_pcm16le, encode_pcm16le, parse_tts_request


class DecodePcmTests(unittest.TestCase):
    def test_decodes_little_endian_int16_to_unit_floats(self):
        body = np.array([0, 16384, -32768], dtype="<i2").tobytes()
        samples = decode_pcm16le(body)
        self.assertEqual(samples.dtype, np.float32)
        np.testing.assert_allclose(samples, [0.0, 0.5, -1.0])

    def test_rejects_empty_body(self):
        with self.assertRaises(ValueError):
            decode_pcm16le(b"")

    def test_rejects_odd_length_body(self):
        with self.assertRaises(ValueError):
            decode_pcm16le(b"\x00\x01\x02")


class EncodePcmTests(unittest.TestCase):
    def test_clips_and_scales_to_int16(self):
        encoded = encode_pcm16le([0.0, 0.5, 2.0, -2.0])
        np.testing.assert_array_equal(
            np.frombuffer(encoded, dtype="<i2"), [0, 16383, 32767, -32767]
        )


class ParseTtsRequestTests(unittest.TestCase):
    def test_reads_text_and_language(self):
        body = json.dumps({"text": "hola", "language": "es"}).encode("utf-8")
        self.assertEqual(parse_tts_request(body), ("hola", "es"))

    def test_language_defaults_to_english(self):
        body = json.dumps({"text": "hi"}).encode("utf-8")
        self.assertEqual(parse_tts_request(body), ("hi", "en"))

    def test_rejects_missing_or_blank_text(self):
        for payload in ({}, {"text": "  "}, {"text": 5}):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                parse_tts_request(json.dumps(payload).encode("utf-8"))

    def test_rejects_non_string_language(self):
        with self.assertRaises(ValueError):
            parse_tts_request(json.dumps({"text": "hi", "language": 1}).encode("utf-8"))

    def test_rejects_invalid_json(self):
        with self.assertRaises(ValueError):
            parse_tts_request(b"{not json")


class ConstantsTests(unittest.TestCase):
    def test_stt_sample_rate_is_16k(self):
        self.assertEqual(STT_SAMPLE_RATE, 16000)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `python3 -m unittest discover -s backend -p 'test_*.py' -v`
Expected: `ModuleNotFoundError: No module named 'pcm'`.

- [ ] **Step 3: Implement `backend/pcm.py`**

```python
# Copyright 2026 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Request/response helpers for the raw-PCM /v2 speech endpoints used by the Rust UI."""

import json

import numpy as np

# Moonshine STT expects 16 kHz mono.
STT_SAMPLE_RATE = 16000


def decode_pcm16le(body):
    """16-bit little-endian PCM bytes -> float32 samples in [-1, 1)."""
    if len(body) == 0:
        raise ValueError("Empty audio body")
    if len(body) % 2:
        raise ValueError("PCM body must have an even number of bytes")
    return np.frombuffer(body, dtype="<i2").astype(np.float32) / 32768.0


def encode_pcm16le(samples):
    """Float samples (clipped to [-1, 1]) -> 16-bit little-endian PCM bytes."""
    clipped = np.clip(np.asarray(samples, dtype=np.float32), -1.0, 1.0)
    return (clipped * 32767.0).astype("<i2").tobytes()


def parse_tts_request(body):
    """JSON body {"text", "language"} -> (text, language). Raises ValueError."""
    data = json.loads(body.decode("utf-8"))
    if not isinstance(data, dict):
        raise ValueError("Request body must be a JSON object")
    text = data.get("text")
    language = data.get("language", "en")
    if not isinstance(text, str) or not text.strip():
        raise ValueError('Missing "text"')
    if not isinstance(language, str):
        raise ValueError('"language" must be a string')
    return text, language
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `python3 -m unittest discover -s backend -p 'test_*.py' -v`
Expected: all tests pass.

- [ ] **Step 5: Wire the endpoints into `backend/server.py`**

After the line `from collections import OrderedDict` add:
```python
from pcm import STT_SAMPLE_RATE, decode_pcm16le, encode_pcm16le, parse_tts_request
```

Inside `ProxyHTTPRequestHandler`, directly after `handle_stt` (before `handle_volume`), add:
```python
    def send_json(self, status, payload):
        body = json.dumps(payload).encode('utf-8')
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def read_body(self):
        content_length = int(self.headers.get('Content-Length', 0))
        return self.rfile.read(content_length) if content_length > 0 else b''

    def handle_stt_v2(self):
        # Raw 16-bit PCM in, used by the Rust UI (no base64/JSON wrapping).
        query = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
        language = query.get('language', ['en'])[0]
        if language not in SUPPORTED_STT_LANGS:
            self.send_json(400, {"error": f"Unsupported language: {language}"})
            return
        try:
            samples = decode_pcm16le(self.read_body())
        except ValueError as e:
            self.send_json(400, {"error": str(e)})
            return
        try:
            with _stt_lock:
                recognizer = get_stt_recognizer(language)
                transcript = recognizer.transcribe_without_streaming(samples, STT_SAMPLE_RATE)
            text = " ".join(line.text for line in transcript.lines)
            print(f"[STT v2] Transcribed: {text}")
            self.send_json(200, {"text": text})
        except Exception as e:
            traceback.print_exc()
            self.send_json(500, {"error": str(e)})

    def handle_tts_v2(self):
        # JSON in, raw 16-bit PCM out; the sample rate travels in X-Sample-Rate.
        try:
            text, language = parse_tts_request(self.read_body())
        except ValueError as e:
            self.send_json(400, {"error": str(e)})
            return
        if language not in TTS_LANG_MAP:
            self.send_json(400, {"error": f"Unsupported language: {language}"})
            return
        try:
            with _tts_lock:
                engine = get_tts_engine(language)
                audio, sample_rate = engine.synthesize(text)
            body = encode_pcm16le(audio)
            self.send_response(200)
            self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Length', str(len(body)))
            self.send_header('X-Sample-Rate', str(int(sample_rate)))
            self.end_headers()
            self.wfile.write(body)
        except Exception as e:
            traceback.print_exc()
            self.send_json(500, {"error": str(e)})
```

At the very start of `do_POST`, before the `/proxy` check, add:
```python
        if self.path.startswith('/v2/stt'):
            self.handle_stt_v2()
            return
        if self.path.startswith('/v2/tts'):
            self.handle_tts_v2()
            return
```

- [ ] **Step 6: Verify the server still compiles and tests pass**

Run: `python3 -m py_compile backend/server.py && python3 -m unittest discover -s backend -p 'test_*.py'`
Expected: no output from `py_compile`; tests OK.

- [ ] **Step 7: Commit**

```bash
git add backend/pcm.py backend/test_pcm.py backend/server.py
git commit -m "Add raw-PCM /v2 speech endpoints for the Rust UI"
```

---

### Task 7: Audio conversion, UI blips and the playback queue

**Files:**
- Create: `translator-io/Cargo.toml`, `translator-io/src/lib.rs`, `translator-io/src/audio_convert.rs`, `translator-io/src/blip.rs`, `translator-io/src/playback_queue.rs`
- Modify: `Cargo.toml` (members add `"translator-io"`)

**Interfaces:**
- Consumes: `AudioClip` (Task 5), `Blip` (Task 5).
- Produces:
  - `audio_convert::{downmix_to_mono(&[f32], u16) -> Vec<f32>, resample_linear(&[f32], u32, u32) -> Vec<f32>, f32_to_pcm16_le(&[f32]) -> Vec<u8>, pcm16_le_to_f32(&[u8]) -> Option<Vec<f32>>, rms(&[f32]) -> f32}`
  - `blip::{BLIP_SAMPLE_RATE: u32 = 48_000, synthesize_blip(Blip, u32) -> AudioClip}`
  - `playback_queue::PlaybackSink` trait (`enqueue(&self, AudioClip)`, `clear(&self)`, `is_idle(&self) -> bool`; `Send + Sync`), `playback_queue::PlaybackQueue` (Clone; `new(u32)`, `fill(&self, &mut [f32], u16)`), `playback_queue::DiscardingSink`.

- [ ] **Step 1: Create the crate**

Root `Cargo.toml` members: `members = ["translator-core", "translator-io"]`.

`translator-io/Cargo.toml`:
```toml
[package]
name = "translator-io"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
translator-core = { path = "../translator-core" }
```
(Later tasks append dependencies.)

`translator-io/src/lib.rs`:
```rust
//! Device-, process- and network-bound parts of the translator.

pub mod audio_convert;
pub mod blip;
pub mod playback_queue;
```

- [ ] **Step 2: Write the failing tests**

`translator-io/src/audio_convert.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(downmix_to_mono(&[0.25, -0.25], 1), [0.25, -0.25]);
    }

    #[test]
    fn resample_keeps_same_rate_and_interpolates() {
        assert_eq!(resample_linear(&[0.1, 0.2], 16_000, 16_000), [0.1, 0.2]);
        let input: Vec<f32> = (0..48).map(|i| i as f32).collect();
        let output = resample_linear(&input, 48_000, 16_000);
        assert_eq!(output.len(), 16);
        assert_eq!(output[1], 3.0);
        let upsampled = resample_linear(&[0.0, 1.0], 16_000, 32_000);
        assert_eq!(upsampled, [0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn resample_of_empty_input_is_empty() {
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn pcm16_round_trip() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0];
        let decoded = pcm16_le_to_f32(&f32_to_pcm16_le(&samples)).unwrap();
        for (a, b) in samples.iter().zip(&decoded) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
        assert_eq!(f32_to_pcm16_le(&[2.0]), 32767i16.to_le_bytes());
    }

    #[test]
    fn odd_pcm_length_is_rejected() {
        assert_eq!(pcm16_le_to_f32(&[0, 1, 2]), None);
    }

    #[test]
    fn rms_of_constant_signal() {
        assert!((rms(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert_eq!(rms(&[]), 0.0);
    }
}
```

`translator-io/src/blip.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blip_lengths_match_the_old_web_audio_sounds() {
        assert_eq!(synthesize_blip(Blip::Speaker, 48_000).samples.len(), 4_800);
        assert_eq!(synthesize_blip(Blip::Language, 48_000).samples.len(), 4_800);
        assert_eq!(synthesize_blip(Blip::Ping, 48_000).samples.len(), 14_400);
    }

    #[test]
    fn blips_stay_quiet_and_start_silent() {
        for blip in [Blip::Speaker, Blip::Language, Blip::Ping] {
            let clip = synthesize_blip(blip, BLIP_SAMPLE_RATE);
            assert_eq!(clip.sample_rate, BLIP_SAMPLE_RATE);
            assert_eq!(clip.samples[0], 0.0);
            assert!(clip.samples.iter().all(|s| s.abs() <= 0.1 + 1e-6));
            assert!(clip.samples.iter().any(|s| s.abs() > 0.01));
        }
    }
}
```

`translator-io/src/playback_queue.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_duplicates_mono_into_every_channel_then_pads_with_silence() {
        let queue = PlaybackQueue::new(48_000);
        queue.enqueue(AudioClip { samples: vec![0.1, 0.2], sample_rate: 48_000 });
        let mut out = [9.0; 6];
        queue.fill(&mut out, 2);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 0.0, 0.0]);
        assert!(queue.is_idle());
    }

    #[test]
    fn enqueue_resamples_to_the_output_rate() {
        let queue = PlaybackQueue::new(48_000);
        queue.enqueue(AudioClip { samples: vec![0.0; 160], sample_rate: 16_000 });
        let mut out = vec![1.0; 480];
        queue.fill(&mut out, 1);
        assert!(queue.is_idle());
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn clear_empties_the_queue() {
        let queue = PlaybackQueue::new(48_000);
        queue.enqueue(AudioClip { samples: vec![0.5; 10], sample_rate: 48_000 });
        assert!(!queue.is_idle());
        queue.clear();
        assert!(queue.is_idle());
    }

    #[test]
    fn clones_share_the_same_buffer() {
        let queue = PlaybackQueue::new(48_000);
        let clone = queue.clone();
        clone.enqueue(AudioClip { samples: vec![0.5; 4], sample_rate: 48_000 });
        assert!(!queue.is_idle());
    }

    #[test]
    fn discarding_sink_is_always_idle() {
        let sink = DiscardingSink;
        sink.enqueue(AudioClip { samples: vec![0.5; 10], sample_rate: 48_000 });
        assert!(sink.is_idle());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p translator-io`
Expected: compile errors.

- [ ] **Step 4: Implement `audio_convert.rs`** (above its tests)

```rust
/// Averages interleaved frames into one channel.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Linear-interpolation resampler, a port of the old `audioHelpers.js::resample`.
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = f64::from(from_rate) / f64::from(to_rate);
    let output_len = (samples.len() as f64 / ratio).round() as usize;
    let last = samples.len() - 1;
    (0..output_len)
        .map(|i| {
            let position = i as f64 * ratio;
            let index = (position.floor() as usize).min(last);
            let fraction = (position - index as f64) as f32;
            let current = samples[index];
            let next = samples.get(index + 1).copied().unwrap_or(current);
            current + fraction * (next - current)
        })
        .collect()
}

pub fn f32_to_pcm16_le(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
        .collect()
}

/// `None` when the byte count is odd (truncated stream).
pub fn pcm16_le_to_f32(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
            .collect(),
    )
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}
```

- [ ] **Step 5: Implement `blip.rs`** (above its tests)

```rust
use std::f32::consts::TAU;

use translator_core::controller::Blip;
use translator_core::pipeline::AudioClip;

pub const BLIP_SAMPLE_RATE: u32 = 48_000;
const SILENT_GAIN: f32 = 0.001;

/// Envelope of one sound, copied from the old `audio-blip.js` oscillator settings.
struct BlipShape {
    start_hz: f32,
    end_hz: f32,
    sweep_seconds: f32,
    peak_gain: f32,
    attack_seconds: f32,
    duration_seconds: f32,
}

fn shape(blip: Blip) -> BlipShape {
    match blip {
        Blip::Speaker => BlipShape {
            start_hz: 600.0,
            end_hz: 800.0,
            sweep_seconds: 0.05,
            peak_gain: 0.08,
            attack_seconds: 0.01,
            duration_seconds: 0.1,
        },
        Blip::Ping => BlipShape {
            start_hz: 880.0,
            end_hz: 880.0,
            sweep_seconds: 0.0,
            peak_gain: 0.1,
            attack_seconds: 0.02,
            duration_seconds: 0.3,
        },
        Blip::Language => BlipShape {
            start_hz: 400.0,
            end_hz: 200.0,
            sweep_seconds: 0.05,
            peak_gain: 0.08,
            attack_seconds: 0.01,
            duration_seconds: 0.1,
        },
    }
}

/// Sine with an exponential pitch sweep, linear attack and exponential decay.
pub fn synthesize_blip(blip: Blip, sample_rate: u32) -> AudioClip {
    let shape = shape(blip);
    let rate = sample_rate as f32;
    let count = (shape.duration_seconds * rate).round() as usize;
    let mut phase = 0.0f32;
    let samples = (0..count)
        .map(|i| {
            let t = i as f32 / rate;
            let frequency = if t < shape.sweep_seconds {
                shape.start_hz * (shape.end_hz / shape.start_hz).powf(t / shape.sweep_seconds)
            } else {
                shape.end_hz
            };
            let gain = if t < shape.attack_seconds {
                shape.peak_gain * t / shape.attack_seconds
            } else {
                let progress =
                    (t - shape.attack_seconds) / (shape.duration_seconds - shape.attack_seconds);
                shape.peak_gain * (SILENT_GAIN / shape.peak_gain).powf(progress)
            };
            let sample = gain * phase.sin();
            phase = (phase + TAU * frequency / rate) % TAU;
            sample
        })
        .collect();
    AudioClip { samples, sample_rate }
}
```

- [ ] **Step 6: Implement `playback_queue.rs`** (above its tests)

```rust
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use translator_core::pipeline::AudioClip;

use crate::audio_convert::resample_linear;

/// Where synthesized speech and UI sounds go.
pub trait PlaybackSink: Send + Sync {
    fn enqueue(&self, clip: AudioClip);
    fn clear(&self);
    fn is_idle(&self) -> bool;
}

/// Mono sample FIFO drained by the output device callback. Clones share the buffer.
#[derive(Clone)]
pub struct PlaybackQueue {
    samples: Arc<Mutex<VecDeque<f32>>>,
    output_rate: u32,
}

impl PlaybackQueue {
    pub fn new(output_rate: u32) -> Self {
        PlaybackQueue { samples: Arc::new(Mutex::new(VecDeque::new())), output_rate }
    }

    /// Fills an interleaved device buffer; frames past the queued audio are silent.
    pub fn fill(&self, out: &mut [f32], channels: u16) {
        let mut samples = self.lock();
        for frame in out.chunks_mut(usize::from(channels.max(1))) {
            frame.fill(samples.pop_front().unwrap_or(0.0));
        }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.samples.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PlaybackSink for PlaybackQueue {
    fn enqueue(&self, clip: AudioClip) {
        let resampled = resample_linear(&clip.samples, clip.sample_rate, self.output_rate);
        self.lock().extend(resampled);
    }

    fn clear(&self) {
        self.lock().clear();
    }

    fn is_idle(&self) -> bool {
        self.lock().is_empty()
    }
}

/// Used when no output device exists: audio is dropped and jobs still finish.
pub struct DiscardingSink;

impl PlaybackSink for DiscardingSink {
    fn enqueue(&self, _clip: AudioClip) {}

    fn clear(&self) {}

    fn is_idle(&self) -> bool {
        true
    }
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p translator-io && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock translator-io
git commit -m "Add audio conversion, UI blips and playback queue"
```

---

### Task 8: HTTP clients for the speech service and the LLM

**Files:**
- Create: `translator-io/src/speech_client.rs`, `translator-io/src/llm_client.rs`, `translator-io/tests/support/mod.rs`, `translator-io/tests/speech_client.rs`, `translator-io/tests/llm_client.rs`
- Modify: `translator-io/Cargo.toml` (dependencies), `translator-io/src/lib.rs` (add `pub mod llm_client;`, `pub mod speech_client;`)

**Interfaces:**
- Consumes: Task 6 HTTP contract; `SpeechToText`, `TextToSpeech`, `Translator`, `PipelineError`, `AudioClip` (Task 5); `SentenceSplitter`, `clean_model_reply`, `translation_system_prompt` (Task 3); `f32_to_pcm16_le`, `pcm16_le_to_f32` (Task 7).
- Produces: `speech_client::SpeechServiceClient::new(&str)` implementing `SpeechToText + TextToSpeech`; `llm_client::LlmClient::new(endpoint: &str, model: &str)` implementing `Translator`; `llm_client::{SseLine, parse_sse_line}`.

- [ ] **Step 1: Add dependencies**

Append to `translator-io/Cargo.toml` `[dependencies]`:
```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ureq = { version = "2.12", default-features = false }
```

- [ ] **Step 2: Write the test HTTP server**

`translator-io/tests/support/mod.rs`:
```rust
//! One-shot HTTP server: accepts a single request, records it and replies
//! with canned bytes.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

pub struct CapturedRequest {
    pub head: String,
    pub body: Vec<u8>,
}

pub fn serve_once(response: Vec<u8>) -> (String, JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        let head_end = loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0, "client closed before sending headers");
            data.extend_from_slice(&buffer[..read]);
            if let Some(position) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).to_string();
        let content_length = head
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while data.len() < head_end + content_length {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..read]);
        }
        stream.write_all(&response).unwrap();
        stream.flush().unwrap();
        CapturedRequest { head, body: data[head_end..].to_vec() }
    });
    (url, handle)
}

pub fn http_response(status_line: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status_line}\r\n");
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()));
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
```

- [ ] **Step 3: Write the failing tests**

`translator-io/tests/speech_client.rs`:
```rust
mod support;

use std::net::TcpListener;

use support::{http_response, serve_once};
use translator_core::language::LanguageCode;
use translator_core::pipeline::{PipelineError, SpeechToText, TextToSpeech};
use translator_io::speech_client::SpeechServiceClient;

/// A localhost URL nothing listens on.
fn unused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

#[test]
fn transcribe_posts_pcm16_and_reads_text() {
    let response = http_response("200 OK", &[("Content-Type", "application/json")], br#"{"text":"hola"}"#);
    let (url, server) = serve_once(response);
    let client = SpeechServiceClient::new(&url);
    let text = client.transcribe(&[0.0, 0.5], LanguageCode::Es).unwrap();
    assert_eq!(text, "hola");
    let request = server.join().unwrap();
    assert!(request.head.starts_with("POST /v2/stt?language=es "));
    assert!(request.head.to_ascii_lowercase().contains("content-type: application/octet-stream"));
    assert_eq!(request.body, [0, 0, 0x00, 0x40]);
}

#[test]
fn transcribe_reports_service_errors_with_body() {
    let response = http_response("500 Internal Server Error", &[], br#"{"error":"model missing"}"#);
    let (url, server) = serve_once(response);
    let error = SpeechServiceClient::new(&url).transcribe(&[0.0], LanguageCode::En).unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error,
        PipelineError::Service {
            service: "speech service",
            status: 500,
            message: r#"{"error":"model missing"}"#.to_string()
        }
    );
}

#[test]
fn transcribe_reports_an_unreachable_service() {
    let error = SpeechServiceClient::new(&unused_url()).transcribe(&[0.0], LanguageCode::En).unwrap_err();
    assert!(matches!(error, PipelineError::Unreachable { service: "speech service", .. }));
}

#[test]
fn synthesize_sends_json_and_decodes_pcm() {
    let body = [0x00, 0x40, 0x00, 0xc0];
    let response = http_response(
        "200 OK",
        &[("Content-Type", "application/octet-stream"), ("X-Sample-Rate", "24000")],
        &body,
    );
    let (url, server) = serve_once(response);
    let clip = SpeechServiceClient::new(&url).synthesize("Hi there", LanguageCode::Zh).unwrap();
    assert_eq!(clip.sample_rate, 24_000);
    assert_eq!(clip.samples, [0.5, -0.5]);
    let request = server.join().unwrap();
    assert!(request.head.starts_with("POST /v2/tts "));
    let json: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(json, serde_json::json!({"text": "Hi there", "language": "zh"}));
}

#[test]
fn synthesize_requires_the_sample_rate_header() {
    let response = http_response("200 OK", &[("Content-Type", "application/octet-stream")], &[0, 0]);
    let (url, server) = serve_once(response);
    let error = SpeechServiceClient::new(&url).synthesize("x", LanguageCode::En).unwrap_err();
    server.join().unwrap();
    assert!(matches!(error, PipelineError::InvalidResponse { .. }));
}
```

`translator-io/tests/llm_client.rs`:
```rust
mod support;

use std::sync::atomic::AtomicBool;

use support::{http_response, serve_once};
use translator_core::language::LanguageCode;
use translator_core::pipeline::{PipelineError, Translator};
use translator_io::llm_client::LlmClient;

fn translate(client: &LlmClient, is_cancelled: bool) -> Result<Vec<String>, PipelineError> {
    let cancel = AtomicBool::new(is_cancelled);
    let mut sentences = Vec::new();
    client.translate_streaming(
        "hello",
        LanguageCode::En,
        LanguageCode::Es,
        &cancel,
        &mut |sentence| sentences.push(sentence),
    )?;
    Ok(sentences)
}

const STREAM_BODY: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"Hola a todos. \"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"¿Qué\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\" tal?\"}}]}\n\n",
    "data: [DONE]\n\n",
);

#[test]
fn streams_sentences_from_server_sent_events() {
    let response = http_response("200 OK", &[("Content-Type", "text/event-stream")], STREAM_BODY.as_bytes());
    let (url, server) = serve_once(response);
    let sentences = translate(&LlmClient::new(&url, "gemma4-e2b"), false).unwrap();
    assert_eq!(sentences, ["Hola a todos.", "¿Qué tal?"]);
    let request = server.join().unwrap();
    assert!(request.head.starts_with("POST /chat/completions "));
    let json: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(json["model"], "gemma4-e2b");
    assert_eq!(json["stream"], true);
    assert_eq!(json["messages"][1], serde_json::json!({"role": "user", "content": "hello"}));
    let system = json["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("from English into Spanish"));
}

#[test]
fn falls_back_to_a_regular_completion() {
    let body = br#"{"choices":[{"message":{"role":"assistant","content":"Translation: Hola. Adios."}}]}"#;
    let response = http_response("200 OK", &[("Content-Type", "application/json")], body);
    let (url, server) = serve_once(response);
    let sentences = translate(&LlmClient::new(&url, "gemma4-e2b"), false).unwrap();
    server.join().unwrap();
    assert_eq!(sentences, ["Hola.", "Adios."]);
}

#[test]
fn reports_http_errors() {
    let response = http_response("503 Service Unavailable", &[], b"busy");
    let (url, server) = serve_once(response);
    let error = translate(&LlmClient::new(&url, "gemma4-e2b"), false).unwrap_err();
    server.join().unwrap();
    assert_eq!(error, PipelineError::Service { service: "LLM", status: 503, message: "busy".to_string() });
}

#[test]
fn cancelled_translation_emits_nothing() {
    let response = http_response("200 OK", &[("Content-Type", "text/event-stream")], STREAM_BODY.as_bytes());
    let (url, server) = serve_once(response);
    let sentences = translate(&LlmClient::new(&url, "gemma4-e2b"), true).unwrap();
    server.join().unwrap();
    assert!(sentences.is_empty());
}
```

Unit tests for the SSE parser, at the bottom of `translator-io/src/llm_client.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_deltas_done_and_noise() {
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#),
            Ok(SseLine::Delta("Hi".to_string()))
        );
        assert_eq!(parse_sse_line("data: [DONE]"), Ok(SseLine::Done));
        assert_eq!(parse_sse_line(""), Ok(SseLine::Ignored));
        assert_eq!(parse_sse_line(": keep-alive"), Ok(SseLine::Ignored));
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{}}]}"#), Ok(SseLine::Ignored));
        assert_eq!(parse_sse_line(r#"data: {"choices":[]}"#), Ok(SseLine::Ignored));
        assert!(parse_sse_line("data: {broken").is_err());
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p translator-io`
Expected: compile errors (modules missing).

- [ ] **Step 5: Implement `speech_client.rs`**

```rust
use std::io::Read;
use std::time::Duration;

use serde::Deserialize;
use translator_core::language::LanguageCode;
use translator_core::pipeline::{AudioClip, PipelineError, SpeechToText, TextToSpeech};

use crate::audio_convert::{f32_to_pcm16_le, pcm16_le_to_f32};

const SERVICE: &str = "speech service";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Client for the Python service's raw-PCM `/v2/stt` and `/v2/tts` endpoints.
pub struct SpeechServiceClient {
    base_url: String,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
struct SttResponse {
    text: String,
}

impl SpeechServiceClient {
    pub fn new(base_url: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();
        SpeechServiceClient { base_url: base_url.trim_end_matches('/').to_string(), agent }
    }
}

fn request_error(error: ureq::Error) -> PipelineError {
    match error {
        ureq::Error::Status(status, response) => PipelineError::Service {
            service: SERVICE,
            status,
            message: response.into_string().unwrap_or_default(),
        },
        ureq::Error::Transport(transport) => {
            PipelineError::Unreachable { service: SERVICE, message: transport.to_string() }
        }
    }
}

fn invalid(message: impl ToString) -> PipelineError {
    PipelineError::InvalidResponse { service: SERVICE, message: message.to_string() }
}

impl SpeechToText for SpeechServiceClient {
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError> {
        let url = format!("{}/v2/stt?language={}", self.base_url, language.code());
        let response = self
            .agent
            .post(&url)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&f32_to_pcm16_le(samples))
            .map_err(request_error)?;
        let body = response.into_string().map_err(invalid)?;
        let parsed: SttResponse = serde_json::from_str(&body).map_err(invalid)?;
        Ok(parsed.text)
    }
}

impl TextToSpeech for SpeechServiceClient {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError> {
        let url = format!("{}/v2/tts", self.base_url);
        let body = serde_json::json!({ "text": text, "language": language.code() }).to_string();
        let response = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(request_error)?;
        let sample_rate: u32 = response
            .header("X-Sample-Rate")
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| invalid("missing X-Sample-Rate header"))?;
        let mut bytes = Vec::new();
        response.into_reader().read_to_end(&mut bytes).map_err(invalid)?;
        let samples = pcm16_le_to_f32(&bytes).ok_or_else(|| invalid("odd PCM byte count"))?;
        Ok(AudioClip { samples, sample_rate })
    }
}
```

- [ ] **Step 6: Implement `llm_client.rs`** (above its unit tests)

```rust
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use translator_core::language::LanguageCode;
use translator_core::pipeline::{PipelineError, Translator};
use translator_core::text::{clean_model_reply, translation_system_prompt, SentenceSplitter};

const SERVICE: &str = "LLM";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenAI-compatible chat-completions client (litert-lm serve), streaming via SSE.
pub struct LlmClient {
    endpoint: String,
    model: String,
    agent: ureq::Agent,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SseLine {
    Delta(String),
    Done,
    Ignored,
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<CompletionChoice>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
}

#[derive(Deserialize)]
struct CompletionMessage {
    content: Option<String>,
}

/// Parses one line of a `text/event-stream` body.
pub fn parse_sse_line(line: &str) -> Result<SseLine, String> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(SseLine::Ignored);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(SseLine::Done);
    }
    let chunk: StreamChunk = serde_json::from_str(data).map_err(|error| error.to_string())?;
    Ok(chunk
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.delta.content)
        .map_or(SseLine::Ignored, SseLine::Delta))
}

impl LlmClient {
    pub fn new(endpoint: &str, model: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();
        LlmClient {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
            agent,
        }
    }
}

fn request_error(error: ureq::Error) -> PipelineError {
    match error {
        ureq::Error::Status(status, response) => PipelineError::Service {
            service: SERVICE,
            status,
            message: response.into_string().unwrap_or_default(),
        },
        ureq::Error::Transport(transport) => {
            PipelineError::Unreachable { service: SERVICE, message: transport.to_string() }
        }
    }
}

fn invalid(message: impl ToString) -> PipelineError {
    PipelineError::InvalidResponse { service: SERVICE, message: message.to_string() }
}

impl Translator for LlmClient {
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": true,
            "messages": [
                { "role": "system", "content": translation_system_prompt(source, target) },
                { "role": "user", "content": text },
            ],
        })
        .to_string();
        let response = self
            .agent
            .post(&format!("{}/chat/completions", self.endpoint))
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(request_error)?;

        let mut splitter = SentenceSplitter::new();
        let mut emit = |sentences: Vec<String>| {
            for sentence in sentences {
                let cleaned = clean_model_reply(&sentence);
                if !cleaned.is_empty() {
                    on_sentence(cleaned);
                }
            }
        };

        if response.content_type() != "text/event-stream" {
            // The server ignored "stream": read one regular completion.
            let completion: Completion =
                serde_json::from_str(&response.into_string().map_err(invalid)?).map_err(invalid)?;
            if cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let content = completion
                .choices
                .into_iter()
                .next()
                .and_then(|choice| choice.message.content)
                .unwrap_or_default();
            emit(splitter.push(&content));
            emit(splitter.finish());
            return Ok(());
        }

        for line in BufReader::new(response.into_reader()).lines() {
            if cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let line = line.map_err(|error| PipelineError::Unreachable {
                service: SERVICE,
                message: error.to_string(),
            })?;
            match parse_sse_line(&line).map_err(invalid)? {
                SseLine::Delta(fragment) => emit(splitter.push(&fragment)),
                SseLine::Done => break,
                SseLine::Ignored => {}
            }
        }
        if !cancel.load(Ordering::SeqCst) {
            emit(splitter.finish());
        }
        Ok(())
    }
}
```

Note on the fallback test: `"Translation: Hola. Adios."` splits into `"Translation: Hola."` and `"Adios."`; cleaning strips the label from the first, giving `["Hola.", "Adios."]`.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p translator-io && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add translator-io Cargo.lock
git commit -m "Add HTTP clients for speech service and streaming LLM"
```

---

### Task 9: System volume control

**Files:**
- Create: `translator-io/src/volume.rs`
- Modify: `translator-io/src/lib.rs` (add `pub mod volume;`)

**Interfaces:**
- Consumes: `RotateDirection` (Task 1).
- Produces: `volume::CommandRunner` trait (`run(&self, program: &str, args: &[&str]) -> Option<String>`; `None` = missing tool, non-zero exit or timeout), `volume::SystemCommandRunner`, `volume::VolumeControl<R: CommandRunner>` with `new(R)`, `current_percent(&self) -> Option<u16>`, `adjust(&self, RotateDirection) -> Option<u16>`.

- [ ] **Step 1: Write the failing tests** (bottom of `volume.rs`)

```rust
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// Answers commands by their full text; anything unscripted "is not installed".
    #[derive(Default)]
    struct FakeRunner {
        outputs: HashMap<String, String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn with(outputs: &[(&str, &str)]) -> Self {
            FakeRunner {
                outputs: outputs.iter().map(|(c, o)| (c.to_string(), o.to_string())).collect(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            let command = format!("{program} {}", args.join(" "));
            self.calls.lock().unwrap().push(command.clone());
            self.outputs.get(&command).cloned()
        }
    }

    #[test]
    fn reads_wpctl_first() {
        let runner = FakeRunner::with(&[("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.75\n")]);
        assert_eq!(VolumeControl::new(runner).current_percent(), Some(75));
    }

    #[test]
    fn falls_back_to_pactl_then_amixer() {
        let pactl = FakeRunner::with(&[(
            "pactl get-sink-volume @DEFAULT_SINK@",
            "Volume: front-left: 49151 /  75% / -7.50 dB,   front-right: 49151 /  75% / -7.50 dB",
        )]);
        assert_eq!(VolumeControl::new(pactl).current_percent(), Some(75));
        let amixer = FakeRunner::with(&[("amixer sget Master", "  Mono: Playback 52 [81%] [-12.00dB] [on]")]);
        assert_eq!(VolumeControl::new(amixer).current_percent(), Some(81));
    }

    #[test]
    fn missing_tools_give_none_without_panicking() {
        let control = VolumeControl::new(FakeRunner::default());
        assert_eq!(control.current_percent(), None);
        assert_eq!(control.adjust(RotateDirection::Next), None);
    }

    #[test]
    fn adjust_uses_five_percent_steps_and_rereads() {
        let runner = FakeRunner::with(&[
            ("wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-", ""),
            ("wpctl get-volume @DEFAULT_AUDIO_SINK@", "Volume: 0.70"),
        ]);
        let control = VolumeControl::new(runner);
        assert_eq!(control.adjust(RotateDirection::Previous), Some(70));
        let calls = control.runner.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            ["wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-", "wpctl get-volume @DEFAULT_AUDIO_SINK@"]
        );
    }

    #[test]
    fn adjust_up_falls_back_to_pactl() {
        let runner = FakeRunner::with(&[
            ("pactl set-sink-volume @DEFAULT_SINK@ +5%", ""),
            ("pactl get-sink-volume @DEFAULT_SINK@", "Volume: front-left: 1 / 80% / 0 dB"),
        ]);
        assert_eq!(VolumeControl::new(runner).adjust(RotateDirection::Next), Some(80));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-io volume`
Expected: compile errors.

- [ ] **Step 3: Implement `volume.rs`** (above its tests)

```rust
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use translator_core::language::RotateDirection;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Runs an external program and returns its stdout, or `None` when the program
/// is missing, fails or times out.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let mut command = Command::new(program);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        // PipeWire's wpctl finds its socket through XDG_RUNTIME_DIR, which systemd
        // services may lack.
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            if let Ok(metadata) = std::fs::metadata("/proc/self") {
                command.env("XDG_RUNTIME_DIR", format!("/run/user/{}", metadata.uid()));
            }
        }
        let mut child = command.spawn().ok()?;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match child.try_wait().ok()? {
                Some(status) => {
                    let mut stdout = String::new();
                    child.stdout.take()?.read_to_string(&mut stdout).ok()?;
                    return status.success().then_some(stdout);
                }
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                None => thread::sleep(COMMAND_POLL_INTERVAL),
            }
        }
    }
}

/// System output volume via wpctl (PipeWire), then pactl (PulseAudio), then amixer (ALSA).
pub struct VolumeControl<R: CommandRunner> {
    runner: R,
}

impl<R: CommandRunner> VolumeControl<R> {
    pub fn new(runner: R) -> Self {
        VolumeControl { runner }
    }

    pub fn current_percent(&self) -> Option<u16> {
        self.runner
            .run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .and_then(|output| parse_wpctl_volume(&output))
            .or_else(|| {
                self.runner
                    .run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])
                    .and_then(|output| parse_pactl_volume(&output))
            })
            .or_else(|| {
                self.runner
                    .run("amixer", &["sget", "Master"])
                    .and_then(|output| parse_amixer_volume(&output))
            })
    }

    /// Steps the volume by 5 % and returns the new level.
    pub fn adjust(&self, direction: RotateDirection) -> Option<u16> {
        let is_up = direction == RotateDirection::Next;
        let attempts: [(&str, [&str; 3]); 3] = [
            ("wpctl", ["set-volume", "@DEFAULT_AUDIO_SINK@", if is_up { "5%+" } else { "5%-" }]),
            ("pactl", ["set-sink-volume", "@DEFAULT_SINK@", if is_up { "+5%" } else { "-5%" }]),
            ("amixer", ["sset", "Master", if is_up { "5%+" } else { "5%-" }]),
        ];
        let is_changed = attempts
            .iter()
            .any(|(program, args)| self.runner.run(program, args).is_some());
        if is_changed {
            self.current_percent()
        } else {
            None
        }
    }
}

/// "Volume: 0.75" → 75
fn parse_wpctl_volume(output: &str) -> Option<u16> {
    let value = output.split("Volume:").nth(1)?.split_whitespace().next()?;
    let fraction: f32 = value.parse().ok()?;
    Some((fraction * 100.0).round() as u16)
}

/// "... / 75% / ..." → 75
fn parse_pactl_volume(output: &str) -> Option<u16> {
    output
        .split_whitespace()
        .find_map(|token| token.strip_suffix('%')?.parse().ok())
}

/// "... [81%] ..." → 81
fn parse_amixer_volume(output: &str) -> Option<u16> {
    output
        .split('[')
        .find_map(|part| part.split_once("%]")?.0.parse().ok())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p translator-io && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add translator-io
git commit -m "Add system volume control with wpctl, pactl and amixer fallbacks"
```

---

### Task 10: Job runner

**Files:**
- Create: `translator-io/src/runner.rs`, `translator-io/tests/runner.rs`
- Modify: `translator-io/src/lib.rs` (add `pub mod runner;`)

**Interfaces:**
- Consumes: pipeline traits (Task 5), `JobRequest`, `JobId`, `JobEvent` (Task 5), `PlaybackSink` (Task 7).
- Produces: `runner::Services { stt: Arc<dyn SpeechToText>, translator: Arc<dyn Translator>, tts: Arc<dyn TextToSpeech> }` (Clone); `runner::EventSink = Arc<dyn Fn(JobEvent) + Send + Sync>`; `runner::JobRunner::new(Services, Arc<dyn PlaybackSink>, EventSink)`, `start(&self, JobRequest, Vec<f32>)`, `cancel(&self, JobId)`.
- Event contract: in order `Transcribed`, then (non-empty transcript) zero or more `SentenceTranslated`, `TranslationFinished`, then (only when `enable_tts`) `PlaybackFinished` after the queue drains. Any error ends with `Failed`. No events are emitted after the job is cancelled.

- [ ] **Step 1: Write the failing tests**

`translator-io/tests/runner.rs`:
```rust
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use translator_core::controller::{JobEvent, JobId, JobRequest};
use translator_core::language::LanguageCode;
use translator_core::pipeline::{AudioClip, PipelineError, SpeechToText, TextToSpeech, Translator};
use translator_io::playback_queue::PlaybackSink;
use translator_io::runner::{JobRunner, Services};

const QUIET_PERIOD: Duration = Duration::from_millis(300);

struct ScriptedStt(Result<String, PipelineError>);

impl SpeechToText for ScriptedStt {
    fn transcribe(&self, _samples: &[f32], _language: LanguageCode) -> Result<String, PipelineError> {
        self.0.clone()
    }
}

struct ScriptedTranslator(Vec<&'static str>);

impl Translator for ScriptedTranslator {
    fn translate_streaming(
        &self,
        _text: &str,
        _source: LanguageCode,
        _target: LanguageCode,
        _cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        for sentence in &self.0 {
            on_sentence(sentence.to_string());
        }
        Ok(())
    }
}

/// Signals when it starts, then waits (up to 2 s) for cancellation.
struct BlockingTranslator(Mutex<Sender<()>>);

impl Translator for BlockingTranslator {
    fn translate_streaming(
        &self,
        _text: &str,
        _source: LanguageCode,
        _target: LanguageCode,
        cancel: &AtomicBool,
        _on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        self.0.lock().unwrap().send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !cancel.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

struct ScriptedTts(Result<AudioClip, PipelineError>);

impl TextToSpeech for ScriptedTts {
    fn synthesize(&self, _text: &str, _language: LanguageCode) -> Result<AudioClip, PipelineError> {
        self.0.clone()
    }
}

#[derive(Default)]
struct RecordingSink {
    clips: Mutex<Vec<AudioClip>>,
    clears: AtomicUsize,
}

impl PlaybackSink for RecordingSink {
    fn enqueue(&self, clip: AudioClip) {
        self.clips.lock().unwrap().push(clip);
    }

    fn clear(&self) {
        self.clears.fetch_add(1, Ordering::SeqCst);
    }

    fn is_idle(&self) -> bool {
        true
    }
}

fn clip() -> AudioClip {
    AudioClip { samples: vec![0.1; 160], sample_rate: 16_000 }
}

fn request(id: u64, enable_tts: bool) -> JobRequest {
    JobRequest { id: JobId(id), source: LanguageCode::En, target: LanguageCode::Es, enable_tts }
}

fn runner(
    stt: impl SpeechToText + 'static,
    translator: impl Translator + 'static,
    tts: impl TextToSpeech + 'static,
) -> (JobRunner, Arc<RecordingSink>, Receiver<JobEvent>) {
    let sink = Arc::new(RecordingSink::default());
    let (sender, receiver) = mpsc::channel();
    let services = Services { stt: Arc::new(stt), translator: Arc::new(translator), tts: Arc::new(tts) };
    let on_event = Arc::new(move |event| {
        let _ = sender.send(event);
    });
    (JobRunner::new(services, sink.clone(), on_event), sink, receiver)
}

/// Collects events until none arrive for `QUIET_PERIOD`.
fn events(receiver: &Receiver<JobEvent>) -> Vec<JobEvent> {
    let mut collected = Vec::new();
    while let Ok(event) = receiver.recv_timeout(QUIET_PERIOD) {
        collected.push(event);
    }
    collected
}

#[test]
fn full_job_speaks_every_sentence() {
    let (runner, sink, receiver) = runner(
        ScriptedStt(Ok("hello".to_string())),
        ScriptedTranslator(vec!["Hola.", "Adiós."]),
        ScriptedTts(Ok(clip())),
    );
    runner.start(request(1, true), vec![0.0; 16_000]);
    let job = JobId(1);
    assert_eq!(
        events(&receiver),
        [
            JobEvent::Transcribed { job, text: "hello".to_string() },
            JobEvent::SentenceTranslated { job, sentence: "Hola.".to_string() },
            JobEvent::SentenceTranslated { job, sentence: "Adiós.".to_string() },
            JobEvent::TranslationFinished { job },
            JobEvent::PlaybackFinished { job },
        ]
    );
    assert_eq!(sink.clips.lock().unwrap().len(), 2);
}

#[test]
fn without_speech_output_nothing_is_played() {
    let (runner, sink, receiver) =
        runner(ScriptedStt(Ok("hello".to_string())), ScriptedTranslator(vec!["Hola."]), ScriptedTts(Ok(clip())));
    runner.start(request(1, false), Vec::new());
    let collected = events(&receiver);
    assert_eq!(collected.last(), Some(&JobEvent::TranslationFinished { job: JobId(1) }));
    assert!(sink.clips.lock().unwrap().is_empty());
}

#[test]
fn empty_transcript_stops_after_transcription() {
    let (runner, _sink, receiver) =
        runner(ScriptedStt(Ok(" ".to_string())), ScriptedTranslator(vec!["x"]), ScriptedTts(Ok(clip())));
    runner.start(request(1, true), Vec::new());
    assert_eq!(events(&receiver), [JobEvent::Transcribed { job: JobId(1), text: " ".to_string() }]);
}

#[test]
fn transcription_errors_are_reported() {
    let error = PipelineError::Unreachable { service: "speech service", message: "refused".to_string() };
    let (runner, _sink, receiver) = runner(ScriptedStt(Err(error.clone())), ScriptedTranslator(vec![]), ScriptedTts(Ok(clip())));
    runner.start(request(1, true), Vec::new());
    assert_eq!(events(&receiver), [JobEvent::Failed { job: JobId(1), message: error.to_string() }]);
}

#[test]
fn speech_errors_are_reported_after_the_translation() {
    let error = PipelineError::Service { service: "speech service", status: 500, message: "tts".to_string() };
    let (runner, _sink, receiver) = runner(
        ScriptedStt(Ok("hello".to_string())),
        ScriptedTranslator(vec!["Hola."]),
        ScriptedTts(Err(error.clone())),
    );
    runner.start(request(1, true), Vec::new());
    let collected = events(&receiver);
    assert_eq!(collected.last(), Some(&JobEvent::Failed { job: JobId(1), message: error.to_string() }));
    assert!(collected.contains(&JobEvent::TranslationFinished { job: JobId(1) }));
}

#[test]
fn cancelled_job_emits_nothing_more_and_clears_playback() {
    let (started_sender, started) = mpsc::channel();
    let (runner, sink, receiver) = runner(
        ScriptedStt(Ok("hello".to_string())),
        BlockingTranslator(Mutex::new(started_sender)),
        ScriptedTts(Ok(clip())),
    );
    runner.start(request(1, true), Vec::new());
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    runner.cancel(JobId(1));
    assert_eq!(events(&receiver), [JobEvent::Transcribed { job: JobId(1), text: "hello".to_string() }]);
    assert!(sink.clears.load(Ordering::SeqCst) >= 1);
}

#[test]
fn starting_a_new_job_cancels_the_previous_one() {
    let (started_sender, started) = mpsc::channel();
    let (runner, _sink, receiver) = runner(
        ScriptedStt(Ok("hello".to_string())),
        BlockingTranslator(Mutex::new(started_sender)),
        ScriptedTts(Ok(clip())),
    );
    runner.start(request(1, true), Vec::new());
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    runner.start(request(2, true), Vec::new());
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    runner.cancel(JobId(2));
    let collected = events(&receiver);
    assert!(collected.iter().all(|e| !matches!(e, JobEvent::TranslationFinished { .. })));
    assert!(collected.contains(&JobEvent::Transcribed { job: JobId(2), text: "hello".to_string() }));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-io --test runner`
Expected: compile errors (`runner` module missing).

- [ ] **Step 3: Implement `runner.rs`**

```rust
//! Runs one translation job at a time on background threads:
//! speech-to-text → streaming translation → speech synthesis → playback.
//! Sentences are synthesized while later ones are still being translated.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use translator_core::controller::{JobEvent, JobId, JobRequest};
use translator_core::pipeline::{PipelineError, SpeechToText, TextToSpeech, Translator};

use crate::playback_queue::PlaybackSink;

const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct Services {
    pub stt: Arc<dyn SpeechToText>,
    pub translator: Arc<dyn Translator>,
    pub tts: Arc<dyn TextToSpeech>,
}

/// Receives job events on the job's thread.
pub type EventSink = Arc<dyn Fn(JobEvent) + Send + Sync>;

pub struct JobRunner {
    services: Services,
    playback: Arc<dyn PlaybackSink>,
    on_event: EventSink,
    active_job: Mutex<Option<(JobId, Arc<AtomicBool>)>>,
}

struct JobContext {
    services: Services,
    playback: Arc<dyn PlaybackSink>,
    on_event: EventSink,
    cancel: Arc<AtomicBool>,
}

impl JobRunner {
    pub fn new(services: Services, playback: Arc<dyn PlaybackSink>, on_event: EventSink) -> Self {
        JobRunner { services, playback, on_event, active_job: Mutex::new(None) }
    }

    /// Starts a job; a job still running is cancelled first.
    pub fn start(&self, request: JobRequest, samples: Vec<f32>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let previous = lock(&self.active_job).replace((request.id, Arc::clone(&cancel)));
        if let Some((_, previous_cancel)) = previous {
            previous_cancel.store(true, Ordering::SeqCst);
        }
        let context = JobContext {
            services: self.services.clone(),
            playback: Arc::clone(&self.playback),
            on_event: Arc::clone(&self.on_event),
            cancel,
        };
        thread::spawn(move || run_job(context, request, samples));
    }

    /// Stops the job (if it is the active one) and drops queued audio.
    pub fn cancel(&self, job: JobId) {
        let mut active = lock(&self.active_job);
        if active.as_ref().is_some_and(|(id, _)| *id == job) {
            if let Some((_, cancel)) = active.take() {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        self.playback.clear();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn run_job(context: JobContext, request: JobRequest, samples: Vec<f32>) {
    let JobContext { services, playback, on_event, cancel } = context;
    let job = request.id;
    let emit = |event: JobEvent| {
        if !cancel.load(Ordering::SeqCst) {
            on_event(event);
        }
    };

    let transcript = match services.stt.transcribe(&samples, request.source) {
        Ok(text) => text,
        Err(error) => {
            emit(JobEvent::Failed { job, message: error.to_string() });
            return;
        }
    };
    let has_speech = !transcript.trim().is_empty();
    emit(JobEvent::Transcribed { job, text: transcript.clone() });
    if !has_speech || cancel.load(Ordering::SeqCst) {
        return;
    }

    let (sentence_sender, sentence_receiver) = mpsc::channel::<String>();
    let speaker = request.enable_tts.then(|| {
        let tts = Arc::clone(&services.tts);
        let playback = Arc::clone(&playback);
        let cancel = Arc::clone(&cancel);
        let target = request.target;
        thread::spawn(move || -> Result<(), PipelineError> {
            for sentence in sentence_receiver {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let clip = tts.synthesize(&sentence, target)?;
                if !cancel.load(Ordering::SeqCst) {
                    playback.enqueue(clip);
                }
            }
            Ok(())
        })
    });

    let translation = services.translator.translate_streaming(
        &transcript,
        request.source,
        request.target,
        &cancel,
        &mut |sentence: String| {
            emit(JobEvent::SentenceTranslated { job, sentence: sentence.clone() });
            if request.enable_tts {
                let _ = sentence_sender.send(sentence);
            }
        },
    );
    drop(sentence_sender);
    if let Err(error) = translation {
        emit(JobEvent::Failed { job, message: error.to_string() });
        return;
    }
    emit(JobEvent::TranslationFinished { job });

    let Some(speaker) = speaker else {
        return;
    };
    let speech = speaker.join().unwrap_or_else(|_| {
        Err(PipelineError::InvalidResponse {
            service: "speech service",
            message: "speech thread panicked".to_string(),
        })
    });
    if let Err(error) = speech {
        emit(JobEvent::Failed { job, message: error.to_string() });
        return;
    }
    while !playback.is_idle() && !cancel.load(Ordering::SeqCst) {
        thread::sleep(PLAYBACK_POLL_INTERVAL);
    }
    emit(JobEvent::PlaybackFinished { job });
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p translator-io && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add translator-io
git commit -m "Add job runner that pipelines translation and speech"
```

---

### Task 11: Microphone and speaker devices (cpal)

**Files:**
- Create: `translator-io/src/audio_device.rs`
- Modify: `translator-io/Cargo.toml` (add `cpal = "0.15"` and `thiserror = "2"`), `translator-io/src/lib.rs` (add `pub mod audio_device;`)

**Interfaces:**
- Consumes: `downmix_to_mono`, `resample_linear`, `rms` (Task 7), `PlaybackQueue` (Task 7), `STT_SAMPLE_RATE` (Task 5).
- Produces: `audio_device::AudioError`; `audio_device::CapturedAudio { samples: Vec<f32> (16 kHz mono), duration: Duration }`; `audio_device::AudioRecorder::open(Option<&str>) -> Result<AudioRecorder, AudioError>`, `start(&self)`, `stop(&self) -> CapturedAudio`, `level(&self) -> f32`; `audio_device::AudioPlayer::open(Option<&str>) -> Result<AudioPlayer, AudioError>`, `queue(&self) -> PlaybackQueue`.
- Both device types hold a `cpal::Stream`, which is not `Send`: create and keep them on the UI thread.

This task has no unit tests (it needs real hardware); the pure parts it relies on are tested in Task 7. It is verified by building and by the manual run in Task 12.

- [ ] **Step 1: Add dependencies**

Append to `translator-io/Cargo.toml` `[dependencies]`:
```toml
cpal = "0.15"
thiserror = "2"
```

- [ ] **Step 2: Implement `audio_device.rs`**

```rust
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use translator_core::pipeline::STT_SAMPLE_RATE;

use crate::audio_convert::{downmix_to_mono, resample_linear, rms};
use crate::playback_queue::PlaybackQueue;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio {0} device available")]
    NoDevice(&'static str),
    #[error("audio device error: {0}")]
    Device(String),
    #[error("unsupported sample format {0}")]
    UnsupportedFormat(String),
}

fn device_error(error: impl ToString) -> AudioError {
    AudioError::Device(error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One recording, converted for the speech recognizer.
pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub duration: Duration,
}

#[derive(Default)]
struct CaptureBuffer {
    is_capturing: bool,
    samples: Vec<f32>,
    started_at: Option<Instant>,
    recent_level: f32,
}

/// Keeps the microphone stream open; samples are kept only between `start` and `stop`.
pub struct AudioRecorder {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<CaptureBuffer>>,
    sample_rate: u32,
    channels: u16,
}

impl AudioRecorder {
    pub fn open(device_name: Option<&str>) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()
                .map_err(device_error)?
                .find(|device| device.name().is_ok_and(|candidate| candidate == name))
                .ok_or(AudioError::NoDevice("input"))?,
            None => host.default_input_device().ok_or(AudioError::NoDevice("input"))?,
        };
        let supported = device.default_input_config().map_err(device_error)?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let buffer = Arc::new(Mutex::new(CaptureBuffer::default()));
        let stream = match sample_format {
            SampleFormat::F32 => build_input::<f32>(&device, &config, Arc::clone(&buffer)),
            SampleFormat::I16 => build_input::<i16>(&device, &config, Arc::clone(&buffer)),
            SampleFormat::U16 => build_input::<u16>(&device, &config, Arc::clone(&buffer)),
            other => Err(AudioError::UnsupportedFormat(format!("{other:?}"))),
        }?;
        stream.play().map_err(device_error)?;
        Ok(AudioRecorder {
            _stream: stream,
            buffer,
            sample_rate: config.sample_rate.0,
            channels: config.channels,
        })
    }

    pub fn start(&self) {
        let mut buffer = lock(&self.buffer);
        buffer.samples.clear();
        buffer.is_capturing = true;
        buffer.started_at = Some(Instant::now());
    }

    pub fn stop(&self) -> CapturedAudio {
        let (interleaved, duration) = {
            let mut buffer = lock(&self.buffer);
            buffer.is_capturing = false;
            let duration = buffer.started_at.take().map(|start| start.elapsed()).unwrap_or_default();
            (std::mem::take(&mut buffer.samples), duration)
        };
        let mono = downmix_to_mono(&interleaved, self.channels);
        CapturedAudio { samples: resample_linear(&mono, self.sample_rate, STT_SAMPLE_RATE), duration }
    }

    /// RMS of the most recent device buffer, for the visualizer.
    pub fn level(&self) -> f32 {
        lock(&self.buffer).recent_level
    }
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<CaptureBuffer>>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> = data.iter().map(|sample| sample.to_sample::<f32>()).collect();
                let mut buffer = lock(&buffer);
                buffer.recent_level = rms(&samples);
                if buffer.is_capturing {
                    buffer.samples.extend_from_slice(&samples);
                }
            },
            |error| eprintln!("[audio] input stream error: {error}"),
            None,
        )
        .map_err(device_error)
}

/// Keeps the speaker stream open and plays whatever is pushed into its queue.
pub struct AudioPlayer {
    _stream: cpal::Stream,
    queue: PlaybackQueue,
}

impl AudioPlayer {
    pub fn open(device_name: Option<&str>) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .output_devices()
                .map_err(device_error)?
                .find(|device| device.name().is_ok_and(|candidate| candidate == name))
                .ok_or(AudioError::NoDevice("output"))?,
            None => host.default_output_device().ok_or(AudioError::NoDevice("output"))?,
        };
        let supported = device.default_output_config().map_err(device_error)?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let queue = PlaybackQueue::new(config.sample_rate.0);
        let stream = match sample_format {
            SampleFormat::F32 => build_output::<f32>(&device, &config, queue.clone()),
            SampleFormat::I16 => build_output::<i16>(&device, &config, queue.clone()),
            SampleFormat::U16 => build_output::<u16>(&device, &config, queue.clone()),
            other => Err(AudioError::UnsupportedFormat(format!("{other:?}"))),
        }?;
        stream.play().map_err(device_error)?;
        Ok(AudioPlayer { _stream: stream, queue })
    }

    pub fn queue(&self) -> PlaybackQueue {
        self.queue.clone()
    }
}

fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: PlaybackQueue,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut mixed = vec![0.0f32; data.len()];
                queue.fill(&mut mixed, channels);
                for (out, sample) in data.iter_mut().zip(mixed) {
                    *out = T::from_sample(sample);
                }
            },
            |error| eprintln!("[audio] output stream error: {error}"),
            None,
        )
        .map_err(device_error)
}
```

- [ ] **Step 3: Verify it builds and lints clean**

Run: `cargo build -p translator-io && cargo test -p translator-io && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: builds (needs ALSA headers, `libasound2-dev`); all earlier tests still pass; no warnings.

- [ ] **Step 4: Commit**

```bash
git add translator-io Cargo.lock
git commit -m "Add cpal microphone recorder and speaker player"
```

---

### Task 12: Slint UI binary with mock mode

**Files:**
- Create: `translator-ui/Cargo.toml`, `translator-ui/build.rs`, `translator-ui/ui/app.slint`, `translator-ui/src/main.rs`, `translator-ui/src/app.rs`, `translator-ui/src/view.rs`, `translator-ui/src/mock.rs`, `Makefile`
- Modify: `Cargo.toml` (members add `"translator-ui"`)

**Interfaces:**
- Consumes: everything from Tasks 1–11.
- Produces: binary `translator` (`translator [--mock] [--config <path>]`); `make test`, `make run-mock`, `make run`, `make fmt`, `make clippy`, `make build-pi`.

- [ ] **Step 1: Create the crate and Makefile**

Root `Cargo.toml` members: `members = ["translator-core", "translator-io", "translator-ui"]`.

`translator-ui/Cargo.toml`:
```toml
[package]
name = "translator-ui"
edition.workspace = true
version.workspace = true
license.workspace = true

[[bin]]
name = "translator"
path = "src/main.rs"

[features]
default = ["desktop"]
# Desktop development: X11/Wayland window.
desktop = ["slint/backend-winit", "slint/renderer-femtovg"]
# Raspberry Pi kiosk: draw straight to the display via KMS, no window system.
kms = ["slint/backend-linuxkms-noseat", "slint/renderer-femtovg"]

[dependencies]
translator-core = { path = "../translator-core" }
translator-io = { path = "../translator-io" }
slint = { version = "1.17", default-features = false, features = ["compat-1-2", "std"] }

[build-dependencies]
slint-build = "1.17"
```

`translator-ui/build.rs`:
```rust
fn main() {
    slint_build::compile("ui/app.slint").expect("Slint UI must compile");
}
```

`Makefile` (recipe lines start with a TAB):
```make
# Gemma Translator (Rust UI) — build orchestration.
# `make run-mock` works on any Linux desktop; `build-pi` targets the Raspberry Pi.

CARGO     := cargo
PI_TARGET := aarch64-unknown-linux-gnu

.DEFAULT_GOAL := help

## help: list available targets
.PHONY: help
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'

## test: run Rust and Python unit tests
.PHONY: test
test:
	$(CARGO) test --workspace
	python3 -m unittest discover -s backend -p 'test_*.py'

## run-mock: run the UI on the desktop with mock speech and translation
.PHONY: run-mock
run-mock:
	$(CARGO) run -p translator-ui -- --mock

## run: run the UI against the local speech service and litert-lm
.PHONY: run
run:
	$(CARGO) run -p translator-ui

## fmt: format all Rust code
.PHONY: fmt
fmt:
	$(CARGO) fmt --all

## clippy: lint with warnings as errors
.PHONY: clippy
clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

## build-pi: release build for the Pi with the KMS backend (needs the aarch64 toolchain)
.PHONY: build-pi
build-pi:
	$(CARGO) build --release -p translator-ui --no-default-features --features kms --target $(PI_TARGET)
```

- [ ] **Step 2: Write `ui/app.slint`**

```slint
// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// 480x320 retro-terminal layout: two language lanes, a level visualizer,
// the transcript panel and a status line, with a settings overlay on top.
// All state lives in Rust; this file only renders it and forwards keys.

export struct LaneView {
    label: string,
    language: string,
    is-active: bool,
    is-recording: bool,
}

export struct SettingsRow {
    label: string,
    value: string,
    is-selected: bool,
}

component Brackets inherits Rectangle {
    in property <color> line-color;
    width: 100%;
    height: 100%;
    Rectangle { x: 0; y: 0; width: 14px; height: 2px; background: root.line-color; }
    Rectangle { x: 0; y: 0; width: 2px; height: 14px; background: root.line-color; }
    Rectangle { x: root.width - 14px; y: 0; width: 14px; height: 2px; background: root.line-color; }
    Rectangle { x: root.width - 2px; y: 0; width: 2px; height: 14px; background: root.line-color; }
    Rectangle { x: 0; y: root.height - 2px; width: 14px; height: 2px; background: root.line-color; }
    Rectangle { x: 0; y: root.height - 14px; width: 2px; height: 14px; background: root.line-color; }
    Rectangle { x: root.width - 14px; y: root.height - 2px; width: 14px; height: 2px; background: root.line-color; }
    Rectangle { x: root.width - 2px; y: root.height - 14px; width: 2px; height: 14px; background: root.line-color; }
}

component LanePanel inherits Rectangle {
    in property <LaneView> view;
    in property <color> theme;
    in property <bool> show-brackets;
    property <color> ink: root.view.is-recording ? #000000 : root.theme;
    background: root.view.is-recording ? root.theme : #000000;
    border-width: 1px;
    border-color: root.theme;

    VerticalLayout {
        padding: 8px;
        spacing: 4px;
        alignment: center;
        Text {
            text: "PERSON " + root.view.label;
            color: root.ink;
            font-size: 11px;
            horizontal-alignment: center;
        }
        Text {
            text: root.view.language;
            color: root.ink;
            font-size: 22px;
            font-weight: 700;
            horizontal-alignment: center;
        }
        Text {
            text: root.view.is-recording ? "● REC" : "◀ ▶";
            color: root.ink;
            font-size: 10px;
            horizontal-alignment: center;
        }
    }

    if root.show-brackets && root.view.is-active: Brackets {
        line-color: root.ink;
    }
}

component Visualizer inherits Rectangle {
    in property <[float]> levels;
    in property <color> theme;
    HorizontalLayout {
        spacing: 1px;
        for level in root.levels: Rectangle {
            Rectangle {
                height: max(1px, parent.height * level);
                y: parent.height - self.height;
                background: root.theme;
            }
        }
    }
}

component TranscriptPanel inherits Rectangle {
    in property <string> source-label;
    in property <string> transcription;
    in property <string> target-label;
    in property <string> translation;
    in property <bool> is-translation-right-to-left;
    in property <color> theme;
    border-width: 1px;
    border-color: root.theme;

    VerticalLayout {
        padding: 6px;
        spacing: 2px;
        Text { text: root.source-label; color: root.theme; font-size: 9px; }
        Text { text: root.transcription; color: #ffffff; font-size: 12px; wrap: word-wrap; }
        Text { text: root.target-label; color: root.theme; font-size: 9px; }
        Text {
            text: root.translation;
            color: #ffffff;
            font-size: 14px;
            wrap: word-wrap;
            vertical-stretch: 1;
            vertical-alignment: top;
            horizontal-alignment: root.is-translation-right-to-left
                ? TextHorizontalAlignment.right
                : TextHorizontalAlignment.left;
        }
    }
}

component SettingsOverlay inherits Rectangle {
    in property <[SettingsRow]> rows;
    in property <color> theme;
    background: #000000e6;

    VerticalLayout {
        padding: 16px;
        spacing: 6px;
        Text { text: "SETTINGS"; color: root.theme; font-size: 14px; font-weight: 700; }
        for row in root.rows: Rectangle {
            height: 28px;
            background: row.is-selected ? root.theme : transparent;
            HorizontalLayout {
                padding-left: 8px;
                padding-right: 8px;
                Text {
                    text: row.label;
                    color: row.is-selected ? #000000 : root.theme;
                    vertical-alignment: center;
                    horizontal-stretch: 1;
                }
                Text {
                    text: "◀ " + row.value + " ▶";
                    color: row.is-selected ? #000000 : #ffffff;
                    vertical-alignment: center;
                }
            }
        }
        Text { text: "↑↓ select   ◀▶ change   Esc close"; color: root.theme; font-size: 9px; }
    }
}

export component AppWindow inherits Window {
    width: 480px;
    height: 320px;
    title: "Gemma Translator";
    background: #000000;
    default-font-family: "DejaVu Sans Mono";

    in property <color> theme-color: #ffa500;
    in property <LaneView> lane1;
    in property <LaneView> lane2;
    in property <bool> show-active-brackets: true;
    in property <string> source-label;
    in property <string> transcription;
    in property <string> target-label;
    in property <string> translation;
    in property <bool> is-translation-right-to-left;
    in property <string> status-text;
    in property <[float]> visualizer-levels;
    in property <bool> is-settings-open;
    in property <[SettingsRow]> settings-rows;

    // Return true when the key was handled.
    callback key-pressed(string) -> bool;
    callback key-released(string) -> bool;

    forward-focus: key-handler;

    key-handler := FocusScope {
        width: 100%;
        height: 100%;
        key-pressed(event) => {
            // Auto-repeat would toggle lanes or rotate languages repeatedly.
            if (event.repeat) {
                return accept;
            }
            if (root.key-pressed(event.text)) {
                return accept;
            }
            return reject;
        }
        key-released(event) => {
            if (root.key-released(event.text)) {
                return accept;
            }
            return reject;
        }

        VerticalLayout {
            padding: 6px;
            spacing: 6px;
            HorizontalLayout {
                spacing: 6px;
                height: 110px;
                LanePanel {
                    view: root.lane1;
                    theme: root.theme-color;
                    show-brackets: root.show-active-brackets;
                }
                LanePanel {
                    view: root.lane2;
                    theme: root.theme-color;
                    show-brackets: root.show-active-brackets;
                }
            }
            Visualizer {
                height: 20px;
                levels: root.visualizer-levels;
                theme: root.theme-color;
            }
            TranscriptPanel {
                vertical-stretch: 1;
                source-label: root.source-label;
                transcription: root.transcription;
                target-label: root.target-label;
                translation: root.translation;
                is-translation-right-to-left: root.is-translation-right-to-left;
                theme: root.theme-color;
            }
            Text {
                text: root.status-text;
                color: root.theme-color;
                font-size: 10px;
            }
        }
    }

    if root.is-settings-open: SettingsOverlay {
        width: 100%;
        height: 100%;
        rows: root.settings-rows;
        theme: root.theme-color;
    }
}
```

- [ ] **Step 3: Write the failing view and CLI tests**

`translator-ui/src/view.rs` test module:
```rust
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
        let effects = controller.on_capture_finished(std::time::Duration::from_secs(1));
        let translator_core::controller::Effect::StartJob(request) = &effects[0] else {
            panic!("expected a job");
        };
        controller.on_job_event(JobEvent::Failed { job: request.id, message: "down".to_string() });
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
        assert_eq!((color.red(), color.green(), color.blue()), (0x21, 0x96, 0xf3));
    }
}
```

`translator-ui/src/main.rs` test module (at the bottom of the file):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn parses_mock_and_config() {
        assert_eq!(
            parse_args(args(&["--mock", "--config", "/tmp/c.toml"])),
            Ok(CliOptions { is_mock: true, config_path: Some(PathBuf::from("/tmp/c.toml")) })
        );
        assert_eq!(parse_args(args(&[])), Ok(CliOptions { is_mock: false, config_path: None }));
    }

    #[test]
    fn rejects_unknown_and_incomplete_arguments() {
        assert!(parse_args(args(&["--verbose"])).is_err());
        assert!(parse_args(args(&["--config"])).is_err());
    }
}
```

`translator-ui/src/mock.rs` test module:
```rust
#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    #[test]
    fn mock_translator_emits_two_sentences_in_the_target_language() {
        let mut sentences = Vec::new();
        MockTranslator
            .translate_streaming("x", LanguageCode::En, LanguageCode::Zh, &AtomicBool::new(false), &mut |s| {
                sentences.push(s)
            })
            .unwrap();
        assert_eq!(sentences.len(), 2);
        assert!(sentences[0].ends_with('。'));
    }

    #[test]
    fn mock_translator_stops_when_cancelled() {
        let mut sentences = Vec::new();
        MockTranslator
            .translate_streaming("x", LanguageCode::En, LanguageCode::Es, &AtomicBool::new(true), &mut |s| {
                sentences.push(s)
            })
            .unwrap();
        assert!(sentences.is_empty());
    }

    #[test]
    fn mock_speech_is_a_short_tone() {
        let clip = MockTextToSpeech.synthesize("x", LanguageCode::En).unwrap();
        assert_eq!(clip.sample_rate, 16_000);
        assert_eq!(clip.samples.len(), 6_400);
    }
}
```

- [ ] **Step 4: Implement `view.rs`** (above its tests)

```rust
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
        language: controller.language(lane).display_name().to_uppercase().into(),
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
                SettingsItem::EnableTts => {
                    ("VOICE OUTPUT", if config.enable_tts { "ON" } else { "OFF" }.to_string())
                }
                SettingsItem::VisualizerBars => ("VISUALIZER BARS", config.visualizer_bars.to_string()),
                SettingsItem::Volume => (
                    "VOLUME",
                    volume_percent.map_or_else(|| "N/A".to_string(), |percent| format!("{percent}%")),
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
```

Note: `update_levels(&mut levels, 4, None)` on `[0.0, 0.0, 0.5, 1.0]` gives `[0.0, 0.0, 0.4, 0.8]` exactly in f32 (0.5×0.8 and 1.0×0.8 round to the same literals).

- [ ] **Step 5: Implement `mock.rs`** (above its tests)

```rust
//! Stand-in services for `--mock`: no speech service or model needed, so the UI
//! can be developed on any desktop. Each step is slow enough to see.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use translator_core::language::LanguageCode;
use translator_core::pipeline::{AudioClip, PipelineError, SpeechToText, TextToSpeech, Translator};
use translator_io::runner::Services;

const MOCK_STEP_DELAY: Duration = Duration::from_millis(300);
const MOCK_POLL_INTERVAL: Duration = Duration::from_millis(30);
const MOCK_SAMPLE_RATE: u32 = 16_000;
const MOCK_TONE_HZ: f32 = 440.0;
const MOCK_TONE_SECONDS: f32 = 0.4;
const MOCK_TONE_GAIN: f32 = 0.2;

pub fn services() -> Services {
    Services {
        stt: Arc::new(MockSpeechToText),
        translator: Arc::new(MockTranslator),
        tts: Arc::new(MockTextToSpeech),
    }
}

struct MockSpeechToText;

impl SpeechToText for MockSpeechToText {
    fn transcribe(&self, _samples: &[f32], language: LanguageCode) -> Result<String, PipelineError> {
        thread::sleep(MOCK_STEP_DELAY);
        Ok(format!("This is a mock {} transcription.", language.display_name()))
    }
}

struct MockTranslator;

impl Translator for MockTranslator {
    fn translate_streaming(
        &self,
        _text: &str,
        _source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        for sentence in mock_sentences(target) {
            if !wait_unless_cancelled(cancel) {
                return Ok(());
            }
            on_sentence(sentence.to_string());
        }
        Ok(())
    }
}

/// Sleeps one mock step; false when cancelled meanwhile.
fn wait_unless_cancelled(cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + MOCK_STEP_DELAY;
    while Instant::now() < deadline {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        thread::sleep(MOCK_POLL_INTERVAL);
    }
    !cancel.load(Ordering::SeqCst)
}

/// Sample text per language, to exercise CJK and right-to-left rendering.
fn mock_sentences(target: LanguageCode) -> [&'static str; 2] {
    match target {
        LanguageCode::Ar => ["هذه ترجمة تجريبية.", "تظهر جملة بعد جملة."],
        LanguageCode::En => ["This is a mock translation.", "It arrives one sentence at a time."],
        LanguageCode::Es => ["Esta es una traducción simulada.", "Llega frase por frase."],
        LanguageCode::Ja => ["これは模擬翻訳です。", "一文ずつ表示されます。"],
        LanguageCode::Zh => ["这是一段模拟翻译。", "句子会逐句出现。"],
        LanguageCode::Ko => ["이것은 모의 번역입니다.", "문장 단위로 표시됩니다."],
    }
}

struct MockTextToSpeech;

impl TextToSpeech for MockTextToSpeech {
    fn synthesize(&self, _text: &str, _language: LanguageCode) -> Result<AudioClip, PipelineError> {
        let count = (MOCK_TONE_SECONDS * MOCK_SAMPLE_RATE as f32) as usize;
        let samples = (0..count)
            .map(|i| MOCK_TONE_GAIN * (TAU * MOCK_TONE_HZ * i as f32 / MOCK_SAMPLE_RATE as f32).sin())
            .collect();
        Ok(AudioClip { samples, sample_rate: MOCK_SAMPLE_RATE })
    }
}
```

- [ ] **Step 6: Implement `app.rs`**

```rust
//! UI-thread state: turns key presses into controller commands and carries out
//! the controller's effects on the audio devices, job runner and config file.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

use slint::{ModelRc, VecModel};
use translator_core::config::Config;
use translator_core::controller::{Controller, Effect, JobEvent, Phase};
use translator_core::keys::{command_for_key_press, command_for_key_release, Key, KeyboardMode};
use translator_core::lane::Lane;
use translator_core::pipeline::STT_SAMPLE_RATE;
use translator_core::settings::SettingsCursor;
use translator_io::audio_device::{AudioPlayer, AudioRecorder, CapturedAudio};
use translator_io::blip::{synthesize_blip, BLIP_SAMPLE_RATE};
use translator_io::llm_client::LlmClient;
use translator_io::playback_queue::{DiscardingSink, PlaybackSink};
use translator_io::runner::{JobRunner, Services};
use translator_io::speech_client::SpeechServiceClient;
use translator_io::volume::{SystemCommandRunner, VolumeControl};

use crate::view::{lane_view, settings_rows, status_line, theme_color, update_levels};
use crate::{mock, AppWindow, SettingsRow};

/// Microphone RMS is small; scale it so normal speech fills the bars.
const LEVEL_GAIN: f32 = 8.0;

/// The microphone, or silence of the right length when no microphone exists.
enum Capture {
    Device(AudioRecorder),
    Silent { started_at: Option<Instant> },
}

impl Capture {
    fn start(&mut self) {
        match self {
            Capture::Device(recorder) => recorder.start(),
            Capture::Silent { started_at } => *started_at = Some(Instant::now()),
        }
    }

    fn stop(&mut self) -> CapturedAudio {
        match self {
            Capture::Device(recorder) => recorder.stop(),
            Capture::Silent { started_at } => {
                let duration = started_at.take().map(|start| start.elapsed()).unwrap_or_default();
                let count = (duration.as_secs_f32() * STT_SAMPLE_RATE as f32) as usize;
                CapturedAudio { samples: vec![0.0; count], duration }
            }
        }
    }

    fn level(&self) -> f32 {
        match self {
            Capture::Device(recorder) => recorder.level(),
            Capture::Silent { .. } => 0.0,
        }
    }
}

pub struct AppState {
    controller: Controller,
    settings_cursor: SettingsCursor,
    capture: Capture,
    playback: Arc<dyn PlaybackSink>,
    _player: Option<AudioPlayer>,
    runner: JobRunner,
    job_events: Receiver<JobEvent>,
    volume: VolumeControl<SystemCommandRunner>,
    volume_percent: Option<u16>,
    config_path: Option<PathBuf>,
    pending_samples: Option<Vec<f32>>,
    levels: Vec<f32>,
    levels_model: Rc<VecModel<f32>>,
    settings_model: Rc<VecModel<SettingsRow>>,
}

impl AppState {
    pub fn new(config: Config, config_path: Option<PathBuf>, is_mock: bool) -> Self {
        let capture = match AudioRecorder::open(config.audio_input_device.as_deref()) {
            Ok(recorder) => Capture::Device(recorder),
            Err(error) => {
                eprintln!("[audio] microphone unavailable ({error}); recording silence");
                Capture::Silent { started_at: None }
            }
        };
        let (player, playback): (Option<AudioPlayer>, Arc<dyn PlaybackSink>) =
            match AudioPlayer::open(config.audio_output_device.as_deref()) {
                Ok(player) => {
                    let queue = player.queue();
                    (Some(player), Arc::new(queue))
                }
                Err(error) => {
                    eprintln!("[audio] speaker unavailable ({error}); audio is discarded");
                    (None, Arc::new(DiscardingSink))
                }
            };
        let services = if is_mock {
            mock::services()
        } else {
            let speech = Arc::new(SpeechServiceClient::new(&config.speech_service_url));
            Services {
                stt: speech.clone(),
                translator: Arc::new(LlmClient::new(&config.llm_endpoint, &config.llm_model)),
                tts: speech,
            }
        };
        let (sender, job_events) = mpsc::channel();
        let runner = JobRunner::new(
            services,
            Arc::clone(&playback),
            Arc::new(move |event| {
                let _ = sender.send(event);
            }),
        );
        let volume = VolumeControl::new(SystemCommandRunner);
        let volume_percent = volume.current_percent();
        AppState {
            controller: Controller::new(config),
            settings_cursor: SettingsCursor::default(),
            capture,
            playback,
            _player: player,
            runner,
            job_events,
            volume,
            volume_percent,
            config_path,
            pending_samples: None,
            levels: Vec::new(),
            levels_model: Rc::new(VecModel::default()),
            settings_model: Rc::new(VecModel::default()),
        }
    }

    /// Connects the list models to the window once; `render` updates their contents.
    pub fn bind_models(&self, window: &AppWindow) {
        window.set_visualizer_levels(ModelRc::from(self.levels_model.clone()));
        window.set_settings_rows(ModelRc::from(self.settings_model.clone()));
    }

    pub fn handle_key_pressed(&mut self, text: &str) -> bool {
        let Some(key) = Key::from_event_text(text) else {
            return false;
        };
        if self.controller.is_settings_open() && key != Key::Escape {
            if let Some(action) = self.settings_cursor.handle_key(key) {
                let effects = self.controller.apply_settings_action(action);
                self.execute(effects);
            }
            return true;
        }
        let mode = self.controller.config().keyboard_mode;
        let Some(command) = command_for_key_press(mode, key, self.controller.active_lane()) else {
            return false;
        };
        let effects = self.controller.handle_command(command);
        self.execute(effects);
        true
    }

    pub fn handle_key_released(&mut self, text: &str) -> bool {
        let Some(key) = Key::from_event_text(text) else {
            return false;
        };
        let mode = self.controller.config().keyboard_mode;
        let Some(command) = command_for_key_release(mode, key, self.controller.active_lane()) else {
            return false;
        };
        let effects = self.controller.handle_command(command);
        self.execute(effects);
        true
    }

    /// Per-frame work: deliver job events and advance the visualizer.
    pub fn tick(&mut self) {
        while let Ok(event) = self.job_events.try_recv() {
            let effects = self.controller.on_job_event(event);
            self.execute(effects);
        }
        let bar_count = self.controller.config().visualizer_bars as usize;
        let level = matches!(self.controller.phase(), Phase::Recording(_))
            .then(|| self.capture.level() * LEVEL_GAIN);
        update_levels(&mut self.levels, bar_count, level);
    }

    pub fn render(&self, window: &AppWindow) {
        let controller = &self.controller;
        let config = controller.config();
        let transcript = controller.transcript();
        window.set_theme_color(theme_color(config.theme_color));
        window.set_lane1(lane_view(controller, Lane::One));
        window.set_lane2(lane_view(controller, Lane::Two));
        window.set_show_active_brackets(config.keyboard_mode == KeyboardMode::Landscape);
        window.set_source_label(transcript.source_label.as_str().into());
        window.set_transcription(transcript.transcription.as_str().into());
        window.set_target_label(transcript.target_label.as_str().into());
        window.set_translation(transcript.translation.as_str().into());
        window.set_is_translation_right_to_left(transcript.is_translation_right_to_left);
        window.set_status_text(status_line(controller).into());
        window.set_is_settings_open(controller.is_settings_open());
        self.levels_model.set_vec(self.levels.clone());
        self.settings_model
            .set_vec(settings_rows(config, &self.settings_cursor, self.volume_percent));
    }

    fn execute(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::PlayBlip(blip) => self.playback.enqueue(synthesize_blip(blip, BLIP_SAMPLE_RATE)),
                Effect::StopPlayback => self.playback.clear(),
                Effect::StartCapture => self.capture.start(),
                Effect::StopCapture => {
                    let captured = self.capture.stop();
                    self.pending_samples = Some(captured.samples);
                    let follow_up = self.controller.on_capture_finished(captured.duration);
                    self.execute(follow_up);
                }
                Effect::CancelJob(job) => self.runner.cancel(job),
                Effect::StartJob(request) => {
                    let samples = self.pending_samples.take().unwrap_or_default();
                    self.runner.start(request, samples);
                }
                Effect::AdjustVolume(direction) => self.volume_percent = self.volume.adjust(direction),
                Effect::SaveConfig => self.save_config(),
            }
        }
    }

    fn save_config(&self) {
        let Some(path) = &self.config_path else {
            return;
        };
        if let Err(error) = self.controller.config().save(path) {
            eprintln!("[config] {error}");
        }
    }
}
```

- [ ] **Step 7: Implement `main.rs`** (above its tests)

```rust
//! `translator`: the Gemma Translator kiosk UI.

mod app;
mod mock;
mod view;

slint::include_modules!();

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use translator_core::config::Config;

const USAGE: &str = "usage: translator [--mock] [--config <path>]";
/// Job events, visualizer and rendering are updated at ~33 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(30);

#[derive(Debug, PartialEq, Eq)]
struct CliOptions {
    is_mock: bool,
    config_path: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliOptions, String> {
    let mut options = CliOptions { is_mock: false, config_path: None };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mock" => options.is_mock = true,
            "--config" => {
                let path = args.next().ok_or("--config needs a path")?;
                options.config_path = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config_path = options.config_path.or_else(Config::default_path);
    let config = match &config_path {
        Some(path) => Config::load(path).unwrap_or_else(|error| {
            eprintln!("[config] {error}; using defaults");
            Config::default()
        }),
        None => Config::default(),
    };
    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("[ui] cannot create window: {error}");
            return ExitCode::FAILURE;
        }
    };

    let state = Rc::new(RefCell::new(app::AppState::new(config, config_path, options.is_mock)));
    state.borrow().bind_models(&window);
    state.borrow().render(&window);

    let key_state = Rc::clone(&state);
    window.on_key_pressed(move |text| key_state.borrow_mut().handle_key_pressed(&text));
    let release_state = Rc::clone(&state);
    window.on_key_released(move |text| release_state.borrow_mut().handle_key_released(&text));

    let frame_timer = slint::Timer::default();
    let weak_window = window.as_weak();
    let frame_state = Rc::clone(&state);
    frame_timer.start(slint::TimerMode::Repeated, FRAME_INTERVAL, move || {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let mut state = frame_state.borrow_mut();
        state.tick();
        state.render(&window);
    });

    match window.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[ui] {error}");
            ExitCode::FAILURE
        }
    }
}
```

- [ ] **Step 8: Run all tests, format and lint**

Run: `make test && cargo fmt --all -- --check && make clippy`
Expected: all Rust and Python tests pass; no format diff; no clippy warnings.

- [ ] **Step 9: Manual check in mock mode**

Run (on a machine with a display): `make run-mock`
Check, and note the result in the task report:
- Window 480×320 shows PERSON 1 ARABIC (bracketed, active) and PERSON 2 ENGLISH.
- Space switches the bracket; ←/→ rotate lane 1's language and never land on lane 2's.
- Hold Z ≥ 1 s: lane inverts while recording, bars move if a microphone exists; on release the transcript panel fills with the mock text, the translation appears sentence by sentence, a tone plays per sentence, the status line returns to READY.
- Tap Z briefly (< 300 ms): nothing happens.
- Esc opens settings; ↑/↓ select, ←/→ change; switching to VERTICAL hides the brackets; Esc closes; the config file at `~/.config/gemma-translator/config.toml` reflects the change.
- Set lane 2 to Arabic and record lane 1: the translation is right-aligned.
If no display is available (headless CI), record "manual check skipped: no display" in the report instead.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock Makefile translator-ui
git commit -m "Add Slint kiosk UI with mock mode"
```

---

### Task 13: Documentation

**Files:**
- Modify: `README.md` (new section after "Raspberry Pi Appliance Deployment", before "Project Structure"), `CLAUDE.md`

- [ ] **Step 1: Add the README section**

Insert into `README.md` directly before `## Project Structure`:
````markdown
## Rust UI (experimental)

A native Rust + [Slint](https://slint.dev) replacement for the Chromium kiosk UI lives in the Cargo workspace at the repo root (`translator-core`, `translator-io`, `translator-ui`). It records and plays audio itself and calls the Python service's `/v2/stt` and `/v2/tts` endpoints and `litert-lm serve` directly. See `docs/superpowers/specs/2026-10-09-rust-frontend-design.md`.

```bash
make test       # Rust + Python unit tests
make run-mock   # desktop UI with mock speech/translation (no models needed)
make run        # desktop UI against backend/server.py and litert-lm
make build-pi   # release build for the Pi (KMS backend, aarch64)
```

Settings are stored in `~/.config/gemma-translator/config.toml`. In the Rust UI, `Esc` opens the settings page (↑/↓ select, ←/→ change); all other keys match the tables above. Recordings shorter than 300 ms are ignored.
````

- [ ] **Step 2: Update `CLAUDE.md`**

In the `## Commands` code block, append:
```bash
make test               # Rust workspace tests + backend/test_*.py
make run-mock           # Rust/Slint UI with mock services
cargo test -p translator-core controller   # one crate, tests matching "controller"
```

Append a new section at the end of `CLAUDE.md`:
```markdown
## Rust UI (phase A of docs/superpowers/specs/2026-10-09-rust-frontend-design.md)

- `translator-core`: pure logic (languages, key mapping, sentence splitting, config, the `Controller` state machine). It must not depend on cpal, ureq or slint. The controller returns `Effect`s instead of doing I/O; `translator-ui/src/app.rs` executes them.
- `translator-io`: cpal audio, HTTP clients (`/v2/stt`, `/v2/tts` on :3000, streaming chat completions on :9379), volume commands, and `JobRunner`, which pipelines STT → streaming translation → per-sentence TTS on background threads and reports `JobEvent`s.
- `translator-ui`: Slint binary `translator`; `--mock` swaps in fake services. Job events reach the UI thread through an mpsc channel drained by a 30 ms timer.
- Python `/v2` endpoints take and return raw 16-bit PCM (`backend/pcm.py`); the old `/api/*` endpoints still serve the React UI.
```

- [ ] **Step 3: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "Document the Rust UI workspace"
```
