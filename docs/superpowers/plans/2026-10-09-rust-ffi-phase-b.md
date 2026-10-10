# Rust FFI Inference Phase B (desktop-buildable part) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run speech recognition, translation and speech synthesis inside the Rust `translator` process. The binary calls `libmoonshine.so` and `liblitert-lm.so` directly, and a Rust `translator-models` tool prepares every model file. All of this is selectable by config and developed and tested on the x86_64 desktop.

**Architecture:**
- Two new binding crates, `moonshine-sys` and `litert-lm-sys`, load the vendor libraries at run time with `libloading`. They expose owned, single-thread handle types:
  - `moonshine-sys`: `Transcriber` and `Synthesizer`.
  - `litert-lm-sys`: `Engine` and `Conversation`.
- `translator-io` gains `native::NativeSpeech` and `native::NativeTranslator`. Each owns one worker thread that holds the engine handles and implements the existing `SpeechToText` / `TextToSpeech` / `Translator` traits by message passing. `JobRunner`, the controller and the UI are unchanged.
- `translator-models` is a new binary. It downloads Gemma, the six Moonshine STT models and the Moonshine TTS assets into one model directory. It uses `libmoonshine.so` to resolve the TTS file list.
- The HTTP path from phase A stays the default (`inference_backend = "http"`) until the Pi acceptance run passes.

**Tech Stack:**
- Rust stable (edition 2021).
- `libloading` 0.8, `serde_json` 1, `thiserror` 2.
- `ureq` 2.12 with the `tls` feature (rustls), in `translator-models` only.
- `sha2` 0.10.
- Native libraries come from the pinned wheels `moonshine-voice` 0.0.65 and `litert-lm-api` 0.13.1, unpacked with `unzip` and patched with `patchelf`.

**Spec:** `docs/superpowers/specs/2026-10-09-rust-frontend-design.md`. Relevant sections are §1, §3.1, §4.2, §5, §5.1, §6.2, §8, §9 and §11 phase B steps 7–9. In scope: everything that can be built and verified on the desktop. The Pi-only checks (glibc, aarch64 load test, clean-Pi model download, 8-hour run) are listed as a manual checklist at the end. The step 11 cleanup is listed at the end as follow-up work. Both are out of scope for task execution.

**Precondition:** Phase A tasks 1–13 are merged into `develop`, at or after commit `0e0a223`. Phase A step 6 (Pi deployment switch) does not have to be done for tasks 1–9.

## Global Constraints

- Workspace members after this plan: `translator-core`, `translator-io`, `translator-ui`, `moonshine-sys`, `litert-lm-sys`, `translator-models`.
- `translator-core` must not depend on `*-sys`, `cpal`, `ureq`, `slint` or `libloading`.
- `make test` (no native libraries, no models, no network) must stay green. Every test that needs a native library or model files is `#[ignore]`d and runs through `make test-ffi`. The 1000-iteration leak test runs only through `make test-soak`.
- Pinned versions: `moonshine-voice` 0.0.65 and `litert-lm-api` 0.13.1. Every FFI signature in this plan was copied from those wheels' ctypes declarations (`moonshine_voice/moonshine_api.py`, `litert_lm/_ffi.py`). When upgrading, re-check every signature.
- Moonshine header version passed to every create call: `20000`.
- Model cache: each speech thread keeps at most 2 transcribers and 2 synthesizers (LRU), the same as `MAX_MODELS = 2` in `server.py`.
- Each translation uses a new litert-lm conversation (spec §6.3).
- Every `extern "C"` callback body runs inside `catch_unwind`, so a panic never crosses the FFI boundary.
- No TODOs or placeholders. Only add functions the change actually calls.
- Commit messages in imperative mood.
- Do not touch `frontend/`, `backend/`, `start.sh`, `setup.sh`, `deploy-pi.sh` or `deploy/`.
- The working tree has the user's uncommitted `.gitignore` edit and an untracked `package-lock.json`. Never stage either. Ignore generated directories with a `.gitignore` containing `*` inside the directory itself.
- Naming:
  - Booleans are prefixed `is`/`has`/`should`/`can`.
  - Environment variables are `SCREAMING_SNAKE_CASE`: `TRANSLATOR_NATIVE_LIB_DIR` and `TRANSLATOR_MODEL_DIR`, used by tests only.
  - TOML keys are snake_case.

### Rulings that refine the spec (recorded so reviewers do not flag them)

1. **Run-time loading instead of link-time rpath (spec §8.4).** The sys crates `dlopen` the libraries with `libloading` from `<directory of the translator executable>/lib`, so `make test`, `make run-mock` and cross-builds need no native library. `patchelf --set-rpath '$ORIGIN'` is still applied to `libmoonshine.so`, because `dlopen` resolves the library's own `NEEDED` entry (`libonnxruntime-13ab8084.so.1`) through that library's RPATH.
2. **`moonshine_get_tts_voices` is not bound** (spec §8.1 lists it). Voice validity is already covered by `moonshine_get_tts_dependencies` (deployment time) and by the synthesizer create call (run time). Binding a function nobody calls breaks the "no unused helpers" rule.
3. **No litert-lm session config.** Python sets an empty session config. A probe on 2026-10-09 showed that streaming, cancel and engine reuse all work without one.
4. **Thinking content.** Going by the model's channel config (`channels { channel_name: "thought" }`, printed when the engine loads) and `Message.channels` in `_messages.py`, the litert-lm conversation API puts thinking text in a message's `channels` field, not in `content`. The 2026-10-09 probe saw no thinking at all. `litert_lm_sys::chunk_text` keeps only `content[]` items with `"type": "text"`. `clean_model_reply` still runs on each sentence.
5. **Backend switch.** The new config key `inference_backend` is `"http"` (default) or `"native"`. The default changes, and the HTTP clients are deleted, only in step 11 after the Pi acceptance run. Until then both paths must work.
6. **Download integrity.**
   - Gemma is pinned to Hugging Face commit `b3ca0d2f076785a8f4b2219ddbd2bdb99954eae1` and checked against sha256 `181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c` (2 588 147 712 bytes). This is the same file `litert-lm import` produced: `cmp` was run against the imported copy.
   - Moonshine's CDN publishes no hashes, so those files are checked against `Content-Length`.
   - Every file is written to `<name>.part` and renamed only after it is verified.
   - Downloads do not resume. A failed file is fetched again from the start on the next run.
7. **TTS language tags and asset lists were verified on 2026-10-09** with the installed wheel. The tags are `en_us`, `ar_msa`, `es_es`, `ja_jp`, `zh_hans` (voice `kokoro_zf_xiaoxiao`) and `ko_kr`.
   - The dependency lists do not depend on what is already present under `g2p_root`.
   - The `ja_jp` assets live under `ja/`, and `ko_kr`'s under `ko/`.
8. **Threads (spec §6.2).**
   - One speech thread owns all Moonshine handles, for both STT and TTS.
   - One model thread owns the litert-lm engine.
   - Job threads talk to both over `mpsc` channels. A job that is cancelled returns at once and leaves the worker to finish cancelling.
9. **Model directory layout** (`model_dir`, default `/var/lib/gemma-translator/models`):
   ```
   gemma/gemma-4-E2B-it.litertlm
   cache/                         litert-lm compile cache (xnnpack), must be writable
   stt/<model name>/<component>   e.g. stt/medium-streaming-en/encoder.ort
   tts/<asset key>                g2p_root, e.g. tts/kokoro/model.onnx
   ```
10. **Korean STT model name.** Python calls it `base-ko` but downloads `tiny-ko` (arch `TINY`). The table uses the true name, `tiny-ko`.
11. **Preloading.** The native speech thread loads both lanes' STT and TTS models at startup (spec §3 "變更" row). The matching `server.py` change belongs to phase A step 6 and is not part of this plan.

## Review Focus

1. **Native libraries missing or in the wrong directory.** Every job ends with `Error: …` naming the expected path, and the UI keeps running. Tests: Task 7 `missing_library_fails_every_request`, Task 8 `missing_library_fails_translation`.
2. **A language's model files are missing** (for example, `translator-models` was interrupted). Only jobs for that language fail, with a message that says to run `translator-models`. Other languages still work. Test: Task 7 `missing_model_names_the_tool`.
3. **Pressing record while a translation is streaming.** The job thread returns within 100 ms, nothing more is spoken, and the next translation on the same engine works. Tests: Task 8 `caller_cancel_returns_promptly_and_flags_worker`, Task 6 `cancel_ends_stream_and_engine_stays_usable`.
4. **A download that is cut off, corrupted or answered with 404.** No file appears at the final path and no `.part` file is left behind, and the next run fetches the file again. Tests: Task 4 `truncated_body_leaves_no_file`, `hash_mismatch_leaves_no_file`, `http_error_is_reported`.
5. **Hours of use.** Repeated transcriptions do not grow memory, which answers the open question in spec §11.1 of who frees `TranscriptC`. Test: Task 5 `soak_repeated_transcription_does_not_leak`.

---

## File Structure

```
Cargo.toml                               add three workspace members
Makefile                                 native-libs / models / test-ffi / test-soak / run-native
scripts/fetch-native-libs.sh             download pinned wheels, verify sha256, unpack .so, patchelf
native/<arch>/lib/                       generated (self-ignoring), not committed
models/                                  generated dev model dir (self-ignoring), not committed
translator-core/
  src/lib.rs                             + pub mod models
  src/models.rs                          NEW: SttModelArch, SttModel, TtsVoice, ModelPaths, per-language table
  src/pipeline.rs                        + PipelineError::Engine
  src/config.rs                          + InferenceBackend, inference_backend, model_dir
moonshine-sys/
  Cargo.toml
  src/lib.rs                             module list + re-exports
  src/ffi.rs                             #[repr(C)] structs, fn-pointer types, constants
  src/library.rs                         MoonshineLibrary (dlopen + symbols), MoonshineError, OptionArray, tts_dependencies
  src/transcriber.rs                     ModelArch, Transcriber
  src/synthesizer.rs                     Synthesizer
  tests/library.rs                       load/version/error/dependency tests
  tests/engines.rs                       transcriber/synthesizer tests (+ soak)
litert-lm-sys/
  Cargo.toml
  src/lib.rs                             module list + re-exports
  src/ffi.rs                             fn-pointer types, stream callback type
  src/library.rs                         LiteRtLmLibrary, LiteRtLmError
  src/engine.rs                          Engine, Conversation, StreamEnd, chunk_text, stream callback
  tests/engine.rs                        stream / cancel / reuse tests
translator-models/
  Cargo.toml
  src/main.rs                            CLI, run()
  src/catalog.rs                         what to download: Gemma, STT, TTS → FileSpec list
  src/download.rs                        ensure_file: .part, Content-Length, sha256, rename
translator-io/
  Cargo.toml                             + moonshine-sys, litert-lm-sys
  src/lib.rs                             + pub mod native
  src/native/mod.rs                      default_lib_dir, engine_error
  src/native/lru.rs                      LruCache
  src/native/speech.rs                   NativeSpeech (speech thread)
  src/native/translator.rs               NativeTranslator (model thread)
  tests/native_services.rs               speech + translator services against real libraries (ignored)
  tests/native_pipeline.rs               end-to-end JobRunner over native services (ignored)
translator-ui/src/app.rs                 choose HTTP or native services from config
README.md, CLAUDE.md                     native inference section
```

---

### Task 1: Model table, model paths, engine error and config keys (translator-core)

**Files:**
- Create: `translator-core/src/models.rs`
- Modify: `translator-core/src/lib.rs`, `translator-core/src/pipeline.rs`, `translator-core/src/config.rs`

**Interfaces:**
- Consumes: `LanguageCode` and `LanguageCode::ALL` (`translator-core/src/language.rs`).
- Produces:
  - `models::SttModelArch { Tiny, Base, MediumStreaming }`
  - `models::SttModel { name: &'static str, arch: SttModelArch, download_url: &'static str, components: &'static [&'static str] }`
  - `models::TtsVoice { language_tag: &'static str, voice: Option<&'static str> }`
  - `TtsVoice::synthesizer_options<'a>(&self, tts_root: &'a str) -> Vec<(&'static str, &'a str)>`
  - `LanguageCode::stt_model(self) -> SttModel`
  - `LanguageCode::tts_voice(self) -> TtsVoice`
  - `models::ModelPaths` (Clone), with:
    - `ModelPaths::new(root: impl Into<PathBuf>)`
    - `.root() -> &Path`
    - `.gemma_file() -> PathBuf`
    - `.llm_cache_dir() -> PathBuf`
    - `.stt_dir(SttModel) -> PathBuf`
    - `.tts_root() -> PathBuf`
  - `models::GEMMA_FILE_NAME`
  - `PipelineError::Engine { service: &'static str, message: String }`
  - `config::InferenceBackend { Http, Native }`
  - `Config.inference_backend`
  - `Config.model_dir: PathBuf`

- [ ] **Step 1: Write the failing tests**

Create `translator-core/src/models.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LanguageCode;

    #[test]
    fn every_language_has_an_stt_model_matching_moonshine_download_py() {
        let ko = LanguageCode::Ko.stt_model();
        assert_eq!(ko.name, "tiny-ko");
        assert_eq!(ko.arch, SttModelArch::Tiny);
        assert_eq!(
            ko.download_url,
            "https://download.moonshine.ai/model/tiny-ko/quantized/tiny-ko"
        );
        assert_eq!(
            ko.components,
            ["encoder_model.ort", "decoder_model_merged.ort", "tokenizer.bin"]
        );

        let en = LanguageCode::En.stt_model();
        assert_eq!(en.name, "medium-streaming-en");
        assert_eq!(en.arch, SttModelArch::MediumStreaming);
        assert_eq!(
            en.download_url,
            "https://download.moonshine.ai/model/medium-streaming-en/quantized"
        );
        assert_eq!(en.components.len(), 8);
        assert!(en.components.contains(&"decoder_kv_with_attention.ort"));

        for language in [LanguageCode::Ar, LanguageCode::Es, LanguageCode::Ja, LanguageCode::Zh] {
            let model = language.stt_model();
            assert_eq!(model.arch, SttModelArch::Base);
            assert_eq!(model.name, format!("base-{}", language.code()));
            assert_eq!(
                model.download_url,
                format!(
                    "https://download.moonshine.ai/model/base-{0}/quantized/base-{0}",
                    language.code()
                )
            );
        }
    }

    #[test]
    fn tts_tags_and_voices_match_the_verified_catalog() {
        let tags: Vec<_> = LanguageCode::ALL
            .iter()
            .map(|language| language.tts_voice().language_tag)
            .collect();
        assert_eq!(tags, ["ar_msa", "en_us", "es_es", "ja_jp", "zh_hans", "ko_kr"]);
        assert_eq!(LanguageCode::Zh.tts_voice().voice, Some("kokoro_zf_xiaoxiao"));
        assert_eq!(LanguageCode::En.tts_voice().voice, None);
    }

    #[test]
    fn synthesizer_options_carry_root_and_optional_voice() {
        assert_eq!(
            LanguageCode::Zh.tts_voice().synthesizer_options("/m/tts"),
            [("g2p_root", "/m/tts"), ("voice", "kokoro_zf_xiaoxiao")]
        );
        assert_eq!(
            LanguageCode::Es.tts_voice().synthesizer_options("/m/tts"),
            [("g2p_root", "/m/tts")]
        );
    }

    #[test]
    fn model_paths_follow_the_documented_layout() {
        let paths = ModelPaths::new("/models");
        assert_eq!(paths.root(), std::path::Path::new("/models"));
        assert_eq!(
            paths.gemma_file(),
            std::path::Path::new("/models/gemma/gemma-4-E2B-it.litertlm")
        );
        assert_eq!(paths.llm_cache_dir(), std::path::Path::new("/models/cache"));
        assert_eq!(
            paths.stt_dir(LanguageCode::Ko.stt_model()),
            std::path::Path::new("/models/stt/tiny-ko")
        );
        assert_eq!(paths.tts_root(), std::path::Path::new("/models/tts"));
    }
}
```

