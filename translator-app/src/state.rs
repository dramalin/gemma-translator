//! UI-independent application state: turns key presses into controller
//! commands and carries out the controller's effects on the audio devices,
//! job runner and config file.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

use translator_core::config::Config;
use translator_core::controller::{Controller, Effect, JobEvent, Phase};
use translator_core::keys::{
    command_for_key_press, command_for_key_release, command_for_key_toggle, Command, Key,
    KeyboardMode,
};
use translator_core::lane::Lane;
use translator_core::models::ModelPaths;
use translator_core::pipeline::STT_SAMPLE_RATE;
use translator_core::settings::SettingsCursor;
use translator_io::audio_device::{AudioPlayer, AudioRecorder, CapturedAudio};
use translator_io::blip::{synthesize_blip, BLIP_SAMPLE_RATE};
use translator_io::native::{self, speech::NativeSpeech, translator::NativeTranslator};
use translator_io::playback_queue::{DiscardingSink, PlaybackSink};
use translator_io::runner::{JobRunner, Services};
use translator_io::volume::{CommandRunner, SystemCommandRunner, VolumeWorker};

use crate::mock;
use crate::view::{lane_view, settings_rows, status_line, update_levels, AppView};

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
                let duration = started_at
                    .take()
                    .map(|start| start.elapsed())
                    .unwrap_or_default();
                let count = (duration.as_secs_f32() * STT_SAMPLE_RATE as f32) as usize;
                CapturedAudio::new(vec![0.0; count], STT_SAMPLE_RATE, 1, duration)
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

pub struct AppState {
    controller: Controller,
    settings_cursor: SettingsCursor,
    capture: Capture,
    playback: Arc<dyn PlaybackSink>,
    _player: Option<AudioPlayer>,
    runner: JobRunner,
    job_events: Receiver<JobEvent>,
    volume: VolumeWorker,
    volume_percent: Option<u16>,
    config_path: Option<PathBuf>,
    levels: Vec<f32>,
    record_style: RecordStyle,
    audio_warning: Option<String>,
}

impl AppState {
    /// Opens the configured (or default) microphone and speaker; without them
    /// it records silence and discards audio, so jobs still run, and
    /// `AppView::audio_warning` says why.
    /// `model_dir_override` (from `--model-dir`) replaces `config.model_dir`
    /// for this run only; the saved config keeps its own value.
    pub fn new(
        config: Config,
        config_path: Option<PathBuf>,
        is_mock: bool,
        record_style: RecordStyle,
        model_dir_override: Option<PathBuf>,
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
            native_services(&config, model_dir_override.as_deref())
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
            levels: Vec::new(),
            record_style,
            audio_warning,
        }
    }

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
        self.run_command(command);
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
        self.run_command(command);
        true
    }

    /// Per-frame work: deliver job events and volume readings, and advance the visualizer.
    pub fn tick(&mut self) {
        while let Ok(event) = self.job_events.try_recv() {
            let effects = self.controller.on_job_event(event);
            self.execute(effects);
        }
        if let Some(percent) = self.volume.latest_reading() {
            self.volume_percent = percent;
        }
        let bar_count = self.controller.config().visualizer_bars as usize;
        let level = matches!(self.controller.phase(), Phase::Recording(_))
            .then(|| self.capture.level() * LEVEL_GAIN);
        update_levels(&mut self.levels, bar_count, level);
    }

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

    /// The window lost keyboard focus, so the key release that would end a
    /// recording may never arrive: end it now.
    pub fn handle_focus_lost(&mut self) {
        if let Phase::Recording(lane) = self.controller.phase() {
            self.run_command(Command::StopRecording(lane));
        }
    }

    fn run_command(&mut self, command: Command) {
        let effects = self.controller.handle_command(command);
        self.execute(effects);
    }

    fn execute(&mut self, effects: Vec<Effect>) {
        self.execute_with_capture(effects, None);
    }

    /// `captured` is the recording a `StartJob` among `effects` transcribes;
    /// without one (or with the recording discarded) the job gets no samples.
    fn execute_with_capture(&mut self, effects: Vec<Effect>, mut captured: Option<CapturedAudio>) {
        for effect in effects {
            match effect {
                Effect::PlayBlip(blip) => self
                    .playback
                    .enqueue(synthesize_blip(blip, BLIP_SAMPLE_RATE)),
                Effect::StopPlayback => self.playback.clear(),
                Effect::StartCapture => self.capture.start(),
                Effect::StopCapture => {
                    let recording = self.capture.stop();
                    let follow_up = self
                        .controller
                        .on_capture_finished(recording.duration, recording.has_samples());
                    self.execute_with_capture(follow_up, Some(recording));
                }
                Effect::CancelJob(job) => self.runner.cancel(job),
                Effect::StartJob(request) => {
                    let recording = captured.take();
                    self.runner.start(
                        request,
                        Box::new(move || {
                            recording
                                .map(CapturedAudio::into_stt_samples)
                                .unwrap_or_default()
                        }),
                    );
                }
                Effect::AdjustVolume(direction) => self.volume.adjust(direction),
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

/// In-process inference: one speech thread (preloading both lanes' languages)
/// and one model thread, both loading their libraries from `<exe dir>/lib`.
fn native_services(config: &Config, model_dir_override: Option<&Path>) -> Services {
    let lib_dir = native::default_lib_dir();
    let paths = ModelPaths::new(model_root(config, model_dir_override));
    let speech = Arc::new(NativeSpeech::spawn(
        lib_dir.clone(),
        paths.clone(),
        vec![config.lane1_language, config.lane2_language],
    ));
    Services {
        stt: speech.clone(),
        translator: Arc::new(NativeTranslator::spawn(lib_dir, paths)),
        tts: speech,
    }
}

/// The model directory: the `--model-dir` override for this run, else the config's.
fn model_root(config: &Config, model_dir_override: Option<&Path>) -> PathBuf {
    model_dir_override.map_or_else(|| config.model_dir.clone(), Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use translator_core::controller::MIN_RECORDING_DURATION;

    use super::*;

    #[test]
    fn override_reaches_only_the_model_paths() {
        let config = Config {
            model_dir: PathBuf::from("/var/lib/gemma-translator/models"),
            ..Config::default()
        };
        assert_eq!(
            model_root(&config, Some(Path::new("/repo/models"))),
            PathBuf::from("/repo/models")
        );
        assert_eq!(
            model_root(&config, None),
            PathBuf::from("/var/lib/gemma-translator/models")
        );
        // The override is a separate value: the config that gets saved is untouched.
        assert_eq!(
            config.model_dir,
            PathBuf::from("/var/lib/gemma-translator/models")
        );
    }

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
