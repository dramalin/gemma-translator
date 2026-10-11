//! Every file the translator needs at run time, as download specs.

use std::path::Path;

use moonshine_sys::DependencyFile;
use translator_core::models::ModelPaths;

use crate::download::FileSpec;

/// Gemma 4 E2B, pinned to one Hugging Face commit (byte-identical to what
/// `litert-lm import` produced on 2026-10-09).
const GEMMA_URL: &str = "https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm/resolve/b3ca0d2f076785a8f4b2219ddbd2bdb99954eae1/gemma-4-E2B-it.litertlm";
const GEMMA_SHA256: &str = "181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c";
/// Silero VAD v5 (MIT), pinned to the v5.1.2 tag.
const VAD_URL: &str =
    "https://raw.githubusercontent.com/snakers4/silero-vad/v5.1.2/src/silero_vad/data/silero_vad.onnx";
const VAD_SHA256: &str = "2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f";

pub fn gemma_file(paths: &ModelPaths) -> FileSpec {
    FileSpec {
        url: GEMMA_URL.to_string(),
        destination: paths.gemma_file(),
        expected_sha256: Some(GEMMA_SHA256),
        expected_size: None,
        expected_crc32c: None,
    }
}

pub fn vad_file(paths: &ModelPaths) -> FileSpec {
    FileSpec {
        url: VAD_URL.to_string(),
        destination: paths.vad_file(),
        expected_sha256: Some(VAD_SHA256),
        expected_size: None,
        expected_crc32c: None,
    }
}

/// Download specs for the files of a Moonshine dependency manifest
/// (`moonshine_get_stt_dependencies` or `moonshine_get_tts_dependencies`),
/// whose names are relative to `directory`.
pub fn manifest_files<'a>(
    directory: &Path,
    files: impl IntoIterator<Item = &'a DependencyFile>,
) -> Vec<FileSpec> {
    files
        .into_iter()
        .map(|file| FileSpec {
            url: file.url.clone(),
            destination: directory.join(&file.name),
            expected_sha256: None,
            expected_size: file.size,
            expected_crc32c: (file.checksum_type == "crc32c" && !file.checksum.is_empty())
                .then(|| file.checksum.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
    fn manifest_files_carry_the_size_and_crc32c() {
        let file = |name: &str, checksum_type: &str| DependencyFile {
            name: name.to_string(),
            url: format!("https://download.moonshine.ai/model/x/{name}"),
            size: Some(5),
            checksum: "mnG7TA==".to_string(),
            checksum_type: checksum_type.to_string(),
        };
        let files = [file("encoder.ort", "crc32c"), file("a/b.json", "md5")];
        let specs = manifest_files(Path::new("/m/stt/x"), &files);
        assert_eq!(specs[0].destination, Path::new("/m/stt/x/encoder.ort"));
        assert_eq!(
            specs[0].url,
            "https://download.moonshine.ai/model/x/encoder.ort"
        );
        assert_eq!(specs[0].expected_size, Some(5));
        assert_eq!(specs[0].expected_crc32c.as_deref(), Some("mnG7TA=="));
        assert_eq!(specs[1].destination, Path::new("/m/stt/x/a/b.json"));
        assert_eq!(specs[1].expected_crc32c, None);
    }
}
