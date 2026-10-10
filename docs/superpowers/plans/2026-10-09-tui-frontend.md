# Terminal UI (TUI) Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `translator-tui`, a terminal frontend for use in a desktop terminal or over SSH. It has the same features as the Slint `translator` and shares its state layer, with toggle-style recording.

**Architecture:**
- The UI-independent half of `translator-ui` moves into a new library crate, `translator-app`:
  - `AppState` with its audio, job runner, volume and config handling;
  - the view model;
  - the mock services;
  - CLI parsing and config loading.
- `AppState` exposes `handle_key` / `tick` / `view`, and `view` returns a plain `AppView` that each frontend draws.
- The Slint binary keeps only the Slint conversion.
- The new `translator-tui` binary draws `AppView` with ratatui. It reads keys with crossterm, filters auto-repeat on the recording keys, and redirects fd 2 to a log file so C-library output cannot corrupt the screen.

**Tech Stack:** Rust stable (1.93 locally, edition 2021); ratatui 0.30, using its re-exported `ratatui::crossterm`; libc 0.2; the existing workspace crates.

**Spec:** `docs/superpowers/specs/2026-10-09-tui-frontend-design.md`

## Global Constraints

- Workspace members after this plan: `litert-lm-sys`, `moonshine-sys`, `translator-app`, `translator-core`, `translator-io`, `translator-models`, `translator-tui`, `translator-ui`, in alphabetical order.
- `translator-core` must not depend on `cpal`, `ureq`, `slint`, `*-sys`, `ratatui`, `crossterm` or `libloading`. `translator-app` must not depend on `slint`, `ratatui` or `crossterm`.
- `translator-tui` must not depend on `slint`.
- Slint UI behaviour must not change. It records while the key is held (`RecordStyle::Hold`).
- The TUI records by toggling (`RecordStyle::Toggle`): a press of the recording key starts recording, the next press stops it, and key releases are ignored.
- TUI minimum size is 40×12. Below it, the TUI shows only a "too small" message.
- The auto-repeat window is 1 s and applies to the recording keys Z and X only. The TUI frame interval is 30 ms.
- The TUI log file is `$XDG_STATE_HOME/gemma-translator/translator-tui.log`, falling back to `~/.local/state/gemma-translator/translator-tui.log`, opened in append mode.
- Quit with Ctrl-C. SIGTERM and SIGHUP also end the TUI cleanly. Ctrl and Alt combinations are never translator keys.
- Both binaries take the same CLI: `[--mock] [--config <path>]`. Usage errors exit with code 2.
- The TUI exits with code 2 when stdout is not a terminal, when `TERM` is unset or `dumb`, or when raw mode cannot be enabled.
- Run `cargo fmt --all` before every `cargo fmt --all -- --check`: the code blocks below are not guaranteed to be rustfmt-clean as printed.
- `make test` needs no terminal and no audio device.
- No TODOs or placeholders. Only add functions the change actually calls. Commit messages use the imperative mood.
- The main checkout may contain untracked files. Never stage anything outside the files your task lists.

### Rulings that refine the spec (recorded so reviewers do not flag them)

1. **UI text is English**, matching the Slint UI (`READY`, `PERSON 1`). The spec's Chinese strings ("視窗太小", "Z 錄音") describe meaning only. The TUI shows:
   - `Terminal too small (min 40x12)`
   - the key hints listed in Task 4.
2. **`AppView` adds two fields to the spec's list:**
   - `phase: Phase`
   - `keyboard_mode: KeyboardMode`

   The TUI needs both to build its key hints ("Z again to stop" vs "X again to stop").
3. **The TUI does not wire focus loss.** `handle_focus_lost` exists for the Slint hold style, where a lost key release would leave recording stuck. In toggle style, the recording ends on the next press, so a focus change must not stop it.
4. **Terminal setup does not use `ratatui::init()`.** Its panic hook restores the terminal on every thread, but JobRunner catches job-thread panics and the TUI keeps running (`translator-io/src/runner.rs:97`). So `terminal::enter()` / `terminal::leave()` do raw mode and the alternate screen, and they return errors instead of panicking. The TUI's single panic hook restores the terminal and stderr only for a main-thread panic, then prints the panic message and the log path.
5. **The repeat filter covers only the recording keys, and the kitty keyboard protocol is not used** (spec §5.2 as amended). Under that protocol, plain text keys report repeats only in "all keys as escape codes" mode. Without the protocol, a terminal reports a held key as a stream of presses. Z and X ignore a press within 1 s of the previous press of the same key. Other keys are not filtered, so quick arrow taps all count.
6. **`AppView` has a third extra field, `audio_warning: Option<String>`.** It is set when the microphone or speaker could not be opened. The TUI shows it on the status line, because its stderr goes to the log file. The Slint renderer ignores it, so Slint behaviour does not change.
7. **Accent colour.** `Color::Rgb` is used only when `COLORTERM` is `truecolor` or `24bit`. Otherwise the six theme colours map to named or 256-colour equivalents.
8. **`AppState::headless` runs no volume commands.** Its `VolumeWorker` uses a `CommandRunner` that returns `None`, so `make test` never shells out to `wpctl`.

## Review Focus

1. **Every way out of the TUI** (Ctrl-C, SIGTERM or SIGHUP, a returned error, a main-thread panic) leaves the shell usable: cooked mode, main screen, visible cursor, and stderr back on the terminal. A job-thread panic leaves the TUI running. Tests: Task 5, `sigterm_requests_a_clean_quit` and `stderr_redirect_sends_fd_2_to_the_file_and_restores_it_on_drop`. The panic paths are in the manual checklist.
2. **Holding Z** toggles once, not repeatedly, in every terminal. Quick arrow taps all count. Tests: Task 3, `held_key_stream_is_accepted_once` and `non_recording_keys_are_never_filtered`.
3. **libmoonshine / liblitert-lm writing to fd 2** never lands on the TUI screen. Test: Task 5, `stderr_redirect_sends_fd_2_to_the_file_and_restores_it_on_drop`, which covers the redirect half.
4. **A terminal resized very small, even 1×1, or back to normal** never panics, and a too-small terminal shows the hint. Test: Task 4, `tiny_terminals_show_the_hint_without_panicking`.
5. **The Slint UI after the refactor** still starts recording on press and stops on release. Test: Task 2, `hold_style_stops_recording_on_release`.
6. **No microphone on the SSH host:** the TUI says so instead of silently transcribing silence. Test: Task 4, `audio_warning_is_shown_in_the_status_line`.

---

## File Structure

```
Cargo.toml                        members + translator-app, translator-tui
Makefile                          run-tui
translator-core/src/keys.rs       + command_for_key_toggle
translator-app/
  Cargo.toml
  src/lib.rs                      module list, re-exports
  src/cli.rs                      CliOptions, parse_args, load_config, unused_backup_path (from translator-ui/src/main.rs)
  src/mock.rs                     moved from translator-ui/src/mock.rs
  src/state.rs                    AppState, RecordStyle, KeyAction (from translator-ui/src/app.rs)
  src/view.rs                     AppView, LaneView, SettingsRowView and builders (from translator-ui/src/view.rs)
translator-ui/
  src/main.rs                     window + timer, uses translator-app
  src/render.rs                   AppView → Slint properties/models (replaces app.rs + view.rs)
  (deleted: src/app.rs, src/view.rs, src/mock.rs)
translator-tui/
  Cargo.toml
  src/main.rs                     startup order, main loop
  src/keys.rs                     crossterm KeyEvent → Key (no Ctrl/Alt), quit detection, RepeatFilter (Z/X)
  src/render.rs                   draw(frame, &AppView, is_truecolor), accent colour, status hints
  src/terminal.rs                 enter/leave, log_path, StderrRedirect, panic hook, quit signals, TERM/COLORTERM checks
README.md, CLAUDE.md              TUI section
```

---

### Task 1: Toggle-style key mapping in translator-core

**Files:**
- Modify: `translator-core/src/keys.rs`

