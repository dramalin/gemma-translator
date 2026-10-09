//! One-shot HTTP server: accepts a single request, records it and replies
//! with canned bytes.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

pub struct CapturedRequest {
    pub head: String,
    pub body: Vec<u8>,
}

pub fn serve_once(response: Vec<u8>) -> (String, JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut data = Vec::new();
        let mut buffer = [0u8; 4096];
        let head_end = loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0, "client closed before sending headers");
            data.extend_from_slice(&buffer[..read]);
            if let Some(position) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).to_string();
        let content_length = head
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while data.len() < head_end + content_length {
            let read = stream.read(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            data.extend_from_slice(&buffer[..read]);
        }
        stream.write_all(&response).unwrap();
        stream.flush().unwrap();
        CapturedRequest {
            head,
            body: data[head_end..].to_vec(),
        }
    });
    (url, handle)
}

pub fn http_response(status_line: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status_line}\r\n");
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut bytes = response.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
