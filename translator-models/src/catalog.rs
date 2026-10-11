//! Every file the translator needs at run time, as download specs.

use std::collections::BTreeSet;

use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;

use crate::download::FileSpec;

/// Gemma 4 E2B, pinned to one Hugging Face commit (byte-identical to what
/// `litert-lm import` produced on 2026-10-09).
const GEMMA_URL: &str = "https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm/resolve/b3ca0d2f076785a8f4b2219ddbd2bdb99954eae1/gemma-4-E2B-it.litertlm";
const GEMMA_SHA256: &str = "181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c";
const TTS_BASE_URL: &str = "https://download.moonshine.ai/tts/";
/// Silero VAD v5 (MIT), pinned to the v5.1.2 tag.
const VAD_URL: &str =
    "https://raw.githubusercontent.com/snakers4/silero-vad/v5.1.2/src/silero_vad/data/silero_vad.onnx";
const VAD_SHA256: &str = "2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f";

pub fn gemma_file(paths: &ModelPaths) -> FileSpec {
    FileSpec {
        url: GEMMA_URL.to_string(),
        destination: paths.gemma_file(),
        expected_sha256: Some(GEMMA_SHA256),
    }
}

pub fn vad_file(paths: &ModelPaths) -> FileSpec {
    FileSpec {
        url: VAD_URL.to_string(),
        destination: paths.vad_file(),
        expected_sha256: Some(VAD_SHA256),
    }
}

pub fn stt_files(paths: &ModelPaths) -> Vec<FileSpec> {
    LanguageCode::ALL
        .into_iter()
        .flat_map(|language| {
            let model = language.stt_model();
            let directory = paths.stt_dir(model);
            model.components.iter().map(move |component| FileSpec {
                url: format!("{}/{component}", model.download_url),
                destination: directory.join(component),
                expected_sha256: None,
            })
        })
        .collect()
}

/// `keys` are asset paths relative to the TTS root, as returned by
/// `moonshine_get_tts_dependencies`.
pub fn tts_files(paths: &ModelPaths, keys: &BTreeSet<String>) -> Vec<FileSpec> {
    let root = paths.tts_root();
    keys.iter()
        .map(|key| FileSpec {
            url: format!("{TTS_BASE_URL}{}", encode_key(key)),
            destination: root.join(key),
            expected_sha256: None,
        })
        .collect()
}

/// Percent-encodes each path segment, matching the CDN's key layout.
fn encode_key(key: &str) -> String {
    key.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                        (byte as char).to_string()
                    }
                    _ => format!("%{byte:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use translator_core::models::{GEMMA_FILE_NAME, VAD_FILE_NAME};

    use super::*;

    #[test]
    fn gemma_url_ends_in_the_file_name_the_runtime_loads() {
        assert!(GEMMA_URL.ends_with(&format!("/{GEMMA_FILE_NAME}")));
    }

    #[test]
    fn gemma_is_pinned_by_commit_and_hash() {
        let file = gemma_file(&ModelPaths::new("/m"));
        assert!(file
            .url
            .contains("/resolve/b3ca0d2f076785a8f4b2219ddbd2bdb99954eae1/"));
        assert_eq!(
            file.destination,
            Path::new("/m/gemma/gemma-4-E2B-it.litertlm")
        );
        assert_eq!(
            file.expected_sha256,
            Some("181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c")
        );
    }

    #[test]
    fn vad_is_pinned_by_tag_and_hash() {
        let file = vad_file(&ModelPaths::new("/m"));
        assert!(file.url.contains("/v5.1.2/"));
        assert!(file.url.ends_with(&format!("/{VAD_FILE_NAME}")));
        assert_eq!(file.destination, Path::new("/m/vad/silero_vad.onnx"));
        assert_eq!(file.expected_sha256, Some(VAD_SHA256));
    }

    #[test]
    fn stt_files_cover_every_component_of_every_language() {
        let files = stt_files(&ModelPaths::new("/m"));
        assert_eq!(files.len(), 8 + 5 * 3);
        let korean_encoder = files
            .iter()
            .find(|file| file.destination == Path::new("/m/stt/tiny-ko/encoder_model.ort"))
            .unwrap();
        assert_eq!(
            korean_encoder.url,
            "https://download.moonshine.ai/model/tiny-ko/quantized/tiny-ko/encoder_model.ort"
        );
        assert!(files.iter().all(|file| file.expected_sha256.is_none()));
    }

    #[test]
    fn tts_files_map_keys_to_cdn_urls_under_the_tts_root() {
        let keys: BTreeSet<String> = ["kokoro/model.onnx", "ar_msa/piper-voices/a b.onnx"]
            .into_iter()
            .map(String::from)
            .collect();
        let files = tts_files(&ModelPaths::new("/m"), &keys);
        assert_eq!(files.len(), 2);
        assert_eq!(
            files[0].url,
            "https://download.moonshine.ai/tts/ar_msa/piper-voices/a%20b.onnx"
        );
        assert_eq!(
            files[0].destination,
            Path::new("/m/tts/ar_msa/piper-voices/a b.onnx")
        );
        assert_eq!(
            files[1].url,
            "https://download.moonshine.ai/tts/kokoro/model.onnx"
        );
    }
}