**Interfaces:**
- Consumes:
  - `translator_core::controller::Phase`
  - `translator_core::lane::Lane`
  - the existing `command_for_key_press(mode, key, active_lane) -> Option<Command>`
- Produces: `pub fn command_for_key_toggle(mode: KeyboardMode, key: Key, active_lane: Lane, phase: Phase) -> Option<Command>`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `translator-core/src/keys.rs`:

```rust
    #[test]
    fn toggle_starts_then_stops_the_landscape_recording() {
        use crate::controller::Phase;
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
        use crate::controller::Phase;
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
        use crate::controller::Phase;
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
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p translator-core keys`
Expected: compile error, because `command_for_key_toggle` is not defined.

- [ ] **Step 3: Implement**

Add after `command_for_key_release` in `translator-core/src/keys.rs`, and add `use crate::controller::Phase;` to the module's imports:

```rust
/// Toggle-style recording for terminals, which do not report key releases:
/// a recording key starts recording on its lane, or stops it when that lane
/// is already recording. Every other key maps as in `command_for_key_press`.
pub fn command_for_key_toggle(
    mode: KeyboardMode,
    key: Key,
    active_lane: Lane,
    phase: Phase,
) -> Option<Command> {
    let recording_key_lane = match (mode, key) {
        (KeyboardMode::Landscape, Key::Z) => Some(active_lane),
        (KeyboardMode::Vertical, Key::Z) => Some(Lane::One),
        (KeyboardMode::Vertical, Key::X) => Some(Lane::Two),
        _ => None,
    };
    match recording_key_lane {
        Some(lane) if phase == Phase::Recording(lane) => Some(Command::StopRecording(lane)),
        Some(lane) => Some(Command::StartRecording(lane)),
        None => command_for_key_press(mode, key, active_lane),
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p translator-core && cargo clippy -p translator-core --all-targets -- -D warnings`
Expected: everything passes, with no warnings.

- [ ] **Step 5: Commit**

```bash
git add translator-core/src/keys.rs
git commit -m "Add toggle-style recording key mapping for terminal frontends"
```

---

### Task 2: Extract the shared state layer into translator-app

This task moves code; it does not rewrite it. Use `git mv` so history follows the files, then apply the listed edits. At the end, the Slint UI behaves exactly as before.

**Files:**
- Create: `translator-app/Cargo.toml`, `translator-app/src/lib.rs`
- Move: `translator-ui/src/mock.rs` → `translator-app/src/mock.rs` (no content change)
- Move + edit: `translator-ui/src/app.rs` → `translator-app/src/state.rs`
- Move + edit: `translator-ui/src/view.rs` → `translator-app/src/view.rs`
- Create: `translator-app/src/cli.rs`, with code and tests moved out of `translator-ui/src/main.rs`
- Create: `translator-ui/src/render.rs`
- Modify: `translator-ui/src/main.rs`, `translator-ui/Cargo.toml`, root `Cargo.toml`

**Interfaces:**
- Consumes: `command_for_key_toggle` (Task 1).
- Produces (all `pub` in `translator_app`):
  - `RecordStyle { Hold, Toggle }`, deriving Debug, Clone, Copy, PartialEq, Eq.
  - `KeyAction { Pressed, Released }`, with the same derives.
  - `AppState::new(config: Config, config_path: Option<PathBuf>, is_mock: bool, record_style: RecordStyle) -> AppState`
  - `AppState::headless(config: Config, record_style: RecordStyle) -> AppState`: mock services, silent capture, discarded playback, no config saving.
  - `AppState::handle_key(&mut self, key: Key, action: KeyAction) -> bool`
  - `AppState::handle_focus_lost(&mut self)`
  - `AppState::tick(&mut self)`
  - `AppState::view(&self) -> AppView`
  - `AppView`:
    - `theme_rgb: (u8, u8, u8)`
    - `lanes: [LaneView; 2]`
    - `keyboard_mode: KeyboardMode`
    - `phase: Phase`
    - `show_active_marker: bool`
    - `source_label`, `transcription`, `target_label`, `translation`: `String`
    - `is_translation_right_to_left: bool`
    - `status: String`
    - `levels: Vec<f32>`
    - `is_settings_open: bool`
    - `settings_rows: Vec<SettingsRowView>`
  - `LaneView { label: String, language: String, is_active: bool, is_recording: bool }`
  - `SettingsRowView { label: String, value: String, is_selected: bool }`
  - `cli::CliOptions { is_mock: bool, config_path: Option<PathBuf> }`
  - `cli::parse_args(args: impl IntoIterator<Item = String>) -> Result<CliOptions, String>`
  - `cli::load_config(path: PathBuf) -> (Config, Option<PathBuf>)`

- [ ] **Step 1: Create the crate and move the files**

```bash
mkdir -p translator-app/src
git mv translator-ui/src/mock.rs translator-app/src/mock.rs
git mv translator-ui/src/app.rs translator-app/src/state.rs
git mv translator-ui/src/view.rs translator-app/src/view.rs
```

Add `"translator-app"` to the root `Cargo.toml` `members`, keeping alphabetical order: it goes after `"moonshine-sys"`.

`translator-app/Cargo.toml`:

```toml
[package]
name = "translator-app"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
translator-core = { path = "../translator-core" }
translator-io = { path = "../translator-io" }

[dev-dependencies]
tempfile = "3"
```

`translator-app/src/lib.rs`:

```rust
//! UI-independent application state shared by the Slint (`translator`) and
//! terminal (`translator-tui`) frontends: each frontend turns its key events
//! into `handle_key` calls, calls `tick` every frame and draws `view()`.

pub mod cli;
pub mod mock;
pub mod state;
pub mod view;

pub use state::{AppState, KeyAction, RecordStyle};
pub use view::{AppView, LaneView, SettingsRowView};
```

In `translator-ui/Cargo.toml`:
- Add `translator-app = { path = "../translator-app" }` under `[dependencies]`.
- Remove `translator-io`, which nothing in translator-ui uses after the move.
- Remove `tempfile` from `[dev-dependencies]`. Only `cli.rs` used it, and the CLI tests move to translator-app.

- [ ] **Step 2: Make `view.rs` Slint-free**

In `translator-app/src/view.rs`:

1. **Replace the imports and the header** with:

```rust
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
```

2. **Delete `pub fn theme_color`.** It moves to the Slint renderer in Step 5.
3. **`lane_view`:** make it `pub(crate)` and return `LaneView` with `String` fields:
   - replace `"1".into()` / `"2".into()` with `"1".to_string()` / `"2".to_string()`;
   - replace `.to_uppercase().into()` with `.to_uppercase()`.
4. **`status_line` and `update_levels`:** make them `pub(crate)`.
5. **`settings_rows`:** make it `pub(crate)` and return `Vec<SettingsRowView>`. Build `SettingsRowView { label: label.to_string(), value, is_selected: … }`, where `value` is already a `String`.
6. **Tests:**
   - Delete `slint_special_keys_match_core_key_codes` and `theme_color_converts_to_slint`; both move to `translator-ui/src/render.rs` in Step 5.
   - The other tests stay. Change `rows.iter().map(|row| row.value.as_str())` as needed; it already works on `String`.

- [ ] **Step 3: Make `state.rs` Slint-free and add the key/record-style API**

In `translator-app/src/state.rs`:

1. **Imports:**
   - Remove `use std::rc::Rc;`, `use slint::{…};`, `use crate::{mock, AppWindow, SettingsRow};` and `use crate::view::{…};`.
   - Add:

```rust
use translator_core::keys::command_for_key_toggle;
use translator_io::volume::CommandRunner;

use crate::mock;
use crate::view::{lane_view, settings_rows, status_line, update_levels, AppView};
```

   - Keep the existing `translator_core::keys` import, minus `KeyboardMode` if it becomes unused. It stays used in `view()` below, so keep it.
2. **Module doc:** replace with

```rust
//! UI-independent application state: turns key presses into controller
//! commands and carries out the controller's effects on the audio devices,
//! job runner and config file.
```

3. **Add above `pub struct AppState`:**

