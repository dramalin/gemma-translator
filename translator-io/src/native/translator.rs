use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use litert_lm_sys::{Engine, LiteRtLmLibrary, StreamEnd};
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{PipelineError, Translator};
use translator_core::text::{clean_model_reply, translation_system_prompt, SentenceSplitter};

use super::engine_error;
use super::worker::Worker;

const SERVICE: &str = "translation engine";
/// How often a waiting job thread checks its cancel flag.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Streaming translation on one thread that owns the litert-lm engine.
/// Dropping it waits for the thread to free the engine (see `Worker`).
pub struct NativeTranslator {
    worker: Worker<TranslateRequest>,
}

struct TranslateRequest {
    text: String,
    source: LanguageCode,
    target: LanguageCode,
    events: Sender<TranslationEvent>,
    /// Set by the job thread when its caller cancels; the model thread then
    /// cancels generation.
    cancel: Arc<AtomicBool>,
}

enum TranslationEvent {
    Fragment(String),
    Finished(Result<(), PipelineError>),
}

impl NativeTranslator {
    /// Starts the model thread, which loads `liblitert-lm` and Gemma
    /// before serving requests (requests sent meanwhile wait in the queue).
    pub fn spawn(lib_dir: PathBuf, paths: ModelPaths) -> NativeTranslator {
        NativeTranslator {
            worker: Worker::spawn("model", move |requests| {
                run_model_thread(&lib_dir, &paths, requests)
            }),
        }
    }
}

fn stopped() -> PipelineError {
    engine_error(SERVICE, "model thread stopped")
}

impl Translator for NativeTranslator {
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        let request_cancel = Arc::new(AtomicBool::new(false));
        let (events, receiver) = mpsc::channel();
        self.worker
            .send(TranslateRequest {
                text: text.to_string(),
                source,
                target,
                events,
                cancel: Arc::clone(&request_cancel),
            })
            .map_err(|_| stopped())?;

        let mut splitter = SentenceSplitter::new();
        let mut emit = |sentences: Vec<String>| {
            for sentence in sentences {
                let cleaned = clean_model_reply(&sentence);
                if !cleaned.is_empty() {
                    on_sentence(cleaned);
                }
            }
        };
        loop {
            if cancel.load(Ordering::SeqCst) {
                // The model thread finishes cancelling on its own.
                request_cancel.store(true, Ordering::SeqCst);
                return Ok(());
            }
            match receiver.recv_timeout(CANCEL_POLL_INTERVAL) {
                Ok(TranslationEvent::Fragment(fragment)) => emit(splitter.push(&fragment)),
                Ok(TranslationEvent::Finished(result)) => {
                    result?;
                    if !cancel.load(Ordering::SeqCst) {
                        emit(splitter.finish());
                    }
                    return Ok(());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(stopped()),
            }
        }
    }
}

fn run_model_thread(lib_dir: &Path, paths: &ModelPaths, requests: Receiver<TranslateRequest>) {
    let engine = match load_engine(lib_dir, paths) {
        Ok(engine) => engine,
        Err(error) => {
            let message = error.to_string();
            eprintln!("[model] {message}");
            for request in requests {
                let _ = request
                    .events
                    .send(TranslationEvent::Finished(Err(engine_error(
                        SERVICE, &message,
                    ))));
            }
            return;
        }
    };
    for request in requests {
        let result = if request.cancel.load(Ordering::SeqCst) {
            Ok(())
        } else {
            translate_once(&engine, &request)
        };
        let _ = request.events.send(TranslationEvent::Finished(result));
    }
}

/// The engine keeps its own reference to the library, so only the engine is returned.
fn load_engine(lib_dir: &Path, paths: &ModelPaths) -> Result<Engine, String> {
    let library = LiteRtLmLibrary::load(lib_dir).map_err(|error| error.to_string())?;
    let cache_dir = paths.llm_cache_dir();
    fs::create_dir_all(&cache_dir).map_err(|error| format!("{}: {error}", cache_dir.display()))?;
    Engine::new(&library, &paths.gemma_file(), &cache_dir).map_err(|error| error.to_string())
}

