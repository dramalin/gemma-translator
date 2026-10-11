//! Needs the native libraries: `make test-ffi`.

use std::path::PathBuf;

use onnxruntime_sys::{OrtError, OrtLibrary, Session};

fn lib_dir() -> PathBuf {
    std::env::var_os("TRANSLATOR_NATIVE_LIB_DIR")
        .expect("set TRANSLATOR_NATIVE_LIB_DIR (make test-ffi does)")
        .into()
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn loads_the_bundled_library() {
    let library = OrtLibrary::load(&lib_dir()).unwrap();
    assert!(library.version().starts_with("1."), "{}", library.version());
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn a_missing_model_is_an_error_with_ort_message() {
    let library = OrtLibrary::load(&lib_dir()).unwrap();
    let error = Session::load(&library, &PathBuf::from("/nonexistent/model.onnx"))
        .err()
        .unwrap();
    match error {
        OrtError::Call { function, message } => {
            assert_eq!(function, "CreateSession");
            assert!(!message.is_empty());
        }
        other => panic!("{other}"),
    }
}
