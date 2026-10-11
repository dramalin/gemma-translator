use std::path::PathBuf;

use onnxruntime_sys::OrtLibrary;
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{TextToSpeech, STT_SAMPLE_RATE};
use translator_io::audio_convert::resample_linear;
use translator_io::native::speech::NativeSpeech;
use translator_io::native::vad::SileroVad;
use translator_io::speech_trim::{trim_speech, TrimMode, VAD_CHUNK};

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")),
    )
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn detector_hears_synthesized_speech_and_not_silence() {
    let lib_dir = env_dir("TRANSLATOR_NATIVE_LIB_DIR");
    let paths = ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR"));
    let speech = NativeSpeech::spawn(lib_dir.clone(), paths.clone(), Vec::new());
    let clip = speech
        .synthesize("Good morning, how are you today?", LanguageCode::En)
        .unwrap();
    let voice = resample_linear(&clip.samples, clip.sample_rate, STT_SAMPLE_RATE);
    let second = STT_SAMPLE_RATE as usize;
    let mut samples = vec![0.0; second];
    samples.extend(&voice);
    samples.extend(vec![0.0; second]);

    let library = OrtLibrary::load(&lib_dir).unwrap();
    let mut vad = SileroVad::load(&library, &paths.vad_file()).unwrap();
    let probabilities = vad.probabilities(&samples).unwrap();
    assert_eq!(probabilities.len(), samples.len().div_ceil(VAD_CHUNK));
    let silent_chunks = second / VAD_CHUNK - 1;
    let lead = &probabilities[..silent_chunks];
    assert!(lead.iter().all(|&p| p < 0.05), "silence: {lead:?}");
    let voiced = &probabilities[second / VAD_CHUNK..(second + voice.len()) / VAD_CHUNK];
    let heard = voiced.iter().filter(|&&p| p > 0.5).count();
    assert!(heard * 2 > voiced.len(), "speech: {voiced:?}");
    // Every call starts from the same state.
    assert_eq!(vad.probabilities(&samples).unwrap(), probabilities);

    let trimmed = trim_speech(
        samples.clone(),
        STT_SAMPLE_RATE,
        TrimMode::Vad,
        Some(&probabilities),
    );
    assert!(trimmed.len() < samples.len() - second, "{}", trimmed.len());
    assert!(trimmed.len() >= voice.len() / 2, "{}", trimmed.len());
}