```rust
/// How the recording keys behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordStyle {
    /// Record while the key is held (Slint: windows report releases).
    Hold,
    /// Press to start, press again to stop (terminals report no releases).
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Pressed,
    Released,
}
```

4. **`AppState` fields:** remove `levels_model` and `settings_model`, and add `record_style: RecordStyle` and `audio_warning: Option<String>`. Add these two private types next to `Capture`:

```rust
/// The audio side of a new AppState.
struct Devices {
    capture: Capture,
    player: Option<AudioPlayer>,
    playback: Arc<dyn PlaybackSink>,
}

/// Volume commands for headless state: none are run, the volume reads N/A.
struct NoVolumeCommands;

impl CommandRunner for NoVolumeCommands {
    fn run(&self, _program: &str, _args: &[&str]) -> Option<String> {
        None
    }
}
```
5. **Replace `pub fn new(…)`** with these three functions. The body of `new` is the old device and services code split up; `assemble` holds the old tail that builds the runner and the struct.

```rust
    /// Opens the configured (or default) microphone and speaker; without them
    /// it records silence and discards audio, so jobs still run, and
    /// `AppView::audio_warning` says why.
    pub fn new(
        config: Config,
        config_path: Option<PathBuf>,
        is_mock: bool,
        record_style: RecordStyle,
    ) -> Self {
        let mut warnings = Vec::new();
        let capture = match AudioRecorder::open(config.audio_input_device.as_deref()) {
            Ok(recorder) => Capture::Device(recorder),
            Err(error) => {
                eprintln!("[audio] microphone unavailable ({error}); recording silence");
                warnings.push("no microphone: recording silence");
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
                    warnings.push("no speaker: audio discarded");
                    (None, Arc::new(DiscardingSink))
                }
            };
        let services = if is_mock {
            mock::services()
        } else {
            configured_services(&config)
        };
        Self::assemble(
            config,
            config_path,
            record_style,
            Devices {
                capture,
                player,
                playback,
            },
            services,
            VolumeWorker::spawn(SystemCommandRunner),
            (!warnings.is_empty()).then(|| warnings.join(", ")),
        )
    }

    /// Mock services, silent recordings, discarded audio, no volume commands
    /// and no config file: runs anywhere, including tests without audio devices.
    pub fn headless(config: Config, record_style: RecordStyle) -> Self {
        Self::assemble(
            config,
            None,
            record_style,
            Devices {
                capture: Capture::Silent { started_at: None },
                player: None,
                playback: Arc::new(DiscardingSink),
            },
            mock::services(),
            VolumeWorker::spawn(NoVolumeCommands),
            None,
        )
    }

    fn assemble(
        config: Config,
        config_path: Option<PathBuf>,
        record_style: RecordStyle,
        devices: Devices,
        services: Services,
        volume: VolumeWorker,
        audio_warning: Option<String>,
    ) -> Self {
        let (sender, job_events) = mpsc::channel();
        let runner = JobRunner::new(
            services,
            Arc::clone(&devices.playback),
            Arc::new(move |event| {
                let _ = sender.send(event);
            }),
        );
        AppState {
            controller: Controller::new(config),
            settings_cursor: SettingsCursor::default(),
            capture: devices.capture,
            playback: devices.playback,
            _player: devices.player,
            runner,
            job_events,
            volume,
            volume_percent: None,
            config_path,
            pending_capture: None,
            levels: Vec::new(),
            record_style,
            audio_warning,
        }
    }
```

6. **Add `configured_services`** as a free function next to `native_services`. It is the old `match config.inference_backend` block:

```rust
/// The HTTP services (phase A) or the in-process ones, per the config.
fn configured_services(config: &Config) -> Services {
    match config.inference_backend {
        InferenceBackend::Http => {
            let speech = Arc::new(SpeechServiceClient::new(&config.speech_service_url));
            Services {
                stt: speech.clone(),
                translator: Arc::new(LlmClient::new(&config.llm_endpoint, &config.llm_model)),
                tts: speech,
            }
        }
        InferenceBackend::Native => native_services(config),
    }
}
```

7. **Delete `bind_models`, `render` and `set_if_changed`.**
8. **Replace `handle_key_pressed` / `handle_key_released`** with:

```rust
    /// Returns true when the key was handled.
    pub fn handle_key(&mut self, key: Key, action: KeyAction) -> bool {
        match action {
            KeyAction::Pressed => self.handle_key_pressed(key),
            KeyAction::Released => self.handle_key_released(key),
        }
    }

    fn handle_key_pressed(&mut self, key: Key) -> bool {
        if self.controller.is_settings_open() && key != Key::Escape {
            if let Some(action) = self.settings_cursor.handle_key(key) {
                let effects = self.controller.apply_settings_action(action);
                self.execute(effects);
            }
            return true;
        }
        let mode = self.controller.config().keyboard_mode;
        let lane = self.controller.active_lane();
        let command = match self.record_style {
            RecordStyle::Hold => command_for_key_press(mode, key, lane),
            RecordStyle::Toggle => command_for_key_toggle(mode, key, lane, self.controller.phase()),
        };
        let Some(command) = command else {
            return false;
        };
        let effects = self.controller.handle_command(command);
        self.execute(effects);
        true
    }

    fn handle_key_released(&mut self, key: Key) -> bool {
        if self.record_style == RecordStyle::Toggle {
            return false;
        }
        let mode = self.controller.config().keyboard_mode;
        let Some(command) = command_for_key_release(mode, key, self.controller.active_lane())
        else {
            return false;
        };
        let effects = self.controller.handle_command(command);
        self.execute(effects);
        true
    }
```

9. **Add `view`** after `tick`:

```rust
    /// The frame to draw now.
    pub fn view(&self) -> AppView {
        let controller = &self.controller;
        let config = controller.config();
        let transcript = controller.transcript();
        AppView {
            theme_rgb: config.theme_color.rgb(),
            lanes: [
                lane_view(controller, Lane::One),
                lane_view(controller, Lane::Two),
            ],
            keyboard_mode: config.keyboard_mode,
            phase: controller.phase(),
            show_active_marker: config.keyboard_mode == KeyboardMode::Landscape,
            source_label: transcript.source_label.clone(),
            transcription: transcript.transcription.clone(),
            target_label: transcript.target_label.clone(),
            translation: transcript.translation.clone(),
            is_translation_right_to_left: transcript.is_translation_right_to_left,
            status: status_line(controller),
            audio_warning: self.audio_warning.clone(),
            levels: self.levels.clone(),
            is_settings_open: controller.is_settings_open(),
            settings_rows: settings_rows(config, &self.settings_cursor, self.volume_percent),
        }
    }
```

10. **Append a test module** at the end of `state.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use translator_core::controller::MIN_RECORDING_DURATION;

    use super::*;

    /// Ticks until `done(view)` holds, failing after 10 s.
    fn tick_until(state: &mut AppState, done: impl Fn(&AppView) -> bool) -> AppView {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            state.tick();
            let view = state.view();
            if done(&view) {
                return view;
            }
            assert!(Instant::now() < deadline, "gave up waiting: {view:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn toggle_style_records_until_the_second_press_and_finishes_the_job() {
        let mut state = AppState::headless(Config::default(), RecordStyle::Toggle);
        assert!(state.handle_key(Key::Z, KeyAction::Pressed));
        assert_eq!(state.view().phase, Phase::Recording(Lane::One));
        assert!(!state.handle_key(Key::Z, KeyAction::Released));
        assert_eq!(state.view().phase, Phase::Recording(Lane::One));
        thread::sleep(MIN_RECORDING_DURATION + Duration::from_millis(50));
        assert!(state.handle_key(Key::Z, KeyAction::Pressed));
        let view = tick_until(&mut state, |view| {
            view.phase == Phase::Idle && !view.translation.is_empty()
        });
        assert_eq!(view.status, "READY");
        assert!(!view.transcription.is_empty());
    }

    #[test]
    fn hold_style_stops_recording_on_release() {
        let mut state = AppState::headless(Config::default(), RecordStyle::Hold);
        assert!(state.handle_key(Key::Z, KeyAction::Pressed));
        assert_eq!(state.view().phase, Phase::Recording(Lane::One));
        // A second press while held (no release yet) does not stop it.
        state.handle_key(Key::Z, KeyAction::Pressed);
        assert_eq!(state.view().phase, Phase::Recording(Lane::One));
        assert!(state.handle_key(Key::Z, KeyAction::Released));
        // Released right away: shorter than the minimum, so it is discarded.
        assert_eq!(state.view().phase, Phase::Idle);
    }

    #[test]
    fn settings_keys_go_to_the_cursor_while_settings_are_open() {
        let mut state = AppState::headless(Config::default(), RecordStyle::Toggle);
        assert!(state.handle_key(Key::Escape, KeyAction::Pressed));
        assert!(state.view().is_settings_open);
        assert!(state.handle_key(Key::ArrowRight, KeyAction::Pressed));
        assert_eq!(state.view().settings_rows[0].value, "VERTICAL");
        assert!(state.handle_key(Key::Z, KeyAction::Pressed));
        assert_eq!(state.view().phase, Phase::Idle);
    }
}
```

   `MIN_RECORDING_DURATION` is already `pub` in `translator-core/src/controller.rs:13`. The third test relies on Right on the first settings row (keyboard mode) toggling Landscape to Vertical; the first row is `KEYBOARD`.