Add to `translator-core/src/lib.rs` (keep alphabetical order):

```rust
pub mod models;
```

Append to the `tests` module in `translator-core/src/config.rs`:

```rust
    #[test]
    fn inference_backend_and_model_dir_have_defaults_and_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let defaults = Config::load(&path).unwrap();
        assert_eq!(defaults.inference_backend, InferenceBackend::Http);
        assert_eq!(
            defaults.model_dir,
            std::path::PathBuf::from("/var/lib/gemma-translator/models")
        );

        std::fs::write(
            &path,
            "inference_backend = \"native\"\nmodel_dir = \"/home/pi/models\"\n",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.inference_backend, InferenceBackend::Native);
        assert_eq!(config.model_dir, std::path::PathBuf::from("/home/pi/models"));
    }
```

Append to `translator-core/src/pipeline.rs`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-core`
Expected: compile errors: `SttModelArch`, `ModelPaths`, `stt_model`, `InferenceBackend` and `PipelineError::Engine` are not defined.

- [ ] **Step 3: Implement**

Prepend to `translator-core/src/models.rs`, above the test module:

```rust
//! Spec §3.1: which Moonshine model and voice each language uses, and where
//! `translator-models` puts every model file. Single source for the native
//! inference path and the model tool.

use std::path::{Path, PathBuf};

use crate::language::LanguageCode;

/// File name of the Gemma 4 E2B model inside `<model_dir>/gemma/`.
pub const GEMMA_FILE_NAME: &str = "gemma-4-E2B-it.litertlm";

/// Moonshine model architectures used by the six languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttModelArch {
    Tiny,
    Base,
    MediumStreaming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SttModel {
    /// Directory name under `<model_dir>/stt/`.
    pub name: &'static str,
    pub arch: SttModelArch,
    /// Each component is fetched from `<download_url>/<component>`.
    pub download_url: &'static str,
    pub components: &'static [&'static str],
}

/// Moonshine TTS language tag plus an optional voice override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtsVoice {
    pub language_tag: &'static str,
    pub voice: Option<&'static str>,
}

/// Components of a non-streaming model (`download.py::get_components_for_model_info`).
const NON_STREAMING_COMPONENTS: &[&str] =
    &["encoder_model.ort", "decoder_model_merged.ort", "tokenizer.bin"];

/// Components of the English streaming model, including the attention decoder
/// that `download.py` adds for English.
const ENGLISH_STREAMING_COMPONENTS: &[&str] = &[
    "adapter.ort",
    "cross_kv.ort",
    "decoder_kv.ort",
    "encoder.ort",
    "frontend.ort",
    "streaming_config.json",
    "tokenizer.bin",
    "decoder_kv_with_attention.ort",
];

impl TtsVoice {
    /// Options for `moonshine_create_tts_synthesizer_from_files` and
    /// `moonshine_get_tts_dependencies`.
    pub fn synthesizer_options<'a>(&self, tts_root: &'a str) -> Vec<(&'static str, &'a str)> {
        let mut options = vec![("g2p_root", tts_root)];
        if let Some(voice) = self.voice {
            options.push(("voice", voice));
        }
        options
    }
}

impl LanguageCode {
    pub fn stt_model(self) -> SttModel {
        let base = |name: &'static str, download_url: &'static str| SttModel {
            name,
            arch: SttModelArch::Base,
            download_url,
            components: NON_STREAMING_COMPONENTS,
        };
        match self {
            LanguageCode::Ar => base(
                "base-ar",
                "https://download.moonshine.ai/model/base-ar/quantized/base-ar",
            ),
            LanguageCode::En => SttModel {
                name: "medium-streaming-en",
                arch: SttModelArch::MediumStreaming,
                download_url: "https://download.moonshine.ai/model/medium-streaming-en/quantized",
                components: ENGLISH_STREAMING_COMPONENTS,
            },
            LanguageCode::Es => base(
                "base-es",
                "https://download.moonshine.ai/model/base-es/quantized/base-es",
            ),
            LanguageCode::Ja => base(
                "base-ja",
                "https://download.moonshine.ai/model/base-ja/quantized/base-ja",
            ),
            LanguageCode::Zh => base(
                "base-zh",
                "https://download.moonshine.ai/model/base-zh/quantized/base-zh",
            ),
            LanguageCode::Ko => SttModel {
                name: "tiny-ko",
                arch: SttModelArch::Tiny,
                download_url: "https://download.moonshine.ai/model/tiny-ko/quantized/tiny-ko",
                components: NON_STREAMING_COMPONENTS,
            },
        }
    }

    pub fn tts_voice(self) -> TtsVoice {
        let (language_tag, voice) = match self {
            LanguageCode::Ar => ("ar_msa", None),
            LanguageCode::En => ("en_us", None),
            LanguageCode::Es => ("es_es", None),
            LanguageCode::Ja => ("ja_jp", None),
            LanguageCode::Zh => ("zh_hans", Some("kokoro_zf_xiaoxiao")),
            LanguageCode::Ko => ("ko_kr", None),
        };
        TtsVoice { language_tag, voice }
    }
}

/// Layout of the model directory filled by `translator-models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    root: PathBuf,
}

impl ModelPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ModelPaths { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn gemma_file(&self) -> PathBuf {
        self.root.join("gemma").join(GEMMA_FILE_NAME)
    }

    /// Writable directory for litert-lm's compiled-model cache.
    pub fn llm_cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn stt_dir(&self, model: SttModel) -> PathBuf {
        self.root.join("stt").join(model.name)
    }

    /// Moonshine's `g2p_root`: TTS asset keys are paths relative to it.
    pub fn tts_root(&self) -> PathBuf {
        self.root.join("tts")
    }
}
```

In `translator-core/src/pipeline.rs`, add this variant to `PipelineError` after `InvalidResponse`:

```rust
    #[error("{service} failed: {message}")]
    Engine {
        service: &'static str,
        message: String,
    },
```

In `translator-core/src/config.rs`:

- Add `use std::path::PathBuf;` (merge it into the existing `std::path` import).
- Add this enum above `pub struct Config`:

```rust
/// Where inference runs: the phase A HTTP services, or in-process through FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InferenceBackend {
    Http,
    Native,
}
```

- Add these fields to `Config`, after `llm_model`:

```rust
    pub inference_backend: InferenceBackend,
    pub model_dir: PathBuf,
```

- Add these fields to `Config::default()`, after `llm_model`:

```rust
            inference_backend: InferenceBackend::Http,
            model_dir: PathBuf::from("/var/lib/gemma-translator/models"),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p translator-core && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all tests pass, no warnings. Other crates still build, because `Config` is constructed with `..Config::default()` everywhere. If a match on `PipelineError` elsewhere stops compiling, add an `Engine` arm with the same handling as `InvalidResponse`.

- [ ] **Step 5: Commit**

```bash
git add translator-core
git commit -m "Add the per-language model table, model paths and native backend config"
```

---

### Task 2: Fetch the native libraries

**Files:**
- Create: `scripts/fetch-native-libs.sh` (mode 755)
- Modify: `Makefile`

**Interfaces:**
- Produces:
  - `native/<arch>/lib/` containing `libmoonshine.so` (RPATH `$ORIGIN`), `libonnxruntime-*.so.1` and `liblitert-lm.so`. `native/` contains a `.gitignore` with `*`.
  - Make variables `NATIVE_ARCH` and `NATIVE_LIB_DIR`, and target `native-libs`.

- [ ] **Step 1: Install the tool and confirm the starting state**

Run: `sudo apt-get install -y patchelf unzip && ls native 2>&1`
Expected: `ls: cannot access 'native'`. (If `sudo` is not available to the agent, ask the user to run `! sudo apt-get install -y patchelf unzip`.)

- [ ] **Step 2: Write the script**

```bash
#!/bin/bash
# Downloads the pinned moonshine-voice and litert-lm-api wheels, verifies their
# sha256, and unpacks the native libraries into native/<arch>/lib/.
# Usage: scripts/fetch-native-libs.sh [x86_64|aarch64]   (default: this machine)
set -euo pipefail

ARCH="${1:-$(uname -m)}"
PYPI=https://files.pythonhosted.org/packages
case "$ARCH" in
    x86_64)
        MOONSHINE_URL="$PYPI/de/b9/f45749837ecb75a7de2ffed261c0ec12333f66e2e7696b15655717a0e28f/moonshine_voice-0.0.65-py3-none-manylinux_2_34_x86_64.whl"
        MOONSHINE_SHA256=946e0a056dd3a13ee8cc28dec84c0f01676743a841bae5e6c52a87483b10241b
        LITERT_URL="$PYPI/36/27/bb0c2e084d59938bc12f960db50e7e8e056337ed286f31468aaa39bc9d9a/litert_lm_api-0.13.1-py3-none-manylinux_2_27_x86_64.whl"
        LITERT_SHA256=b981d55e057be9664f30670d5c1faa6e738ad516a2617dbdac10dc6b5b409cca
        ;;
    aarch64)
        MOONSHINE_URL="$PYPI/c4/b7/fe75106ffb8001ef43da1c4b75968c82f0322d9880dd3efccca1f315b556/moonshine_voice-0.0.65-py3-none-manylinux_2_34_aarch64.whl"
        MOONSHINE_SHA256=e6200c95950f6385a346b220ad046e586221d9a38db30152fa0cd12a809e9e86
        LITERT_URL="$PYPI/12/a7/c23db30dc19ad07a188eeeab48fd8d676554e8647dac6e9dbf62be0f82f1/litert_lm_api-0.13.1-py3-none-manylinux_2_27_aarch64.whl"
        LITERT_SHA256=4092c4f7ad75fb23d83bc5b00a9aa6896a19d75f148515efeef0d079ba22ef74
        ;;
    *)
        echo "usage: $0 [x86_64|aarch64]" >&2
        exit 2
        ;;
esac

for tool in curl sha256sum unzip patchelf; do
    if ! command -v "$tool" >/dev/null; then
        echo "error: '$tool' is required (sudo apt-get install -y $tool)" >&2
        exit 1
    fi
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/native/$ARCH/lib"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fetch() {
    local url="$1" sha256="$2" out="$3"
    curl -fL --retry 3 -o "$out" "$url"
    echo "$sha256  $out" | sha256sum -c --quiet -
}

fetch "$MOONSHINE_URL" "$MOONSHINE_SHA256" "$WORK/moonshine.whl"
fetch "$LITERT_URL" "$LITERT_SHA256" "$WORK/litert.whl"

rm -rf "$DEST"
mkdir -p "$DEST"
printf '*\n' > "$ROOT/native/.gitignore"
unzip -q -j "$WORK/moonshine.whl" 'moonshine_voice/libmoonshine.so' \
    'moonshine_voice.libs/libonnxruntime-*.so.1' -d "$DEST"
unzip -q -j "$WORK/litert.whl" 'litert_lm/liblitert-lm.so' -d "$DEST"
# libmoonshine.so looks for onnxruntime in ../moonshine_voice.libs; point it at its own directory.
patchelf --set-rpath '$ORIGIN' "$DEST/libmoonshine.so"

echo "Native libraries for $ARCH in $DEST:"
ls -1 "$DEST"
```

- [ ] **Step 3: Add the Makefile targets**

Add after the `CARGO`/`PI_TARGET` variables:

```make
NATIVE_ARCH    := $(shell uname -m)
NATIVE_LIB_DIR := $(CURDIR)/native/$(NATIVE_ARCH)/lib
```

Add before `## fmt:`:

```make
## native-libs: download libmoonshine / liblitert-lm for this machine into native/<arch>/lib
.PHONY: native-libs
native-libs:
	scripts/fetch-native-libs.sh $(NATIVE_ARCH)
```

- [ ] **Step 4: Run it and verify the libraries resolve**

Run: `make native-libs && ldd native/x86_64/lib/libmoonshine.so native/x86_64/lib/liblitert-lm.so | grep -c 'not found'; readelf -d native/x86_64/lib/libmoonshine.so | grep -E 'RPATH|RUNPATH'; git status --short`
Expected:
- Three files are listed.
- The `grep -c` prints `0`.
- RPATH is `[$ORIGIN]`.
- `git status` shows no `native/` entries, only the user's pre-existing `.gitignore` and `package-lock.json`, plus the two new tracked changes.

If `liblitert-lm.so` reports `libvulkan.so.1 => not found`, run `sudo apt-get install -y libvulkan1` and re-check.

- [ ] **Step 5: Commit**

```bash
git add scripts/fetch-native-libs.sh Makefile
git commit -m "Add a script that fetches the pinned native inference libraries"
```

---

### Task 3: `moonshine-sys` library loading and TTS dependency lookup

