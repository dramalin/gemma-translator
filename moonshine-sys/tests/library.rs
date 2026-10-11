use std::path::PathBuf;

use moonshine_sys::{MoonshineError, MoonshineLibrary};

fn native_lib_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("TRANSLATOR_NATIVE_LIB_DIR")
            .expect("set TRANSLATOR_NATIVE_LIB_DIR (make test-ffi does)"),
    )
}

#[test]
fn missing_library_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let error = MoonshineLibrary::load(dir.path()).err().unwrap();
    assert!(matches!(error, MoonshineError::Load { .. }));
    assert!(
        error.to_string().contains(moonshine_sys::LIBRARY_FILE_NAME),
        "{error}"
    );
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn reports_header_version_and_error_strings() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    assert_eq!(library.version(), 20000);
    assert_eq!(library.error_message(-3), "Invalid argument");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn tts_dependencies_match_the_lists_verified_with_python() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_str().unwrap();

    let korean = library
        .tts_dependencies("ko_kr", &[("g2p_root", root)])
        .unwrap();
    assert_eq!(
        korean,
        [
            "ko/dict.tsv",
            "ko/piper-voices/ko_KR-melotts-medium.onnx",
            "ko/piper-voices/ko_KR-melotts-medium.onnx.json",
        ]
    );

    let chinese = library
        .tts_dependencies(
            "zh_hans",
            &[("g2p_root", root), ("voice", "kokoro_zf_xiaoxiao")],
        )
        .unwrap();
    assert_eq!(
        chinese.last().map(String::as_str),
        Some("kokoro/voices/zf_xiaoxiao.kokorovoice")
    );
    assert!(chinese.contains(&"zh_hans/dict.tsv".to_string()));

    let english = library
        .tts_dependencies("en_us", &[("g2p_root", root)])
        .unwrap();
    assert!(english.contains(&"kokoro/voices/af_heart.kokorovoice".to_string()));
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn unknown_tts_language_is_an_error() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let error = library
        .tts_dependencies("xx_nowhere", &[("g2p_root", root.path().to_str().unwrap())])
        .unwrap_err();
    assert!(matches!(error, MoonshineError::Call { .. }), "{error}");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn option_with_nul_byte_is_rejected_before_the_call() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let error = library
        .tts_dependencies("en_us", &[("g2p_root", "bad\0path")])
        .unwrap_err();
    assert!(matches!(error, MoonshineError::InvalidArgument(_)));
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn tts_voices_list_prefixed_ids_and_what_is_on_disk() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("kokoro/voices")).unwrap();
    std::fs::write(root.path().join("kokoro/voices/zm_yunxi.kokorovoice"), b"").unwrap();
    let root = root.path().to_str().unwrap();
    let chinese = library
        .tts_voices("zh_hans", &[("g2p_root", root)])
        .unwrap();
    let ids: Vec<&str> = chinese.iter().map(|voice| voice.id.as_str()).collect();
    assert!(ids.contains(&"kokoro_zf_xiaoxiao"), "{ids:?}");
    assert!(ids.contains(&"piper_zh_CN-huayan-medium"), "{ids:?}");
    assert!(!ids.contains(&"kokoro_af_heart"), "{ids:?}");
    let found: Vec<&str> = chinese
        .iter()
        .filter(|voice| voice.found)
        .map(|voice| voice.id.as_str())
        .collect();
    assert_eq!(found, ["kokoro_zm_yunxi"]);
    for tag in ["en_us", "es_es", "ja_jp", "ar_msa", "ko_kr"] {
        let voices = library.tts_voices(tag, &[("g2p_root", root)]).unwrap();
        assert!(!voices.is_empty(), "{tag}");
    }
}
