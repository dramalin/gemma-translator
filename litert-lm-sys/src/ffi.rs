//! C declarations, copied from litert-lm-api 0.13.1 `litert_lm/_ffi.py`.

use std::os::raw::{c_char, c_int, c_void};

/// `LiteRtLmStreamCallback`: (user_data, chunk, is_final, error).
pub type StreamCallback = unsafe extern "C" fn(
    user_data: *mut c_void,
    chunk: *const c_char,
    is_final: bool,
    error: *const c_char,
);

pub type EngineSettingsCreateFn = unsafe extern "C" fn(
    model_path: *const c_char,
    backend: *const c_char,
    vision_backend: *const c_char,
    audio_backend: *const c_char,
) -> *mut c_void;
pub type EngineSettingsDeleteFn = unsafe extern "C" fn(settings: *mut c_void);
pub type EngineSettingsSetCacheDirFn =
    unsafe extern "C" fn(settings: *mut c_void, cache_dir: *const c_char);
pub type EngineCreateFn = unsafe extern "C" fn(settings: *mut c_void) -> *mut c_void;
pub type EngineDeleteFn = unsafe extern "C" fn(engine: *mut c_void);
pub type ConversationConfigCreateFn = unsafe extern "C" fn() -> *mut c_void;
pub type ConversationConfigDeleteFn = unsafe extern "C" fn(config: *mut c_void);
pub type ConversationConfigSetSystemMessageFn =
    unsafe extern "C" fn(config: *mut c_void, system_message: *const c_char);
pub type ConversationCreateFn =
    unsafe extern "C" fn(engine: *mut c_void, config: *mut c_void) -> *mut c_void;
pub type ConversationDeleteFn = unsafe extern "C" fn(conversation: *mut c_void);
pub type ConversationSendMessageStreamFn = unsafe extern "C" fn(
    conversation: *mut c_void,
    message_json: *const c_char,
    extra_context_json: *const c_char,
    visual_token_budget: *const c_void,
    callback: StreamCallback,
    user_data: *mut c_void,
) -> c_int;
pub type ConversationCancelProcessFn = unsafe extern "C" fn(conversation: *mut c_void);
