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
    fn transcribe(
        &self,
        _samples: &[f32],
        _language: LanguageCode,
    ) -> Result<String, PipelineError> {
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
    AudioClip {
        samples: vec![0.1; 160],
        sample_rate: 16_000,
    }
}

fn request(id: u64, enable_tts: bool) -> JobRequest {
    JobRequest {
        id: JobId(id),
        source: LanguageCode::En,
        target: LanguageCode::Es,
        enable_tts,
    }
}

fn runner(
    stt: impl SpeechToText + 'static,
    translator: impl Translator + 'static,
    tts: impl TextToSpeech + 'static,
) -> (JobRunner, Arc<RecordingSink>, Receiver<JobEvent>) {
    let sink = Arc::new(RecordingSink::default());
    let (sender, receiver) = mpsc::channel();
    let services = Services {
        stt: Arc::new(stt),
        translator: Arc::new(translator),
        tts: Arc::new(tts),
    };
    let on_event = Arc::new(move |event| {
        let _ = sender.send(event);
    });
    (
        JobRunner::new(services, sink.clone(), on_event),
        sink,
        receiver,
    )
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
            JobEvent::Transcribed {
                job,
                text: "hello".to_string()
            },
            JobEvent::SentenceTranslated {
                job,
                sentence: "Hola.".to_string()
            },
            JobEvent::SentenceTranslated {
                job,
                sentence: "Adiós.".to_string()
            },
            JobEvent::TranslationFinished { job },
            JobEvent::PlaybackFinished { job },
        ]
    );
    assert_eq!(sink.clips.lock().unwrap().len(), 2);
}

#[test]
fn without_speech_output_nothing_is_played() {
    let (runner, sink, receiver) = runner(
        ScriptedStt(Ok("hello".to_string())),
        ScriptedTranslator(vec!["Hola."]),
        ScriptedTts(Ok(clip())),
    );
    runner.start(request(1, false), Vec::new());
    let collected = events(&receiver);
    assert_eq!(
        collected.last(),
        Some(&JobEvent::TranslationFinished { job: JobId(1) })
    );
    assert!(sink.clips.lock().unwrap().is_empty());
}

#[test]
fn empty_transcript_stops_after_transcription() {
    let (runner, _sink, receiver) = runner(
        ScriptedStt(Ok(" ".to_string())),
        ScriptedTranslator(vec!["x"]),
        ScriptedTts(Ok(clip())),
    );
    runner.start(request(1, true), Vec::new());
    assert_eq!(
        events(&receiver),
        [JobEvent::Transcribed {
            job: JobId(1),
            text: " ".to_string()
        }]
    );
}

#[test]
fn transcription_errors_are_reported() {
    let error = PipelineError::Unreachable {
        service: "speech service",
        message: "refused".to_string(),
    };
    let (runner, _sink, receiver) = runner(
        ScriptedStt(Err(error.clone())),
        ScriptedTranslator(vec![]),
        ScriptedTts(Ok(clip())),
    );
    runner.start(request(1, true), Vec::new());
    assert_eq!(
        events(&receiver),
        [JobEvent::Failed {
            job: JobId(1),
            message: error.to_string()
        }]
    );
}

#[test]
fn speech_errors_are_reported_after_the_translation() {
    let error = PipelineError::Service {
        service: "speech service",
        status: 500,
        message: "tts".to_string(),
    };
    let (runner, _sink, receiver) = runner(
        ScriptedStt(Ok("hello".to_string())),
        ScriptedTranslator(vec!["Hola."]),
        ScriptedTts(Err(error.clone())),
    );
    runner.start(request(1, true), Vec::new());
    let collected = events(&receiver);
    assert_eq!(
        collected.last(),
        Some(&JobEvent::Failed {
            job: JobId(1),
            message: error.to_string()
        })
    );
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
    assert_eq!(
        events(&receiver),
        [JobEvent::Transcribed {
            job: JobId(1),
            text: "hello".to_string()
        }]
    );
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
    assert!(collected
        .iter()
        .all(|e| !matches!(e, JobEvent::TranslationFinished { .. })));
    assert!(collected.contains(&JobEvent::Transcribed {
        job: JobId(2),
        text: "hello".to_string()
    }));
}