- [ ] **Step 4: Create `cli.rs`**

Move `CliOptions`, `parse_args`, `load_config` and `unused_backup_path` out of `translator-ui/src/main.rs` into `translator-app/src/cli.rs`, together with their tests:
- `parses_mock_and_config`
- `rejects_unknown_and_incomplete_arguments`
- `invalid_config_is_moved_aside_before_anything_is_saved`
- `missing_or_valid_config_is_loaded_in_place`
- `a_second_broken_config_keeps_the_first_backup`
- `config_that_cannot_be_moved_aside_disables_saving`

Changes while moving:
- File header: `//! Command line and config loading shared by both frontends.`
- `pub struct CliOptions` with `pub is_mock` and `pub config_path`; `pub fn parse_args`; `pub fn load_config`. `unused_backup_path` stays private.
- Imports: `use std::path::{Path, PathBuf};` and `use translator_core::config::Config;`. The tests also need `use translator_core::keys::KeyboardMode;` and `use super::*;`.
- `USAGE` and `FRAME_INTERVAL` stay in `translator-ui/src/main.rs`. Each binary prints its own usage.

- [ ] **Step 5: Slint renderer and main**

`translator-ui/src/render.rs`:

```rust
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
    set_if_changed(&models.levels, view.levels.clone());
    set_if_changed(
        &models.settings_rows,
        view.settings_rows.iter().map(settings_row).collect(),
    );
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

/// Replaces the model's rows only when they differ, so Slint does not redo
/// the list for unchanged contents on every frame.
fn set_if_changed<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.iter().ne(rows.iter().cloned()) {
        model.set_vec(rows);
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
```

In `translator-ui/src/main.rs`:
- Replace `mod app; mod mock; mod view;` with `mod render;`.
- Remove the moved CLI and config code, and its tests.
- Import:

```rust
use translator_app::cli::{load_config, parse_args};
use translator_app::{AppState, KeyAction, RecordStyle};
use translator_core::keys::Key;
```

Replace the window-wiring section, from `let state = …` up to the frame timer, with:

```rust
    let state = Rc::new(RefCell::new(AppState::new(
        config,
        config_path,
        options.is_mock,
        RecordStyle::Hold,
    )));
    let models = Rc::new(render::Models::bind(&window));
    render::render(&state.borrow().view(), &window, &models);

    let key_state = Rc::clone(&state);
    window.on_key_pressed(move |text| {
        Key::from_event_text(&text)
            .is_some_and(|key| key_state.borrow_mut().handle_key(key, KeyAction::Pressed))
    });
    let release_state = Rc::clone(&state);
    window.on_key_released(move |text| {
        Key::from_event_text(&text)
            .is_some_and(|key| release_state.borrow_mut().handle_key(key, KeyAction::Released))
    });
    let focus_state = Rc::clone(&state);
    window.on_keyboard_focus_lost(move || focus_state.borrow_mut().handle_focus_lost());

    let frame_timer = slint::Timer::default();
    let weak_window = window.as_weak();
    let frame_state = Rc::clone(&state);
    let frame_models = Rc::clone(&models);
    frame_timer.start(slint::TimerMode::Repeated, FRAME_INTERVAL, move || {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let mut state = frame_state.borrow_mut();
        state.tick();
        render::render(&state.view(), &window, &frame_models);
    });
```

Remove any `std::path` imports that are now unused from `main.rs`.

- [ ] **Step 6: Verify**

Run: `cargo fmt --all && cargo test -p translator-app -p translator-ui && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check && make test`
Expected:
- all pass;
- translator-app has the moved view tests, the 6 CLI tests and the 3 new state tests;
- translator-ui has 2 tests.

Then run `make run-mock` for about ten seconds and check that:
- the window opens;
- holding Z records;
- releasing produces the mock transcript and translation.

A subagent without a display skips this manual check and says so in its report.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock translator-app translator-ui
git commit -m "Extract the UI-independent state into translator-app"
```

---

### Task 3: TUI crate, key translation and the auto-repeat filter

**Files:**
- Create: `translator-tui/Cargo.toml`, `translator-tui/src/main.rs` (a stub for now), `translator-tui/src/keys.rs`
- Modify: root `Cargo.toml` (members)

**Interfaces:**
- Consumes: `translator_core::keys::Key`.
- Produces:
  - `keys::key_from_event(event: &KeyEvent) -> Option<Key>`
  - `keys::is_quit(event: &KeyEvent) -> bool`
  - `keys::RepeatFilter::new(window: Duration)`
  - `RepeatFilter::accept(&mut self, key: Key, kind: KeyEventKind, now: Instant) -> bool`
  - `keys::REPEAT_WINDOW: Duration = 700 ms`

- [ ] **Step 1: Create the crate and write the failing tests**

Add `"translator-tui"` to the root `members`, after `"translator-models"`.

`translator-tui/Cargo.toml`:

```toml
[package]
name = "translator-tui"
edition.workspace = true
version.workspace = true
license.workspace = true

[[bin]]
name = "translator-tui"
path = "src/main.rs"

[dependencies]
translator-app = { path = "../translator-app" }
translator-core = { path = "../translator-core" }
ratatui = "0.30"
libc = "0.2"

[dev-dependencies]
tempfile = "3"
```

`translator-tui/src/main.rs` (stub; Task 5 replaces it):

```rust
//! `translator-tui`: the Gemma Translator in a terminal.

mod keys;

