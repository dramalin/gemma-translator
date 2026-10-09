//! In-process inference through the vendor libraries (spec phase B). Each
//! service owns one worker thread that holds every native handle; job threads
//! talk to it over channels.

pub mod lru;
pub mod speech;
pub mod translator;

use std::fmt::Display;
use std::path::PathBuf;

use translator_core::pipeline::PipelineError;

/// `<directory of the running executable>/lib`, where deployment puts the .so files.
pub fn default_lib_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|directory| directory.join("lib")))
        .unwrap_or_else(|| PathBuf::from("lib"))
}

pub(crate) fn engine_error(service: &'static str, error: impl Display) -> PipelineError {
    PipelineError::Engine {
        service,
        message: error.to_string(),
    }
}
