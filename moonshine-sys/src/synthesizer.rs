use std::marker::PhantomData;
use std::ptr;
use std::sync::Arc;

use crate::ffi::{MOONSHINE_ERROR_NONE, MOONSHINE_HEADER_VERSION};
use crate::library::{c_string, free_native, MoonshineError, MoonshineLibrary, OptionArray};

/// A text-to-speech engine for one language. Not `Send`.
pub struct Synthesizer {
    library: Arc<MoonshineLibrary>,
    handle: i32,
    _single_thread: PhantomData<*const ()>,
}

impl Synthesizer {
    /// `options` must include `g2p_root`; the file list is resolved by the C side.
    pub fn create(
        library: &Arc<MoonshineLibrary>,
        language_tag: &str,
        options: &[(&str, &str)],
    ) -> Result<Synthesizer, MoonshineError> {
        let language_tag = c_string(language_tag)?;
        let options = OptionArray::new(options)?;
        // SAFETY: valid C strings and option array; NULL file list as in tts.py.
        let handle = unsafe {
            (library.create_tts_synthesizer_from_files)(
                language_tag.as_ptr(),
                ptr::null(),
                0,
                options.as_ptr(),
                options.len(),
                MOONSHINE_HEADER_VERSION,
            )
        };
        if handle < 0 {
            return Err(library.call_error("moonshine_create_tts_synthesizer_from_files", handle));
        }
        Ok(Synthesizer {
            library: Arc::clone(library),
            handle,
            _single_thread: PhantomData,
        })
    }

    /// Returns mono samples in [-1, 1] and their sample rate.
    pub fn synthesize(&mut self, text: &str) -> Result<(Vec<f32>, u32), MoonshineError> {
        const FUNCTION: &str = "moonshine_text_to_speech";
        let text = c_string(text)?;
        let mut audio: *mut f32 = ptr::null_mut();
        let mut size: u64 = 0;
        let mut sample_rate: i32 = 0;
        // SAFETY: out-pointers are valid; `audio` receives a malloc'd buffer.
        let code = unsafe {
            (self.library.text_to_speech)(
                self.handle,
                text.as_ptr(),
                ptr::null(),
                0,
                &mut audio,
                &mut size,
                &mut sample_rate,
            )
        };
        let samples = if audio.is_null() || size == 0 {
            Vec::new()
        } else {
            // SAFETY: `audio` holds `size` floats.
            unsafe { std::slice::from_raw_parts(audio, size as usize) }.to_vec()
        };
        if !audio.is_null() {
            // SAFETY: allocated by the C API with malloc; not used afterwards.
            unsafe { free_native(audio.cast()) };
        }
        if code != MOONSHINE_ERROR_NONE {
            return Err(self.library.call_error(FUNCTION, code));
        }
        if sample_rate <= 0 {
            return Err(MoonshineError::InvalidOutput {
                function: FUNCTION,
                message: format!("sample rate {sample_rate}"),
            });
        }
        Ok((samples, sample_rate as u32))
    }
}

impl Drop for Synthesizer {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful create and is freed once.
        unsafe { (self.library.free_tts_synthesizer)(self.handle) }
    }
}
