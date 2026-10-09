//! Device-, process- and network-bound parts of the translator.

pub mod audio_convert;
pub mod audio_device;
pub mod blip;
mod http_error;
pub mod llm_client;
pub mod native;
pub mod playback_queue;
pub mod runner;
pub mod speech_client;
pub mod volume;
