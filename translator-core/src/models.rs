//! Which Moonshine model and voice each language uses, and where
//! `translator-models` puts every model file. Single source for the native
//! inference path and the model tool.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::language::LanguageCode;

/// File name of the Gemma 4 E2B model inside `<model_dir>/gemma/`.
pub const GEMMA_FILE_NAME: &str = "gemma-4-E2B-it.litertlm";

/// File name of the Silero voice activity detector inside `<model_dir>/vad/`.
pub const VAD_FILE_NAME: &str = "silero_vad.onnx";

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

/// Moonshine TTS language tag, voice and speaking rate.
#[derive(Debug, Clone, PartialEq)]
pub struct TtsVoice {
    pub language_tag: &'static str,
    /// A Moonshine voice id (`kokoro_...` or `piper_...`), or `None` for
    /// Moonshine's default for the language.
    pub voice: Option<String>,
    pub speed: f64,
}

/// Moonshine's speaking rate; also the config default.
pub const DEFAULT_TTS_SPEED: f64 = 1.0;
/// The speaking rates the config accepts. Moonshine takes any positive rate,
/// but outside this range speech gets hard to follow.
pub const TTS_SPEED_RANGE: std::ops::RangeInclusive<f64> = 0.5..=2.0;

/// The config's voice choices and speaking rate.
#[derive(Debug, Clone, PartialEq)]
pub struct TtsSettings {
    pub speed: f64,
    /// Voice ids by language; other languages use `LanguageCode::tts_voice`.
    pub voices: BTreeMap<LanguageCode, String>,
}

impl Default for TtsSettings {
    fn default() -> Self {
        TtsSettings {
            speed: DEFAULT_TTS_SPEED,
            voices: BTreeMap::new(),
        }
    }
}

impl TtsSettings {
    /// The voice `language` speaks with: the configured one, else the built-in.
    pub fn voice(&self, language: LanguageCode) -> TtsVoice {
        let built_in = language.tts_voice();
        TtsVoice {
            voice: self.voices.get(&language).cloned().or(built_in.voice),
            speed: self.speed,
            ..built_in
        }
    }
}

/// Components of a non-streaming model, as Moonshine publishes them.
const NON_STREAMING_COMPONENTS: &[&str] = &[
    "encoder_model.ort",
    "decoder_model_merged.ort",
    "tokenizer.bin",
];

/// Components of the English streaming model, including the attention decoder
/// that only the English model has.
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

/// Moonshine stops decoding after this many tokens per second of audio. Its
/// default suits alphabetic scripts, but a CJK character takes several
/// byte-level tokens, so the default cuts ja/zh/ko sentences short; 13 was
/// measured to end the truncation.
///
/// `vad_threshold` 0 turns off Moonshine's own voice activity detection, so
/// the whole recording is one segment. Its VAD keeps state from earlier calls,
/// which moved the segment start by up to 0.3 s and changed the transcript of
/// the same audio from call to call. Without it, about 4 s of silence in the
/// segment makes the model repeat itself, which
/// `text::drop_trailing_repetition` cuts off; `translator_io::speech_trim`
/// cuts the silence around the speech beforehand with a fresh VAD.
const TRANSCRIBER_OPTIONS: &[(&str, &str)] = &[("vad_threshold", "0")];
const CJK_TRANSCRIBER_OPTIONS: &[(&str, &str)] =
    &[("vad_threshold", "0"), ("max_tokens_per_second", "13.0")];