fn translate_once(engine: &Engine, request: &TranslateRequest) -> Result<(), PipelineError> {
    // A fresh conversation per translation: no context carries over.
    let mut conversation = engine
        .create_conversation(&translation_system_prompt(request.source, request.target))
        .map_err(|error| engine_error(SERVICE, error))?;
    let events = &request.events;
    conversation
        .send_message_stream(
            &request.text,
            &mut |text| {
                let _ = events.send(TranslationEvent::Fragment(text.to_string()));
            },
            &|| request.cancel.load(Ordering::SeqCst),
        )
        .map(|end| {
            if end == StreamEnd::MaxTokensReached {
                eprintln!("[model] translation stopped at the token limit");
            }
        })
        .map_err(|error| engine_error(SERVICE, error))
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    fn with_fake_worker(
        script: impl FnOnce(TranslateRequest) + Send + 'static,
    ) -> NativeTranslator {
        NativeTranslator {
            worker: Worker::spawn("test", move |requests: Receiver<TranslateRequest>| {
                if let Ok(request) = requests.recv() {
                    script(request);
                }
            }),
        }
    }

    fn translate(
        translator: &NativeTranslator,
        cancel: &AtomicBool,
    ) -> (Result<(), PipelineError>, Vec<String>) {
        let mut sentences = Vec::new();
        let result = translator.translate_streaming(
            "hello",
            LanguageCode::En,
            LanguageCode::Es,
            cancel,
            &mut |sentence| sentences.push(sentence),
        );
        (result, sentences)
    }

    #[test]
    fn fragments_become_cleaned_sentences() {
        let translator = with_fake_worker(|request| {
            assert_eq!(request.text, "hello");
            for fragment in ["\"Hola mun", "do. ¿Cómo es", "tás?\""] {
                request
                    .events
                    .send(TranslationEvent::Fragment(fragment.to_string()))
                    .unwrap();
            }
            request
                .events
                .send(TranslationEvent::Finished(Ok(())))
                .unwrap();
        });
        let (result, sentences) = translate(&translator, &AtomicBool::new(false));
        assert_eq!(result, Ok(()));
        assert_eq!(sentences, ["Hola mundo.", "¿Cómo estás?"]);
    }

    #[test]
    fn worker_error_is_returned() {
        let translator = with_fake_worker(|request| {
            let error = engine_error(SERVICE, "boom");
            request
                .events
                .send(TranslationEvent::Finished(Err(error)))
                .unwrap();
        });
        let (result, _) = translate(&translator, &AtomicBool::new(false));
        assert_eq!(result, Err(engine_error(SERVICE, "boom")));
    }

    #[test]
    fn caller_cancel_returns_promptly_and_flags_worker() {
        let (flag_sender, flag_receiver) = mpsc::channel();
        let translator = with_fake_worker(move |request| {
            request
                .events
                .send(TranslationEvent::Fragment("Uno. ".to_string()))
                .unwrap();
            // Like the real worker, keep generating until told to stop.
            while !request.cancel.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            flag_sender.send(()).unwrap();
        });
        let cancel = AtomicBool::new(false);
        let mut sentences = Vec::new();
        let started = Instant::now();
        let result = translator.translate_streaming(
            "hello",
            LanguageCode::En,
            LanguageCode::Es,
            &cancel,
            &mut |sentence| {
                sentences.push(sentence);
                cancel.store(true, Ordering::SeqCst);
            },
        );
        assert_eq!(result, Ok(()));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(sentences, ["Uno."]);
        flag_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker saw the cancel");
    }

    #[test]
    fn stopped_worker_is_an_error() {
        let translator = NativeTranslator {
            worker: Worker::spawn("test", |requests: Receiver<TranslateRequest>| {
                drop(requests)
            }),
        };
        let (result, _) = translate(&translator, &AtomicBool::new(false));
        assert!(result.unwrap_err().to_string().contains("stopped"));
    }

    #[test]
    fn missing_library_fails_translation() {
        let dir = tempfile::tempdir().unwrap();
        let translator =
            NativeTranslator::spawn(dir.path().to_path_buf(), ModelPaths::new(dir.path()));
        let (result, _) = translate(&translator, &AtomicBool::new(false));
        assert!(result
            .unwrap_err()
            .to_string()
            .contains(litert_lm_sys::LITERT_LM_LIBRARY_FILE_NAME));
    }
}
