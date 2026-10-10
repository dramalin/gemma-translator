//! Runs one translation job at a time on background threads:
//! speech-to-text → streaming translation → speech synthesis → playback.
//! Sentences are synthesized while later ones are still being translated.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use translator_core::controller::{JobEvent, JobId, JobRequest};
use translator_core::pipeline::{SpeechToText, TextToSpeech, Translator};

use crate::chinese_script::{displayed_transcript, spoken_text};
use crate::playback_queue::PlaybackSink;
use crate::sync::lock;

const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct Services {
    pub stt: Arc<dyn SpeechToText>,
    pub translator: Arc<dyn Translator>,
    pub tts: Arc<dyn TextToSpeech>,
}

/// Receives job events on the job's thread.
pub type EventSink = Arc<dyn Fn(JobEvent) + Send + Sync>;

/// Produces the job's 16 kHz mono samples; runs on the job thread so audio
/// conversion stays off the caller's (UI) thread.
pub type SampleSource = Box<dyn FnOnce() -> Vec<f32> + Send>;

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
        JobRunner {
            services,
            playback,
            on_event,
            active_job: Mutex::new(None),
        }
    }

    /// Starts a job; a job still running is cancelled first.
    pub fn start(&self, request: JobRequest, samples: SampleSource) {
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
        let cancelled = lock(&self.active_job).take_if(|(id, _)| *id == job);
        if let Some((_, cancel)) = cancelled {
            cancel.store(true, Ordering::SeqCst);
        }
        // Must follow the store above: `enqueue_unless_cancelled` relies on the
        // flag being set before the clear so no clip of this job survives it.
        self.playback.clear();
    }
}

/// Runs the job and turns a panic in a service into `JobEvent::Failed`,
/// so the UI is never left waiting for a job that died.
fn run_job(context: JobContext, request: JobRequest, samples: SampleSource) {
    let on_event = Arc::clone(&context.on_event);
    let cancel = Arc::clone(&context.cancel);
    let job = request.id;
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        run_stages(context, request, samples());
    }));
    if outcome.is_err() {
        fail(&cancel, &on_event, job, "job thread panicked".to_string());
    }
}

/// Ends the job with `Failed` unless it was already cancelled or failed.
/// The flag is set *before* the event, so once the UI reacts to `Failed`
/// (and clears playback) no further clip of this job can be queued.
fn fail(cancel: &AtomicBool, on_event: &EventSink, job: JobId, message: String) {
    if !cancel.swap(true, Ordering::SeqCst) {
        on_event(JobEvent::Failed { job, message });
    }
}

fn run_stages(context: JobContext, request: JobRequest, samples: Vec<f32>) {
    let JobContext {
        services,
        playback,
        on_event,
        cancel,
    } = context;
    let job = request.id;
    let emit = |event: JobEvent| {
        if !cancel.load(Ordering::SeqCst) {
            on_event(event);
        }
    };

    let transcript = match services.stt.transcribe(&samples, request.source) {
        Ok(text) => displayed_transcript(&text, request.source).into_owned(),
        Err(error) => {
            emit(JobEvent::Failed {
                job,
                message: error.to_string(),
            });
            return;
        }
    };
    let has_speech = !transcript.trim().is_empty();
    emit(JobEvent::Transcribed {
        job,
        text: transcript.clone(),
    });
    if !has_speech || cancel.load(Ordering::SeqCst) {
        return;
    }

    let (sentence_sender, sentence_receiver) = mpsc::channel::<String>();
    let speaker = request.enable_tts.then(|| {
        let tts = Arc::clone(&services.tts);
        let playback = Arc::clone(&playback);
        let cancel = Arc::clone(&cancel);
        let on_event = Arc::clone(&on_event);
        let target = request.target;
        // A synthesis failure ends the job at once; setting `cancel` also
        // stops the translation that is still streaming.
        thread::spawn(move || {
            for sentence in sentence_receiver {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                match tts.synthesize(&spoken_text(&sentence, target), target) {
                    Ok(clip) => playback.enqueue_unless_cancelled(clip, &cancel),
                    Err(error) => {
                        fail(&cancel, &on_event, job, error.to_string());
                        break;
                    }
                }
            }
        })
    });

    let translation = services.translator.translate_streaming(
        &transcript,
        request.source,
        request.target,
        &cancel,
        &mut |sentence: String| {
            emit(JobEvent::SentenceTranslated {
                job,
                sentence: sentence.clone(),
            });
            if request.enable_tts {
                let _ = sentence_sender.send(sentence);
            }
        },
    );
    drop(sentence_sender);
    if let Err(error) = translation {
        // The controller goes Idle on Failed: stop speaking what was already sent.
        fail(&cancel, &on_event, job, error.to_string());
        if let Some(speaker) = speaker {
            let _ = speaker.join();
        }
        return;
    }
    emit(JobEvent::TranslationFinished { job });

    let Some(speaker) = speaker else {
        return;
    };
    if speaker.join().is_err() {
        fail(
            &cancel,
            &on_event,
            job,
            "speech thread panicked".to_string(),
        );
        return;
    }
    while !playback.is_idle() && !cancel.load(Ordering::SeqCst) {
        thread::sleep(PLAYBACK_POLL_INTERVAL);
    }
    emit(JobEvent::PlaybackFinished { job });
}
