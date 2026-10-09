use std::io::Read;
use std::time::Duration;

use serde::Deserialize;
use translator_core::language::LanguageCode;
use translator_core::pipeline::{AudioClip, PipelineError, SpeechToText, TextToSpeech};

use crate::audio_convert::{f32_to_pcm16_le, pcm16_le_to_f32};

const SERVICE: &str = "speech service";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Client for the Python service's raw-PCM `/v2/stt` and `/v2/tts` endpoints.
pub struct SpeechServiceClient {
    base_url: String,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
struct SttResponse {
    text: String,
}

impl SpeechServiceClient {
    pub fn new(base_url: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();
        SpeechServiceClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            agent,
        }
    }
}

fn request_error(error: ureq::Error) -> PipelineError {
    match error {
        ureq::Error::Status(status, response) => PipelineError::Service {
            service: SERVICE,
            status,
            message: response.into_string().unwrap_or_default(),
        },
        ureq::Error::Transport(transport) => PipelineError::Unreachable {
            service: SERVICE,
            message: transport.to_string(),
        },
    }
}

fn invalid(message: impl ToString) -> PipelineError {
    PipelineError::InvalidResponse {
        service: SERVICE,
        message: message.to_string(),
    }
}

impl SpeechToText for SpeechServiceClient {
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError> {
        let url = format!("{}/v2/stt?language={}", self.base_url, language.code());
        let response = self
            .agent
            .post(&url)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&f32_to_pcm16_le(samples))
            .map_err(request_error)?;
        let body = response.into_string().map_err(invalid)?;
        let parsed: SttResponse = serde_json::from_str(&body).map_err(invalid)?;
        Ok(parsed.text)
    }
}

impl TextToSpeech for SpeechServiceClient {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError> {
        let url = format!("{}/v2/tts", self.base_url);
        let body = serde_json::json!({ "text": text, "language": language.code() }).to_string();
        let response = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(request_error)?;
        let sample_rate = response
            .header("X-Sample-Rate")
            .ok_or_else(|| invalid("missing X-Sample-Rate header"))?
            .parse::<u32>()
            .ok()
            .filter(|rate| *rate > 0)
            .ok_or_else(|| invalid("invalid X-Sample-Rate header"))?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(invalid)?;
        let samples = pcm16_le_to_f32(&bytes).ok_or_else(|| invalid("odd PCM byte count"))?;
        Ok(AudioClip {
            samples,
            sample_rate,
        })
    }
}
