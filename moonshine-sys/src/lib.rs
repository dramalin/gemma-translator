//! Run-time bindings to `libmoonshine.so` from the moonshine-voice 0.0.65 wheel.
//! Signatures mirror the wheel's `moonshine_api.py` ctypes declarations.

mod ffi;
mod library;

pub use library::{MoonshineError, MoonshineLibrary, LIBRARY_FILE_NAME};
