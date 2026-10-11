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
    TinyStreaming,
    SmallStreaming,
    MediumStreaming,
}

impl SttModelArch {
    pub fn is_streaming(self) -> bool {
        !matches!(self, SttModelArch::Tiny)
    }
}

/// Silence appended after the trimmed speech for a streaming model, in ms.
/// Its encoder looks ahead, so the last word of a recording that ends with
/// only `speech_trim`'s 300 ms of room sound came out wrong ("served" as
/// "sir", "bill" as "bills"); with moonshine-voice 0.1.5 English WER rose
/// from about 1.0% to 2.4%. Digital silence after it restored English and
/// improved every other streaming language on the eval sets; 250 ms was the
/// only length that beat 0.0.65 for both English and Chinese, since 500 ms
/// or more again gave Chinese invented endings in loud fan noise.
pub const STREAMING_TAIL_PADDING_MS: u32 = 250;

/// A language's speech recognition model. Its files, their URLs and checksums
/// come from `moonshine_get_stt_dependencies` for the language and `arch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SttModel {
    /// Directory name under `<model_dir>/stt/`.
    pub name: &'static str,
    pub arch: SttModelArch,
}

impl SttModel {
    /// Silence to append after the trimmed speech (see
    /// `STREAMING_TAIL_PADDING_MS`); none for the non-streaming Korean model,
    /// which was not measured with it.
    pub fn tail_padding_ms(self) -> u32 {
        if self.arch.is_streaming() {
            STREAMING_TAIL_PADDING_MS
        } else {
            0
        }
    }
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

/// Moonshine stops decoding after this many tokens per second of audio. Its
/// default suits alphabetic scripts, but a CJK character takes several
/// byte-level tokens, so the default cuts ja/zh/ko sentences short; 13 was
/// measured to end the truncation on base-zh, and tiny-streaming-zh shows
/// none with it either.
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
    /// Pinned rather than taken from Moonshine's catalog default, so a newer
    /// library cannot switch a language's model unnoticed. Each is the
    /// moonshine-voice 0.1.5 default for its language.
    pub fn stt_model(self) -> SttModel {
        let (name, arch) = match self {
            LanguageCode::Ar => ("tiny-streaming-ar", SttModelArch::TinyStreaming),
            LanguageCode::En => ("medium-streaming-en", SttModelArch::MediumStreaming),
            LanguageCode::Es => ("small-streaming-es", SttModelArch::SmallStreaming),
            LanguageCode::Ja => ("small-streaming-ja", SttModelArch::SmallStreaming),
            LanguageCode::Zh => ("tiny-streaming-zh", SttModelArch::TinyStreaming),
            LanguageCode::Ko => ("tiny-ko", SttModelArch::Tiny),
        };
        SttModel { name, arch }
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

    /// The directory every speech recognition model has its own folder in.
    pub fn stt_root(&self) -> PathBuf {
        self.root.join("stt")
    }

    pub fn stt_dir(&self, model: SttModel) -> PathBuf {
        self.stt_root().join(model.name)
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
        let names: Vec<_> = LanguageCode::ALL
            .iter()
            .map(|language| language.stt_model().name)
            .collect();
        assert_eq!(
            names,
            [
                "tiny-streaming-ar",
                "medium-streaming-en",
                "small-streaming-es",
                "small-streaming-ja",
                "tiny-streaming-zh",
                "tiny-ko"
            ]
        );
        assert_eq!(LanguageCode::Ko.stt_model().arch, SttModelArch::Tiny);
    }

    #[test]
    fn only_streaming_models_get_tail_padding() {
        assert_eq!(LanguageCode::Ko.stt_model().tail_padding_ms(), 0);
        for language in LanguageCode::ALL {
            if language != LanguageCode::Ko {
                assert_eq!(language.stt_model().tail_padding_ms(), 250, "{language:?}");
            }
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
