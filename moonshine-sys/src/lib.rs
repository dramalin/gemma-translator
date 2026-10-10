//! Run-time bindings to `libmoonshine` (`.so` on Linux, `.dylib` on macOS) from the moonshine-voice 0.0.65 wheel.
//! Signatures mirror the wheel's `moonshine_api.py` ctypes declarations.

mod ffi;
mod library;
mod synthesizer;
mod transcriber;

pub use library::{MoonshineError, MoonshineLibrary, LIBRARY_FILE_NAME};
pub use synthesizer::Synthesizer;
pub use transcriber::{ModelArch, Transcriber};
