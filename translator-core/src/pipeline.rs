use std::sync::atomic::AtomicBool;

use crate::language::LanguageCode;

/// Sample rate the speech recognizer expects.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Mono audio with its own sample rate (synthesis rates vary per engine).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PipelineError {
    #[error("cannot reach {service}: {message}")]
    Unreachable {
        service: &'static str,
        message: String,
    },
    #[error("{service} returned HTTP {status}: {message}")]
    Service {
        service: &'static str,
        status: u16,
        message: String,
    },
    #[error("invalid response from {service}: {message}")]
    InvalidResponse {
        service: &'static str,
        message: String,
    },
    #[error("{service} did not answer within {seconds} s")]
    TimedOut { service: &'static str, seconds: u64 },
    #[error("{service} failed: {message}")]
    Engine {
        service: &'static str,
        message: String,
    },
}

pub trait SpeechToText: Send + Sync {
    /// `samples` must be 16 kHz mono.
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError>;
}

pub trait Translator: Send + Sync {
    /// Calls `on_sentence` once per completed sentence. Returns `Ok(())` early
    /// once `cancel` is set.
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError>;
}

pub trait TextToSpeech: Send + Sync {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_error_names_the_service() {
        let error = PipelineError::Engine {
            service: "speech engine",
            message: "model missing".to_string(),
        };
        assert_eq!(error.to_string(), "speech engine failed: model missing");
    }
}