fn main() {}
```

`translator-tui/src/keys.rs`, tests only for now:

```rust
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
        assert!(is_quit(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert!(!is_quit(&press(KeyCode::Char('c'))));
        assert!(!is_quit(&press(KeyCode::Char('q'))));
        assert!(!is_quit(&press(KeyCode::Esc)));
    }

    #[test]
    fn held_key_stream_is_accepted_once() {
        let start = Instant::now();
        let mut filter = RepeatFilter::new(REPEAT_WINDOW);
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
        let mut filter = RepeatFilter::new(REPEAT_WINDOW);
        for step in 0..5 {
            let now = start + Duration::from_millis(33 * step);
            assert!(filter.accept(Key::ArrowRight, KeyEventKind::Press, now));
        }
        assert!(filter.accept(Key::Z, KeyEventKind::Press, start));
        assert!(filter.accept(Key::Space, KeyEventKind::Press, start + Duration::from_millis(10)));
    }

    #[test]
    fn releases_and_repeats_are_dropped() {
        let start = Instant::now();
        let mut filter = RepeatFilter::new(REPEAT_WINDOW);
        assert!(!filter.accept(Key::Z, KeyEventKind::Release, start));
        assert!(!filter.accept(Key::ArrowLeft, KeyEventKind::Repeat, start));
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p translator-tui`
Expected: compile errors, because `key_from_event`, `is_quit`, `RepeatFilter` and `REPEAT_WINDOW` are missing.

- [ ] **Step 3: Implement**

Prepend to `translator-tui/src/keys.rs`:

```rust
//! Terminal key events → translator keys, and the auto-repeat filter that
//! makes toggle-style recording usable when a recording key is held.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use translator_core::keys::Key;

/// A recording key pressed again this soon after its previous press is OS
/// auto-repeat.
pub const REPEAT_WINDOW: Duration = Duration::from_millis(700);

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
    window: Duration,
    last_record_press: Option<(Key, Instant)>,
}

impl RepeatFilter {
    pub fn new(window: Duration) -> Self {
        RepeatFilter {
            window,
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
        let is_repeat = self
            .last_record_press
            .is_some_and(|(last_key, at)| last_key == key && now.duration_since(at) < self.window);
        self.last_record_press = Some((key, now));
        !is_repeat
    }
}
```

`main.rs` uses nothing from `keys` yet, so `cargo build` warns about dead code until Task 5. Do not add `allow` attributes. Clippy for this task runs only on the other crates (Step 4).

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p translator-tui && cargo fmt --all -- --check && cargo clippy --workspace --exclude translator-tui --all-targets -- -D warnings`
Expected: 6 tests pass. Clippy is clean for every crate except translator-tui, whose dead-code warnings Task 5 resolves.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock translator-tui
git commit -m "Add translator-tui with key translation and an auto-repeat filter"
```

---

### Task 4: TUI rendering

**Files:**
- Create: `translator-tui/src/render.rs`
- Modify: `translator-tui/src/main.rs` (add `mod render;`)

**Interfaces:**
- Consumes: `translator_app::{AppView, LaneView, SettingsRowView}` (Task 2).
- Produces:
  - `render::draw(frame: &mut ratatui::Frame, view: &AppView, is_truecolor: bool)`
  - `render::MIN_WIDTH: u16 = 40`
  - `render::MIN_HEIGHT: u16 = 12`

- [ ] **Step 1: Write the failing tests**

`translator-tui/src/render.rs`, tests only:

```rust
#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use translator_core::config::ThemeColor;
    use translator_core::controller::Phase;
    use translator_core::keys::KeyboardMode;
    use translator_core::lane::Lane;

    use super::*;

    fn view() -> AppView {
        AppView {
            theme_rgb: ThemeColor::Orange.rgb(),
            lanes: [
                LaneView {
                    label: "1".to_string(),
                    language: "ARABIC".to_string(),
                    is_active: true,
                    is_recording: false,
                },
                LaneView {
                    label: "2".to_string(),
                    language: "CHINESE".to_string(),
                    is_active: false,
                    is_recording: false,
                },
            ],
            keyboard_mode: KeyboardMode::Landscape,
            phase: Phase::Idle,
            show_active_marker: true,
            source_label: "Arabic (Source)".to_string(),
            transcription: "مرحبا".to_string(),
            target_label: "Chinese (Translation)".to_string(),
            translation: "早上好，你好吗？".to_string(),
            is_translation_right_to_left: false,
            status: "READY".to_string(),
            audio_warning: None,
            levels: vec![0.0, 0.5, 1.0, 0.25],
            is_settings_open: false,
            settings_rows: vec![
                SettingsRowView {
                    label: "KEYBOARD".to_string(),
                    value: "LANDSCAPE".to_string(),
                    is_selected: true,
                },
                SettingsRowView {
                    label: "VOLUME".to_string(),
                    value: "75%".to_string(),
                    is_selected: false,
                },
            ],
        }
    }

    /// Draws at the given size and returns each row's text. A wide (CJK)
    /// character's continuation cell holds a space, so callers strip spaces
    /// before searching for CJK text; Arabic letters are one cell wide.
    fn rows(view: &AppView, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, view, true)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    fn screen(view: &AppView, width: u16, height: u16) -> String {
        rows(view, width, height).join("\n")
    }

    #[test]
    fn main_screen_shows_lanes_transcript_and_status() {
        let text = screen(&view(), 80, 24);
        for expected in [
            "PERSON 1",
            "▶ ARABIC",
            "PERSON 2",
            "CHINESE",
            "Arabic (Source)",
            "READY",
            "Ctrl-C quit",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in\n{text}");
        }
    }

    #[test]
    fn cjk_translation_is_drawn_in_full() {
        let compact: String = screen(&view(), 80, 24).chars().filter(|c| *c != ' ').collect();
        assert!(compact.contains("早上好，你好吗？"), "{compact}");
    }

    #[test]
    fn recording_shows_rec_and_the_stop_hint() {
        let mut recording = view();
        recording.phase = Phase::Recording(Lane::One);
        recording.lanes[0].is_recording = true;
        recording.status = "RECORDING".to_string();
        let text = screen(&recording, 80, 24);
        assert!(text.contains("● REC"), "{text}");
        assert!(text.contains("Z again to stop"), "{text}");
    }

    #[test]
    fn vertical_mode_names_x_for_lane_two_and_marks_no_lane() {
        let mut recording = view();
        recording.keyboard_mode = KeyboardMode::Vertical;
        recording.show_active_marker = false;
        recording.phase = Phase::Recording(Lane::Two);
        recording.lanes[1].is_recording = true;
        let text = screen(&recording, 80, 24);
        assert!(text.contains("X again to stop"), "{text}");
        assert!(!text.contains("▶ ARABIC") && !text.contains("▶ CHINESE"), "{text}");
    }

    #[test]
    fn audio_warning_is_shown_in_the_status_line() {
        let mut silent = view();
        silent.audio_warning = Some("no microphone: recording silence".to_string());
        let text = screen(&silent, 80, 24);
        assert!(text.contains("no microphone: recording silence"), "{text}");
    }

    #[test]
    fn settings_popup_lists_rows() {
        let mut settings = view();
        settings.is_settings_open = true;
        let text = screen(&settings, 80, 24);
        assert!(text.contains("SETTINGS"), "{text}");
        assert!(text.contains("KEYBOARD"), "{text}");
        assert!(text.contains("75%"), "{text}");
        assert!(text.contains("Esc close"), "{text}");
    }

    #[test]
    fn minimum_size_still_draws_the_layout() {
        let text = screen(&view(), MIN_WIDTH, MIN_HEIGHT);
        assert!(text.contains("PERSON 1"), "{text}");
        assert!(text.contains("READY"), "{text}");
    }

    #[test]
    fn tiny_terminals_show_the_hint_without_panicking() {
        let text = screen(&view(), MIN_WIDTH - 1, MIN_HEIGHT);
        assert!(text.contains("too small"), "{text}");
        for (width, height) in [(1, 1), (5, 2), (39, 30), (200, 11)] {
            rows(&view(), width, height);
        }
    }

    #[test]
    fn right_to_left_translation_is_right_aligned() {
        let mut arabic = view();
        arabic.transcription = "hello".to_string();
        arabic.translation = "مرحبا".to_string();
        arabic.is_translation_right_to_left = true;
        let rows = rows(&arabic, 80, 24);
        let row = rows
            .iter()
            .find(|row| row.contains("مرحبا"))
            .expect("translation row");
        let inner = row.trim_end_matches('│').trim_end();
        assert!(inner.ends_with("مرحبا"), "{row:?}");
    }

    #[test]
    fn accent_falls_back_to_named_colours_without_truecolor() {
        assert_eq!(accent_color(ThemeColor::Orange.rgb(), true), Color::Rgb(0xff, 0xa5, 0x00));
        assert_eq!(accent_color(ThemeColor::Orange.rgb(), false), Color::Indexed(214));
        assert_eq!(accent_color(ThemeColor::Blue.rgb(), false), Color::LightBlue);
        assert_eq!(accent_color(ThemeColor::Red.rgb(), false), Color::LightRed);
        assert_eq!(accent_color(ThemeColor::Green.rgb(), false), Color::Green);
        assert_eq!(accent_color(ThemeColor::Yellow.rgb(), false), Color::Yellow);
        assert_eq!(accent_color(ThemeColor::White.rgb(), false), Color::White);
    }
}
```

Add `mod render;` to `main.rs`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p translator-tui render`
Expected: compile error, because `draw`, `accent_color`, `MIN_WIDTH` and `MIN_HEIGHT` are missing.

- [ ] **Step 3: Implement**

Prepend to `translator-tui/src/render.rs`:

```rust
//! Draws one `AppView` frame with ratatui.

use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;
use translator_app::{AppView, LaneView, SettingsRowView};
use translator_core::controller::Phase;
use translator_core::keys::KeyboardMode;
use translator_core::lane::Lane;

pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 12;
const BAR_GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const SETTINGS_WIDTH: u16 = 36;

/// `is_truecolor`: the terminal takes 24-bit colour (`COLORTERM`).
pub fn draw(frame: &mut Frame, view: &AppView, is_truecolor: bool) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let hint = Paragraph::new(format!("Terminal too small (min {MIN_WIDTH}x{MIN_HEIGHT})"))
            .wrap(Wrap { trim: true });
        frame.render_widget(hint, area);
        return;
    }
    let accent = accent_color(view.theme_rgb, is_truecolor);
    let [lanes_area, levels_area, transcript_area, status_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .areas(area);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(lanes_area);
    draw_lane(frame, left, &view.lanes[0], view.show_active_marker, accent);
    draw_lane(frame, right, &view.lanes[1], view.show_active_marker, accent);
    frame.render_widget(
        Paragraph::new(level_bars(&view.levels, levels_area.width)).style(Style::new().fg(accent)),
        levels_area,
    );
    draw_transcript(frame, transcript_area, view, accent);
    frame.render_widget(status_line(view, accent), status_area);
    if view.is_settings_open {
        draw_settings(frame, area, &view.settings_rows, accent);
    }
}

/// The theme colour as RGB, or the closest named / 256-colour value for
/// terminals without 24-bit colour (where RGB would be dropped and the
/// recording lane's black text would sit on the default background).
fn accent_color((red, green, blue): (u8, u8, u8), is_truecolor: bool) -> Color {
    if is_truecolor {
        return Color::Rgb(red, green, blue);
    }
    match (red, green, blue) {
        (0xff, 0x44, 0x44) => Color::LightRed,
        (0xff, 0xeb, 0x3b) => Color::Yellow,
        (0x21, 0x96, 0xf3) => Color::LightBlue,
        (0x4c, 0xaf, 0x50) => Color::Green,
        (0xff, 0xa5, 0x00) => Color::Indexed(214),
        _ => Color::White,
    }
}

fn draw_lane(frame: &mut Frame, area: Rect, lane: &LaneView, show_active_marker: bool, accent: Color) {
    let marker = if show_active_marker && lane.is_active { "▶ " } else { "  " };
    let badge = if lane.is_recording { "● REC" } else { "◀ ▶" };
    let style = if lane.is_recording {
        Style::new().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(accent)
    };
    let block = Block::bordered()
        .title(format!(" PERSON {} ", lane.label))
        .border_style(Style::new().fg(accent));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [name_area, badge_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(6)]).areas(inner);
    frame.render_widget(
        Paragraph::new(format!("{marker}{}", lane.language)).style(style.add_modifier(Modifier::BOLD)),
        name_area,
    );
    frame.render_widget(
        Paragraph::new(badge).style(style).alignment(Alignment::Right),
        badge_area,
    );
}

/// The newest `width` levels as bar glyphs.
fn level_bars(levels: &[f32], width: u16) -> String {
    let skip = levels.len().saturating_sub(usize::from(width));
    levels[skip..]
        .iter()
        .map(|level| {
            let index = (level.clamp(0.0, 1.0) * (BAR_GLYPHS.len() - 1) as f32).round() as usize;
            BAR_GLYPHS[index]
        })
        .collect()
}

fn draw_transcript(frame: &mut Frame, area: Rect, view: &AppView, accent: Color) {
    let [source_area, target_area] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let border = Style::new().fg(accent);
    frame.render_widget(
        Paragraph::new(view.transcription.as_str())
            .wrap(Wrap { trim: true })
            .block(Block::bordered().title(format!(" {} ", view.source_label)).border_style(border)),
        source_area,
    );
    let alignment = if view.is_translation_right_to_left {
        Alignment::Right
    } else {
        Alignment::Left
    };
    frame.render_widget(
        Paragraph::new(view.translation.as_str())
            .wrap(Wrap { trim: true })
            .alignment(alignment)
            .block(Block::bordered().title(format!(" {} ", view.target_label)).border_style(border)),
        target_area,
    );
}

/// Phase or error, then any audio warning, then the key hints.
fn status_line(view: &AppView, accent: Color) -> Paragraph<'static> {
    let mut spans = vec![Span::styled(
        view.status.clone(),
        Style::new().fg(accent).add_modifier(Modifier::BOLD),
    )];
    if let Some(warning) = &view.audio_warning {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(warning.clone(), Style::new().add_modifier(Modifier::BOLD)));
    }
    spans.push(Span::raw(" · "));
    spans.push(Span::raw(status_hint(view)));
    Paragraph::new(Line::from(spans))
}

/// Key hints for the current mode and phase; the idle landscape line fits 80
/// columns together with the status word.
fn status_hint(view: &AppView) -> &'static str {
    if view.is_settings_open {
        return "↑/↓ choose  ←/→ change  Esc close";
    }
    match (view.phase, view.keyboard_mode) {
        (Phase::Recording(Lane::Two), KeyboardMode::Vertical) => "X again to stop",
        (Phase::Recording(_), _) => "Z again to stop",
        (_, KeyboardMode::Landscape) => {
            "Z tap to record  Space lane  ←/→ language  Esc settings  Ctrl-C quit"
        }
        (_, KeyboardMode::Vertical) => {
            "Z/X tap to record  ←/→ −/+ language  Esc settings  Ctrl-C quit"
        }
    }
}

fn draw_settings(frame: &mut Frame, area: Rect, rows: &[SettingsRowView], accent: Color) {
    let height = rows.len() as u16 + 3;
    let [popup] = Layout::horizontal([Constraint::Length(SETTINGS_WIDTH)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(popup);
    let lines: Vec<Line> = rows
        .iter()
        .map(|row| {
            let text = format!(" {:<16}{:>16} ", row.label, row.value);
            if row.is_selected {
                Line::styled(text, Style::new().fg(Color::Black).bg(accent))
            } else {
                Line::styled(text, Style::new().fg(accent))
            }
        })
        .chain([Line::raw(" ↑/↓ choose  ←/→ change  Esc close")])
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" SETTINGS ")
                .border_style(Style::new().fg(accent)),
        ),
        popup,
    );
}
```

- **Narrow terminals:** at 40 columns the status line is cut off on the right. The test at that size checks only the status word.
- **Text order:** the renderer never reorders text. TestBackend keeps characters in logical order, so `مرحبا` is contiguous in the row string.

- [ ] **Step 4: Run the tests**

Run: `cargo fmt --all && cargo test -p translator-tui && cargo fmt --all -- --check`
Expected: 16 tests pass (6 key tests + 10 render tests).

- [ ] **Step 5: Commit**

```bash
git add translator-tui
git commit -m "Draw the TUI: lanes, levels, transcript, status and settings"
```

---

### Task 5: Terminal handling, stderr redirect, signals, main loop, docs

**Files:**
- Create: `translator-tui/src/terminal.rs`
- Replace: `translator-tui/src/main.rs`
- Modify: `Makefile`, `README.md`, `CLAUDE.md`

**Interfaces:**
- Consumes:
  - `translator_app::{AppState, KeyAction, RecordStyle}`
  - `translator_app::cli::{parse_args, load_config}`
  - `keys::{key_from_event, is_quit, RepeatFilter, REPEAT_WINDOW}`
  - `render::draw`
- Produces:
  - `terminal::log_path(state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf>`
  - `terminal::StderrRedirect::to_file(path: &Path) -> io::Result<StderrRedirect>`
  - `StderrRedirect::saved_fd(&self) -> RawFd`
  - `terminal::install_panic_hook(saved_stderr: Option<RawFd>, log: Option<PathBuf>)`
  - `terminal::install_quit_signals()`
  - `terminal::is_quit_requested() -> bool`
  - `terminal::is_truecolor(colorterm: Option<&str>) -> bool`
  - `terminal::is_usable_term(term: Option<&str>) -> bool`
  - `terminal::Tui`
  - `terminal::enter() -> io::Result<Tui>`
  - `terminal::leave()`
  - the binary `translator-tui [--mock] [--config <path>]`
  - `make run-tui ARGS=...`

- [ ] **Step 1: Write the failing tests**

`translator-tui/src/terminal.rs`, tests only:

```rust
#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn log_path_prefers_xdg_state_home() {
        assert_eq!(
            log_path(Some(OsString::from("/state")), Some(OsString::from("/home/u"))),
            Some(PathBuf::from("/state/gemma-translator/translator-tui.log"))
        );
    }

    #[test]
    fn log_path_falls_back_to_home_local_state() {
        let expected = Some(PathBuf::from(
            "/home/u/.local/state/gemma-translator/translator-tui.log",
        ));
        assert_eq!(log_path(None, Some(OsString::from("/home/u"))), expected);
        assert_eq!(log_path(Some(OsString::new()), Some(OsString::from("/home/u"))), expected);
        assert_eq!(log_path(None, None), None);
    }

    /// One test drives both checks, so no other test in this binary can see
    /// fd 2 redirected.
    #[test]
    fn stderr_redirect_sends_fd_2_to_the_file_and_restores_it_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("translator-tui.log");
        let message = b"from C\n";
        {
            let redirect = StderrRedirect::to_file(&path).unwrap();
            assert!(redirect.saved_fd() > 2);
            // Write straight to fd 2, as a C library would.
            // SAFETY: valid buffer and length.
            let written = unsafe { libc::write(2, message.as_ptr().cast(), message.len()) };
            assert_eq!(written, message.len() as isize);
        }
        assert_eq!(std::fs::read(&path).unwrap(), message);
        // Restored: this goes to the real stderr, not the file.
        let after = b"after restore\n";
        // SAFETY: valid buffer and length.
        unsafe { libc::write(2, after.as_ptr().cast(), after.len()) };
        assert_eq!(std::fs::read(&path).unwrap(), message);
    }

    #[test]
    fn stderr_redirect_to_an_unwritable_path_is_an_error() {
        assert!(StderrRedirect::to_file(Path::new("/proc/no/such/dir/log")).is_err());
    }

    #[test]
    fn sigterm_requests_a_clean_quit() {
        install_quit_signals();
        assert!(!is_quit_requested());
        // SAFETY: the handler installed above only sets an atomic flag.
        unsafe { libc::raise(libc::SIGTERM) };
        assert!(is_quit_requested());
    }

    #[test]
    fn truecolor_comes_from_colorterm() {
        assert!(is_truecolor(Some("truecolor")));
        assert!(is_truecolor(Some("24bit")));
        assert!(!is_truecolor(Some("")));
        assert!(!is_truecolor(None));
    }

    #[test]
    fn dumb_or_missing_term_is_unusable() {
        assert!(is_usable_term(Some("xterm-256color")));
        assert!(!is_usable_term(Some("dumb")));
        assert!(!is_usable_term(Some("")));
        assert!(!is_usable_term(None));
    }
}
```

Review Focus items 1 and 3 both point at the stderr test, which keeps fd 2 redirected inside a single test only. The test writes "after restore" to the real stderr; that is expected test noise and happens once.

Add `mod terminal;` to `main.rs`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p translator-tui terminal`
Expected: compile errors, because the `terminal` functions are missing.

- [ ] **Step 3: Implement `terminal.rs`**

Prepend to `translator-tui/src/terminal.rs`:

```rust
//! Keeping the terminal usable: raw mode and the alternate screen, fd 2 sent
//! to a log file while the TUI owns the screen (native libraries write there
//! directly), clean exits on SIGTERM/SIGHUP, and a main-thread panic hook.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Stdout};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

static IS_QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Raw mode and the alternate screen. Unlike `ratatui::init`, this installs
/// no panic hook (see `install_panic_hook`) and reports failure instead of
/// panicking.
pub fn enter() -> io::Result<Tui> {
    enable_raw_mode()?;
    if let Err(error) = execute!(io::stdout(), EnterAlternateScreen) {
        leave();
        return Err(error);
    }
    Terminal::new(CrosstermBackend::new(io::stdout())).inspect_err(|_| leave())
}

/// Cooked mode, main screen, visible cursor. Errors are ignored: after an
/// SSH drop the terminal may already be gone.
pub fn leave() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// `$XDG_STATE_HOME/gemma-translator/translator-tui.log`, else
/// `$HOME/.local/state/...`; `None` when neither is set.
pub fn log_path(state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = state_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local").join("state")))?;
    Some(base.join("gemma-translator").join("translator-tui.log"))
}

/// While alive, fd 2 writes to the log file; dropping it puts the original
/// stderr back.
pub struct StderrRedirect {
    saved: RawFd,
}

impl StderrRedirect {
    pub fn to_file(path: &Path) -> io::Result<StderrRedirect> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        // SAFETY: duplicating the process's own stderr descriptor.
        let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both descriptors are open; fd 2 now refers to the file,
        // which stays open after `file` is dropped because fd 2 holds it.
        if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
            let error = io::Error::last_os_error();
            // SAFETY: `saved` was returned by dup above and is not used again.
            unsafe { libc::close(saved) };
            return Err(error);
        }
        Ok(StderrRedirect { saved })
    }

    pub fn saved_fd(&self) -> RawFd {
        self.saved
    }
}

