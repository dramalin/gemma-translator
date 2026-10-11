//! C declarations, copied from moonshine-voice 0.0.65 `moonshine_api.py`.

use std::os::raw::{c_char, c_void};

/// `MOONSHINE_HEADER_VERSION`; passed to every create call.
pub const MOONSHINE_HEADER_VERSION: i32 = 20000;
pub const MOONSHINE_ERROR_NONE: i32 = 0;

/// `moonshine_option_t`
#[repr(C)]
pub struct MoonshineOption {
    pub name: *const c_char,
    pub value: *const c_char,
}

/// `transcript_word_t`
#[repr(C)]
pub struct TranscriptWord {
    pub text: *const c_char,
    pub start: f32,
    pub end: f32,
    pub confidence: f32,
}

/// `transcript_line_t`: every field is declared so the array stride is right.
#[repr(C)]
pub struct TranscriptLine {
    pub text: *const c_char,
    pub audio_data: *const f32,
    pub audio_data_count: usize,
    pub start_time: f32,
    pub duration: f32,
    pub id: u64,
    pub is_complete: i8,
    pub is_updated: i8,
    pub is_new: i8,
    pub has_text_changed: i8,
    pub has_speaker_id: i8,
    pub speaker_id: u64,
    pub speaker_index: u32,
    pub last_transcription_latency_ms: u32,
    pub words: *const TranscriptWord,
    pub word_count: u64,
}

/// `transcript_t`
#[repr(C)]
pub struct Transcript {
    pub lines: *const TranscriptLine,
    pub line_count: u64,
}

pub type GetVersionFn = unsafe extern "C" fn() -> i32;
pub type ErrorToStringFn = unsafe extern "C" fn(code: i32) -> *const c_char;
pub type LoadTranscriberFromFilesFn = unsafe extern "C" fn(
    path: *const c_char,
    model_arch: u32,
    options: *const MoonshineOption,
    options_count: u64,
    header_version: i32,
) -> i32;
pub type FreeTranscriberFn = unsafe extern "C" fn(handle: i32);
pub type TranscribeWithoutStreamingFn = unsafe extern "C" fn(
    handle: i32,
    audio: *const f32,
    audio_length: u64,
    sample_rate: i32,
    flags: u32,
    out_transcript: *mut *mut Transcript,
) -> i32;
pub type CreateTtsSynthesizerFromFilesFn = unsafe extern "C" fn(
    language: *const c_char,
    filenames: *const *const c_char,
    filenames_count: u64,
    options: *const MoonshineOption,
    options_count: u64,
    header_version: i32,
) -> i32;
pub type FreeTtsSynthesizerFn = unsafe extern "C" fn(handle: i32);
pub type GetTtsDependenciesFn = unsafe extern "C" fn(
    languages: *const c_char,
    options: *const MoonshineOption,
    options_count: u64,
    out_json: *mut *mut c_char,
) -> i32;
pub type GetTtsVoicesFn = unsafe extern "C" fn(
    languages: *const c_char,
    options: *const MoonshineOption,
    options_count: u64,
    out_json: *mut *mut c_char,
) -> i32;
pub type TextToSpeechFn = unsafe extern "C" fn(
    handle: i32,
    text: *const c_char,
    options: *const MoonshineOption,
    options_count: u64,
    out_audio: *mut *mut f32,
    out_size: *mut u64,
    out_sample_rate: *mut i32,
) -> i32;

extern "C" {
    /// libc `free`: the C API returns `malloc`ed strings and audio buffers.
    pub fn free(pointer: *mut c_void);
}
