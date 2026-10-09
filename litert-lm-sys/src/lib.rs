//! Run-time bindings to `liblitert-lm.so` from the litert-lm-api 0.13.1 wheel.
//! Signatures mirror the wheel's `litert_lm/_ffi.py` ctypes declarations.

mod engine;
mod ffi;
mod library;

pub use engine::{chunk_text, Conversation, Engine, StreamEnd};
pub use library::{LiteRtLmError, LiteRtLmLibrary, LITERT_LM_LIBRARY_FILE_NAME};