impl Drop for StderrRedirect {
    fn drop(&mut self) {
        // SAFETY: `saved` is the duplicate made in `to_file`; it is closed once.
        unsafe {
            libc::dup2(self.saved, libc::STDERR_FILENO);
            libc::close(self.saved);
        }
    }
}

/// Must be called on the main thread. A panic there restores the terminal
/// and stderr, then prints the message and the log path. Panics on other
/// threads (JobRunner catches job panics and the TUI keeps running) only go
/// to the default hook, which writes to the log file.
pub fn install_panic_hook(saved_stderr: Option<RawFd>, log: Option<PathBuf>) {
    let main_thread = thread::current().id();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_main_thread = thread::current().id() == main_thread;
        if is_main_thread {
            leave();
            if let Some(fd) = saved_stderr {
                // SAFETY: `fd` is the StderrRedirect's duplicate; if it was
                // already closed, dup2 fails harmlessly with EBADF.
                unsafe { libc::dup2(fd, libc::STDERR_FILENO) };
            }
        }
        default_hook(info);
        if is_main_thread {
            if let Some(log) = &log {
                eprintln!("log: {}", log.display());
            }
        }
    }));
}

extern "C" fn request_quit(_signal: libc::c_int) {
    IS_QUIT_REQUESTED.store(true, Ordering::SeqCst);
}

