//! Fetches one file: written to `<name>.part`, checked against Content-Length
//! and (when known) sha256, then renamed into place. A file already at its
//! final path was verified when it was written, so it is never fetched again.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSpec {
    pub url: String,
    pub destination: PathBuf,
    pub expected_sha256: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadOutcome {
    AlreadyPresent,
    Downloaded { bytes: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("{url}: HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("{url}: {message}")]
    Transport { url: String, message: String },
    #[error("{url}: expected {expected} bytes, received {received}")]
    Truncated {
        url: String,
        expected: u64,
        received: u64,
    },
    #[error("{url}: sha256 {actual} does not match {expected}")]
    HashMismatch {
        url: String,
        expected: &'static str,
        actual: String,
    },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .build()
}

/// `on_progress(received, total)` runs after every chunk; `total` comes from
/// Content-Length when the server sends one.
pub fn ensure_file(
    agent: &ureq::Agent,
    spec: &FileSpec,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<DownloadOutcome, DownloadError> {
    if spec.destination.is_file() {
        return Ok(DownloadOutcome::AlreadyPresent);
    }
    let parent = spec
        .destination
        .parent()
        .expect("model files always live inside the model directory");
    fs::create_dir_all(parent).map_err(io_error(parent))?;
    let partial = partial_path(&spec.destination);
    match download_to(agent, spec, &partial, on_progress) {
        Ok(bytes) => {
            fs::rename(&partial, &spec.destination).map_err(io_error(&spec.destination))?;
            Ok(DownloadOutcome::Downloaded { bytes })
        }
        Err(error) => {
            let _ = fs::remove_file(&partial);
            Err(error)
        }
    }
}

fn download_to(
    agent: &ureq::Agent,
    spec: &FileSpec,
    partial: &Path,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<u64, DownloadError> {
    let url = &spec.url;
    let response = agent.get(url).call().map_err(|error| match error {
        ureq::Error::Status(status, _) => DownloadError::Status {
            url: url.clone(),
            status,
        },
        ureq::Error::Transport(transport) => DownloadError::Transport {
            url: url.clone(),
            message: transport.to_string(),
        },
    })?;
    let expected_length = response
        .header("Content-Length")
        .and_then(|value| value.trim().parse::<u64>().ok());

    // Large writes: the socket hands over small chunks, and this file is gigabytes.
    let mut file =
        BufWriter::with_capacity(1 << 18, File::create(partial).map_err(io_error(partial))?);
    let mut reader = response.into_reader();
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut received = 0u64;
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(DownloadError::Transport {
                    url: url.clone(),
                    message: error.to_string(),
                })
            }
        };
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read]).map_err(io_error(partial))?;
        received += read as u64;
        on_progress(received, expected_length);
    }
    file.flush().map_err(io_error(partial))?;
    file.get_ref().sync_all().map_err(io_error(partial))?;

    if let Some(expected) = expected_length {
        if received != expected {
            return Err(DownloadError::Truncated {
                url: url.clone(),
                expected,
                received,
            });
        }
    }
    if let Some(expected) = spec.expected_sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected {
            return Err(DownloadError::HashMismatch {
                url: url.clone(),
                expected,
                actual,
            });
        }
    }
    Ok(received)
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .expect("destinations are file paths")
        .to_os_string();
    name.push(".part");
    destination.with_file_name(name)
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> DownloadError + '_ {
    move |source| DownloadError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    /// Serves one request with the given raw response bytes, then closes.
    fn serve_once(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            stream.write_all(&response).unwrap();
        });
        url
    }

    fn ok_response(declared_length: usize, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn spec(url: String, destination: PathBuf, expected_sha256: Option<&'static str>) -> FileSpec {
        FileSpec {
            url,
            destination,
            expected_sha256,
        }
    }

    fn fetch(spec: &FileSpec) -> Result<DownloadOutcome, DownloadError> {
        ensure_file(&agent(), spec, &mut |_, _| {})
    }

    // sha256("hello")
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn downloads_verifies_and_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("a").join("b").join("file.bin");
        let url = serve_once(ok_response(5, b"hello"));
        let outcome = fetch(&spec(url, destination.clone(), Some(HELLO_SHA256))).unwrap();
        assert_eq!(outcome, DownloadOutcome::Downloaded { bytes: 5 });
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello");
        assert!(!partial_path(&destination).exists());
    }

    #[test]
    fn existing_file_is_not_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        std::fs::write(&destination, b"old").unwrap();
        // Port 9 (discard) is closed: any request would fail.
        let url = "http://127.0.0.1:9/file.bin".to_string();
        let outcome = fetch(&spec(url, destination.clone(), None)).unwrap();
        assert_eq!(outcome, DownloadOutcome::AlreadyPresent);
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    }

    #[test]
    fn truncated_body_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        let url = serve_once(ok_response(10, b"hell"));
        assert!(fetch(&spec(url, destination.clone(), None)).is_err());
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
    }

    #[test]
    fn hash_mismatch_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        let url = serve_once(ok_response(5, b"jello"));
        let error = fetch(&spec(url, destination.clone(), Some(HELLO_SHA256))).unwrap_err();
        assert!(
            matches!(error, DownloadError::HashMismatch { .. }),
            "{error}"
        );
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
    }

    #[test]
    fn http_error_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        let url = serve_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        let error = fetch(&spec(url, destination.clone(), None)).unwrap_err();
        assert!(
            matches!(error, DownloadError::Status { status: 404, .. }),
            "{error}"
        );
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
    }

    #[test]
    fn progress_reports_bytes_received_and_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let url = serve_once(ok_response(5, b"hello"));
        let mut reports = Vec::new();
        ensure_file(
            &agent(),
            &spec(url, dir.path().join("file.bin"), None),
            &mut |received, total| reports.push((received, total)),
        )
        .unwrap();
        assert_eq!(reports.last(), Some(&(5, Some(5))));
    }
}
