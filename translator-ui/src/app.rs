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
use translator_io::volume::{SystemCommandRunner, VolumeWorker};

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
                let duration = started_at
                    .take()
                    .map(|start| start.elapsed())
                    .unwrap_or_default();
                let count = (duration.as_secs_f32() * STT_SAMPLE_RATE as f32) as usize;
                CapturedAudio {
                    samples: vec![0.0; count],
                    duration,
                }
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
    volume: VolumeWorker,
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
        let volume = VolumeWorker::spawn(SystemCommandRunner);
        AppState {
            controller: Controller::new(config),
            settings_cursor: SettingsCursor::default(),
            capture,
            playback,
            _player: player,
            runner,
            job_events,
            volume,
            volume_percent: None,
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
        let Some(command) = command_for_key_release(mode, key, self.controller.active_lane())
        else {
            return false;
        };
        let effects = self.controller.handle_command(command);
        self.execute(effects);
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
        self.settings_model.set_vec(settings_rows(
            config,
            &self.settings_cursor,
            self.volume_percent,
        ));
    }

    fn execute(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::PlayBlip(blip) => self
                    .playback
                    .enqueue(synthesize_blip(blip, BLIP_SAMPLE_RATE)),
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