/// SIGTERM (`kill`) and SIGHUP (the SSH session dropped) end the main loop
/// like Ctrl-C, so the terminal is restored.
pub fn install_quit_signals() {
    let handler = request_quit as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic (async-signal-safe).
        unsafe { libc::signal(signal, handler) };
    }
}

pub fn is_quit_requested() -> bool {
    IS_QUIT_REQUESTED.load(Ordering::SeqCst)
}

/// `COLORTERM=truecolor` or `24bit`.
pub fn is_truecolor(colorterm: Option<&str>) -> bool {
    matches!(colorterm, Some("truecolor" | "24bit"))
}

/// A terminal type the TUI can draw on: set, and not `dumb`.
pub fn is_usable_term(term: Option<&str>) -> bool {
    matches!(term, Some(term) if !term.is_empty() && term != "dumb")
}
```

- [ ] **Step 4: Replace `main.rs`**

```rust
//! `translator-tui`: the Gemma Translator in a terminal (desktop or SSH).
//! Recording toggles: tap Z to start, tap it again to stop.

mod keys;
mod render;
mod terminal;

use std::io::{self, IsTerminal};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event};
use translator_app::cli::{load_config, parse_args};
use translator_app::{AppState, KeyAction, RecordStyle};
use translator_core::config::Config;

