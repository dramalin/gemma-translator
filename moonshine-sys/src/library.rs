use std::ffi::{CStr, CString};
use std::mem::ManuallyDrop;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;

use crate::ffi::{self, MoonshineOption, MOONSHINE_ERROR_NONE};

pub const LIBRARY_FILE_NAME: &str = "libmoonshine.so";

#[derive(Debug, thiserror::Error)]
pub enum MoonshineError {
    #[error("cannot load {path}: {message}")]
    Load { path: PathBuf, message: String },
    #[error("{name} missing from libmoonshine.so: {message}")]
    Symbol { name: String, message: String },
    #[error("{function} failed ({code}): {message}")]
    Call {
        function: &'static str,
        code: i32,
        message: String,
    },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("unexpected output from {function}: {message}")]
    InvalidOutput {
        function: &'static str,
        message: String,
    },
}

/// The loaded library and every function this crate calls. Function pointers
/// stay valid for the rest of the process: `_library` is never closed.
pub struct MoonshineLibrary {
    get_version: ffi::GetVersionFn,
    error_to_string: ffi::ErrorToStringFn,
    pub(crate) load_transcriber_from_files: ffi::LoadTranscriberFromFilesFn,
    pub(crate) free_transcriber: ffi::FreeTranscriberFn,
    pub(crate) transcribe_without_streaming: ffi::TranscribeWithoutStreamingFn,
    pub(crate) create_tts_synthesizer_from_files: ffi::CreateTtsSynthesizerFromFilesFn,
    pub(crate) free_tts_synthesizer: ffi::FreeTtsSynthesizerFn,
    get_tts_dependencies: ffi::GetTtsDependenciesFn,
    pub(crate) text_to_speech: ffi::TextToSpeechFn,
    /// Never closed: onnxruntime and libstdc++ register thread-local
    /// destructors that run when a thread that called into the library
    /// exits, possibly after the last `Arc` is dropped. Unmapping the code
    /// first crashes that thread (seen under gdb with litert-lm, which
    /// keeps its library open for the same reason).
    _library: ManuallyDrop<libloading::Library>,
}

/// Looks up `name` (NUL-terminated) and copies out the function pointer.
///
/// # Safety
/// `T` must be the exact C signature of the symbol.
unsafe fn symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T, MoonshineError> {
    library
        .get::<T>(name)
        .map(|found| *found)
        .map_err(|error| MoonshineError::Symbol {
            name: String::from_utf8_lossy(&name[..name.len() - 1]).into_owned(),
            message: error.to_string(),
        })
}

impl MoonshineLibrary {
    pub fn load(directory: &Path) -> Result<Arc<MoonshineLibrary>, MoonshineError> {
        let path = directory.join(LIBRARY_FILE_NAME);
        // SAFETY: loading runs the library's initialisers; the pinned wheel's
        // library has no initialisers with preconditions.
        let library =
            unsafe { libloading::Library::new(&path) }.map_err(|error| MoonshineError::Load {
                path: path.clone(),
                message: error.to_string(),
            })?;
        // SAFETY: every type below is the ctypes signature from moonshine_api.py.
        unsafe {
            Ok(Arc::new(MoonshineLibrary {
                get_version: symbol(&library, b"moonshine_get_version\0")?,
                error_to_string: symbol(&library, b"moonshine_error_to_string\0")?,
                load_transcriber_from_files: symbol(
                    &library,
                    b"moonshine_load_transcriber_from_files\0",
                )?,
                free_transcriber: symbol(&library, b"moonshine_free_transcriber\0")?,
                transcribe_without_streaming: symbol(
                    &library,
                    b"moonshine_transcribe_without_streaming\0",
                )?,
                create_tts_synthesizer_from_files: symbol(
                    &library,
                    b"moonshine_create_tts_synthesizer_from_files\0",
                )?,
                free_tts_synthesizer: symbol(&library, b"moonshine_free_tts_synthesizer\0")?,
                get_tts_dependencies: symbol(&library, b"moonshine_get_tts_dependencies\0")?,
                text_to_speech: symbol(&library, b"moonshine_text_to_speech\0")?,
                _library: ManuallyDrop::new(library),
            }))
        }
    }

