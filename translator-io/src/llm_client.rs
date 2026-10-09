use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use translator_core::language::LanguageCode;
use translator_core::pipeline::{PipelineError, Translator};
use translator_core::text::{clean_model_reply, translation_system_prompt, SentenceSplitter};

use crate::http_error::{read_error, request_error};

const SERVICE: &str = "LLM";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Per read: a non-streaming reply arrives only once the whole translation is
/// done, which on the Pi can take well over 30 s.
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI-compatible chat-completions client (litert-lm serve), streaming via SSE.
pub struct LlmClient {
    endpoint: String,
    model: String,
    agent: ureq::Agent,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SseLine {
    Delta(String),
    Done,
    Ignored,
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<CompletionChoice>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
}

#[derive(Deserialize)]
struct CompletionMessage {
    content: Option<String>,
}

/// Parses one line of a `text/event-stream` body.
pub fn parse_sse_line(line: &str) -> Result<SseLine, String> {
    let Some(data) = line.strip_prefix("data:") else {
        return Ok(SseLine::Ignored);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(SseLine::Done);
    }
    let chunk: StreamChunk = serde_json::from_str(data).map_err(|error| error.to_string())?;
    Ok(chunk
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.delta.content)
        .map_or(SseLine::Ignored, SseLine::Delta))
}

impl LlmClient {
    pub fn new(endpoint: &str, model: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .build();
        LlmClient {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
            agent,
        }
    }
}

fn invalid(message: impl ToString) -> PipelineError {
    PipelineError::InvalidResponse {
        service: SERVICE,
        message: message.to_string(),
    }
}

impl Translator for LlmClient {
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        let body = serde_json::json!({
            "model": self.model,
            "stream": true,
            "messages": [
                { "role": "system", "content": translation_system_prompt(source, target) },
                { "role": "user", "content": text },
            ],
        })
        .to_string();
        let response = self
            .agent
            .post(&format!("{}/chat/completions", self.endpoint))
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|error| request_error(SERVICE, READ_TIMEOUT, error))?;

        let mut splitter = SentenceSplitter::new();
        let mut emit = |sentences: Vec<String>| {
            for sentence in sentences {
                let cleaned = clean_model_reply(&sentence);
                if !cleaned.is_empty() {
                    on_sentence(cleaned);
                }
            }
        };

        if response.content_type() != "text/event-stream" {
            // The server ignored "stream": read one regular completion.
            let completion: Completion = serde_json::from_str(
                &response
                    .into_string()
                    .map_err(|error| read_error(SERVICE, READ_TIMEOUT, error))?,
            )
            .map_err(invalid)?;
            if cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let content = completion
                .choices
                .into_iter()
                .next()
                .and_then(|choice| choice.message.content)
                .unwrap_or_default();
            emit(splitter.push(&content));
            emit(splitter.finish());
            return Ok(());
        }

        for line in BufReader::new(response.into_reader()).lines() {
            if cancel.load(Ordering::SeqCst) {
                return Ok(());
            }
            let line = line.map_err(|error| read_error(SERVICE, READ_TIMEOUT, error))?;
            match parse_sse_line(&line).map_err(invalid)? {
                SseLine::Delta(fragment) => emit(splitter.push(&fragment)),
                SseLine::Done => break,
                SseLine::Ignored => {}
            }
        }
        if !cancel.load(Ordering::SeqCst) {
            emit(splitter.finish());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_deltas_done_and_noise() {
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#),
            Ok(SseLine::Delta("Hi".to_string()))
        );
        assert_eq!(parse_sse_line("data: [DONE]"), Ok(SseLine::Done));
        assert_eq!(parse_sse_line(""), Ok(SseLine::Ignored));
        assert_eq!(parse_sse_line(": keep-alive"), Ok(SseLine::Ignored));
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[{"delta":{}}]}"#),
            Ok(SseLine::Ignored)
        );
        assert_eq!(
            parse_sse_line(r#"data: {"choices":[]}"#),
            Ok(SseLine::Ignored)
        );
        assert!(parse_sse_line("data: {broken").is_err());
    }
}