use crate::keys::{is_quit, key_from_event, RepeatFilter, REPEAT_WINDOW};
use crate::terminal::{
    enter, install_panic_hook, install_quit_signals, is_quit_requested, is_truecolor,
    is_usable_term, leave, log_path, StderrRedirect, Tui,
};

const USAGE: &str = "usage: translator-tui [--mock] [--config <path>]";
/// Job events, visualizer and drawing are updated at ~33 fps.
const FRAME_INTERVAL: Duration = Duration::from_millis(30);

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let term = std::env::var("TERM").ok();
    if !io::stdout().is_terminal() || !is_usable_term(term.as_deref()) {
        eprintln!("translator-tui needs a terminal (stdout is not one, or TERM is unset or dumb)");
        return ExitCode::from(2);
    }
    let (config, config_path) = match options.config_path.or_else(Config::default_path) {
        Some(path) => load_config(path),
        None => (Config::default(), None),
    };

    let log = log_path(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"));
    let redirect = match &log {
        Some(path) => match StderrRedirect::to_file(path) {
            Ok(redirect) => Some(redirect),
            Err(error) => {
                eprintln!(
                    "warning: cannot log to {} ({error}); library output may garble the screen",
                    path.display()
                );
                None
            }
        },
        None => {
            eprintln!("warning: no XDG_STATE_HOME or HOME; library output may garble the screen");
            None
        }
    };
    install_panic_hook(redirect.as_ref().map(StderrRedirect::saved_fd), log.clone());
    install_quit_signals();
    let is_truecolor = is_truecolor(std::env::var("COLORTERM").ok().as_deref());

    let mut state = AppState::new(config, config_path, options.is_mock, RecordStyle::Toggle);
    let mut terminal = match enter() {
        Ok(terminal) => terminal,
        Err(error) => {
            drop(redirect);
            eprintln!("translator-tui: cannot set up the terminal: {error}");
            return ExitCode::from(2);
        }
    };
    let result = run(&mut terminal, &mut state, is_truecolor);
    leave();
    drop(terminal);
    drop(redirect);

    if let Some(path) = &log {
        println!("log: {}", path.display());
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("translator-tui: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(terminal: &mut Tui, state: &mut AppState, is_truecolor: bool) -> io::Result<()> {
    let mut filter = RepeatFilter::new(REPEAT_WINDOW);
    while !is_quit_requested() {
        let view = state.view();
        terminal.draw(|frame| render::draw(frame, &view, is_truecolor))?;
        let has_event = match event::poll(FRAME_INTERVAL) {
            Ok(has_event) => has_event,
            // A quit signal interrupts the wait; the loop condition sees it.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => false,
            Err(error) => return Err(error),
        };
        if has_event {
            if let Event::Key(key_event) = event::read()? {
                if is_quit(&key_event) {
                    return Ok(());
                }
                if let Some(key) = key_from_event(&key_event) {
                    if filter.accept(key, key_event.kind, Instant::now()) {
                        state.handle_key(key, KeyAction::Pressed);
                    }
                }
            }
        }
        state.tick();
    }
    Ok(())
}
```

Notes:
- **`poll` returns early on a key**, so the 30 ms frame interval is an upper bound. `tick` is cheap and safe to call more often.
- **Resize events** need no handling. The next `terminal.draw` uses the new size automatically.
- **Ctrl-Z** is not a translator key (Task 3) and the TUI does not suspend; raw mode turns off the terminal's own suspend.

- [ ] **Step 5: Makefile and docs**

Add to `Makefile` after `run-mock`:

```make
## run-tui: run the terminal UI (make run-tui ARGS=--mock for mock services)
.PHONY: run-tui
run-tui:
	$(CARGO) run -p translator-tui -- $(ARGS)
```

`README.md`: add this subsection under "Rust UI (experimental)":

````markdown
### Terminal UI

`translator-tui` is the same translator in a terminal, for a desktop terminal
or an SSH session (audio uses the machine it runs on):

```bash
make run-tui ARGS=--mock   # mock services
make run-tui               # uses inference_backend from the config
```

Terminals do not report key releases, so recording toggles: tap `Z` (or `Z`/`X`
in vertical mode) to start and tap again to stop. Holding the key, or tapping
it again within 0.7 s, counts as one tap. `Esc` opens settings and `Ctrl-C`
quits; Ctrl-Z does not suspend. A missing microphone or speaker is shown on
the status line. Native-library output goes to
`~/.local/state/gemma-translator/translator-tui.log`. Arabic is shown in
logical order; right-to-left display depends on the terminal (Konsole and
mlterm handle it). Without `COLORTERM=truecolor` the theme colour is
approximated.
````

`CLAUDE.md`:
- In the "Rust UI" section, change "`translator-ui/src/app.rs` executes them" to "`translator-app/src/state.rs` executes them".
- Add `translator-app` and `translator-tui` to the workspace list sentence, so it names eight crates.
- Add these bullets:

```markdown
- `translator-app`: UI-independent `AppState` (audio, job runner, volume, config saving), the `AppView` view model, mock services and CLI parsing, shared by both frontends. Frontends call `handle_key`, `tick` and `view`; `RecordStyle::Hold` (Slint) vs `Toggle` (TUI). `AppState::headless` runs without audio devices or volume commands, for tests.
- `translator-tui`: ratatui/crossterm terminal frontend (`make run-tui`). Filters auto-repeat on the recording keys (700 ms window), ignores Ctrl/Alt combinations, exits cleanly on SIGTERM/SIGHUP, and redirects fd 2 to `~/.local/state/gemma-translator/translator-tui.log` while running; only a main-thread panic restores the terminal.
```

- Add `make run-tui` to the Commands block.

- [ ] **Step 6: Verify**

Run: `cargo fmt --all && make test && make clippy && cargo fmt --all -- --check`
Expected:
- Everything passes. The translator-tui tests are 6 + 10 + 7 = 23.
- Clippy is clean for the whole workspace now that `main.rs` uses `keys`, `render` and `terminal`.

Smoke tests without a usable terminal:
- `cargo run -p translator-tui -- --mock < /dev/null | cat` exits with code 2 and prints `translator-tui needs a terminal …`.
- `TERM=dumb cargo run -p translator-tui -- --mock` does the same, even from a real terminal.

- [ ] **Step 7: Commit**

```bash
git add translator-tui Makefile README.md CLAUDE.md
git commit -m "Run the TUI: terminal setup, stderr log redirect, quit signals and the main loop"
```

---

## Manual checks (for the user; not run by agents)

1. Run `make run-tui ARGS=--mock` in a local terminal. Tap Z, wait, tap Z again. Check that:
   - the mock transcript and translation appear;
   - holding Z for 3 s neither stops nor restarts the recording;
   - tapping ← quickly several times rotates the language each time.
2. Over SSH, run `translator-tui` with `inference_backend = "native"` and translate English to Chinese. Check that:
   - the Chinese characters line up;
   - the log file contains litert-lm's lines;
   - the screen has none of them.
   - On a host without a microphone, the status line shows `no microphone: recording silence`.
3. Resize the terminal below 40×12 and back. Check that the hint appears and the layout returns.
4. Check each way of quitting:
   - **Ctrl-C:** the shell echoes typed characters, the cursor is visible, and `log: …` is printed.
   - **`kill <pid>` from another shell:** the same.
   - **Closing the SSH window:** no stray `translator-tui` process is left on the host.
5. Check the panic paths:
   - Temporarily add `panic!()` in the main loop. The panic message and `log: …` should appear on a normal screen.
   - Temporarily add `panic!()` inside the mock translator. The TUI keeps running, shows the job as failed, and the panic message is in the log file.
   - Remove both `panic!()` lines afterwards.
6. Under `TERM=xterm` without `COLORTERM`, check that the recording lane's text stays readable.
