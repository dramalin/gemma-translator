//! Whole job (speech → text → translation → speech) through JobRunner with
//! the in-process services, as the UI runs it with inference_backend = "native".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use translator_core::controller::{JobEvent, JobId, JobRequest};
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{AudioClip, TextToSpeech, STT_SAMPLE_RATE};
use translator_io::audio_convert::resample_linear;
use translator_io::native::speech::NativeSpeech;
use translator_io::native::translator::NativeTranslator;
use translator_io::playback_queue::PlaybackSink;
use translator_io::runner::{JobRunner, Services};

#[derive(Default)]
struct CollectingSink {
    clips: Mutex<Vec<AudioClip>>,
}

impl PlaybackSink for CollectingSink {
    fn enqueue(&self, clip: AudioClip) {
        self.clips.lock().unwrap().push(clip);
    }

    fn enqueue_unless_cancelled(&self, clip: AudioClip, cancel: &AtomicBool) {
        if !cancel.load(Ordering::SeqCst) {
            self.clips.lock().unwrap().push(clip);
        }
    }

    fn clear(&self) {
        self.clips.lock().unwrap().clear();
    }

    fn is_idle(&self) -> bool {
        true
    }
}

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")),
    )
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn english_speech_becomes_spoken_chinese() {
    let lib_dir = env_dir("TRANSLATOR_NATIVE_LIB_DIR");
    let paths = ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR"));
    let speech = Arc::new(NativeSpeech::spawn(
        lib_dir.clone(),
        paths.clone(),
        vec![LanguageCode::En, LanguageCode::Zh],
    ));
    let utterance = speech
        .synthesize("Good morning. How are you today?", LanguageCode::En)
        .unwrap();
    let samples = resample_linear(&utterance.samples, utterance.sample_rate, STT_SAMPLE_RATE);

    let sink = Arc::new(CollectingSink::default());
    let (sender, events) = mpsc::channel();
    let runner = JobRunner::new(
        Services {
            stt: speech.clone(),
            translator: Arc::new(NativeTranslator::spawn(lib_dir, paths)),
            tts: speech,
        },
        sink.clone(),
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    );
    runner.start(
        JobRequest {
            id: JobId(1),
            source: LanguageCode::En,
            target: LanguageCode::Zh,
            enable_tts: true,
        },
        Box::new(move || samples),
    );

    let mut transcript = String::new();
    let mut sentences = Vec::new();
    loop {
        match events
            .recv_timeout(Duration::from_secs(120))
            .expect("job finished in time")
        {
            JobEvent::Transcribed { text, .. } => transcript = text,
            JobEvent::SentenceTranslated { sentence, .. } => sentences.push(sentence),
            JobEvent::TranslationFinished { .. } => {}
            JobEvent::PlaybackFinished { .. } => break,
            JobEvent::Failed { message, .. } => panic!("job failed: {message}"),
        }
    }
    assert!(
        transcript.to_lowercase().contains("morning"),
        "{transcript:?}"
    );
    assert!(!sentences.is_empty());
    assert!(!sink.clips.lock().unwrap().is_empty(), "nothing was spoken");
}