**Files:**
- Create: `moonshine-sys/Cargo.toml`, `moonshine-sys/src/lib.rs`, `moonshine-sys/src/ffi.rs`, `moonshine-sys/src/library.rs`, `moonshine-sys/tests/library.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Consumes: `native/<arch>/lib` (Task 2), through the environment variable `TRANSLATOR_NATIVE_LIB_DIR` in the ignored tests.
- Produces:
  - `MoonshineLibrary::load(directory: &Path) -> Result<Arc<MoonshineLibrary>, MoonshineError>`
  - `MoonshineLibrary::version(&self) -> i32`
  - `MoonshineLibrary::error_message(&self, code: i32) -> String`
  - `MoonshineLibrary::tts_dependencies(&self, language_tag: &str, options: &[(&str, &str)]) -> Result<Vec<String>, MoonshineError>`
  - Crate-private, used by Task 5:
    - `MoonshineLibrary::call_error(&self, function: &'static str, code: i32) -> MoonshineError`
    - `OptionArray`
    - `c_string`
    - `free_native`
    - the fields of `MoonshineLibrary`
  - `MoonshineError` variants `Load`, `Symbol`, `Call`, `InvalidArgument`, `InvalidOutput`
  - `moonshine_sys::LIBRARY_FILE_NAME`

- [ ] **Step 1: Create the crate and write the failing tests**

Add `"moonshine-sys"` to `members` in the root `Cargo.toml`.

`moonshine-sys/Cargo.toml`:

```toml
[package]
name = "moonshine-sys"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
libloading = "0.8"
serde_json = "1"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

`moonshine-sys/src/lib.rs`:

```rust
//! Run-time bindings to `libmoonshine.so` from the moonshine-voice 0.0.65 wheel.
//! Signatures mirror the wheel's `moonshine_api.py` ctypes declarations.

mod ffi;
mod library;

pub use library::{MoonshineError, MoonshineLibrary, LIBRARY_FILE_NAME};
```

`moonshine-sys/tests/library.rs`:

```rust
use std::path::PathBuf;

use moonshine_sys::{MoonshineError, MoonshineLibrary};

fn native_lib_dir() -> PathBuf {
    PathBuf::from(
        std::env::var_os("TRANSLATOR_NATIVE_LIB_DIR")
            .expect("set TRANSLATOR_NATIVE_LIB_DIR (make test-ffi does)"),
    )
}

#[test]
fn missing_library_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let error = MoonshineLibrary::load(dir.path()).err().unwrap();
    assert!(matches!(error, MoonshineError::Load { .. }));
    assert!(error.to_string().contains("libmoonshine.so"), "{error}");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn reports_header_version_and_error_strings() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    assert_eq!(library.version(), 20000);
    assert_eq!(library.error_message(-3), "Invalid argument");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn tts_dependencies_match_the_lists_verified_with_python() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let root = root.path().to_str().unwrap();

    let korean = library.tts_dependencies("ko_kr", &[("g2p_root", root)]).unwrap();
    assert_eq!(
        korean,
        [
            "ko/dict.tsv",
            "ko/piper-voices/ko_KR-melotts-medium.onnx",
            "ko/piper-voices/ko_KR-melotts-medium.onnx.json",
        ]
    );

    let chinese = library
        .tts_dependencies(
            "zh_hans",
            &[("g2p_root", root), ("voice", "kokoro_zf_xiaoxiao")],
        )
        .unwrap();
    assert_eq!(
        chinese.last().map(String::as_str),
        Some("kokoro/voices/zf_xiaoxiao.kokorovoice")
    );
    assert!(chinese.contains(&"zh_hans/dict.tsv".to_string()));

    let english = library.tts_dependencies("en_us", &[("g2p_root", root)]).unwrap();
    assert!(english.contains(&"kokoro/voices/af_heart.kokorovoice".to_string()));
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn unknown_tts_language_is_an_error() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let error = library
        .tts_dependencies("xx_nowhere", &[("g2p_root", root.path().to_str().unwrap())])
        .unwrap_err();
    assert!(matches!(error, MoonshineError::Call { .. }), "{error}");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn option_with_nul_byte_is_rejected_before_the_call() {
    let library = MoonshineLibrary::load(&native_lib_dir()).unwrap();
    let error = library
        .tts_dependencies("en_us", &[("g2p_root", "bad\0path")])
        .unwrap_err();
    assert!(matches!(error, MoonshineError::InvalidArgument(_)));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p moonshine-sys`
Expected: compile error, because `library` and `ffi` do not exist yet.

- [ ] **Step 3: Implement**

`moonshine-sys/src/ffi.rs`:

```rust
//! C declarations, copied from moonshine-voice 0.0.65 `moonshine_api.py`.

use std::os::raw::{c_char, c_void};

/// `MOONSHINE_HEADER_VERSION`; passed to every create call.
pub const MOONSHINE_HEADER_VERSION: i32 = 20000;
pub const MOONSHINE_ERROR_NONE: i32 = 0;

/// `moonshine_option_t`
#[repr(C)]
pub struct MoonshineOption {
    pub name: *const c_char,
    pub value: *const c_char,
}

/// `transcript_word_t`
#[repr(C)]
pub struct TranscriptWord {
    pub text: *const c_char,
    pub start: f32,
    pub end: f32,
    pub confidence: f32,
}

/// `transcript_line_t`: every field is declared so the array stride is right.
#[repr(C)]
pub struct TranscriptLine {
    pub text: *const c_char,
    pub audio_data: *const f32,
    pub audio_data_count: usize,
    pub start_time: f32,
    pub duration: f32,
    pub id: u64,
    pub is_complete: i8,
    pub is_updated: i8,
    pub is_new: i8,
    pub has_text_changed: i8,
    pub has_speaker_id: i8,
    pub speaker_id: u64,
    pub speaker_index: u32,
    pub last_transcription_latency_ms: u32,
    pub words: *const TranscriptWord,
    pub word_count: u64,
}

/// `transcript_t`
#[repr(C)]
pub struct Transcript {
    pub lines: *const TranscriptLine,
    pub line_count: u64,
}

pub type GetVersionFn = unsafe extern "C" fn() -> i32;
pub type ErrorToStringFn = unsafe extern "C" fn(code: i32) -> *const c_char;
pub type LoadTranscriberFromFilesFn = unsafe extern "C" fn(
    path: *const c_char,
    model_arch: u32,
    options: *const MoonshineOption,
    options_count: u64,
    header_version: i32,
) -> i32;
pub type FreeTranscriberFn = unsafe extern "C" fn(handle: i32);
pub type TranscribeWithoutStreamingFn = unsafe extern "C" fn(
    handle: i32,
    audio: *const f32,
    audio_length: u64,
    sample_rate: i32,
    flags: u32,
    out_transcript: *mut *mut Transcript,
) -> i32;
pub type CreateTtsSynthesizerFromFilesFn = unsafe extern "C" fn(
    language: *const c_char,
    filenames: *const *const c_char,
    filenames_count: u64,
    options: *const MoonshineOption,
    options_count: u64,
    header_version: i32,
) -> i32;
pub type FreeTtsSynthesizerFn = unsafe extern "C" fn(handle: i32);
pub type GetTtsDependenciesFn = unsafe extern "C" fn(
    languages: *const c_char,
    options: *const MoonshineOption,
    options_count: u64,
    out_json: *mut *mut c_char,
) -> i32;
pub type TextToSpeechFn = unsafe extern "C" fn(
    handle: i32,
    text: *const c_char,
    options: *const MoonshineOption,
    options_count: u64,
    out_audio: *mut *mut f32,
    out_size: *mut u64,
    out_sample_rate: *mut i32,
) -> i32;

extern "C" {
    /// libc `free`: the C API returns `malloc`ed strings and audio buffers.
    pub fn free(pointer: *mut c_void);
}
```

`moonshine-sys/src/library.rs`:

```rust
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;

use crate::ffi::{self, MoonshineOption, MOONSHINE_ERROR_NONE};

pub const LIBRARY_FILE_NAME: &str = "libmoonshine.so";

#[derive(Debug, thiserror::Error)]
pub enum MoonshineError {
    #[error("cannot load {path}: {message}")]
    Load { path: PathBuf, message: String },
    #[error("{name} missing from libmoonshine.so: {message}")]
    Symbol { name: String, message: String },
    #[error("{function} failed ({code}): {message}")]
    Call {
        function: &'static str,
        code: i32,
        message: String,
    },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("unexpected output from {function}: {message}")]
    InvalidOutput {
        function: &'static str,
        message: String,
    },
}

/// The loaded library and every function this crate calls. Function pointers
/// stay valid while `_library` is alive, which is as long as `self`.
pub struct MoonshineLibrary {
    get_version: ffi::GetVersionFn,
    error_to_string: ffi::ErrorToStringFn,
    pub(crate) load_transcriber_from_files: ffi::LoadTranscriberFromFilesFn,
    pub(crate) free_transcriber: ffi::FreeTranscriberFn,
    pub(crate) transcribe_without_streaming: ffi::TranscribeWithoutStreamingFn,
    pub(crate) create_tts_synthesizer_from_files: ffi::CreateTtsSynthesizerFromFilesFn,
    pub(crate) free_tts_synthesizer: ffi::FreeTtsSynthesizerFn,
    get_tts_dependencies: ffi::GetTtsDependenciesFn,
    pub(crate) text_to_speech: ffi::TextToSpeechFn,
    _library: libloading::Library,
}

/// Looks up `name` (NUL-terminated) and copies out the function pointer.
///
/// # Safety
/// `T` must be the exact C signature of the symbol.
unsafe fn symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T, MoonshineError> {
    library
        .get::<T>(name)
        .map(|found| *found)
        .map_err(|error| MoonshineError::Symbol {
            name: String::from_utf8_lossy(&name[..name.len() - 1]).into_owned(),
            message: error.to_string(),
        })
}

impl MoonshineLibrary {
    pub fn load(directory: &Path) -> Result<Arc<MoonshineLibrary>, MoonshineError> {
        let path = directory.join(LIBRARY_FILE_NAME);
        // SAFETY: loading runs the library's initialisers; the pinned wheel's
        // library has no initialisers with preconditions.
        let library = unsafe { libloading::Library::new(&path) }.map_err(|error| {
            MoonshineError::Load {
                path: path.clone(),
                message: error.to_string(),
            }
        })?;
        // SAFETY: every type below is the ctypes signature from moonshine_api.py.
        unsafe {
            Ok(Arc::new(MoonshineLibrary {
                get_version: symbol(&library, b"moonshine_get_version\0")?,
                error_to_string: symbol(&library, b"moonshine_error_to_string\0")?,
                load_transcriber_from_files: symbol(
                    &library,
                    b"moonshine_load_transcriber_from_files\0",
                )?,
                free_transcriber: symbol(&library, b"moonshine_free_transcriber\0")?,
                transcribe_without_streaming: symbol(
                    &library,
                    b"moonshine_transcribe_without_streaming\0",
                )?,
                create_tts_synthesizer_from_files: symbol(
                    &library,
                    b"moonshine_create_tts_synthesizer_from_files\0",
                )?,
                free_tts_synthesizer: symbol(&library, b"moonshine_free_tts_synthesizer\0")?,
                get_tts_dependencies: symbol(&library, b"moonshine_get_tts_dependencies\0")?,
                text_to_speech: symbol(&library, b"moonshine_text_to_speech\0")?,
                _library: library,
            }))
        }
    }

    pub fn version(&self) -> i32 {
        // SAFETY: no arguments, returns a plain integer.
        unsafe { (self.get_version)() }
    }

    pub fn error_message(&self, code: i32) -> String {
        // SAFETY: returns a static string or NULL.
        let message = unsafe { (self.error_to_string)(code) };
        if message.is_null() {
            return format!("error {code}");
        }
        // SAFETY: non-NULL results are NUL-terminated static strings.
        unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned()
    }

    pub(crate) fn call_error(&self, function: &'static str, code: i32) -> MoonshineError {
        MoonshineError::Call {
            function,
            code,
            message: self.error_message(code),
        }
    }

    /// Asset keys (paths relative to `g2p_root`) the synthesizer for
    /// `language_tag` needs, from `moonshine_get_tts_dependencies`.
    pub fn tts_dependencies(
        &self,
        language_tag: &str,
        options: &[(&str, &str)],
    ) -> Result<Vec<String>, MoonshineError> {
        const FUNCTION: &str = "moonshine_get_tts_dependencies";
        let language_tag = c_string(language_tag)?;
        let options = OptionArray::new(options)?;
        let mut json: *mut c_char = ptr::null_mut();
        // SAFETY: all pointers are valid for the call; `json` receives a malloc'd string.
        let code = unsafe {
            (self.get_tts_dependencies)(
                language_tag.as_ptr(),
                options.as_ptr(),
                options.len(),
                &mut json,
            )
        };
        // SAFETY: `json` is NULL or a malloc'd NUL-terminated string we now own.
        let text = unsafe { take_c_string(json) };
        if code != MOONSHINE_ERROR_NONE {
            return Err(self.call_error(FUNCTION, code));
        }
        let text = text.unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_str(&text).map_err(|error| MoonshineError::InvalidOutput {
            function: FUNCTION,
            message: error.to_string(),
        })
    }
}

pub(crate) fn c_string(text: &str) -> Result<CString, MoonshineError> {
    CString::new(text).map_err(|_| MoonshineError::InvalidArgument(format!("{text:?} contains a NUL byte")))
}

/// Copies a malloc'd C string and frees it.
///
/// # Safety
/// `pointer` must be NULL or a NUL-terminated string allocated with `malloc`.
unsafe fn take_c_string(pointer: *mut c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let text = CStr::from_ptr(pointer).to_string_lossy().into_owned();
    free_native(pointer.cast());
    Some(text)
}

/// Frees memory the C API allocated with `malloc`.
///
/// # Safety
/// `pointer` must come from the C API and must not be used afterwards.
pub(crate) unsafe fn free_native(pointer: *mut std::os::raw::c_void) {
    ffi::free(pointer);
}

/// A `moonshine_option_t` array and the strings it points into.
pub(crate) struct OptionArray {
    _strings: Vec<CString>,
    entries: Vec<MoonshineOption>,
}

impl OptionArray {
    pub(crate) fn new(options: &[(&str, &str)]) -> Result<OptionArray, MoonshineError> {
        let mut strings = Vec::with_capacity(options.len() * 2);
        let mut entries = Vec::with_capacity(options.len());
        for (name, value) in options {
            let name = c_string(name)?;
            let value = c_string(value)?;
            entries.push(MoonshineOption {
                name: name.as_ptr(),
                value: value.as_ptr(),
            });
            // Moving a CString does not move its heap buffer, so the pointers stay valid.
            strings.push(name);
            strings.push(value);
        }
        Ok(OptionArray {
            _strings: strings,
            entries,
        })
    }

    pub(crate) fn as_ptr(&self) -> *const MoonshineOption {
        if self.entries.is_empty() {
            ptr::null()
        } else {
            self.entries.as_ptr()
        }
    }

    pub(crate) fn len(&self) -> u64 {
        self.entries.len() as u64
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p moonshine-sys && TRANSLATOR_NATIVE_LIB_DIR=$PWD/native/x86_64/lib cargo test -p moonshine-sys -- --ignored`
Expected:
- First run: `missing_library_names_the_path` passes and 4 tests are ignored.
- Second run: the 4 ignored tests pass.

If `unknown_tts_language_is_an_error` gets `Ok(vec![])` instead, the library accepts unknown tags without an error. In that case change the test to assert `is_empty()`, because `translator-models` (Task 4) already treats an empty list as an error.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock moonshine-sys
git commit -m "Add moonshine-sys with run-time loading and TTS dependency lookup"
```

---

### Task 4: `translator-models` tool

**Files:**
- Create: `translator-models/Cargo.toml`, `translator-models/src/main.rs`, `translator-models/src/catalog.rs`, `translator-models/src/download.rs`
- Modify: `Cargo.toml` (members), `Makefile`

**Interfaces:**
- Consumes:
  - `ModelPaths`, `LanguageCode::{stt_model, tts_voice}` and `TtsVoice::synthesizer_options` (Task 1).
  - `MoonshineLibrary::{load, tts_dependencies}` (Task 3).
- Produces:
  - Binary `translator-models --model-dir <dir> [--lib-dir <dir>]`. `--lib-dir` defaults to `<exe dir>/lib`. Exit 0 when every file is present, 1 when any file failed, 2 on usage errors.
  - Make target `models` (variable `MODEL_DIR`, default `$(CURDIR)/models`).
  - `download::FileSpec { url: String, destination: PathBuf, expected_sha256: Option<&'static str> }`
  - `download::ensure_file(&ureq::Agent, &FileSpec) -> Result<DownloadOutcome, DownloadError>`

- [ ] **Step 1: Create the crate and write the failing tests**

Add `"translator-models"` to the workspace `members`.

`translator-models/Cargo.toml`:

```toml
[package]
name = "translator-models"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
translator-core = { path = "../translator-core" }
moonshine-sys = { path = "../moonshine-sys" }
ureq = { version = "2.12", default-features = false, features = ["tls"] }
sha2 = "0.10"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

`translator-models/src/download.rs`, tests only for now:

```rust
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

    // sha256("hello")
    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn downloads_verifies_and_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("a").join("b").join("file.bin");
        let url = serve_once(ok_response(5, b"hello"));
        let outcome = ensure_file(&agent(), &spec(url, destination.clone(), Some(HELLO_SHA256))).unwrap();
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
        let outcome = ensure_file(&agent(), &spec(url, destination.clone(), None)).unwrap();
        assert_eq!(outcome, DownloadOutcome::AlreadyPresent);
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    }

    #[test]
    fn truncated_body_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        let url = serve_once(ok_response(10, b"hell"));
        assert!(ensure_file(&agent(), &spec(url, destination.clone(), None)).is_err());
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
    }

    #[test]
    fn hash_mismatch_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("file.bin");
        let url = serve_once(ok_response(5, b"jello"));
        let error = ensure_file(&agent(), &spec(url, destination.clone(), Some(HELLO_SHA256))).unwrap_err();
        assert!(matches!(error, DownloadError::HashMismatch { .. }), "{error}");
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
        let error = ensure_file(&agent(), &spec(url, destination.clone(), None)).unwrap_err();
        assert!(matches!(error, DownloadError::Status { status: 404, .. }), "{error}");
        assert!(!destination.exists());
    }
}
```

`translator-models/src/catalog.rs`, tests only for now:

```rust
#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    #[test]
    fn gemma_is_pinned_by_commit_and_hash() {
        let file = gemma_file(&ModelPaths::new("/m"));
        assert!(file.url.contains("/resolve/b3ca0d2f076785a8f4b2219ddbd2bdb99954eae1/"));
        assert_eq!(file.destination, Path::new("/m/gemma/gemma-4-E2B-it.litertlm"));
        assert_eq!(
            file.expected_sha256,
            Some("181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c")
        );
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
        assert_eq!(files[0].url, "https://download.moonshine.ai/tts/ar_msa/piper-voices/a%20b.onnx");
        assert_eq!(files[0].destination, Path::new("/m/tts/ar_msa/piper-voices/a b.onnx"));
        assert_eq!(files[1].url, "https://download.moonshine.ai/tts/kokoro/model.onnx");
    }
}
```

`translator-models/src/main.rs`, only enough to compile the test modules:

```rust
mod catalog;
mod download;

