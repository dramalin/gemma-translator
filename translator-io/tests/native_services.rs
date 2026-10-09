use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{SpeechToText, TextToSpeech, Translator, STT_SAMPLE_RATE};
use translator_io::audio_convert::resample_linear;
use translator_io::native::speech::NativeSpeech;
use translator_io::native::translator::NativeTranslator;

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