impl TtsVoice {
    /// Options for `moonshine_create_tts_synthesizer_from_files` and
    /// `moonshine_get_tts_dependencies`.
    /// The speed only matters to the synthesizer and is left out at the default.
    pub fn synthesizer_options(&self, tts_root: &str) -> Vec<(&'static str, String)> {
        let mut options = vec![("g2p_root", tts_root.to_string())];
        if let Some(voice) = &self.voice {
            options.push(("voice", voice.clone()));
        }
        if self.speed != DEFAULT_TTS_SPEED {
            options.push(("speed", self.speed.to_string()));
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

    /// Options for `moonshine_load_transcriber_from_files`.
    pub fn transcriber_options(self) -> &'static [(&'static str, &'static str)] {
        match self {
            LanguageCode::Ja | LanguageCode::Zh | LanguageCode::Ko => CJK_TRANSCRIBER_OPTIONS,
            LanguageCode::Ar | LanguageCode::En | LanguageCode::Es => TRANSCRIBER_OPTIONS,
        }
    }

    pub fn tts_voice(self) -> TtsVoice {
        // zh: Xiaoxiao, softer than Moonshine's default Xiaobei.
        let (language_tag, voice) = match self {
            LanguageCode::Ar => ("ar_msa", None),
            LanguageCode::En => ("en_us", None),
            LanguageCode::Es => ("es_es", None),
            LanguageCode::Ja => ("ja_jp", None),
            LanguageCode::Zh => ("zh_hans", Some("kokoro_zf_xiaoxiao")),
            LanguageCode::Ko => ("ko_kr", None),
        };
        TtsVoice {
            language_tag,
            voice: voice.map(String::from),
            speed: DEFAULT_TTS_SPEED,
        }
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

    /// Silero VAD, which finds where speech starts and ends in a recording.
    pub fn vad_file(&self) -> PathBuf {
        self.root.join("vad").join(VAD_FILE_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_an_stt_model() {
        let ko = LanguageCode::Ko.stt_model();
        assert_eq!(ko.name, "tiny-ko");
        assert_eq!(ko.arch, SttModelArch::Tiny);
        assert_eq!(
            ko.download_url,
            "https://download.moonshine.ai/model/tiny-ko/quantized/tiny-ko"
        );
        assert_eq!(
            ko.components,
            [
                "encoder_model.ort",
                "decoder_model_merged.ort",
                "tokenizer.bin"
            ]
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

        for language in [
            LanguageCode::Ar,
            LanguageCode::Es,
            LanguageCode::Ja,
            LanguageCode::Zh,
        ] {
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
    fn only_cjk_transcribers_raise_the_token_limit_and_none_use_the_vad() {
        for language in LanguageCode::ALL {
            let expected: &[(&str, &str)] = match language {
                LanguageCode::Ja | LanguageCode::Zh | LanguageCode::Ko => {
                    &[("vad_threshold", "0"), ("max_tokens_per_second", "13.0")]
                }
                _ => &[("vad_threshold", "0")],
            };
            assert_eq!(language.transcriber_options(), expected, "{language:?}");
        }
    }

    #[test]
    fn tts_tags_and_voices_match_the_verified_catalog() {
        let tags: Vec<_> = LanguageCode::ALL
            .iter()
            .map(|language| language.tts_voice().language_tag)
            .collect();
        assert_eq!(
            tags,
            ["ar_msa", "en_us", "es_es", "ja_jp", "zh_hans", "ko_kr"]
        );
        assert_eq!(
            LanguageCode::Zh.tts_voice().voice.as_deref(),
            Some("kokoro_zf_xiaoxiao")
        );
        assert_eq!(LanguageCode::En.tts_voice().voice, None);
    }

    #[test]
    fn synthesizer_options_carry_root_and_optional_voice() {
        let options = |voice: TtsVoice| voice.synthesizer_options("/m/tts");
        assert_eq!(
            options(LanguageCode::Zh.tts_voice()),
            [
                ("g2p_root", "/m/tts".to_string()),
                ("voice", "kokoro_zf_xiaoxiao".to_string())
            ]
        );
        assert_eq!(
            options(LanguageCode::Es.tts_voice()),
            [("g2p_root", "/m/tts".to_string())]
        );
    }

    #[test]
    fn configured_voices_and_speed_override_the_built_in_ones() {
        let settings = TtsSettings {
            speed: 1.25,
            voices: BTreeMap::from([(LanguageCode::Zh, "kokoro_zm_yunxi".to_string())]),
        };
        assert_eq!(
            settings
                .voice(LanguageCode::Zh)
                .synthesizer_options("/m/tts"),
            [
                ("g2p_root", "/m/tts".to_string()),
                ("voice", "kokoro_zm_yunxi".to_string()),
                ("speed", "1.25".to_string())
            ]
        );
        let english = settings.voice(LanguageCode::En);
        assert_eq!((english.voice, english.speed), (None, 1.25));
        assert_eq!(
            TtsSettings::default().voice(LanguageCode::Zh),
            LanguageCode::Zh.tts_voice()
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
