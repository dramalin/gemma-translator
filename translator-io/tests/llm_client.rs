mod support;

use std::sync::atomic::AtomicBool;

use support::{http_response, serve_once};
use translator_core::language::LanguageCode;
use translator_core::pipeline::{PipelineError, Translator};
use translator_io::llm_client::LlmClient;

fn translate(client: &LlmClient, is_cancelled: bool) -> Result<Vec<String>, PipelineError> {
    let cancel = AtomicBool::new(is_cancelled);
    let mut sentences = Vec::new();
    client.translate_streaming(
        "hello",
        LanguageCode::En,
        LanguageCode::Es,
        &cancel,
        &mut |sentence| sentences.push(sentence),
    )?;
    Ok(sentences)
}

const STREAM_BODY: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"Hola a todos. \"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"¿Qué\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\" tal?\"}}]}\n\n",
    "data: [DONE]\n\n",
);

#[test]
fn streams_sentences_from_server_sent_events() {
    let response = http_response(
        "200 OK",
        &[("Content-Type", "text/event-stream")],
        STREAM_BODY.as_bytes(),
    );
    let (url, server) = serve_once(response);
    let sentences = translate(&LlmClient::new(&url, "gemma4-e2b"), false).unwrap();
    assert_eq!(sentences, ["Hola a todos.", "¿Qué tal?"]);
    let request = server.join().unwrap();
    assert!(request.head.starts_with("POST /chat/completions "));
    let json: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(json["model"], "gemma4-e2b");
    assert_eq!(json["stream"], true);
    assert_eq!(
        json["messages"][1],
        serde_json::json!({"role": "user", "content": "hello"})
    );
    let system = json["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("from English into Spanish"));
}

#[test]
fn falls_back_to_a_regular_completion() {
    let body =
        br#"{"choices":[{"message":{"role":"assistant","content":"Translation: Hola. Adios."}}]}"#;
    let response = http_response("200 OK", &[("Content-Type", "application/json")], body);
    let (url, server) = serve_once(response);
    let sentences = translate(&LlmClient::new(&url, "gemma4-e2b"), false).unwrap();
    server.join().unwrap();
    assert_eq!(sentences, ["Hola.", "Adios."]);
}

#[test]
fn reports_http_errors() {
    let response = http_response("503 Service Unavailable", &[], b"busy");
    let (url, server) = serve_once(response);
    let error = translate(&LlmClient::new(&url, "gemma4-e2b"), false).unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error,
        PipelineError::Service {
            service: "LLM",
            status: 503,
            message: "busy".to_string()
        }
    );
}

#[test]
fn cancelled_translation_emits_nothing() {
    let response = http_response(
        "200 OK",
        &[("Content-Type", "text/event-stream")],
        STREAM_BODY.as_bytes(),
    );
    let (url, server) = serve_once(response);
    let sentences = translate(&LlmClient::new(&url, "gemma4-e2b"), true).unwrap();
    server.join().unwrap();
    assert!(sentences.is_empty());
}