fn main() {}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-models`
Expected: compile errors: `ensure_file`, `FileSpec`, `gemma_file`, `stt_files` and `tts_files` are not defined.

- [ ] **Step 3: Implement `download.rs`**

Prepend above the test module:

```rust
//! Fetches one file: written to `<name>.part`, checked against Content-Length
//! and (when known) sha256, then renamed into place. A file already at its
//! final path was verified when it was written, so it is never fetched again.

use std::fs::{self, File};
use std::io::{self, Read, Write};
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
    Truncated { url: String, expected: u64, received: u64 },
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

pub fn ensure_file(agent: &ureq::Agent, spec: &FileSpec) -> Result<DownloadOutcome, DownloadError> {
    if spec.destination.is_file() {
        return Ok(DownloadOutcome::AlreadyPresent);
    }
    let parent = spec
        .destination
        .parent()
        .expect("model files always live inside the model directory");
    fs::create_dir_all(parent).map_err(io_error(parent))?;
    let partial = partial_path(&spec.destination);
    match download_to(agent, spec, &partial) {
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

fn download_to(agent: &ureq::Agent, spec: &FileSpec, partial: &Path) -> Result<u64, DownloadError> {
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

    let mut file = File::create(partial).map_err(io_error(partial))?;
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
    }
    file.sync_all().map_err(io_error(partial))?;

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
```

- [ ] **Step 4: Implement `catalog.rs`**

Prepend above the test module:

```rust
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

pub fn gemma_file(paths: &ModelPaths) -> FileSpec {
    FileSpec {
        url: GEMMA_URL.to_string(),
        destination: paths.gemma_file(),
        expected_sha256: Some(GEMMA_SHA256),
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

/// Percent-encodes each path segment, as `download.py::cdn_url_for_tts_asset_key` does.
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
```

- [ ] **Step 5: Implement `main.rs`**

Replace `translator-models/src/main.rs`:

```rust
//! Downloads every model file `translator` needs into one directory
//! (spec §8.5). Run at deployment time; the translator never goes online.

mod catalog;
mod download;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use moonshine_sys::MoonshineLibrary;
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;

use crate::download::{ensure_file, DownloadOutcome};

const USAGE: &str = "usage: translator-models --model-dir <dir> [--lib-dir <dir>]";

#[derive(Debug, PartialEq)]
struct Options {
    model_dir: PathBuf,
    lib_dir: PathBuf,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut model_dir = None;
    let mut lib_dir = None;
    while let Some(argument) = args.next() {
        let target = match argument.as_str() {
            "--model-dir" => &mut model_dir,
            "--lib-dir" => &mut lib_dir,
            other => return Err(format!("unknown argument {other:?}")),
        };
        let value = args.next().ok_or_else(|| format!("{argument} needs a value"))?;
        *target = Some(PathBuf::from(value));
    }
    Ok(Options {
        model_dir: model_dir.ok_or("--model-dir is required")?,
        lib_dir: match lib_dir {
            Some(directory) => directory,
            None => default_lib_dir()?,
        },
    })
}

fn default_lib_dir() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable.parent().ok_or("executable has no parent directory")?;
    Ok(directory.join("lib"))
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: &Options) -> Result<(), String> {
    let paths = ModelPaths::new(&options.model_dir);
    let library = MoonshineLibrary::load(&options.lib_dir).map_err(|error| error.to_string())?;
    let tts_keys = collect_tts_keys(&library, &paths)?;

    let mut files = vec![catalog::gemma_file(&paths)];
    files.extend(catalog::stt_files(&paths));
    files.extend(catalog::tts_files(&paths, &tts_keys));

    let agent = download::agent();
    let mut failures = 0;
    for spec in &files {
        match ensure_file(&agent, spec) {
            Ok(DownloadOutcome::AlreadyPresent) => {
                println!("present  {}", spec.destination.display())
            }
            Ok(DownloadOutcome::Downloaded { bytes }) => {
                println!("fetched  {} ({bytes} bytes)", spec.destination.display())
            }
            Err(error) => {
                eprintln!("failed   {error}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        return Err(format!("{failures} of {} files failed; run again to retry", files.len()));
    }
    println!("all {} model files ready in {}", files.len(), paths.root().display());
    Ok(())
}

fn collect_tts_keys(library: &MoonshineLibrary, paths: &ModelPaths) -> Result<BTreeSet<String>, String> {
    let root = paths.tts_root();
    let root = root.to_str().ok_or("model directory path is not valid UTF-8")?;
    let mut keys = BTreeSet::new();
    for language in LanguageCode::ALL {
        let voice = language.tts_voice();
        let dependencies = library
            .tts_dependencies(voice.language_tag, &voice.synthesizer_options(root))
            .map_err(|error| error.to_string())?;
        // Keys without a '/' are G2P override labels, not files (download.py).
        let files: Vec<String> = dependencies.into_iter().filter(|key| key.contains('/')).collect();
        if files.is_empty() {
            return Err(format!("no TTS assets listed for {}", voice.language_tag));
        }
        keys.extend(files);
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|value| value.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn parses_both_directories() {
        assert_eq!(
            parse_args(args(&["--model-dir", "/m", "--lib-dir", "/l"])),
            Ok(Options {
                model_dir: PathBuf::from("/m"),
                lib_dir: PathBuf::from("/l"),
            })
        );
    }

    #[test]
    fn model_dir_is_required_and_unknown_flags_rejected() {
        assert!(parse_args(args(&["--lib-dir", "/l"])).is_err());
        assert!(parse_args(args(&["--model-dir"])).is_err());
        assert!(parse_args(args(&["--model-dir", "/m", "--fast"])).is_err());
    }
}
```

Add to `Makefile`, after the `native-libs` target:

```make
MODEL_DIR ?= $(CURDIR)/models

## models: download every model into MODEL_DIR (default ./models, ~3.5 GB)
.PHONY: models
models: native-libs
	mkdir -p $(MODEL_DIR)
	[ "$(MODEL_DIR)" != "$(CURDIR)/models" ] || printf '*\n' > $(MODEL_DIR)/.gitignore
	$(CARGO) run --release -p translator-models -- --lib-dir $(NATIVE_LIB_DIR) --model-dir $(MODEL_DIR)
```

Leave `build-pi` unchanged. `translator-models` pulls in rustls/ring, which would need an aarch64 C cross-compiler, so it is built natively on the Pi instead (see the manual checklist).

- [ ] **Step 6: Run the unit tests**

Run: `cargo test -p translator-models && cargo clippy --workspace --all-targets -- -D warnings`
Expected: 10 tests pass, no warnings.

- [ ] **Step 7: Populate the development model directory**

To avoid downloading Gemma again, seed it from the Hugging Face cache. It is byte-identical, which `cmp` showed:

```bash
mkdir -p models/gemma
cp ~/.cache/huggingface/hub/models--litert-community--gemma-4-E2B-it-litert-lm/snapshots/b3ca0d2f076785a8f4b2219ddbd2bdb99954eae1/gemma-4-E2B-it.litertlm models/gemma/
make models
make models
```

Expected:
- The first `make models` ends with `all N model files ready in …/models`, where N is about 50.
- The second run prints only `present` lines.
- `git status --short` shows no `models/` entries.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock translator-models Makefile
git commit -m "Add translator-models to download every model file"
```

---

### Task 5: `moonshine-sys` transcriber and synthesizer

**Files:**
- Create: `moonshine-sys/src/transcriber.rs`, `moonshine-sys/src/synthesizer.rs`, `moonshine-sys/tests/engines.rs`
- Modify: `moonshine-sys/src/lib.rs`

**Interfaces:**
- Consumes:
  - `MoonshineLibrary` fields and helpers `call_error`, `c_string`, `OptionArray` and `free_native` (Task 3).
  - The model directory produced by Task 4.
- Produces:
  - `ModelArch { Tiny = 0, Base = 1, TinyStreaming = 2, BaseStreaming = 3, SmallStreaming = 4, MediumStreaming = 5 }`
  - `Transcriber::load(&Arc<MoonshineLibrary>, model_dir: &Path, ModelArch) -> Result<Transcriber, MoonshineError>`
  - `Transcriber::transcribe(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, MoonshineError>`
  - `Synthesizer::create(&Arc<MoonshineLibrary>, language_tag: &str, options: &[(&str, &str)]) -> Result<Synthesizer, MoonshineError>`
  - `Synthesizer::synthesize(&mut self, text: &str) -> Result<(Vec<f32>, u32), MoonshineError>`
  - Both types are `!Send`, so they stay on the thread that created them.

The tests pass the synthesizer's 24 kHz output straight to `transcribe` with `sample_rate = 24000`. A Python check on 2026-10-09 confirmed that the C side resamples it: the transcript was "Good morning. How are you today?".

- [ ] **Step 1: Write the failing tests**

`moonshine-sys/tests/engines.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use moonshine_sys::{ModelArch, MoonshineLibrary, Synthesizer, Transcriber};

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")))
}

fn library() -> Arc<MoonshineLibrary> {
    MoonshineLibrary::load(&env_dir("TRANSLATOR_NATIVE_LIB_DIR")).unwrap()
}

fn tts_root() -> String {
    env_dir("TRANSLATOR_MODEL_DIR").join("tts").to_str().unwrap().to_string()
}

fn english_transcriber(library: &Arc<MoonshineLibrary>) -> Transcriber {
    let directory = env_dir("TRANSLATOR_MODEL_DIR").join("stt").join("medium-streaming-en");
    Transcriber::load(library, &directory, ModelArch::MediumStreaming).unwrap()
}

fn english_speech(library: &Arc<MoonshineLibrary>) -> (Vec<f32>, u32) {
    let root = tts_root();
    let mut synthesizer = Synthesizer::create(library, "en_us", &[("g2p_root", &root)]).unwrap();
    synthesizer.synthesize("Good morning, how are you today?").unwrap()
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn synthesizes_speech_with_its_sample_rate() {
    let library = library();
    let (samples, sample_rate) = english_speech(&library);
    assert!(sample_rate >= 16_000, "sample rate {sample_rate}");
    assert!(samples.len() > sample_rate as usize / 2, "only {} samples", samples.len());
    assert!(samples.iter().all(|sample| sample.is_finite()));
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn round_trip_speech_is_recognized() {
    let library = library();
    let (samples, sample_rate) = english_speech(&library);
    let mut transcriber = english_transcriber(&library);
    let text = transcriber.transcribe(&samples, sample_rate).unwrap();
    assert!(text.to_lowercase().contains("morning"), "transcript: {text:?}");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn empty_audio_gives_empty_text() {
    let library = library();
    let mut transcriber = english_transcriber(&library);
    assert_eq!(transcriber.transcribe(&[], 16_000).unwrap(), "");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn every_language_synthesizer_can_be_created() {
    let library = library();
    let root = tts_root();
    for (tag, voice) in [
        ("ar_msa", None),
        ("es_es", None),
        ("ja_jp", None),
        ("zh_hans", Some("kokoro_zf_xiaoxiao")),
        ("ko_kr", None),
    ] {
        let mut options = vec![("g2p_root", root.as_str())];
        if let Some(voice) = voice {
            options.push(("voice", voice));
        }
        let result = Synthesizer::create(&library, tag, &options);
        assert!(result.is_ok(), "{tag}: {:?}", result.err());
    }
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn missing_model_directory_is_an_error() {
    let library = library();
    let missing = tempfile::tempdir().unwrap().path().join("nothing-here");
    assert!(Transcriber::load(&library, &missing, ModelArch::Base).is_err());
}

/// Spec §11.1: nothing documents who frees `TranscriptC`. If the C side
/// allocated a new transcript per call and expected the caller to free it,
/// 1000 calls would leak tens of megabytes.
#[test]
#[ignore = "soak test: make test-soak"]
fn soak_repeated_transcription_does_not_leak() {
    let library = library();
    let (samples, sample_rate) = english_speech(&library);
    let mut transcriber = english_transcriber(&library);
    for _ in 0..20 {
        transcriber.transcribe(&samples, sample_rate).unwrap();
    }
    let before = resident_kib();
    for _ in 0..1000 {
        transcriber.transcribe(&samples, sample_rate).unwrap();
    }
    let growth = resident_kib().saturating_sub(before);
    assert!(growth < 16 * 1024, "RSS grew by {growth} KiB over 1000 transcriptions");
}

fn resident_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}
```

Add `tempfile = "3"` under `[dev-dependencies]` of `moonshine-sys` if it is missing; Task 3 already added it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p moonshine-sys --no-run`
Expected: compile error, because `ModelArch`, `Transcriber` and `Synthesizer` are not exported.

- [ ] **Step 3: Implement**

`moonshine-sys/src/transcriber.rs`:

```rust
use std::ffi::CStr;
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use crate::ffi::{Transcript, MOONSHINE_ERROR_NONE, MOONSHINE_HEADER_VERSION};
use crate::library::{c_string, MoonshineError, MoonshineLibrary};

/// `moonshine_voice.ModelArch`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ModelArch {
    Tiny = 0,
    Base = 1,
    TinyStreaming = 2,
    BaseStreaming = 3,
    SmallStreaming = 4,
    MediumStreaming = 5,
}

/// A loaded speech-recognition model. Not `Send`: the handle stays on the
/// thread that created it (spec §8.3).
pub struct Transcriber {
    library: Arc<MoonshineLibrary>,
    handle: i32,
    _single_thread: PhantomData<*const ()>,
}

impl Transcriber {
    pub fn load(
        library: &Arc<MoonshineLibrary>,
        model_dir: &Path,
        arch: ModelArch,
    ) -> Result<Transcriber, MoonshineError> {
        let path = model_dir
            .to_str()
            .ok_or_else(|| MoonshineError::InvalidArgument(format!("{} is not UTF-8", model_dir.display())))?;
        let path = c_string(path)?;
        // SAFETY: `path` is a valid C string; no options are passed.
        let handle = unsafe {
            (library.load_transcriber_from_files)(
                path.as_ptr(),
                arch as u32,
                ptr::null(),
                0,
                MOONSHINE_HEADER_VERSION,
            )
        };
        if handle < 0 {
            return Err(library.call_error("moonshine_load_transcriber_from_files", handle));
        }
        Ok(Transcriber {
            library: Arc::clone(library),
            handle,
            _single_thread: PhantomData,
        })
    }

    /// Transcribes mono samples and joins the transcript lines with spaces,
    /// as `server.py` did.
    pub fn transcribe(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, MoonshineError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let mut transcript: *mut Transcript = ptr::null_mut();
        // SAFETY: `samples` is valid for its length; `transcript` receives a
        // pointer owned by the transcriber (there is no free function; see
        // the soak test).
        let code = unsafe {
            (self.library.transcribe_without_streaming)(
                self.handle,
                samples.as_ptr(),
                samples.len() as u64,
                sample_rate as i32,
                0,
                &mut transcript,
            )
        };
        if code != MOONSHINE_ERROR_NONE {
            return Err(self.library.call_error("moonshine_transcribe_without_streaming", code));
        }
        if transcript.is_null() {
            return Ok(String::new());
        }
        // SAFETY: a successful call leaves a valid transcript until the next
        // call on this handle; `&mut self` rules out a concurrent call.
        let transcript = unsafe { &*transcript };
        if transcript.lines.is_null() || transcript.line_count == 0 {
            return Ok(String::new());
        }
        // SAFETY: `lines` points to `line_count` initialised entries.
        let lines = unsafe { std::slice::from_raw_parts(transcript.lines, transcript.line_count as usize) };
        let texts: Vec<String> = lines
            .iter()
            .filter(|line| !line.text.is_null())
            // SAFETY: non-NULL line texts are NUL-terminated.
            .map(|line| unsafe { CStr::from_ptr(line.text) }.to_string_lossy().into_owned())
            .collect();
        Ok(texts.join(" "))
    }
}

impl Drop for Transcriber {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful load and is freed once.
        unsafe { (self.library.free_transcriber)(self.handle) }
    }
}
```

`moonshine-sys/src/synthesizer.rs`:

```rust
use std::marker::PhantomData;
use std::ptr;
use std::sync::Arc;

use crate::ffi::{MOONSHINE_ERROR_NONE, MOONSHINE_HEADER_VERSION};
use crate::library::{c_string, free_native, MoonshineError, MoonshineLibrary, OptionArray};

/// A text-to-speech engine for one language. Not `Send` (spec §8.3).
pub struct Synthesizer {
    library: Arc<MoonshineLibrary>,
    handle: i32,
    _single_thread: PhantomData<*const ()>,
}

impl Synthesizer {
    /// `options` must include `g2p_root`; the file list is resolved by the C side.
    pub fn create(
        library: &Arc<MoonshineLibrary>,
        language_tag: &str,
        options: &[(&str, &str)],
    ) -> Result<Synthesizer, MoonshineError> {
        let language_tag = c_string(language_tag)?;
        let options = OptionArray::new(options)?;
        // SAFETY: valid C strings and option array; NULL file list as in tts.py.
        let handle = unsafe {
            (library.create_tts_synthesizer_from_files)(
                language_tag.as_ptr(),
                ptr::null(),
                0,
                options.as_ptr(),
                options.len(),
                MOONSHINE_HEADER_VERSION,
            )
        };
        if handle < 0 {
            return Err(library.call_error("moonshine_create_tts_synthesizer_from_files", handle));
        }
        Ok(Synthesizer {
            library: Arc::clone(library),
            handle,
            _single_thread: PhantomData,
        })
    }

    /// Returns mono samples in [-1, 1] and their sample rate.
    pub fn synthesize(&mut self, text: &str) -> Result<(Vec<f32>, u32), MoonshineError> {
        const FUNCTION: &str = "moonshine_text_to_speech";
        let text = c_string(text)?;
        let mut audio: *mut f32 = ptr::null_mut();
        let mut size: u64 = 0;
        let mut sample_rate: i32 = 0;
        // SAFETY: out-pointers are valid; `audio` receives a malloc'd buffer.
        let code = unsafe {
            (self.library.text_to_speech)(
                self.handle,
                text.as_ptr(),
                ptr::null(),
                0,
                &mut audio,
                &mut size,
                &mut sample_rate,
            )
        };
        let samples = if audio.is_null() || size == 0 {
            Vec::new()
        } else {
            // SAFETY: `audio` holds `size` floats.
            unsafe { std::slice::from_raw_parts(audio, size as usize) }.to_vec()
        };
        if !audio.is_null() {
            // SAFETY: allocated by the C API with malloc; not used afterwards.
            unsafe { free_native(audio.cast()) };
        }
        if code != MOONSHINE_ERROR_NONE {
            return Err(self.library.call_error(FUNCTION, code));
        }
        if sample_rate <= 0 {
            return Err(MoonshineError::InvalidOutput {
                function: FUNCTION,
                message: format!("sample rate {sample_rate}"),
            });
        }
        Ok((samples, sample_rate as u32))
    }
}

impl Drop for Synthesizer {
    fn drop(&mut self) {
        // SAFETY: `handle` came from a successful create and is freed once.
        unsafe { (self.library.free_tts_synthesizer)(self.handle) }
    }
}
```

Replace `moonshine-sys/src/lib.rs`:

```rust
//! Run-time bindings to `libmoonshine.so` from the moonshine-voice 0.0.65 wheel.
//! Signatures mirror the wheel's `moonshine_api.py` ctypes declarations.

mod ffi;
mod library;
mod synthesizer;
mod transcriber;

pub use library::{MoonshineError, MoonshineLibrary, LIBRARY_FILE_NAME};
pub use synthesizer::Synthesizer;
pub use transcriber::{ModelArch, Transcriber};
```

- [ ] **Step 4: Add the Makefile targets and run the tests**

Add to `Makefile`, after `models`:

```make
FFI_ENV := TRANSLATOR_NATIVE_LIB_DIR=$(NATIVE_LIB_DIR) TRANSLATOR_MODEL_DIR=$(MODEL_DIR)

## test-ffi: run the tests that need native libraries and models (after make models)
.PHONY: test-ffi
test-ffi:
	$(FFI_ENV) $(CARGO) test --workspace -- --ignored --skip soak --test-threads=1

## test-soak: long-running native memory tests
.PHONY: test-soak
test-soak:
	$(FFI_ENV) $(CARGO) test --release --workspace -- --ignored soak --test-threads=1
```

Run: `cargo test -p moonshine-sys && make test-ffi && make test-soak`
Expected:
- The plain run passes and reports the ignored tests.
- `test-ffi` passes all moonshine ignored tests.
- `test-soak` passes.

If the soak test fails, the transcript is caller-owned. Stop and report this to the user before continuing: it changes the FFI design, which is a §11.1 failure.

- [ ] **Step 5: Commit**

```bash
git add moonshine-sys Makefile
git commit -m "Add the moonshine transcriber and synthesizer bindings"
```

---

### Task 6: `litert-lm-sys` engine and streaming conversation

**Files:**
- Create: `litert-lm-sys/Cargo.toml`, `litert-lm-sys/src/lib.rs`, `litert-lm-sys/src/ffi.rs`, `litert-lm-sys/src/library.rs`, `litert-lm-sys/src/engine.rs`, `litert-lm-sys/tests/engine.rs`
- Modify: `Cargo.toml` (members)

**Interfaces:**
- Consumes: the model directory (Task 4) and native libraries (Task 2).
- Produces:
  - `LiteRtLmLibrary::load(directory: &Path) -> Result<Arc<LiteRtLmLibrary>, LiteRtLmError>`
  - `Engine::new(&Arc<LiteRtLmLibrary>, model_file: &Path, cache_dir: &Path) -> Result<Engine, LiteRtLmError>`. `Engine` is `!Send`.
  - `Engine::create_conversation(&self, system_message: &str) -> Result<Conversation<'_>, LiteRtLmError>`
  - `Conversation::send_message_stream(&mut self, text: &str, on_text: &mut dyn FnMut(&str), should_cancel: &dyn Fn() -> bool) -> Result<StreamEnd, LiteRtLmError>`
  - `StreamEnd { Completed, Cancelled, MaxTokensReached }`
  - `chunk_text(chunk: &str) -> String`
  - `LITERT_LM_LIBRARY_FILE_NAME`

Facts this task relies on, from a probe run with the 0.13.1 library on 2026-10-09:
- `send_message_stream` returns at once, and callbacks arrive later on library threads.
- Each chunk is a delta: `{"role":"assistant","content":[{"type":"text","text":"早上"}]}`.
- The last callback has `chunk = NULL`, `is_final = true` and `error = NULL`.
- After `cancel_process`, the last callback arrives within about 15 ms with `is_final = true` and `error = "CANCELLED: Task cancelled"`, and no callback follows `conversation_delete`.
- The engine accepts new conversations after a cancel.

- [ ] **Step 1: Create the crate and write the failing tests**

Add `"litert-lm-sys"` to the workspace `members`.

`litert-lm-sys/Cargo.toml`:

```toml
[package]
name = "litert-lm-sys"
edition.workspace = true
version.workspace = true
license.workspace = true

[dependencies]
libloading = "0.8"
serde_json = "1"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

`litert-lm-sys/src/lib.rs`:

```rust
//! Run-time bindings to `liblitert-lm.so` from the litert-lm-api 0.13.1 wheel.
//! Signatures mirror the wheel's `litert_lm/_ffi.py` ctypes declarations.

mod engine;
mod ffi;
mod library;

pub use engine::{chunk_text, Conversation, Engine, StreamEnd};
pub use library::{LiteRtLmError, LiteRtLmLibrary, LITERT_LM_LIBRARY_FILE_NAME};
```

Unit tests for `chunk_text`, placed at the bottom of `litert-lm-sys/src/engine.rs`. Create the file with only this module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_text_items_only() {
        assert_eq!(
            chunk_text(r#"{"role":"assistant","content":[{"type":"text","text":"早上"}]}"#),
            "早上"
        );
        assert_eq!(
            chunk_text(
                r#"{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"tool_call","text":"x"},{"type":"text","text":"b"}]}"#
            ),
            "ab"
        );
    }

    #[test]
    fn ignores_thinking_channels() {
        assert_eq!(
            chunk_text(r#"{"role":"assistant","channels":{"thought":"hmm"},"content":[]}"#),
            ""
        );
    }

    #[test]
    fn accepts_string_content_and_falls_back_to_raw_text() {
        assert_eq!(chunk_text(r#"{"role":"assistant","content":"hi"}"#), "hi");
        assert_eq!(chunk_text("not json"), "not json");
    }

    #[test]
    fn classifies_stream_ends() {
        assert_eq!(stream_end(None), Ok(StreamEnd::Completed));
        assert_eq!(stream_end(Some("CANCELLED: Task cancelled".into())), Ok(StreamEnd::Cancelled));
        assert_eq!(
            stream_end(Some("Max number of tokens reached".into())),
            Ok(StreamEnd::MaxTokensReached)
        );
        assert!(matches!(
            stream_end(Some("INTERNAL: boom".into())),
            Err(LiteRtLmError::Stream(message)) if message == "INTERNAL: boom"
        ));
    }
}
```

`litert-lm-sys/tests/engine.rs`:

```rust
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use litert_lm_sys::{Engine, LiteRtLmError, LiteRtLmLibrary, StreamEnd};

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")))
}

fn library() -> Arc<LiteRtLmLibrary> {
    LiteRtLmLibrary::load(&env_dir("TRANSLATOR_NATIVE_LIB_DIR")).unwrap()
}

fn engine(library: &Arc<LiteRtLmLibrary>) -> Engine {
    let models = env_dir("TRANSLATOR_MODEL_DIR");
    Engine::new(
        library,
        &models.join("gemma").join("gemma-4-E2B-it.litertlm"),
        &models.join("cache"),
    )
    .unwrap()
}

#[test]
fn missing_library_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let error = LiteRtLmLibrary::load(dir.path()).err().unwrap();
    assert!(error.to_string().contains("liblitert-lm.so"), "{error}");
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn missing_model_file_is_reported_before_loading() {
    let library = library();
    let dir = tempfile::tempdir().unwrap();
    let error = Engine::new(&library, &dir.path().join("none.litertlm"), dir.path())
        .err()
        .unwrap();
    assert!(matches!(error, LiteRtLmError::ModelMissing(_)), "{error}");
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn streams_a_translation_in_fragments() {
    let library = library();
    let engine = engine(&library);
    let mut conversation = engine
        .create_conversation("Translate the user's English text into Chinese. Output only the translation.")
        .unwrap();
    let mut fragments = Vec::new();
    let end = conversation
        .send_message_stream("Good morning. How are you today?", &mut |text| fragments.push(text.to_string()), &|| false)
        .unwrap();
    assert_eq!(end, StreamEnd::Completed);
    assert!(fragments.len() > 1, "expected several deltas, got {fragments:?}");
    let reply: String = fragments.concat();
    assert!(
        reply.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
        "no Chinese in {reply:?}"
    );
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn cancel_ends_stream_and_engine_stays_usable() {
    let library = library();
    let engine = engine(&library);
    {
        let mut conversation = engine.create_conversation("You are a storyteller.").unwrap();
        let fragments = Cell::new(0);
        let started = Instant::now();
        let end = conversation
            .send_message_stream(
                "Tell a very long story about a dragon.",
                &mut |_| fragments.set(fragments.get() + 1),
                &|| fragments.get() >= 2,
            )
            .unwrap();
        assert_eq!(end, StreamEnd::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
    let mut conversation = engine.create_conversation("Reply with exactly one word.").unwrap();
    let mut reply = String::new();
    let end = conversation
        .send_message_stream("Say yes.", &mut |text| reply.push_str(text), &|| false)
        .unwrap();
    assert_eq!(end, StreamEnd::Completed);
    assert!(!reply.trim().is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p litert-lm-sys`
Expected: compile errors, because `ffi`, `library`, `chunk_text` and `stream_end` are missing.

- [ ] **Step 3: Implement `ffi.rs` and `library.rs`**

`litert-lm-sys/src/ffi.rs`:

```rust
//! C declarations, copied from litert-lm-api 0.13.1 `litert_lm/_ffi.py`.

use std::os::raw::{c_char, c_int, c_void};

/// `LiteRtLmStreamCallback`: (user_data, chunk, is_final, error).
pub type StreamCallback =
    unsafe extern "C" fn(user_data: *mut c_void, chunk: *const c_char, is_final: bool, error: *const c_char);

pub type EngineSettingsCreateFn = unsafe extern "C" fn(
    model_path: *const c_char,
    backend: *const c_char,
    vision_backend: *const c_char,
    audio_backend: *const c_char,
) -> *mut c_void;
pub type EngineSettingsDeleteFn = unsafe extern "C" fn(settings: *mut c_void);
pub type EngineSettingsSetCacheDirFn = unsafe extern "C" fn(settings: *mut c_void, cache_dir: *const c_char);
pub type EngineCreateFn = unsafe extern "C" fn(settings: *mut c_void) -> *mut c_void;
pub type EngineDeleteFn = unsafe extern "C" fn(engine: *mut c_void);
pub type ConversationConfigCreateFn = unsafe extern "C" fn() -> *mut c_void;
pub type ConversationConfigDeleteFn = unsafe extern "C" fn(config: *mut c_void);
pub type ConversationConfigSetSystemMessageFn =
    unsafe extern "C" fn(config: *mut c_void, system_message: *const c_char);
pub type ConversationCreateFn = unsafe extern "C" fn(engine: *mut c_void, config: *mut c_void) -> *mut c_void;
pub type ConversationDeleteFn = unsafe extern "C" fn(conversation: *mut c_void);
pub type ConversationSendMessageStreamFn = unsafe extern "C" fn(
    conversation: *mut c_void,
    message_json: *const c_char,
    extra_context_json: *const c_char,
    visual_token_budget: *const c_void,
    callback: StreamCallback,
    user_data: *mut c_void,
) -> c_int;
pub type ConversationCancelProcessFn = unsafe extern "C" fn(conversation: *mut c_void);
```

`litert-lm-sys/src/library.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ffi;

pub const LITERT_LM_LIBRARY_FILE_NAME: &str = "liblitert-lm.so";

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum LiteRtLmError {
    #[error("cannot load {path}: {message}")]
    Load { path: PathBuf, message: String },
    #[error("{name} missing from liblitert-lm.so: {message}")]
    Symbol { name: String, message: String },
    #[error("model file {0} not found; run translator-models")]
    ModelMissing(PathBuf),
    #[error("{0} failed")]
    Call(&'static str),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("generation failed: {0}")]
    Stream(String),
    #[error("no final callback within {0:?} of cancelling")]
    CancelTimedOut(std::time::Duration),
}

/// The loaded library and every function this crate calls.
pub struct LiteRtLmLibrary {
    pub(crate) engine_settings_create: ffi::EngineSettingsCreateFn,
    pub(crate) engine_settings_delete: ffi::EngineSettingsDeleteFn,
    pub(crate) engine_settings_set_cache_dir: ffi::EngineSettingsSetCacheDirFn,
    pub(crate) engine_create: ffi::EngineCreateFn,
    pub(crate) engine_delete: ffi::EngineDeleteFn,
    pub(crate) conversation_config_create: ffi::ConversationConfigCreateFn,
    pub(crate) conversation_config_delete: ffi::ConversationConfigDeleteFn,
    pub(crate) conversation_config_set_system_message: ffi::ConversationConfigSetSystemMessageFn,
    pub(crate) conversation_create: ffi::ConversationCreateFn,
    pub(crate) conversation_delete: ffi::ConversationDeleteFn,
    pub(crate) conversation_send_message_stream: ffi::ConversationSendMessageStreamFn,
    pub(crate) conversation_cancel_process: ffi::ConversationCancelProcessFn,
    _library: libloading::Library,
}

/// # Safety
/// `T` must be the exact C signature of the symbol.
unsafe fn symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> Result<T, LiteRtLmError> {
    library
        .get::<T>(name)
        .map(|found| *found)
        .map_err(|error| LiteRtLmError::Symbol {
            name: String::from_utf8_lossy(&name[..name.len() - 1]).into_owned(),
            message: error.to_string(),
        })
}

impl LiteRtLmLibrary {
    pub fn load(directory: &Path) -> Result<Arc<LiteRtLmLibrary>, LiteRtLmError> {
        let path = directory.join(LITERT_LM_LIBRARY_FILE_NAME);
        // SAFETY: the pinned wheel's library has no initialisers with preconditions.
        let library = unsafe { libloading::Library::new(&path) }.map_err(|error| LiteRtLmError::Load {
            path: path.clone(),
            message: error.to_string(),
        })?;
        // SAFETY: every type below is the ctypes signature from _ffi.py.
        unsafe {
            Ok(Arc::new(LiteRtLmLibrary {
                engine_settings_create: symbol(&library, b"litert_lm_engine_settings_create\0")?,
                engine_settings_delete: symbol(&library, b"litert_lm_engine_settings_delete\0")?,
                engine_settings_set_cache_dir: symbol(&library, b"litert_lm_engine_settings_set_cache_dir\0")?,
                engine_create: symbol(&library, b"litert_lm_engine_create\0")?,
                engine_delete: symbol(&library, b"litert_lm_engine_delete\0")?,
                conversation_config_create: symbol(&library, b"litert_lm_conversation_config_create\0")?,
                conversation_config_delete: symbol(&library, b"litert_lm_conversation_config_delete\0")?,
                conversation_config_set_system_message: symbol(
                    &library,
                    b"litert_lm_conversation_config_set_system_message\0",
                )?,
                conversation_create: symbol(&library, b"litert_lm_conversation_create\0")?,
                conversation_delete: symbol(&library, b"litert_lm_conversation_delete\0")?,
                conversation_send_message_stream: symbol(
                    &library,
                    b"litert_lm_conversation_send_message_stream\0",
                )?,
                conversation_cancel_process: symbol(&library, b"litert_lm_conversation_cancel_process\0")?,
                _library: library,
            }))
        }
    }
}
```

- [ ] **Step 4: Implement `engine.rs`**

Prepend above the test module:

```rust
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::library::{LiteRtLmError, LiteRtLmLibrary};

/// How often a running stream checks `should_cancel`.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Upper bound on waiting for the library's final callback after a cancel.
const CANCEL_GRACE: Duration = Duration::from_secs(10);
const BACKEND: &[u8] = b"cpu\0";
const EMPTY_EXTRA_CONTEXT: &[u8] = b"{}\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    Completed,
    Cancelled,
    MaxTokensReached,
}

/// A loaded model. Not `Send` (raw pointer): it stays on the thread that created it.
pub struct Engine {
    library: Arc<LiteRtLmLibrary>,
    pointer: NonNull<c_void>,
}

/// One conversation; drop it to free it. Borrowing the engine keeps the engine alive.
pub struct Conversation<'engine> {
    engine: &'engine Engine,
    pointer: NonNull<c_void>,
    /// Target of the callback's `user_data`. Kept until the next send or until
    /// after `conversation_delete`, so a late callback never sees freed memory.
    stream_state: Option<Box<StreamState>>,
}

struct StreamState {
    events: Sender<StreamEvent>,
}

enum StreamEvent {
    Chunk(String),
    Final { error: Option<String> },
}

fn c_string(text: &str) -> Result<CString, LiteRtLmError> {
    CString::new(text).map_err(|_| LiteRtLmError::InvalidArgument(format!("{text:?} contains a NUL byte")))
}

fn path_c_string(path: &Path) -> Result<CString, LiteRtLmError> {
    c_string(
        path.to_str()
            .ok_or_else(|| LiteRtLmError::InvalidArgument(format!("{} is not UTF-8", path.display())))?,
    )
}

impl Engine {
    /// Loads `model_file` on the CPU backend; `cache_dir` must be writable
    /// (litert-lm stores its compiled-model cache there).
    pub fn new(library: &Arc<LiteRtLmLibrary>, model_file: &Path, cache_dir: &Path) -> Result<Engine, LiteRtLmError> {
        if !model_file.is_file() {
            return Err(LiteRtLmError::ModelMissing(model_file.to_path_buf()));
        }
        let model = path_c_string(model_file)?;
        let cache = path_c_string(cache_dir)?;
        // SAFETY: valid C strings; NULL vision/audio backends as in engine.py.
        let settings = unsafe {
            (library.engine_settings_create)(model.as_ptr(), BACKEND.as_ptr().cast(), ptr::null(), ptr::null())
        };
        if settings.is_null() {
            return Err(LiteRtLmError::Call("litert_lm_engine_settings_create"));
        }
        // SAFETY: `settings` is live until the delete below.
        let engine = unsafe {
            (library.engine_settings_set_cache_dir)(settings, cache.as_ptr());
            let engine = (library.engine_create)(settings);
            (library.engine_settings_delete)(settings);
            engine
        };
        let pointer = NonNull::new(engine).ok_or(LiteRtLmError::Call("litert_lm_engine_create"))?;
        Ok(Engine {
            library: Arc::clone(library),
            pointer,
        })
    }

    pub fn create_conversation(&self, system_message: &str) -> Result<Conversation<'_>, LiteRtLmError> {
        let system_message = c_string(system_message)?;
        let library = &self.library;
        // SAFETY: the config is created, used and deleted here; the engine is live.
        let conversation = unsafe {
            let config = (library.conversation_config_create)();
            if config.is_null() {
                return Err(LiteRtLmError::Call("litert_lm_conversation_config_create"));
            }
            (library.conversation_config_set_system_message)(config, system_message.as_ptr());
            let conversation = (library.conversation_create)(self.pointer.as_ptr(), config);
            (library.conversation_config_delete)(config);
            conversation
        };
        let pointer = NonNull::new(conversation).ok_or(LiteRtLmError::Call("litert_lm_conversation_create"))?;
        Ok(Conversation {
            engine: self,
            pointer,
            stream_state: None,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: conversations borrow the engine, so none is alive here.
        unsafe { (self.library.engine_delete)(self.pointer.as_ptr()) }
    }
}

impl Conversation<'_> {
    /// Sends `text` as a user message and blocks until the reply ends.
    /// `on_text` receives each text delta. `should_cancel` is polled every
    /// 20 ms; once it returns true the reply is cancelled, and the call still
    /// waits for the library's final callback before returning.
    pub fn send_message_stream(
        &mut self,
        text: &str,
        on_text: &mut dyn FnMut(&str),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<StreamEnd, LiteRtLmError> {
        let message = c_string(
            &serde_json::json!({ "role": "user", "content": [{ "type": "text", "text": text }] }).to_string(),
        )?;
        let (events, receiver) = mpsc::channel();
        let state = Box::new(StreamState { events });
        let user_data = &*state as *const StreamState as *mut c_void;
        // The previous stream (if any) already delivered its final callback.
        self.stream_state = Some(state);

        let library = &self.engine.library;
        // SAFETY: valid C strings; `user_data` outlives every callback (see `stream_state`).
        let code = unsafe {
            (library.conversation_send_message_stream)(
                self.pointer.as_ptr(),
                message.as_ptr(),
                EMPTY_EXTRA_CONTEXT.as_ptr().cast(),
                ptr::null(),
                on_stream_event,
                user_data,
            )
        };
        if code != 0 {
            return Err(LiteRtLmError::Call("litert_lm_conversation_send_message_stream"));
        }

        let mut cancelled_at: Option<Instant> = None;
        loop {
            match receiver.recv_timeout(CANCEL_POLL_INTERVAL) {
                Ok(StreamEvent::Chunk(chunk)) => {
                    if cancelled_at.is_none() {
                        let text = chunk_text(&chunk);
                        if !text.is_empty() {
                            on_text(&text);
                        }
                    }
                }
                Ok(StreamEvent::Final { error }) => return stream_end(error),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    unreachable!("`stream_state` holds the sender until the next send")
                }
            }
            match cancelled_at {
                None if should_cancel() => {
                    // SAFETY: the conversation is live.
                    unsafe { (library.conversation_cancel_process)(self.pointer.as_ptr()) };
                    cancelled_at = Some(Instant::now());
                }
                Some(started) if started.elapsed() > CANCEL_GRACE => {
                    return Err(LiteRtLmError::CancelTimedOut(CANCEL_GRACE));
                }
                _ => {}
            }
        }
    }
}

impl Drop for Conversation<'_> {
    fn drop(&mut self) {
        // SAFETY: deleted once; `stream_state` is dropped after this body runs.
        unsafe { (self.engine.library.conversation_delete)(self.pointer.as_ptr()) }
    }
}

/// Called on litert-lm's own threads.
unsafe extern "C" fn on_stream_event(
    user_data: *mut c_void,
    chunk: *const c_char,
    is_final: bool,
    error: *const c_char,
) {
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `user_data` is the `StreamState` kept alive by the conversation.
        let state = unsafe { &*(user_data as *const StreamState) };
        let read = |text: *const c_char| {
            // SAFETY: non-NULL strings from the library are NUL-terminated.
            (!text.is_null()).then(|| unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned())
        };
        if let Some(chunk) = read(chunk) {
            let _ = state.events.send(StreamEvent::Chunk(chunk));
        }
        let error = read(error);
        if is_final || error.is_some() {
            let _ = state.events.send(StreamEvent::Final { error });
        }
    }));
}

fn stream_end(error: Option<String>) -> Result<StreamEnd, LiteRtLmError> {
    match error {
        None => Ok(StreamEnd::Completed),
        Some(message) if message.contains("CANCELLED") => Ok(StreamEnd::Cancelled),
        Some(message) if message.contains("Max number of tokens reached") => Ok(StreamEnd::MaxTokensReached),
        Some(message) => Err(LiteRtLmError::Stream(message)),
    }
}

/// The spoken text of one streamed message: the `text` of every
/// `"type": "text"` content item. Thinking arrives in `channels` and is
/// dropped. A chunk that is not JSON is returned as is (conversation.py).
pub fn chunk_text(chunk: &str) -> String {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(chunk) else {
        return chunk.to_string();
    };
    match message.get("content") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter(|item| item.get("type").and_then(|kind| kind.as_str()) == Some("text"))
            .filter_map(|item| item.get("text").and_then(|text| text.as_str()))
            .collect(),
        _ => String::new(),
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p litert-lm-sys && make test-ffi && cargo clippy --workspace --all-targets -- -D warnings`
Expected:
- The 4 unit tests and `missing_library_names_the_path` pass.
- `test-ffi` passes the 3 ignored litert tests in addition to Task 3/5's.
- No warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock litert-lm-sys
git commit -m "Add litert-lm-sys with a cancellable streaming conversation"
```

---

### Task 7: Native speech service (translator-io)

**Files:**
- Create: `translator-io/src/native/mod.rs`, `translator-io/src/native/lru.rs`, `translator-io/src/native/speech.rs`
- Modify: `translator-io/Cargo.toml`, `translator-io/src/lib.rs`

**Interfaces:**
- Consumes:
  - `MoonshineLibrary`, `Transcriber`, `Synthesizer` and `ModelArch` (Tasks 3 and 5).
  - `ModelPaths`, `SttModelArch`, `stt_model` and `tts_voice` (Task 1).
  - `PipelineError::Engine` (Task 1).
  - `STT_SAMPLE_RATE` (`translator-core/src/pipeline.rs`).
- Produces:
  - `native::default_lib_dir() -> PathBuf`
  - `native::NativeSpeech::spawn(lib_dir: PathBuf, paths: ModelPaths, preload: Vec<LanguageCode>) -> NativeSpeech`. It implements `SpeechToText + TextToSpeech`.
  - `native::lru::LruCache<K, V>` with `new(capacity)` and `get_or_try_insert_with`.
  - Crate-private `native::engine_error(service, impl Display) -> PipelineError`.

- [ ] **Step 1: Write the failing tests**

`translator-io/Cargo.toml`: add the following under `[dependencies]`, and add `tempfile = "3"` under a `[dev-dependencies]` section, creating the section if it does not exist.

```toml
moonshine-sys = { path = "../moonshine-sys" }
litert-lm-sys = { path = "../litert-lm-sys" }
```

`translator-io/src/lib.rs`: add `pub mod native;` in alphabetical order.

`translator-io/src/native/mod.rs`:

```rust
//! In-process inference through the vendor libraries (spec phase B). Each
//! service owns one worker thread that holds every native handle; job threads
//! talk to it over channels.

pub mod lru;
pub mod speech;
```

`translator-io/src/native/lru.rs`, tests only for now:

```rust
#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    struct Recorded(&'static str, Rc<RefCell<Vec<&'static str>>>);

    impl Drop for Recorded {
        fn drop(&mut self) {
            self.1.borrow_mut().push(self.0);
        }
    }

    #[test]
    fn evicts_the_least_recently_used_entry_before_creating() {
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let mut cache = LruCache::new(2);
        let make = |name| {
            let dropped = Rc::clone(&dropped);
            move || Ok::<_, ()>(Recorded(name, dropped))
        };
        cache.get_or_try_insert_with("a", make("a")).unwrap();
        cache.get_or_try_insert_with("b", make("b")).unwrap();
        cache.get_or_try_insert_with("a", make("unused")).unwrap();
        assert!(dropped.borrow().is_empty());
        let created_c = cache
            .get_or_try_insert_with("c", || {
                // "b" (least recently used) must already be gone: two models never
                // share memory with a third during a load on the Pi.
                assert_eq!(*dropped.borrow(), ["b"]);
                make("c")()
            })
            .unwrap();
        assert_eq!(created_c.0, "c");
        assert_eq!(cache.get_or_try_insert_with("a", make("again")).unwrap().0, "a");
    }

    #[test]
    fn failed_creation_leaves_no_entry() {
        let mut cache: LruCache<u8, u8> = LruCache::new(2);
        assert_eq!(cache.get_or_try_insert_with(1, || Err("nope")), Err("nope"));
        assert_eq!(cache.get_or_try_insert_with(1, || Ok::<_, &str>(7)), Ok(&mut 7));
    }
}
```

`translator-io/src/native/speech.rs`, tests only for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_fails_every_request() {
        let dir = tempfile::tempdir().unwrap();
        let speech = NativeSpeech::spawn(
            dir.path().to_path_buf(),
            ModelPaths::new(dir.path()),
            vec![LanguageCode::En],
        );
        let error = speech.transcribe(&[0.0; 160], LanguageCode::En).unwrap_err();
        assert!(error.to_string().contains("libmoonshine.so"), "{error}");
        let error = speech.synthesize("hi", LanguageCode::En).unwrap_err();
        assert!(error.to_string().contains("libmoonshine.so"), "{error}");
    }
}
```

Integration tests that need the library, in the new file `translator-io/tests/native_services.rs` (Task 8 appends to it):

```rust
use std::path::PathBuf;

use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{SpeechToText, TextToSpeech, STT_SAMPLE_RATE};
use translator_io::audio_convert::resample_linear;
use translator_io::native::speech::NativeSpeech;

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")))
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn synthesized_speech_round_trips_through_the_speech_thread() {
    let speech = NativeSpeech::spawn(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR")),
        vec![LanguageCode::En, LanguageCode::Zh],
    );
    let clip = speech.synthesize("Good morning, how are you today?", LanguageCode::En).unwrap();
    let samples = resample_linear(&clip.samples, clip.sample_rate, STT_SAMPLE_RATE);
    let text = speech.transcribe(&samples, LanguageCode::En).unwrap();
    assert!(text.to_lowercase().contains("morning"), "transcript: {text:?}");
    let chinese = speech.synthesize("早上好。", LanguageCode::Zh).unwrap();
    assert!(!chinese.samples.is_empty());
}

#[test]
#[ignore = "needs native libraries: make test-ffi"]
fn missing_model_names_the_tool() {
    let empty_models = tempfile::tempdir().unwrap();
    let speech = NativeSpeech::spawn(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(empty_models.path()),
        Vec::new(),
    );
    let error = speech.transcribe(&[0.0; 1600], LanguageCode::Ko).unwrap_err();
    assert!(error.to_string().contains("translator-models"), "{error}");
    assert!(error.to_string().contains("tiny-ko"), "{error}");
    let error = speech.synthesize("hola", LanguageCode::Es).unwrap_err();
    assert!(error.to_string().contains("translator-models"), "{error}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-io native`
Expected: compile errors, because `LruCache` and `NativeSpeech` are missing.

- [ ] **Step 3: Implement**

Prepend to `translator-io/src/native/lru.rs`:

```rust
/// Keeps at most `capacity` values, dropping the least recently used one
/// before a new value is created (models are large; never hold three).
pub struct LruCache<K, V> {
    capacity: usize,
    /// Most recently used last.
    entries: Vec<(K, V)>,
}

impl<K: PartialEq + Copy, V> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        LruCache {
            capacity,
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn get_or_try_insert_with<E>(
        &mut self,
        key: K,
        create: impl FnOnce() -> Result<V, E>,
    ) -> Result<&mut V, E> {
        if let Some(position) = self.entries.iter().position(|(existing, _)| *existing == key) {
            let entry = self.entries.remove(position);
            self.entries.push(entry);
        } else {
            if self.entries.len() >= self.capacity {
                self.entries.remove(0);
            }
            let value = create()?;
            self.entries.push((key, value));
        }
        let (_, value) = self.entries.last_mut().expect("an entry was just pushed");
        Ok(value)
    }
}
```

Add to `translator-io/src/native/mod.rs`, below the two `pub mod` lines:

```rust
use std::fmt::Display;
use std::path::PathBuf;

use translator_core::pipeline::PipelineError;

/// `<directory of the running executable>/lib`, where deployment puts the .so files.
pub fn default_lib_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|directory| directory.join("lib")))
        .unwrap_or_else(|| PathBuf::from("lib"))
}

pub(crate) fn engine_error(service: &'static str, error: impl Display) -> PipelineError {
    PipelineError::Engine {
        service,
        message: error.to_string(),
    }
}
```

Prepend to `translator-io/src/native/speech.rs`:

```rust
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use moonshine_sys::{ModelArch, MoonshineLibrary, Synthesizer, Transcriber};
use translator_core::language::LanguageCode;
use translator_core::models::{ModelPaths, SttModelArch};
use translator_core::pipeline::{AudioClip, PipelineError, SpeechToText, TextToSpeech, STT_SAMPLE_RATE};

use super::engine_error;
use super::lru::LruCache;

const SERVICE: &str = "speech engine";
/// Same as `MAX_MODELS` in the Python service.
const MODELS_PER_KIND: usize = 2;

/// Speech recognition and synthesis on one thread that owns every Moonshine handle.
pub struct NativeSpeech {
    requests: Sender<SpeechRequest>,
}

enum SpeechRequest {
    Transcribe {
        samples: Vec<f32>,
        language: LanguageCode,
        reply: Sender<Result<String, PipelineError>>,
    },
    Synthesize {
        text: String,
        language: LanguageCode,
        reply: Sender<Result<AudioClip, PipelineError>>,
    },
}

impl NativeSpeech {
    /// Starts the speech thread. It loads `libmoonshine.so` from `lib_dir`,
    /// then both kinds of model for each `preload` language, then serves requests.
    pub fn spawn(lib_dir: PathBuf, paths: ModelPaths, preload: Vec<LanguageCode>) -> NativeSpeech {
        let (requests, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("speech".to_string())
            .spawn(move || run_speech_thread(&lib_dir, paths, &preload, receiver))
            .expect("spawning the speech thread");
        NativeSpeech { requests }
    }
}

fn stopped() -> PipelineError {
    engine_error(SERVICE, "speech thread stopped")
}

impl SpeechToText for NativeSpeech {
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send(SpeechRequest::Transcribe {
                samples: samples.to_vec(),
                language,
                reply,
            })
            .map_err(|_| stopped())?;
        response.recv().map_err(|_| stopped())?
    }
}

impl TextToSpeech for NativeSpeech {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError> {
        let (reply, response) = mpsc::channel();
        self.requests
            .send(SpeechRequest::Synthesize {
                text: text.to_string(),
                language,
                reply,
            })
            .map_err(|_| stopped())?;
        response.recv().map_err(|_| stopped())?
    }
}

fn run_speech_thread(
    lib_dir: &Path,
    paths: ModelPaths,
    preload: &[LanguageCode],
    requests: Receiver<SpeechRequest>,
) {
    let library = match MoonshineLibrary::load(lib_dir) {
        Ok(library) => library,
        Err(error) => {
            let message = error.to_string();
            eprintln!("[speech] {message}");
            for request in requests {
                match request {
                    SpeechRequest::Transcribe { reply, .. } => {
                        let _ = reply.send(Err(engine_error(SERVICE, &message)));
                    }
                    SpeechRequest::Synthesize { reply, .. } => {
                        let _ = reply.send(Err(engine_error(SERVICE, &message)));
                    }
                }
            }
            return;
        }
    };
    let mut engines = SpeechEngines {
        library,
        paths,
        transcribers: LruCache::new(MODELS_PER_KIND),
        synthesizers: LruCache::new(MODELS_PER_KIND),
    };
    for &language in preload {
        if let Err(error) = engines.transcriber(language) {
            eprintln!("[speech] preloading {} recognizer: {error}", language.code());
        }
        if let Err(error) = engines.synthesizer(language) {
            eprintln!("[speech] preloading {} voice: {error}", language.code());
        }
    }
    for request in requests {
        match request {
            SpeechRequest::Transcribe { samples, language, reply } => {
                let result = engines
                    .transcriber(language)
                    .and_then(|transcriber| {
                        transcriber
                            .transcribe(&samples, STT_SAMPLE_RATE)
                            .map_err(|error| engine_error(SERVICE, error))
                    });
                let _ = reply.send(result);
            }
            SpeechRequest::Synthesize { text, language, reply } => {
                let result = engines.synthesizer(language).and_then(|synthesizer| {
                    synthesizer
                        .synthesize(&text)
                        .map(|(samples, sample_rate)| AudioClip { samples, sample_rate })
                        .map_err(|error| engine_error(SERVICE, error))
                });
                let _ = reply.send(result);
            }
        }
    }
}

struct SpeechEngines {
    library: Arc<MoonshineLibrary>,
    paths: ModelPaths,
    transcribers: LruCache<LanguageCode, Transcriber>,
    synthesizers: LruCache<LanguageCode, Synthesizer>,
}

impl SpeechEngines {
    fn transcriber(&mut self, language: LanguageCode) -> Result<&mut Transcriber, PipelineError> {
        let model = language.stt_model();
        let directory = self.paths.stt_dir(model);
        let library = &self.library;
        self.transcribers.get_or_try_insert_with(language, || {
            require_directory(&directory)?;
            Transcriber::load(library, &directory, model_arch(model.arch)).map_err(|error| engine_error(SERVICE, error))
        })
    }

    fn synthesizer(&mut self, language: LanguageCode) -> Result<&mut Synthesizer, PipelineError> {
        let root = self.paths.tts_root();
        let library = &self.library;
        self.synthesizers.get_or_try_insert_with(language, || {
            require_directory(&root)?;
            let root_text = root
                .to_str()
                .ok_or_else(|| engine_error(SERVICE, format!("{} is not UTF-8", root.display())))?;
            let voice = language.tts_voice();
            Synthesizer::create(library, voice.language_tag, &voice.synthesizer_options(root_text))
                .map_err(|error| engine_error(SERVICE, error))
        })
    }
}

fn require_directory(directory: &Path) -> Result<(), PipelineError> {
    if directory.is_dir() {
        Ok(())
    } else {
        Err(engine_error(
            SERVICE,
            format!("{} is missing; run translator-models", directory.display()),
        ))
    }
}

fn model_arch(arch: SttModelArch) -> ModelArch {
    match arch {
        SttModelArch::Tiny => ModelArch::Tiny,
        SttModelArch::Base => ModelArch::Base,
        SttModelArch::MediumStreaming => ModelArch::MediumStreaming,
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p translator-io && make test-ffi && cargo clippy --workspace --all-targets -- -D warnings`
Expected: the unit tests pass, `test-ffi` passes both `native_services` tests, and there are no warnings.

- [ ] **Step 5: Commit**

```bash
git add translator-io
git commit -m "Add the in-process speech service on a dedicated Moonshine thread"
```

---

### Task 8: Native translator service (translator-io)

**Files:**
- Create: `translator-io/src/native/translator.rs`
- Modify: `translator-io/src/native/mod.rs` (add `pub mod translator;`)

**Interfaces:**
- Consumes:
  - `LiteRtLmLibrary`, `Engine` and `Conversation::send_message_stream` (Task 6).
  - `ModelPaths::{gemma_file, llm_cache_dir}` (Task 1).
  - `translation_system_prompt`, `clean_model_reply` and `SentenceSplitter` (`translator-core/src/text.rs`).
  - `engine_error` (Task 7).
- Produces: `native::translator::NativeTranslator::spawn(lib_dir: PathBuf, paths: ModelPaths) -> NativeTranslator`, which implements `Translator`.

- [ ] **Step 1: Write the failing tests**

`translator-io/src/native/translator.rs`, tests only. The tests play the model thread themselves through the private `requests` channel:

```rust
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn with_fake_worker(
        script: impl FnOnce(TranslateRequest) + Send + 'static,
    ) -> NativeTranslator {
        let (requests, receiver) = mpsc::channel::<TranslateRequest>();
        thread::spawn(move || {
            if let Ok(request) = receiver.recv() {
                script(request);
            }
        });
        NativeTranslator { requests }
    }

    fn translate(translator: &NativeTranslator, cancel: &AtomicBool) -> (Result<(), PipelineError>, Vec<String>) {
        let mut sentences = Vec::new();
        let result = translator.translate_streaming(
            "hello",
            LanguageCode::En,
            LanguageCode::Es,
            cancel,
            &mut |sentence| sentences.push(sentence),
        );
        (result, sentences)
    }

    #[test]
    fn fragments_become_cleaned_sentences() {
        let translator = with_fake_worker(|request| {
            assert_eq!(request.text, "hello");
            for fragment in ["\"Hola mun", "do. ¿Cómo es", "tás?\""] {
                request.events.send(TranslationEvent::Fragment(fragment.to_string())).unwrap();
            }
            request.events.send(TranslationEvent::Finished(Ok(()))).unwrap();
        });
        let (result, sentences) = translate(&translator, &AtomicBool::new(false));
        assert_eq!(result, Ok(()));
        assert_eq!(sentences, ["Hola mundo.", "¿Cómo estás?"]);
    }

    #[test]
    fn worker_error_is_returned() {
        let translator = with_fake_worker(|request| {
            let error = engine_error(SERVICE, "boom");
            request.events.send(TranslationEvent::Finished(Err(error))).unwrap();
        });
        let (result, _) = translate(&translator, &AtomicBool::new(false));
        assert_eq!(result, Err(engine_error(SERVICE, "boom")));
    }

    #[test]
    fn caller_cancel_returns_promptly_and_flags_worker() {
        let (flag_sender, flag_receiver) = mpsc::channel();
        let translator = with_fake_worker(move |request| {
            request.events.send(TranslationEvent::Fragment("Uno. ".to_string())).unwrap();
            // Like the real worker, keep generating until told to stop.
            while !request.cancel.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            flag_sender.send(()).unwrap();
        });
        let cancel = AtomicBool::new(false);
        let mut sentences = Vec::new();
        let started = Instant::now();
        let result = translator.translate_streaming(
            "hello",
            LanguageCode::En,
            LanguageCode::Es,
            &cancel,
            &mut |sentence| {
                sentences.push(sentence);
                cancel.store(true, Ordering::SeqCst);
            },
        );
        assert_eq!(result, Ok(()));
        assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
        assert_eq!(sentences, ["Uno."]);
        flag_receiver.recv_timeout(Duration::from_secs(1)).expect("worker saw the cancel");
    }

    #[test]
    fn stopped_worker_is_an_error() {
        let (requests, receiver) = mpsc::channel::<TranslateRequest>();
        drop(receiver);
        let translator = NativeTranslator { requests };
        let (result, _) = translate(&translator, &AtomicBool::new(false));
        assert!(result.unwrap_err().to_string().contains("stopped"));
    }

    #[test]
    fn missing_library_fails_translation() {
        let dir = tempfile::tempdir().unwrap();
        let translator = NativeTranslator::spawn(dir.path().to_path_buf(), ModelPaths::new(dir.path()));
        let (result, _) = translate(&translator, &AtomicBool::new(false));
        assert!(result.unwrap_err().to_string().contains("liblitert-lm.so"));
    }
}
```

Add `pub mod translator;` to `translator-io/src/native/mod.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p translator-io native::translator`
Expected: compile errors, because `NativeTranslator`, `TranslateRequest` and `TranslationEvent` are missing.

- [ ] **Step 3: Implement**

Prepend to `translator-io/src/native/translator.rs`:

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use litert_lm_sys::{Engine, LiteRtLmLibrary};
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{PipelineError, Translator};
use translator_core::text::{clean_model_reply, translation_system_prompt, SentenceSplitter};

use super::engine_error;

const SERVICE: &str = "translation engine";
/// How often a waiting job thread checks its cancel flag.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Streaming translation on one thread that owns the litert-lm engine.
pub struct NativeTranslator {
    requests: Sender<TranslateRequest>,
}

struct TranslateRequest {
    text: String,
    source: LanguageCode,
    target: LanguageCode,
    events: Sender<TranslationEvent>,
    /// Set by the job thread when its caller cancels; the model thread then
    /// cancels generation.
    cancel: Arc<AtomicBool>,
}

enum TranslationEvent {
    Fragment(String),
    Finished(Result<(), PipelineError>),
}

impl NativeTranslator {
    /// Starts the model thread, which loads `liblitert-lm.so` and Gemma
    /// before serving requests (requests sent meanwhile wait in the queue).
    pub fn spawn(lib_dir: PathBuf, paths: ModelPaths) -> NativeTranslator {
        let (requests, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("model".to_string())
            .spawn(move || run_model_thread(&lib_dir, &paths, receiver))
            .expect("spawning the model thread");
        NativeTranslator { requests }
    }
}

fn stopped() -> PipelineError {
    engine_error(SERVICE, "model thread stopped")
}

impl Translator for NativeTranslator {
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        let request_cancel = Arc::new(AtomicBool::new(false));
        let (events, receiver) = mpsc::channel();
        self.requests
            .send(TranslateRequest {
                text: text.to_string(),
                source,
                target,
                events,
                cancel: Arc::clone(&request_cancel),
            })
            .map_err(|_| stopped())?;

        let mut splitter = SentenceSplitter::new();
        let mut emit = |sentences: Vec<String>| {
            for sentence in sentences {
                let cleaned = clean_model_reply(&sentence);
                if !cleaned.is_empty() {
                    on_sentence(cleaned);
                }
            }
        };
        loop {
            if cancel.load(Ordering::SeqCst) {
                // The model thread finishes cancelling on its own.
                request_cancel.store(true, Ordering::SeqCst);
                return Ok(());
            }
            match receiver.recv_timeout(CANCEL_POLL_INTERVAL) {
                Ok(TranslationEvent::Fragment(fragment)) => emit(splitter.push(&fragment)),
                Ok(TranslationEvent::Finished(result)) => {
                    result?;
                    if !cancel.load(Ordering::SeqCst) {
                        emit(splitter.finish());
                    }
                    return Ok(());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(stopped()),
            }
        }
    }
}

fn run_model_thread(lib_dir: &Path, paths: &ModelPaths, requests: Receiver<TranslateRequest>) {
    let engine = match load_engine(lib_dir, paths) {
        Ok(engine) => engine,
        Err(error) => {
            let message = error.to_string();
            eprintln!("[model] {message}");
            for request in requests {
                let _ = request
                    .events
                    .send(TranslationEvent::Finished(Err(engine_error(SERVICE, &message))));
            }
            return;
        }
    };
    for request in requests {
        let result = if request.cancel.load(Ordering::SeqCst) {
            Ok(())
        } else {
            translate_once(&engine, &request)
        };
        let _ = request.events.send(TranslationEvent::Finished(result));
    }
}

/// The engine keeps its own reference to the library, so only the engine is returned.
fn load_engine(lib_dir: &Path, paths: &ModelPaths) -> Result<Engine, String> {
    let library = LiteRtLmLibrary::load(lib_dir).map_err(|error| error.to_string())?;
    let cache_dir = paths.llm_cache_dir();
    fs::create_dir_all(&cache_dir).map_err(|error| format!("{}: {error}", cache_dir.display()))?;
    Engine::new(&library, &paths.gemma_file(), &cache_dir).map_err(|error| error.to_string())
}

fn translate_once(engine: &Engine, request: &TranslateRequest) -> Result<(), PipelineError> {
    // A fresh conversation per translation: no context carries over (spec §6.3).
    let mut conversation = engine
        .create_conversation(&translation_system_prompt(request.source, request.target))
        .map_err(|error| engine_error(SERVICE, error))?;
    let events = &request.events;
    conversation
        .send_message_stream(
            &request.text,
            &mut |text| {
                let _ = events.send(TranslationEvent::Fragment(text.to_string()));
            },
            &|| request.cancel.load(Ordering::SeqCst),
        )
        .map(|_| ())
        .map_err(|error| engine_error(SERVICE, error))
}
```

Add an ignored integration test at the end of `translator-io/tests/native_services.rs`:

```rust
use std::sync::atomic::AtomicBool;

use translator_core::pipeline::Translator;
use translator_io::native::translator::NativeTranslator;

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn translates_english_to_chinese_sentence_by_sentence() {
    let translator = NativeTranslator::spawn(
        env_dir("TRANSLATOR_NATIVE_LIB_DIR"),
        ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR")),
    );
    let mut sentences = Vec::new();
    translator
        .translate_streaming(
            "Good morning. How are you today?",
            LanguageCode::En,
            LanguageCode::Zh,
            &AtomicBool::new(false),
            &mut |sentence| sentences.push(sentence),
        )
        .unwrap();
    assert!(!sentences.is_empty());
    assert!(
        sentences.concat().chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
        "{sentences:?}"
    );
}
```

Merge the `use` lines into the file's existing imports.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p translator-io && make test-ffi && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all unit tests pass (`caller_cancel_returns_promptly_and_flags_worker` in under 100 ms), every `test-ffi` test passes, and there are no warnings.

- [ ] **Step 5: Commit**

```bash
git add translator-io
git commit -m "Add the in-process streaming translator on a dedicated litert-lm thread"
```

---

### Task 9: Wire the native backend into the UI, end-to-end test, docs

**Files:**
- Modify: `translator-ui/src/app.rs`, `Makefile`, `README.md`, `CLAUDE.md`
- Create: `translator-io/tests/native_pipeline.rs`

**Interfaces:**
- Consumes:
  - `Config.inference_backend` and `Config.model_dir` (Task 1).
  - `NativeSpeech::spawn` (Task 7).
  - `NativeTranslator::spawn` (Task 8).
  - `default_lib_dir` (Task 7).
  - `JobRunner`, `Services` and `PlaybackSink` (phase A).
- Produces: `make run-native`, and the documentation.

- [ ] **Step 1: Write the failing end-to-end test**

`translator-io/tests/native_pipeline.rs`:

```rust
//! Whole job (speech → text → translation → speech) through JobRunner with
//! the in-process services, as the UI runs it with inference_backend = "native".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use translator_core::controller::{JobEvent, JobId, JobRequest};
use translator_core::language::LanguageCode;
use translator_core::models::ModelPaths;
use translator_core::pipeline::{AudioClip, TextToSpeech, STT_SAMPLE_RATE};
use translator_io::audio_convert::resample_linear;
use translator_io::native::speech::NativeSpeech;
use translator_io::native::translator::NativeTranslator;
use translator_io::playback_queue::PlaybackSink;
use translator_io::runner::{JobRunner, Services};

#[derive(Default)]
struct CollectingSink {
    clips: Mutex<Vec<AudioClip>>,
}

impl PlaybackSink for CollectingSink {
    fn enqueue(&self, clip: AudioClip) {
        self.clips.lock().unwrap().push(clip);
    }

    fn enqueue_unless_cancelled(&self, clip: AudioClip, cancel: &AtomicBool) {
        if !cancel.load(Ordering::SeqCst) {
            self.clips.lock().unwrap().push(clip);
        }
    }

    fn clear(&self) {
        self.clips.lock().unwrap().clear();
    }

    fn is_idle(&self) -> bool {
        true
    }
}

fn env_dir(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name} (make test-ffi does)")))
}

#[test]
#[ignore = "needs native libraries and models: make test-ffi"]
fn english_speech_becomes_spoken_chinese() {
    let lib_dir = env_dir("TRANSLATOR_NATIVE_LIB_DIR");
    let paths = ModelPaths::new(env_dir("TRANSLATOR_MODEL_DIR"));
    let speech = Arc::new(NativeSpeech::spawn(
        lib_dir.clone(),
        paths.clone(),
        vec![LanguageCode::En, LanguageCode::Zh],
    ));
    let utterance = speech.synthesize("Good morning. How are you today?", LanguageCode::En).unwrap();
    let samples = resample_linear(&utterance.samples, utterance.sample_rate, STT_SAMPLE_RATE);

    let sink = Arc::new(CollectingSink::default());
    let (sender, events) = mpsc::channel();
    let runner = JobRunner::new(
        Services {
            stt: speech.clone(),
            translator: Arc::new(NativeTranslator::spawn(lib_dir, paths)),
            tts: speech,
        },
        sink.clone(),
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    );
    runner.start(
        JobRequest {
            id: JobId(1),
            source: LanguageCode::En,
            target: LanguageCode::Zh,
            enable_tts: true,
        },
        samples,
    );

    let mut transcript = String::new();
    let mut sentences = Vec::new();
    loop {
        match events.recv_timeout(Duration::from_secs(120)).expect("job finished in time") {
            JobEvent::Transcribed { text, .. } => transcript = text,
            JobEvent::SentenceTranslated { sentence, .. } => sentences.push(sentence),
            JobEvent::TranslationFinished { .. } => {}
            JobEvent::PlaybackFinished { .. } => break,
            JobEvent::Failed { message, .. } => panic!("job failed: {message}"),
        }
    }
    assert!(transcript.to_lowercase().contains("morning"), "{transcript:?}");
    assert!(!sentences.is_empty());
    assert!(!sink.clips.lock().unwrap().is_empty(), "nothing was spoken");
}
```

`JobRunner` emits `PlaybackFinished` once `is_idle()` returns true (`translator-io/src/runner.rs:204-207`). This sink is always idle, so the event follows directly after the last clip is queued.

- [ ] **Step 2: Run it to verify it passes against Tasks 7–8**

Run: `make test-ffi`
Expected: `english_speech_becomes_spoken_chinese` passes. This test exercises code that already exists, so it may pass on the first run. If it fails, the bug is in Tasks 7–8: fix it there.

- [ ] **Step 3: Select services from config in `translator-ui/src/app.rs`**

Replace the `let services = if is_mock { … }` block with:

```rust
        let services = if is_mock {
            mock::services()
        } else {
            match config.inference_backend {
                InferenceBackend::Http => {
                    let speech = Arc::new(SpeechServiceClient::new(&config.speech_service_url));
                    Services {
                        stt: speech.clone(),
                        translator: Arc::new(LlmClient::new(&config.llm_endpoint, &config.llm_model)),
                        tts: speech,
                    }
                }
                InferenceBackend::Native => native_services(&config),
            }
        };
```

Add this free function to `app.rs`:

```rust
/// In-process inference: one speech thread (preloading both lanes' languages)
/// and one model thread, both loading their libraries from `<exe dir>/lib`.
fn native_services(config: &Config) -> Services {
    let lib_dir = native::default_lib_dir();
    let paths = ModelPaths::new(&config.model_dir);
    let speech = Arc::new(NativeSpeech::spawn(
        lib_dir.clone(),
        paths.clone(),
        vec![config.lane1_language, config.lane2_language],
    ));
    Services {
        stt: speech.clone(),
        translator: Arc::new(NativeTranslator::spawn(lib_dir, paths)),
        tts: speech,
    }
}
```

Add these imports:
- `translator_core::config::InferenceBackend`, merged into the existing `config` import.
- `translator_core::models::ModelPaths`.
- `translator_io::native::{self, speech::NativeSpeech, translator::NativeTranslator}`.

- [ ] **Step 4: Add `run-native` to the Makefile**

Add after `run`:

```make
## run-native: run the UI with in-process inference (config: inference_backend = "native", model_dir = "<repo>/models")
.PHONY: run-native
run-native:
	$(CARGO) build -p translator-ui
	ln -sfn $(NATIVE_LIB_DIR) target/debug/lib
	./target/debug/translator
```

- [ ] **Step 5: Verify by hand on the desktop**

1. Stop `server.py` and `litert-lm serve` if they are running, so a failure cannot be hidden by the HTTP path: `pkill -f backend/server.py; pkill -f 'litert-lm serve'`.
2. In `~/.config/gemma-translator/config.toml`, set `inference_backend = "native"` and `model_dir = "<repo>/models"`.
3. Run `make run-native`. Hold `Z`, say "good morning", and release.

Expected:
- The transcript shows your words.
- Translated sentences appear one by one and are spoken.
- Pressing `Z` again mid-speech stops speech immediately.

4. Rename `target/debug/lib` and run `./target/debug/translator` directly. Expected: recording ends with `Error: speech engine failed: cannot load …/libmoonshine.so …` and the UI keeps responding.
5. Restore the symlink, and set `inference_backend = "http"` back if you prefer.

- [ ] **Step 6: Document it**

`README.md`: under "Rust UI (experimental)", add a subsection "In-process inference (phase B, experimental)". It covers:

````markdown
### In-process inference (phase B, experimental)

The `translator` binary can run speech recognition, translation and speech
synthesis itself, without `server.py` or `litert-lm serve`:

```bash
sudo apt-get install -y patchelf unzip libvulkan1
make native-libs     # pinned libmoonshine / liblitert-lm into native/<arch>/lib
make models          # ~3.5 GB of models into ./models (MODEL_DIR=... to change)
make test-ffi        # FFI tests against those libraries and models
```

Then set in `~/.config/gemma-translator/config.toml`:

```toml
inference_backend = "native"
model_dir = "/path/to/models"
```

and start it with `make run-native`. The libraries are loaded from `lib/`
next to the executable. `make test-soak` runs the long memory test.
````

`CLAUDE.md`: in the "Rust UI" section, add these lines:

```markdown
- `moonshine-sys` / `litert-lm-sys`: run-time (`libloading`) bindings to the vendor libraries from the pinned wheels; signatures are copied from the wheels' ctypes code, so re-check them on any version bump. Handles are `!Send` and live on one worker thread each (`translator-io/src/native/`).
- `translator-models`: downloads Gemma, the Moonshine STT models and TTS assets into `model_dir` (layout in `translator-core/src/models.rs`).
- `inference_backend = "native"` in the config switches the UI from the HTTP services to the native ones. Native tests are `#[ignore]`d: `make native-libs models test-ffi`.
```

Also add `make native-libs`, `make models`, `make test-ffi` and `make run-native` to the Commands block in `CLAUDE.md`.

- [ ] **Step 7: Final checks**

Run: `make test && make clippy && cargo fmt --all -- --check && make test-ffi`
Expected: everything passes. `make test` reports the FFI tests as ignored and does not fail.

- [ ] **Step 8: Commit**

```bash
git add translator-ui translator-io/tests/native_pipeline.rs Makefile README.md CLAUDE.md
git commit -m "Let the UI run inference in-process when inference_backend is native"
```

---

## Manual checklist on the Pi 5 (spec §11 steps 7, 8 and 10; not executed by agents)

**Status (2026-10-10): closed.** The user ran the Pi 5 checks and reported that all of them pass: spec §11 phase A step 1 (Slint on linuxkms), the phase A step 6 acceptance run, and phase B steps 7, 8 and 10 below. Not done yet: the deployment switch that follows phase A step 6, and step 11.

Run these on a Pi 5 8GB with Raspberry Pi OS Bookworm (glibc 2.36):

1. **Step 7 (FFI on aarch64):**
   ```bash
   sudo apt-get install -y patchelf unzip libvulkan1 && make native-libs
   ```
   - `ldd native/aarch64/lib/*.so | grep 'not found'` prints nothing, which covers the glibc check.
   - `make test-ffi` and `make test-soak` pass. These cover both libraries in one process, the callback threads, cancel, and transcript ownership.
2. **Step 8 (models on a clean Pi):** on a fresh image, run
   ```bash
   cargo build --release -p translator-models && \
     mkdir -p target/release/lib && cp native/aarch64/lib/* target/release/lib/ && \
     sudo mkdir -p /var/lib/gemma-translator/models && sudo chown $USER /var/lib/gemma-translator/models && \
     target/release/translator-models --model-dir /var/lib/gemma-translator/models
   ```
   It must end with `all N model files ready`, and a second run must print only `present`.
3. **Step 10 (acceptance):** set `inference_backend = "native"` and run spec §12 against the phase A numbers:
   - memory (`smem`, with no Python processes running)
   - boot-to-ready time
   - release-to-first-speech latency over the 5 × 6 sentences
   - identical transcripts against the Python service for the same recordings
   - every row of §3
   - an 8-hour run with stable RSS

## Follow-up after Pi acceptance (spec §11 step 11; a separate plan)

1. Default `inference_backend` to `native`.
2. Delete:
   - `translator-io/src/speech_client.rs`, `translator-io/src/llm_client.rs` and their tests
   - `speech_service_url`, `llm_endpoint` and `llm_model`
   - `backend/`, `setup.sh`, `start.sh`, `download_model.sh` and `venv`
3. Deploy layout and service:
   - `deploy-pi.sh` installs `translator`, `translator-models` and `lib/` into `/opt/gemma-translator/`.
   - It runs `translator-models --model-dir /var/lib/gemma-translator/models`, with `cache/` writable by the service user.
   - It installs `libvulkan1`.
   - The systemd unit starts only `translator`.
4. Update README and CLAUDE.md.

Step 11 requires phase A step 6 (deployment switch) to be done first.
