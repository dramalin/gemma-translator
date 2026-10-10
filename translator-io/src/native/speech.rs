use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use moonshine_sys::{ModelArch, MoonshineLibrary, Synthesizer, Transcriber};
use translator_core::language::LanguageCode;
use translator_core::models::{ModelPaths, SttModelArch};
use translator_core::pipeline::{
    AudioClip, PipelineError, SpeechToText, TextToSpeech, STT_SAMPLE_RATE,
};

use super::engine_error;
use super::lru::LruCache;

const SERVICE: &str = "speech engine";
/// Same as `MAX_MODELS` in the Python service.
const MODELS_PER_KIND: usize = 2;

/// Speech recognition and synthesis on one thread that owns every Moonshine handle.
pub struct NativeSpeech {
    requests: Sender<SpeechRequest>,
}

enum SpeechRequest {
    Transcribe {
        samples: Vec<f32>,
        language: LanguageCode,
        reply: Sender<Result<String, PipelineError>>,
    },
    Synthesize {
        text: String,
        language: LanguageCode,
        reply: Sender<Result<AudioClip, PipelineError>>,
    },
}

impl NativeSpeech {
    /// Starts the speech thread. It loads `libmoonshine` from `lib_dir`,
    /// then both kinds of model for each `preload` language, then serves requests.
    pub fn spawn(lib_dir: PathBuf, paths: ModelPaths, preload: Vec<LanguageCode>) -> NativeSpeech {
        let (requests, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("speech".to_string())
            .spawn(move || run_speech_thread(&lib_dir, paths, &preload, receiver))
            .expect("spawning the speech thread");
        NativeSpeech { requests }
    }
}

fn stopped() -> PipelineError {
    engine_error(SERVICE, "speech thread stopped")
}

impl SpeechToText for NativeSpeech {
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send(SpeechRequest::Transcribe {
                samples: samples.to_vec(),
                language,
                reply,
            })
            .map_err(|_| stopped())?;
        response.recv().map_err(|_| stopped())?
    }
}

impl TextToSpeech for NativeSpeech {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send(SpeechRequest::Synthesize {
                text: text.to_string(),
                language,
                reply,
            })
            .map_err(|_| stopped())?;
        response.recv().map_err(|_| stopped())?
    }
}

fn run_speech_thread(
    lib_dir: &Path,
    paths: ModelPaths,
    preload: &[LanguageCode],
    requests: Receiver<SpeechRequest>,
) {
    let library = match MoonshineLibrary::load(lib_dir) {
        Ok(library) => library,
        Err(error) => {
            let message = error.to_string();
            eprintln!("[speech] {message}");
            for request in requests {
                match request {
                    SpeechRequest::Transcribe { reply, .. } => {
                        let _ = reply.send(Err(engine_error(SERVICE, &message)));
                    }
                    SpeechRequest::Synthesize { reply, .. } => {
                        let _ = reply.send(Err(engine_error(SERVICE, &message)));
                    }
                }
            }
            return;
        }
    };
    let mut engines = SpeechEngines {
        library,
        paths,
        transcribers: LruCache::new(MODELS_PER_KIND),
        synthesizers: LruCache::new(MODELS_PER_KIND),
    };
    for &language in preload {
        if let Err(error) = engines.transcriber(language) {
            eprintln!(
                "[speech] preloading {} recognizer: {error}",
                language.code()
            );
        }
        if let Err(error) = engines.synthesizer(language) {
            eprintln!("[speech] preloading {} voice: {error}", language.code());
        }
    }
    for request in requests {
        match request {
            SpeechRequest::Transcribe {
                samples,
                language,
                reply,
            } => {
                let result = engines.transcriber(language).and_then(|transcriber| {
                    transcriber
                        .transcribe(&samples, STT_SAMPLE_RATE)
                        .map_err(|error| engine_error(SERVICE, error))
                });
                let _ = reply.send(result);
            }
            SpeechRequest::Synthesize {
                text,
                language,
                reply,
            } => {
                let result = engines.synthesizer(language).and_then(|synthesizer| {
                    synthesizer
                        .synthesize(&text)
                        .map(|(samples, sample_rate)| AudioClip {
                            samples,
                            sample_rate,
                        })
                        .map_err(|error| engine_error(SERVICE, error))
                });
                let _ = reply.send(result);
            }
        }
    }
}

struct SpeechEngines {
    library: Arc<MoonshineLibrary>,
    paths: ModelPaths,
    transcribers: LruCache<LanguageCode, Transcriber>,
    synthesizers: LruCache<LanguageCode, Synthesizer>,
}

impl SpeechEngines {
    fn transcriber(&mut self, language: LanguageCode) -> Result<&mut Transcriber, PipelineError> {
        let model = language.stt_model();
        let directory = self.paths.stt_dir(model);
        let library = &self.library;
        self.transcribers.get_or_try_insert_with(
            language,
            || require_files(&directory, model.components),
            || {
                Transcriber::load(
                    library,
                    &directory,
                    model_arch(model.arch),
                    language.transcriber_options(),
                )
                .map_err(|error| engine_error(SERVICE, error))
            },
        )
    }

    fn synthesizer(&mut self, language: LanguageCode) -> Result<&mut Synthesizer, PipelineError> {
        let root = self.paths.tts_root();
        let library = &self.library;
        self.synthesizers.get_or_try_insert_with(
            language,
            || require_directory(&root),
            || {
                let root_text = root.to_str().ok_or_else(|| {
                    engine_error(SERVICE, format!("{} is not UTF-8", root.display()))
                })?;
                let voice = language.tts_voice();
                Synthesizer::create(
                    library,
                    voice.language_tag,
                    &voice.synthesizer_options(root_text),
                )
                // The C side resolves the voice's files, so a partial download
                // surfaces only as this failure.
                .map_err(|error| {
                    engine_error(
                        SERVICE,
                        format!("{error}; if TTS files are missing, run translator-models"),
                    )
                })
            },
        )
    }
}

/// Fails on the first of `files` missing from `directory`, so an
/// interrupted download points at translator-models.
fn require_files(directory: &Path, files: &[&str]) -> Result<(), PipelineError> {
    match files
        .iter()
        .map(|file| directory.join(file))
        .find(|path| !path.is_file())
    {
        None => Ok(()),
        Some(missing) => Err(engine_error(
            SERVICE,
            format!("{} is missing; run translator-models", missing.display()),
        )),
    }
}

fn require_directory(directory: &Path) -> Result<(), PipelineError> {
    if directory.is_dir() {
        Ok(())
    } else {
        Err(engine_error(
            SERVICE,
            format!("{} is missing; run translator-models", directory.display()),
        ))
    }
}

fn model_arch(arch: SttModelArch) -> ModelArch {
    match arch {
        SttModelArch::Tiny => ModelArch::Tiny,
        SttModelArch::Base => ModelArch::Base,
        SttModelArch::MediumStreaming => ModelArch::MediumStreaming,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_fails_every_request() {
        let dir = tempfile::tempdir().unwrap();
        let speech = NativeSpeech::spawn(
            dir.path().to_path_buf(),
            ModelPaths::new(dir.path()),
            vec![LanguageCode::En],
        );
        let error = speech
            .transcribe(&[0.0; 160], LanguageCode::En)
            .unwrap_err();
        assert!(
            error.to_string().contains(moonshine_sys::LIBRARY_FILE_NAME),
            "{error}"
        );
        let error = speech.synthesize("hi", LanguageCode::En).unwrap_err();
        assert!(
            error.to_string().contains(moonshine_sys::LIBRARY_FILE_NAME),
            "{error}"
        );
    }
}
