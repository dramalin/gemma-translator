//! Turns transport and HTTP errors of the phase A clients into `PipelineError`s.

use std::io;
use std::time::Duration;

use serde::Deserialize;
use translator_core::pipeline::PipelineError;

/// The Python service's error body: `{"error": "..."}`.
#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// `read_timeout` is the client's read timeout, reported when it expires.
pub(crate) fn request_error(
    service: &'static str,
    read_timeout: Duration,
    error: ureq::Error,
) -> PipelineError {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            PipelineError::Service {
                service,
                status,
                message: error_message(body),
            }
        }
        ureq::Error::Transport(transport) => {
            // Only an I/O error on an open connection is a read timeout; a
            // connect timeout (ConnectionFailed) means the service is unreachable.
            let is_timeout = transport.kind() == ureq::ErrorKind::Io
                && std::error::Error::source(&transport)
                    .and_then(|source| source.downcast_ref::<io::Error>())
                    .is_some_and(is_timeout);
            if is_timeout {
                timed_out(service, read_timeout)
            } else {
                PipelineError::Unreachable {
                    service,
                    message: transport.to_string(),
                }
            }
        }
    }
}

/// For errors while reading a response body that has already started.
pub(crate) fn read_error(
    service: &'static str,
    read_timeout: Duration,
    error: io::Error,
) -> PipelineError {
    if is_timeout(&error) {
        timed_out(service, read_timeout)
    } else {
        PipelineError::Unreachable {
            service,
            message: error.to_string(),
        }
    }
}

/// The `error` field of a JSON error body, or the body itself.
fn error_message(body: String) -> String {
    serde_json::from_str::<ErrorBody>(&body).map_or(body, |parsed| parsed.error)
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn timed_out(service: &'static str, read_timeout: Duration) -> PipelineError {
    PipelineError::TimedOut {
        service,
        seconds: read_timeout.as_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(9);

    #[test]
    fn json_error_bodies_are_unwrapped() {
        assert_eq!(
            error_message(r#"{"error":"model missing"}"#.to_string()),
            "model missing"
        );
        assert_eq!(error_message("busy".to_string()), "busy");
        assert_eq!(
            error_message(r#"{"other":1}"#.to_string()),
            r#"{"other":1}"#
        );
    }

    #[test]
    fn read_timeouts_are_reported_as_timeouts() {
        let error = read_error("LLM", TIMEOUT, io::Error::from(io::ErrorKind::WouldBlock));
        assert_eq!(
            error,
            PipelineError::TimedOut {
                service: "LLM",
                seconds: 9
            }
        );
        let error = read_error(
            "LLM",
            TIMEOUT,
            io::Error::from(io::ErrorKind::ConnectionReset),
        );
        assert!(matches!(error, PipelineError::Unreachable { .. }));
    }

    #[test]
    fn status_errors_carry_the_unwrapped_message() {
        let response =
            ureq::Response::new(500, "Internal Server Error", r#"{"error":"boom"}"#).unwrap();
        let error = request_error(
            "speech service",
            TIMEOUT,
            ureq::Error::Status(500, response),
        );
        assert_eq!(
            error,
            PipelineError::Service {
                service: "speech service",
                status: 500,
                message: "boom".to_string()
            }
        );
    }
}
