use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use moonshine_sys::MoonshineLibrary;
use translator_core::language::LanguageCode;
use translator_core::models::{ModelPaths, TtsSettings};
use translator_core::pipeline::{SpeechToText, TextToSpeech, Translator, STT_SAMPLE_RATE};
use translator_io::audio_convert::resample_linear;
use translator_io::native::speech::NativeSpeech;
use translator_io::native::translator::NativeTranslator;
use translator_io::speech_trim::TrimMode;

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")),
    )
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn synthesized_speech_round_trips_through_the_speech_thread() {
    let speech = NativeSpeech::spawn(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR")),
        vec![LanguageCode::En, LanguageCode::Zh],
    );
    let clip = speech
        .synthesize("Good morning, how are you today?", LanguageCode::En)
        .unwrap();
    let samples = resample_linear(&clip.samples, clip.sample_rate, STT_SAMPLE_RATE);
    let text = speech.transcribe(&samples, LanguageCode::En).unwrap();
    assert!(
        text.to_lowercase().contains("morning"),
        "transcript: {text:?}"
    );
    let chinese = speech.synthesize("早上好。", LanguageCode::Zh).unwrap();
    assert!(!chinese.samples.is_empty());
}

fn speech_with(tts: TtsSettings) -> NativeSpeech {
    NativeSpeech::spawn_with(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR")),
        Vec::new(),
        TrimMode::Adaptive,
        tts,
    )
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn a_faster_speed_gives_shorter_speech() {
    let text = "Good morning, how are you today?";
    let normal = speech_with(TtsSettings::default())
        .synthesize(text, LanguageCode::En)
        .unwrap();
    let fast = speech_with(TtsSettings {
        speed: 1.5,
        ..TtsSettings::default()
    })
    .synthesize(text, LanguageCode::En)
    .unwrap();
    let ratio = fast.samples.len() as f64 / normal.samples.len() as f64;
    assert!(ratio < 0.85, "fast/normal length {ratio}");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn an_unusable_voice_falls_back_to_the_built_in_one() {
    let library = MoonshineLibrary::load(&env_dir("TRANSLATOR_NATIVE_LIB_DIR")).unwrap();
    let tts_root = ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR")).tts_root();
    let not_downloaded = library
        .tts_voices("zh_hans", &[("g2p_root", tts_root.to_str().unwrap())])
        .unwrap()
        .into_iter()
        .find(|voice| !voice.found && voice.id.starts_with("kokoro_"))
        .expect("a zh voice that is not downloaded")
        .id;
    let text = "早上好。";
    let built_in = speech_with(TtsSettings::default())
        .synthesize(text, LanguageCode::Zh)
        .unwrap();
    // Another language's voice, then one without files.
    for id in ["kokoro_af_heart", not_downloaded.as_str()] {
        let settings = TtsSettings {
            voices: BTreeMap::from([(LanguageCode::Zh, id.to_string())]),
            ..TtsSettings::default()
        };
        let clip = speech_with(settings)
            .synthesize(text, LanguageCode::Zh)
            .unwrap();
        assert_eq!(clip.samples, built_in.samples, "{id}");
    }
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn missing_model_names_the_tool() {
    let empty_models = tempfile::tempdir().unwrap();
    let speech = NativeSpeech::spawn(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(empty_models.path()),
        Vec::new(),
    );
    let error = speech
        .transcribe(&[0.0; 1600], LanguageCode::Ko)
        .unwrap_err();
    assert!(error.to_string().contains("translator-models"), "{error}");
    assert!(error.to_string().contains("tiny-ko"), "{error}");
    let error = speech.synthesize("hola", LanguageCode::Es).unwrap_err();
    assert!(error.to_string().contains("translator-models"), "{error}");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn partially_downloaded_model_names_the_missing_file() {
    let models = tempfile::tempdir().unwrap();
    let paths = ModelPaths::new(models.path());
    let directory = paths.stt_dir(LanguageCode::Ko.stt_model());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("tokenizer.bin"), b"").unwrap();
    let speech = NativeSpeech::spawn(env_dir("TRANSLATOR_NATIVE_LIB_DIR"), paths, Vec::new());
    let error = speech
        .transcribe(&[0.0; 1600], LanguageCode::Ko)
        .unwrap_err();
    assert!(error.to_string().contains("translator-models"), "{error}");
    assert!(error.to_string().contains("encoder_model.ort"), "{error}");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn translates_english_to_chinese_sentence_by_sentence() {
    let translator = NativeTranslator::spawn(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR")),
    );
    let mut sentences = Vec::new();
    translator
        .translate_streaming(
            "Good morning. How are you today?",
            LanguageCode::En,
            LanguageCode::Zh,
            &AtomicBool::new(false),
            &mut |sentence| sentences.push(sentence),
        )
        .unwrap();
    assert!(!sentences.is_empty());
    assert!(
        sentences
            .concat()
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
        "{sentences:?}"
    );
}