    pub fn version(&self) -> i32 {
        // SAFETY: no arguments, returns a plain integer.
        unsafe { (self.get_version)() }
    }

    pub fn error_message(&self, code: i32) -> String {
        // SAFETY: returns a static string or NULL.
        let message = unsafe { (self.error_to_string)(code) };
        if message.is_null() {
            return format!("error {code}");
        }
        // SAFETY: non-NULL results are NUL-terminated static strings.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }

    pub(crate) fn call_error(&self, function: &'static str, code: i32) -> MoonshineError {
        MoonshineError::Call {
            function,
            code,
            message: self.error_message(code),
        }
    }

    /// Asset keys (paths relative to `g2p_root`) the synthesizer for
    /// `language_tag` needs, from `moonshine_get_tts_dependencies`.
    pub fn tts_dependencies(
        &self,
        language_tag: &str,
        options: &[(&str, &str)],
    ) -> Result<Vec<String>, MoonshineError> {
        const FUNCTION: &str = "moonshine_get_tts_dependencies";
        let language_tag = c_string(language_tag)?;
        let options = OptionArray::new(options)?;
        let mut json: *mut c_char = ptr::null_mut();
        // SAFETY: all pointers are valid for the call; `json` receives a malloc'd string.
        let code = unsafe {
            (self.get_tts_dependencies)(
                language_tag.as_ptr(),
                options.as_ptr(),
                options.len(),
                &mut json,
            )
        };
        // SAFETY: `json` is NULL or a malloc'd NUL-terminated string we now own.
        let text = unsafe { take_c_string(json) };
        if code != MOONSHINE_ERROR_NONE {
            return Err(self.call_error(FUNCTION, code));
        }
        let text = text.unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&text).map_err(|error| MoonshineError::InvalidOutput {
            function: FUNCTION,
            message: error.to_string(),
        })
    }
}

pub(crate) fn c_string(text: &str) -> Result<CString, MoonshineError> {
    CString::new(text)
        .map_err(|_| MoonshineError::InvalidArgument(format!("{text:?} contains a NUL byte")))
}

/// Copies a malloc'd C string and frees it.
///
/// # Safety
/// `pointer` must be NULL or a NUL-terminated string allocated with `malloc`.
unsafe fn take_c_string(pointer: *mut c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let text = CStr::from_ptr(pointer).to_string_lossy().into_owned();
    free_native(pointer.cast());
    Some(text)
}

/// Frees memory the C API allocated with `malloc`.
///
/// # Safety
/// `pointer` must come from the C API and must not be used afterwards.
pub(crate) unsafe fn free_native(pointer: *mut std::os::raw::c_void) {
    ffi::free(pointer);
}

/// A `moonshine_option_t` array and the strings it points into.
pub(crate) struct OptionArray {
    _strings: Vec<CString>,
    entries: Vec<MoonshineOption>,
}

impl OptionArray {
    pub(crate) fn new(options: &[(&str, &str)]) -> Result<OptionArray, MoonshineError> {
        let mut strings = Vec::with_capacity(options.len() * 2);
        let mut entries = Vec::with_capacity(options.len());
        for (name, value) in options {
            let name = c_string(name)?;
            let value = c_string(value)?;
            entries.push(MoonshineOption {
                name: name.as_ptr(),
                value: value.as_ptr(),
            });
            // Moving a CString does not move its heap buffer, so the pointers stay valid.
            strings.push(name);
            strings.push(value);
        }
        Ok(OptionArray {
            _strings: strings,
            entries,
        })
    }

    pub(crate) fn as_ptr(&self) -> *const MoonshineOption {
        if self.entries.is_empty() {
            ptr::null()
        } else {
            self.entries.as_ptr()
        }
    }

    pub(crate) fn len(&self) -> u64 {
        self.entries.len() as u64
    }
}
