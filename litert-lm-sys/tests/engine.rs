use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use litert_lm_sys::{Engine, LiteRtLmError, LiteRtLmLibrary, StreamEnd};

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")),
    )
}

fn library() -> Arc<LiteRtLmLibrary> {
    LiteRtLmLibrary::load(&env_dir("TRANSLATOR_NATIVE_LIB_DIR")).unwrap()
}

fn engine(library: &Arc<LiteRtLmLibrary>) -> Engine {
    let models = env_dir("TRANSLATOR_MODEL_DIR");
    Engine::new(
        library,
        &models.join("gemma").join("gemma-4-E2B-it.litertlm"),
        &models.join("cache"),
    )
    .unwrap()
}

#[test]
fn missing_library_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let error = LiteRtLmLibrary::load(dir.path()).err().unwrap();
    assert!(error.to_string().contains("liblitert-lm.so"), "{error}");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn missing_model_file_is_reported_before_loading() {
    let library = library();
    let dir = tempfile::tempdir().unwrap();
    let error = Engine::new(&library, &dir.path().join("none.litertlm"), dir.path())
        .err()
        .unwrap();
    assert!(matches!(error, LiteRtLmError::ModelMissing(_)), "{error}");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn streams_a_translation_in_fragments() {
    let library = library();
    let engine = engine(&library);
    let mut conversation = engine
        .create_conversation(
            "Translate the user's English text into Chinese. Output only the translation.",
        )
        .unwrap();
    let mut fragments = Vec::new();
    let end = conversation
        .send_message_stream(
            "Good morning. How are you today?",
            &mut |text| fragments.push(text.to_string()),
            &|| false,
        )
        .unwrap();
    assert_eq!(end, StreamEnd::Completed);
    assert!(
        fragments.len() > 1,
        "expected several deltas, got {fragments:?}"
    );
    let reply: String = fragments.concat();
    assert!(
        reply
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
        "no Chinese in {reply:?}"
    );
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn cancel_ends_stream_and_engine_stays_usable() {
    let library = library();
    let engine = engine(&library);
    {
        let mut conversation = engine
            .create_conversation("You are a storyteller.")
            .unwrap();
        let fragments = Cell::new(0);
        let started = Instant::now();
        let end = conversation
            .send_message_stream(
                "Tell a very long story about a dragon.",
                &mut |_| fragments.set(fragments.get() + 1),
                &|| fragments.get() >= 2,
            )
            .unwrap();
        assert_eq!(end, StreamEnd::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
    let mut conversation = engine
        .create_conversation("Reply with exactly one word.")
        .unwrap();
    let mut reply = String::new();
    let end = conversation
        .send_message_stream("Say yes.", &mut |text| reply.push_str(text), &|| false)
        .unwrap();
    assert_eq!(end, StreamEnd::Completed);
    assert!(!reply.trim().is_empty());
}
