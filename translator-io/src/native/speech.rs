use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use moonshine_sys::{DependencyFile, ModelArch, MoonshineLibrary, Synthesizer, Transcriber};
use onnxruntime_sys::OrtLibrary;
use translator_core::language::LanguageCode;
use translator_core::models::{ModelPaths, SttModelArch, TtsSettings, TtsVoice};
use translator_core::pipeline::{
    AudioClip, PipelineError, SpeechToText, TextToSpeech, STT_SAMPLE_RATE,
};
use translator_core::text::clean_transcript;

use super::engine_error;
use super::lru::LruCache;
use super::vad::SileroVad;
use crate::speech_trim::{trim_speech, TrimMode};

const SERVICE: &str = "speech engine";
/// Models kept loaded per kind (recognizer, voice); each is large.
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

impl SpeechRequest {
    /// Answers the request with `error`.
    fn fail(self, error: PipelineError) {
        match self {
            SpeechRequest::Transcribe { reply, .. } => {
                let _ = reply.send(Err(error));
            }
            SpeechRequest::Synthesize { reply, .. } => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

impl NativeSpeech {
    /// Starts the speech thread. It loads `libmoonshine` from `lib_dir`,
    /// then both kinds of model for each `preload` language, then serves
    /// requests. Recordings are trimmed with `TrimMode::Adaptive`, and every
    /// language speaks with its built-in voice.
    pub fn spawn(lib_dir: PathBuf, paths: ModelPaths, preload: Vec<LanguageCode>) -> NativeSpeech {
        NativeSpeech::spawn_with(
            lib_dir,
            paths,
            preload,
            TrimMode::Adaptive,
            TtsSettings::default(),
        )
    }

    /// `spawn` with the config's voices and speaking rate, or with another
    /// way of trimming recordings (for evaluation).
    pub fn spawn_with(
        lib_dir: PathBuf,
        paths: ModelPaths,
        preload: Vec<LanguageCode>,
        trim: TrimMode,
        tts: TtsSettings,
    ) -> NativeSpeech {
        let (requests, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("speech".to_string())
            .spawn(move || run_speech_thread(&lib_dir, paths, &preload, trim, tts, receiver))
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
    trim: TrimMode,
    tts: TtsSettings,
    requests: Receiver<SpeechRequest>,
) {
    let library = match MoonshineLibrary::load(lib_dir) {
        Ok(library) => library,
        Err(error) => {
            let message = error.to_string();
            eprintln!("[speech] {message}");
            for request in requests {
                request.fail(engine_error(SERVICE, &message));
            }
            return;
        }
    };
    let vad = trim
        .uses_vad()
        .then(|| load_vad(lib_dir, &paths.vad_file()))
        .flatten();
    let mut engines = SpeechEngines {
        library,
        paths,
        transcribers: LruCache::new(MODELS_PER_KIND),
        synthesizers: LruCache::new(MODELS_PER_KIND),
        trim,
        vad,
        tts,
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
                let samples = engines.trim(samples);
                let result = engines.transcriber(language).and_then(|transcriber| {
                    transcriber
                        .transcribe(&samples, STT_SAMPLE_RATE)
                        .map(|text| clean_transcript(&text, language))
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

/// The voice activity detector, or `None` (logged) when the library or the
/// model is missing: trimming then goes by level alone.
fn load_vad(lib_dir: &Path, model: &Path) -> Option<SileroVad> {
    if !model.is_file() {
        eprintln!(
            "[speech] {} missing: trimming recordings by level only; run translator-models",
            model.display()
        );
        return None;
    }
    OrtLibrary::load(lib_dir)
        .and_then(|library| SileroVad::load(&library, model))
        .map_err(|error| {
            eprintln!("[speech] voice activity detector: {error}; trimming by level only")
        })
        .ok()
}

struct SpeechEngines {
    library: Arc<MoonshineLibrary>,
    paths: ModelPaths,
    transcribers: LruCache<LanguageCode, Transcriber>,
    synthesizers: LruCache<LanguageCode, Synthesizer>,
    trim: TrimMode,
    vad: Option<SileroVad>,
    tts: TtsSettings,
}

impl SpeechEngines {
    /// Cuts the silence around the speech in a 16 kHz recording.
    fn trim(&mut self, samples: Vec<f32>) -> Vec<f32> {
        let probabilities = match &mut self.vad {
            Some(vad) if self.trim.uses_vad() => match vad.probabilities(&samples) {
                Ok(probabilities) => Some(probabilities),
                Err(error) => {
                    eprintln!("[speech] voice activity detector: {error}");
                    None
                }
            },
            _ => None,
        };
        trim_speech(
            samples,
            STT_SAMPLE_RATE,
            self.trim,
            probabilities.as_deref(),
        )
    }

    fn transcriber(&mut self, language: LanguageCode) -> Result<&mut Transcriber, PipelineError> {
        let model = language.stt_model();
        let directory = self.paths.stt_dir(model);
        let library = &self.library;
        self.transcribers.get_or_try_insert_with(
            language,
            || {
                let files = library
                    .stt_dependencies(language.code(), model_arch(model.arch))
                    .map_err(|error| engine_error(SERVICE, error))?;
                require_files(&directory, &files)
            },
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
        let tts = &self.tts;
        // Chosen by the check, used by the create.
        let chosen = Cell::new(None);
        self.synthesizers.get_or_try_insert_with(
            language,
            || {
                require_directory(&root)?;
                chosen.set(Some(usable_voice(library, &root, tts, language)?));
                Ok(())
            },
            || {
                let voice = chosen.take().expect("the check chose a voice");
                let options = voice.synthesizer_options(tts_root_text(&root)?);
                Synthesizer::create(library, voice.language_tag, &borrowed(&options))
                    .map_err(|error| engine_error(SERVICE, error))
            },
        )
    }
}

/// Fails on the first of `files` missing from `directory` or of another
/// size than the manifest says, so an interrupted download or a model left
/// from an older version points at translator-models.
fn require_files(directory: &Path, files: &[DependencyFile]) -> Result<(), PipelineError> {
    for file in files {
        let path = directory.join(&file.name);
        let problem = match (std::fs::metadata(&path), file.size) {
            (Err(_), _) => "is missing",
            (Ok(metadata), Some(size)) if metadata.len() != size => "is from another model version",
            (Ok(_), _) => continue,
        };
        return Err(engine_error(
            SERVICE,
            format!("{} {problem}; run translator-models", path.display()),
        ));
    }
    Ok(())
}

/// The configured voice for `language`, or (logged) its built-in voice when
/// the configured one is unknown or not downloaded: a synthesis error would
/// end the whole job, translation included.
fn usable_voice(
    library: &MoonshineLibrary,
    root: &Path,
    tts: &TtsSettings,
    language: LanguageCode,
) -> Result<TtsVoice, PipelineError> {
    let configured = tts.voice(language);
    let built_in = TtsVoice {
        speed: configured.speed,
        ..language.tts_voice()
    };
    if configured == built_in {
        return require_voice(library, root, &built_in).map(|()| built_in);
    }
    match require_voice(library, root, &configured) {
        Ok(()) => Ok(configured),
        Err(error) => {
            eprintln!(
                "[speech] {error}; {} speaks with its built-in voice",
                language.code()
            );
            require_voice(library, root, &built_in).map(|()| built_in)
        }
    }
}

/// Fails when `voice` is not one of its language's voices or its files are
/// missing: Moonshine would otherwise fall back to a voice of its choosing,
/// or fail without naming the file.
fn require_voice(
    library: &MoonshineLibrary,
    root: &Path,
    voice: &TtsVoice,
) -> Result<(), PipelineError> {
    let root_text = tts_root_text(root)?;
    if let Some(id) = &voice.voice {
        let known = library
            .tts_voices(voice.language_tag, &[("g2p_root", root_text)])
            .map_err(|error| engine_error(SERVICE, error))?;
        if !known.iter().any(|entry| entry.id == *id) {
            return Err(engine_error(
                SERVICE,
                format!(
                    "{id} is not a {} voice; translator-models lists them",
                    voice.language_tag
                ),
            ));
        }
    }
    let options = voice.synthesizer_options(root_text);
    let files = library
        .tts_dependencies(voice.language_tag, &borrowed(&options))
        .map_err(|error| engine_error(SERVICE, error))?;
    require_files(root, &files)
}

fn tts_root_text(root: &Path) -> Result<&str, PipelineError> {
    root.to_str()
        .ok_or_else(|| engine_error(SERVICE, format!("{} is not UTF-8", root.display())))
}

fn borrowed<'a>(options: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    options
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect()
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
        SttModelArch::TinyStreaming => ModelArch::TinyStreaming,
        SttModelArch::SmallStreaming => ModelArch::SmallStreaming,
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
