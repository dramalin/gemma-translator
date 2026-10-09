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

    /// Called after `Effect::StopCapture` with the length of the recording and
    /// whether the device delivered any samples at all.
    pub fn on_capture_finished(&mut self, duration: Duration, has_samples: bool) -> Vec<Effect> {
        let Phase::Transcribing(lane) = self.phase else {
            return Vec::new();
        };
        if duration < MIN_RECORDING_DURATION || !has_samples {
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
                    RotateDirection::Previous => bars
                        .saturating_sub(VISUALIZER_BARS_STEP)
                        .max(VISUALIZER_BARS_MIN),
                };
            }
            SettingsAction::AdjustVolume(direction) => {
                return vec![Effect::AdjustVolume(direction)]
            }
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
            if self.transcript.transcription == ANALYZING_TEXT {
                self.transcript.transcription = String::new();
            }
            if self.transcript.translation == TRANSLATING_TEXT {
                self.transcript.translation = String::new();
            }
        }
        self.transcript.status = String::new();
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
        match controller
            .on_capture_finished(Duration::from_secs(1), true)
            .as_slice()
        {
            [Effect::StartJob(request)] => request.clone(),
            other => panic!("expected StartJob, got {other:?}"),
        }
    }

    #[test]
    fn start_recording_on_active_lane() {
        let mut c = controller();
        let effects = c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(
            effects,
            [
                Effect::StopPlayback,
                Effect::PlayBlip(Blip::Ping),
                Effect::StartCapture
            ]
        );
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
        assert!(c
            .handle_command(Command::StartRecording(Lane::One))
            .is_empty());
        assert!(c
            .handle_command(Command::StopRecording(Lane::Two))
            .is_empty());
        assert_eq!(c.phase(), Phase::Recording(Lane::One));
    }

    #[test]
    fn recording_without_samples_is_discarded() {
        let mut c = controller();
        c.handle_command(Command::StartRecording(Lane::One));
        c.handle_command(Command::StopRecording(Lane::One));
        assert!(c
            .on_capture_finished(Duration::from_secs(2), false)
            .is_empty());
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn short_recording_is_discarded() {
        let mut c = controller();
        c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(
            c.handle_command(Command::StopRecording(Lane::One)),
            [Effect::StopCapture]
        );
        assert_eq!(c.phase(), Phase::Transcribing(Lane::One));
        assert!(c
            .on_capture_finished(Duration::from_millis(200), true)
            .is_empty());
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn long_recording_translates_into_the_other_lane_language() {
        let mut c = controller();
        let request = record(&mut c, Lane::One);
        assert_eq!(
            request,
            JobRequest {
                id: JobId(1),
                source: LanguageCode::Ar,
                target: LanguageCode::En,
                enable_tts: true
            }
        );
        assert_eq!(c.transcript().source_label, "Arabic (Source)");
        assert_eq!(c.transcript().target_label, "English (Translation)");
        assert!(!c.transcript().is_translation_right_to_left);
    }

    #[test]
    fn full_job_with_speech() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "مرحبا".to_string(),
        });
        assert_eq!(c.phase(), Phase::Translating(Lane::One));
        assert_eq!(c.transcript().transcription, "مرحبا");
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "Hi.".to_string(),
        });
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "Bye.".to_string(),
        });
        assert_eq!(c.transcript().translation, "Hi. Bye.");
        c.on_job_event(JobEvent::TranslationFinished { job });
        assert_eq!(c.phase(), Phase::Speaking(Lane::One));
        c.on_job_event(JobEvent::PlaybackFinished { job });
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn chinese_target_joins_sentences_without_spaces() {
        let mut c = Controller::new(Config {
            lane2_language: LanguageCode::Zh,
            ..Config::default()
        });
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "hello".to_string(),
        });
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "你好。".to_string(),
        });
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "再见。".to_string(),
        });
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
        let mut c = Controller::new(Config {
            enable_tts: false,
            ..Config::default()
        });
        let request = record(&mut c, Lane::One);
        assert!(!request.enable_tts);
        let job = request.id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "hi".to_string(),
        });
        c.on_job_event(JobEvent::TranslationFinished { job });
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn empty_transcription_shows_no_speech() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "  ".to_string(),
        });
        assert_eq!(c.transcript().translation, NO_SPEECH_TEXT);
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn events_from_other_jobs_are_ignored() {
        let mut c = controller();
        record(&mut c, Lane::One);
        let before = c.transcript().clone();
        assert!(c
            .on_job_event(JobEvent::Transcribed {
                job: JobId(99),
                text: "x".to_string()
            })
            .is_empty());
        assert_eq!(c.transcript(), &before);
        assert_eq!(c.phase(), Phase::Transcribing(Lane::One));
    }

    #[test]
    fn recording_during_a_job_cancels_it() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "hi".to_string(),
        });
        let effects = c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(
            effects,
            [
                Effect::CancelJob(job),
                Effect::StopPlayback,
                Effect::PlayBlip(Blip::Ping),
                Effect::StartCapture
            ]
        );
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "late".to_string(),
        });
        assert_eq!(c.transcript().translation, "");
        assert_eq!(c.phase(), Phase::Recording(Lane::One));
    }

    /// Starts a recording on `lane` and discards it as too short.
    fn record_too_short(controller: &mut Controller, lane: Lane) {
        controller.handle_command(Command::StartRecording(lane));
        controller.handle_command(Command::StopRecording(lane));
        assert!(controller
            .on_capture_finished(Duration::from_millis(200), true)
            .is_empty());
    }

    #[test]
    fn cancelling_during_translation_clears_the_translation_placeholder() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "hi".to_string(),
        });
        record_too_short(&mut c, Lane::One);
        assert_eq!(c.phase(), Phase::Idle);
        assert_eq!(c.transcript().transcription, "hi");
        assert_eq!(c.transcript().translation, "");
        assert_eq!(c.transcript().status, "");
    }

    #[test]
    fn cancelling_during_transcription_clears_both_placeholders() {
        let mut c = controller();
        record(&mut c, Lane::One);
        record_too_short(&mut c, Lane::One);
        assert_eq!(c.phase(), Phase::Idle);
        assert_eq!(c.transcript().transcription, "");
        assert_eq!(c.transcript().translation, "");
        assert_eq!(c.transcript().status, "");
    }

    #[test]
    fn cancelling_keeps_the_translation_already_received() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "hi".to_string(),
        });
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "Hello.".to_string(),
        });
        record_too_short(&mut c, Lane::One);
        assert_eq!(c.transcript().transcription, "hi");
        assert_eq!(c.transcript().translation, "Hello.");
    }

    #[test]
    fn starting_a_recording_clears_the_previous_error() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Failed {
            job,
            message: "boom".to_string(),
        });
        c.handle_command(Command::StartRecording(Lane::One));
        assert_eq!(c.transcript().status, "");
    }

    #[test]
    fn transcription_failure_is_reported() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        let effects = c.on_job_event(JobEvent::Failed {
            job,
            message: "boom".to_string(),
        });
        assert_eq!(effects, [Effect::StopPlayback]);
        assert_eq!(c.transcript().transcription, TRANSCRIPTION_FAILED_TEXT);
        assert_eq!(c.transcript().status, "Error: boom");
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn failure_after_translation_keeps_the_text() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Transcribed {
            job,
            text: "hi".to_string(),
        });
        c.on_job_event(JobEvent::SentenceTranslated {
            job,
            sentence: "Hello.".to_string(),
        });
        c.on_job_event(JobEvent::TranslationFinished { job });
        c.on_job_event(JobEvent::Failed {
            job,
            message: "tts down".to_string(),
        });
        assert_eq!(c.transcript().translation, "Hello.");
        assert_eq!(c.transcript().status, "Error: tts down");
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn a_new_job_clears_the_previous_error() {
        let mut c = controller();
        let job = record(&mut c, Lane::One).id;
        c.on_job_event(JobEvent::Failed {
            job,
            message: "boom".to_string(),
        });
        record(&mut c, Lane::One);
        assert_eq!(c.transcript().status, "");
    }

    #[test]
    fn rotation_skips_the_other_lane_and_saves() {
        let mut c = controller();
        let effects = c.handle_command(Command::RotateLanguage(Lane::One, Next));
        assert_eq!(
            effects,
            [Effect::PlayBlip(Blip::Language), Effect::SaveConfig]
        );
        assert_eq!(c.language(Lane::One), LanguageCode::Es);
        c.handle_command(Command::RotateLanguage(Lane::Two, Previous));
        assert_eq!(c.config().lane2_language, LanguageCode::Ar);
    }

    #[test]
    fn rotation_and_lane_switch_are_blocked_while_recording() {
        let mut c = controller();
        c.handle_command(Command::StartRecording(Lane::One));
        assert!(c
            .handle_command(Command::RotateLanguage(Lane::One, Next))
            .is_empty());
        assert!(c.handle_command(Command::ToggleActiveLane).is_empty());
        assert!(c.handle_command(Command::ToggleSettings).is_empty());
        assert_eq!(c.language(Lane::One), LanguageCode::Ar);
    }

    #[test]
    fn toggling_the_active_lane_blips() {
        let mut c = controller();
        assert_eq!(
            c.handle_command(Command::ToggleActiveLane),
            [Effect::PlayBlip(Blip::Speaker)]
        );
        assert_eq!(c.active_lane(), Lane::Two);
    }

    #[test]
    fn open_settings_block_other_commands() {
        let mut c = controller();
        c.handle_command(Command::ToggleSettings);
        assert!(c.is_settings_open());
        assert!(c
            .handle_command(Command::StartRecording(Lane::One))
            .is_empty());
        assert_eq!(c.phase(), Phase::Idle);
        c.handle_command(Command::ToggleSettings);
        assert!(!c.is_settings_open());
    }

    #[test]
    fn settings_actions_update_config_only_while_open() {
        let mut c = controller();
        assert!(c
            .apply_settings_action(SettingsAction::ToggleTts)
            .is_empty());
        assert!(c.config().enable_tts);
        c.handle_command(Command::ToggleSettings);
        assert_eq!(
            c.apply_settings_action(SettingsAction::ToggleKeyboardMode),
            [Effect::SaveConfig]
        );
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
