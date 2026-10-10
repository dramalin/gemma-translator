use std::ffi::CStr;
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use crate::ffi::{Transcript, MOONSHINE_ERROR_NONE, MOONSHINE_HEADER_VERSION};
use crate::library::{c_string, MoonshineError, MoonshineLibrary, OptionArray};

/// `moonshine_voice.ModelArch`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ModelArch {
    Tiny = 0,
    Base = 1,
    TinyStreaming = 2,
    BaseStreaming = 3,
    SmallStreaming = 4,
    MediumStreaming = 5,
}

/// A loaded speech-recognition model. Not `Send`: the handle stays on the
/// thread that created it.
pub struct Transcriber {
    library: Arc<MoonshineLibrary>,
    handle: i32,
    _single_thread: PhantomData<*const ()>,
}

impl Transcriber {
    pub fn load(
        library: &Arc<MoonshineLibrary>,
        model_dir: &Path,
        arch: ModelArch,
        options: &[(&str, &str)],
    ) -> Result<Transcriber, MoonshineError> {
        let path = model_dir.to_str().ok_or_else(|| {
            MoonshineError::InvalidArgument(format!("{} is not UTF-8", model_dir.display()))
        })?;
        let path = c_string(path)?;
        let options = OptionArray::new(options)?;
        // SAFETY: `path` is a valid C string and `options` a valid option array.
        let handle = unsafe {
            (library.load_transcriber_from_files)(
                path.as_ptr(),
                arch as u32,
                options.as_ptr(),
                options.len(),
                MOONSHINE_HEADER_VERSION,
            )
        };
        if handle < 0 {
            return Err(library.call_error("moonshine_load_transcriber_from_files", handle));
        }
        Ok(Transcriber {
            library: Arc::clone(library),
            handle,
            _single_thread: PhantomData,
        })
    }

    /// Transcribes mono samples and joins the transcript lines with spaces,
    /// as `server.py` did.
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
    ) -> Result<String, MoonshineError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut transcript: *mut Transcript = ptr::null_mut();
        // SAFETY: `samples` is valid for its length; `transcript` receives a
        // pointer owned by the transcriber (there is no free function; see
        // the soak test).
        let code = unsafe {
            (self.library.transcribe_without_streaming)(
                self.handle,
                samples.as_ptr(),
                samples.len() as u64,
                sample_rate as i32,
                0,
                &mut transcript,
            )
        };
        if code != MOONSHINE_ERROR_NONE {
            return Err(self
                .library
                .call_error("moonshine_transcribe_without_streaming", code));
        }
        if transcript.is_null() {
            return Ok(String::new());
        }
        // SAFETY: a successful call leaves a valid transcript until the next
        // call on this handle; `&mut self` rules out a concurrent call.
        let transcript = unsafe { &*transcript };
        if transcript.lines.is_null() || transcript.line_count == 0 {
            return Ok(String::new());
        }
        // SAFETY: `lines` points to `line_count` initialised entries.
        let lines =
            unsafe { std::slice::from_raw_parts(transcript.lines, transcript.line_count as usize) };
        let texts: Vec<String> = lines
            .iter()
            .filter(|line| !line.text.is_null())
            // SAFETY: non-NULL line texts are NUL-terminated.
            .map(|line| {
                unsafe { CStr::from_ptr(line.text) }
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        Ok(texts.join(" "))
    }
}

impl Drop for Transcriber {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful load and is freed once.
        unsafe { (self.library.free_transcriber)(self.handle) }
    }
}
