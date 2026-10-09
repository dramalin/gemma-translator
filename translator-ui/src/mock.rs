//! Stand-in services for `--mock`: no speech service or model needed, so the UI
//! can be developed on any desktop. Each step is slow enough to see.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use translator_core::language::LanguageCode;
use translator_core::pipeline::{AudioClip, PipelineError, SpeechToText, TextToSpeech, Translator};
use translator_io::runner::Services;

const MOCK_STEP_DELAY: Duration = Duration::from_millis(300);
const MOCK_POLL_INTERVAL: Duration = Duration::from_millis(30);
const MOCK_SAMPLE_RATE: u32 = 16_000;
const MOCK_TONE_HZ: f32 = 440.0;
const MOCK_TONE_SECONDS: f32 = 0.4;
const MOCK_TONE_GAIN: f32 = 0.2;

pub fn services() -> Services {
    Services {
        stt: Arc::new(MockSpeechToText),
        translator: Arc::new(MockTranslator),
        tts: Arc::new(MockTextToSpeech),
    }
}

struct MockSpeechToText;

impl SpeechToText for MockSpeechToText {
    fn transcribe(
        &self,
        _samples: &[f32],
        language: LanguageCode,
    ) -> Result<String, PipelineError> {
        thread::sleep(MOCK_STEP_DELAY);
        Ok(format!(
            "This is a mock {} transcription.",
            language.display_name()
        ))
    }
}

struct MockTranslator;

impl Translator for MockTranslator {
    fn translate_streaming(
        &self,
        _text: &str,
        _source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError> {
        for sentence in mock_sentences(target) {
            if !wait_unless_cancelled(cancel) {
                return Ok(());
            }
            on_sentence(sentence.to_string());
        }
        Ok(())
    }
}

/// Sleeps one mock step; false when cancelled meanwhile.
fn wait_unless_cancelled(cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + MOCK_STEP_DELAY;
    while Instant::now() < deadline {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        thread::sleep(MOCK_POLL_INTERVAL);
    }
    !cancel.load(Ordering::SeqCst)
}

/// Sample text per language, to exercise CJK and right-to-left rendering.
fn mock_sentences(target: LanguageCode) -> [&'static str; 2] {
    match target {
        LanguageCode::Ar => ["هذه ترجمة تجريبية.", "تظهر جملة بعد جملة."],
        LanguageCode::En => [
            "This is a mock translation.",
            "It arrives one sentence at a time.",
        ],
        LanguageCode::Es => ["Esta es una traducción simulada.", "Llega frase por frase."],
        LanguageCode::Ja => ["これは模擬翻訳です。", "一文ずつ表示されます。"],
        LanguageCode::Zh => ["这是一段模拟翻译。", "句子会逐句出现。"],
        LanguageCode::Ko => ["이것은 모의 번역입니다.", "문장 단위로 표시됩니다."],
    }
}

struct MockTextToSpeech;

impl TextToSpeech for MockTextToSpeech {
    fn synthesize(&self, _text: &str, _language: LanguageCode) -> Result<AudioClip, PipelineError> {
        let count = (MOCK_TONE_SECONDS * MOCK_SAMPLE_RATE as f32) as usize;
        let samples = (0..count)
            .map(|i| {
                MOCK_TONE_GAIN * (TAU * MOCK_TONE_HZ * i as f32 / MOCK_SAMPLE_RATE as f32).sin()
            })
            .collect();
        Ok(AudioClip {
            samples,
            sample_rate: MOCK_SAMPLE_RATE,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    #[test]
    fn mock_translator_emits_two_sentences_in_the_target_language() {
        let mut sentences = Vec::new();
        MockTranslator
            .translate_streaming(
                "x",
                LanguageCode::En,
                LanguageCode::Zh,
                &AtomicBool::new(false),
                &mut |s| sentences.push(s),
            )
            .unwrap();
        assert_eq!(sentences.len(), 2);
        assert!(sentences[0].ends_with('。'));
    }

    #[test]
    fn mock_translator_stops_when_cancelled() {
        let mut sentences = Vec::new();
        MockTranslator
            .translate_streaming(
                "x",
                LanguageCode::En,
                LanguageCode::Es,
                &AtomicBool::new(true),
                &mut |s| sentences.push(s),
            )
            .unwrap();
        assert!(sentences.is_empty());
    }

    #[test]
    fn mock_speech_is_a_short_tone() {
        let clip = MockTextToSpeech.synthesize("x", LanguageCode::En).unwrap();
        assert_eq!(clip.sample_rate, 16_000);
        assert_eq!(clip.samples.len(), 6_400);
    }
}
