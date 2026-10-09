use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ffi;

pub const LITERT_LM_LIBRARY_FILE_NAME: &str = "liblitert-lm.so";

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum LiteRtLmError {
    #[error("cannot load {path}: {message}")]
    Load { path: PathBuf, message: String },
    #[error("{name} missing from liblitert-lm.so: {message}")]
    Symbol { name: String, message: String },
    #[error("model file {0} not found; run translator-models")]
    ModelMissing(PathBuf),
    #[error("{0} failed")]
    Call(&'static str),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("generation failed: {0}")]
    Stream(String),
    #[error("no final callback within {0:?} of cancelling")]
    CancelTimedOut(std::time::Duration),
}

/// The loaded library and every function this crate calls.
pub struct LiteRtLmLibrary {
    pub(crate) engine_settings_create: ffi::EngineSettingsCreateFn,
    pub(crate) engine_settings_delete: ffi::EngineSettingsDeleteFn,
    pub(crate) engine_settings_set_cache_dir: ffi::EngineSettingsSetCacheDirFn,
    pub(crate) engine_create: ffi::EngineCreateFn,
    pub(crate) engine_delete: ffi::EngineDeleteFn,
    pub(crate) conversation_config_create: ffi::ConversationConfigCreateFn,
    pub(crate) conversation_config_delete: ffi::ConversationConfigDeleteFn,
    pub(crate) conversation_config_set_system_message: ffi::ConversationConfigSetSystemMessageFn,
    pub(crate) conversation_create: ffi::ConversationCreateFn,
    pub(crate) conversation_delete: ffi::ConversationDeleteFn,
    pub(crate) conversation_send_message_stream: ffi::ConversationSendMessageStreamFn,
    pub(crate) conversation_cancel_process: ffi::ConversationCancelProcessFn,
    /// Never closed: litert-lm registers thread-local destructors that run
    /// when a thread that called into it exits, possibly after the last
    /// `Arc` is dropped. Unmapping the code first crashes that thread.
    _library: ManuallyDrop<libloading::Library>,
}

/// # Safety
/// `T` must be the exact C signature of the symbol.
unsafe fn symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T, LiteRtLmError> {
    library
        .get::<T>(name)
        .map(|found| *found)
        .map_err(|error| LiteRtLmError::Symbol {
            name: String::from_utf8_lossy(&name[..name.len() - 1]).into_owned(),
            message: error.to_string(),
        })
}

impl LiteRtLmLibrary {
    pub fn load(directory: &Path) -> Result<Arc<LiteRtLmLibrary>, LiteRtLmError> {
        let path = directory.join(LITERT_LM_LIBRARY_FILE_NAME);
        // SAFETY: the pinned wheel's library has no initialisers with preconditions.
        let library =
            unsafe { libloading::Library::new(&path) }.map_err(|error| LiteRtLmError::Load {
                path: path.clone(),
                message: error.to_string(),
            })?;
        // SAFETY: every type below is the ctypes signature from _ffi.py.
        unsafe {
            Ok(Arc::new(LiteRtLmLibrary {
                engine_settings_create: symbol(&library, b"litert_lm_engine_settings_create\0")?,
                engine_settings_delete: symbol(&library, b"litert_lm_engine_settings_delete\0")?,
                engine_settings_set_cache_dir: symbol(
                    &library,
                    b"litert_lm_engine_settings_set_cache_dir\0",
                )?,
                engine_create: symbol(&library, b"litert_lm_engine_create\0")?,
                engine_delete: symbol(&library, b"litert_lm_engine_delete\0")?,
                conversation_config_create: symbol(
                    &library,
                    b"litert_lm_conversation_config_create\0",
                )?,
                conversation_config_delete: symbol(
                    &library,
                    b"litert_lm_conversation_config_delete\0",
                )?,
                conversation_config_set_system_message: symbol(
                    &library,
                    b"litert_lm_conversation_config_set_system_message\0",
                )?,
                conversation_create: symbol(&library, b"litert_lm_conversation_create\0")?,
                conversation_delete: symbol(&library, b"litert_lm_conversation_delete\0")?,
                conversation_send_message_stream: symbol(
                    &library,
                    b"litert_lm_conversation_send_message_stream\0",
                )?,
                conversation_cancel_process: symbol(
                    &library,
                    b"litert_lm_conversation_cancel_process\0",
                )?,
                _library: ManuallyDrop::new(library),
            }))
        }
    }
}
