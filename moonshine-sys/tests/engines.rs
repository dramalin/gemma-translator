use std::path::PathBuf;
use std::sync::Arc;

use moonshine_sys::{ModelArch, MoonshineLibrary, Synthesizer, Transcriber};

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")),
    )
}

fn library() -> Arc<MoonshineLibrary> {
    MoonshineLibrary::load(&env_dir("TRANSLATOR_NATIVE_LIB_DIR")).unwrap()
}

fn tts_root() -> String {
    env_dir("TRANSLATOR_MODEL_DIR")
        .join("tts")
        .to_str()
        .unwrap()
        .to_string()
}

fn english_transcriber(library: &Arc<MoonshineLibrary>) -> Transcriber {
    let directory = env_dir("TRANSLATOR_MODEL_DIR")
        .join("stt")
        .join("medium-streaming-en");
    Transcriber::load(library, &directory, ModelArch::MediumStreaming).unwrap()
}

fn english_speech(library: &Arc<MoonshineLibrary>) -> (Vec<f32>, u32) {
    let root = tts_root();
    let mut synthesizer = Synthesizer::create(library, "en_us", &[("g2p_root", &root)]).unwrap();
    synthesizer
        .synthesize("Good morning, how are you today?")
        .unwrap()
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn synthesizes_speech_with_its_sample_rate() {
    let library = library();
    let (samples, sample_rate) = english_speech(&library);
    assert!(sample_rate >= 16_000, "sample rate {sample_rate}");
    assert!(
        samples.len() > sample_rate as usize / 2,
        "only {} samples",
        samples.len()
    );
    assert!(samples.iter().all(|sample| sample.is_finite()));
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn round_trip_speech_is_recognized() {
    let library = library();
    let (samples, sample_rate) = english_speech(&library);
    let mut transcriber = english_transcriber(&library);
    let text = transcriber.transcribe(&samples, sample_rate).unwrap();
    assert!(
        text.to_lowercase().contains("morning"),
        "transcript: {text:?}"
    );
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn empty_audio_gives_empty_text() {
    let library = library();
    let mut transcriber = english_transcriber(&library);
    assert_eq!(transcriber.transcribe(&[], 16_000).unwrap(), "");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn every_language_synthesizer_can_be_created() {
    let library = library();
    let root = tts_root();
    for (tag, voice) in [
        ("ar_msa", None),
        ("es_es", None),
        ("ja_jp", None),
        ("zh_hans", Some("kokoro_zf_xiaoxiao")),
        ("ko_kr", None),
    ] {
        let mut options = vec![("g2p_root", root.as_str())];
        if let Some(voice) = voice {
            options.push(("voice", voice));
        }
        let result = Synthesizer::create(&library, tag, &options);
        assert!(result.is_ok(), "{tag}: {:?}", result.err());
    }
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn missing_model_directory_is_an_error() {
    let library = library();
    let missing = tempfile::tempdir().unwrap().path().join("nothing-here");
    assert!(Transcriber::load(&library, &missing, ModelArch::Base).is_err());
}

/// Spec §11.1: nothing documents who frees `TranscriptC`. If the C side
/// allocated a new transcript per call and expected the caller to free it,
/// 1000 calls would leak tens of megabytes.
#[test]
#[ignore = "soak test: make test-soak"]
fn soak_repeated_transcription_does_not_leak() {
    let library = library();
    let (samples, sample_rate) = english_speech(&library);
    let mut transcriber = english_transcriber(&library);
    for _ in 0..20 {
        transcriber.transcribe(&samples, sample_rate).unwrap();
    }
    let before = resident_kib();
    for _ in 0..1000 {
        transcriber.transcribe(&samples, sample_rate).unwrap();
    }
    let growth = resident_kib().saturating_sub(before);
    assert!(
        growth < 16 * 1024,
        "RSS grew by {growth} KiB over 1000 transcriptions"
    );
}

fn resident_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}
