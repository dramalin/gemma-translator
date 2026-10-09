mod support;

use std::net::TcpListener;

use support::{http_response, serve_once};
use translator_core::language::LanguageCode;
use translator_core::pipeline::{PipelineError, SpeechToText, TextToSpeech};
use translator_io::speech_client::SpeechServiceClient;

/// A localhost URL nothing listens on.
fn unused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}

#[test]
fn transcribe_posts_pcm16_and_reads_text() {
    let response = http_response(
        "200 OK",
        &[("Content-Type", "application/json")],
        br#"{"text":"hola"}"#,
    );
    let (url, server) = serve_once(response);
    let client = SpeechServiceClient::new(&url);
    let text = client.transcribe(&[0.0, 0.5], LanguageCode::Es).unwrap();
    assert_eq!(text, "hola");
    let request = server.join().unwrap();
    assert!(request.head.starts_with("POST /v2/stt?language=es "));
    assert!(request
        .head
        .to_ascii_lowercase()
        .contains("content-type: application/octet-stream"));
    assert_eq!(request.body, [0, 0, 0x00, 0x40]);
}

#[test]
fn transcribe_reports_service_errors_with_body() {
    let response = http_response(
        "500 Internal Server Error",
        &[],
        br#"{"error":"model missing"}"#,
    );
    let (url, server) = serve_once(response);
    let error = SpeechServiceClient::new(&url)
        .transcribe(&[0.0], LanguageCode::En)
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        error,
        PipelineError::Service {
            service: "speech service",
            status: 500,
            message: r#"{"error":"model missing"}"#.to_string()
        }
    );
}

#[test]
fn transcribe_reports_an_unreachable_service() {
    let error = SpeechServiceClient::new(&unused_url())
        .transcribe(&[0.0], LanguageCode::En)
        .unwrap_err();
    assert!(matches!(
        error,
        PipelineError::Unreachable {
            service: "speech service",
            ..
        }
    ));
}

#[test]
fn synthesize_sends_json_and_decodes_pcm() {
    let body = [0x00, 0x40, 0x00, 0xc0];
    let response = http_response(
        "200 OK",
        &[
            ("Content-Type", "application/octet-stream"),
            ("X-Sample-Rate", "24000"),
        ],
        &body,
    );
    let (url, server) = serve_once(response);
    let clip = SpeechServiceClient::new(&url)
        .synthesize("Hi there", LanguageCode::Zh)
        .unwrap();
    assert_eq!(clip.sample_rate, 24_000);
    assert_eq!(clip.samples, [0.5, -0.5]);
    let request = server.join().unwrap();
    assert!(request.head.starts_with("POST /v2/tts "));
    let json: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"text": "Hi there", "language": "zh"})
    );
}

#[test]
fn synthesize_requires_the_sample_rate_header() {
    let response = http_response(
        "200 OK",
        &[("Content-Type", "application/octet-stream")],
        &[0, 0],
    );
    let (url, server) = serve_once(response);
    let error = SpeechServiceClient::new(&url)
        .synthesize("x", LanguageCode::En)
        .unwrap_err();
    server.join().unwrap();
    assert!(matches!(error, PipelineError::InvalidResponse { .. }));
}

#[test]
fn synthesize_rejects_a_zero_or_unparsable_sample_rate() {
    for sample_rate in ["0", "fast"] {
        let response = http_response(
            "200 OK",
            &[
                ("Content-Type", "application/octet-stream"),
                ("X-Sample-Rate", sample_rate),
            ],
            &[0, 0],
        );
        let (url, server) = serve_once(response);
        let error = SpeechServiceClient::new(&url)
            .synthesize("x", LanguageCode::En)
            .unwrap_err();
        server.join().unwrap();
        assert_eq!(
            error,
            PipelineError::InvalidResponse {
                service: "speech service",
                message: "invalid X-Sample-Rate header".to_string(),
            },
            "X-Sample-Rate: {sample_rate}"
        );
    }
}
