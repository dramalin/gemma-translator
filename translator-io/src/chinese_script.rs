//! Chinese script conversion around the speech engines: the UI shows
//! Traditional characters, but Moonshine `base-zh` transcribes in Simplified
//! and the Kokoro `zh_hans` voice silently skips Traditional-only characters.

use std::borrow::Cow;

use translator_core::language::LanguageCode;
use zhconv::{zhconv, Variant};

/// zhconv's OpenCC tables leave a character alone when it is valid in both
/// scripts (广 → 廣 or 广), unless a phrase entry settles it. These are such
/// characters that Taiwan text nearly always writes in the Traditional form.
const TRADITIONAL_FALLBACKS: &[(char, char)] = &[('万', '萬'), ('广', '廣')];

/// Traditional characters that zhconv keeps as they are (they are also valid
/// Simplified) but the Kokoro zh_hans voice has no reading for, each with the
/// Simplified character it reads the same as.
const SPOKEN_FALLBACKS: &[(char, char)] = &[
    ('剋', '克'),
    ('劄', '札'),
    ('叚', '假'),
    ('夥', '伙'),
    ('彷', '仿'),
    ('徵', '征'),
    ('扞', '捍'),
    ('於', '于'),
    ('昇', '升'),
    ('椀', '碗'),
    ('淼', '渺'),
    ('澂', '澄'),
    ('犇', '奔'),
    ('甦', '苏'),
    ('瞭', '了'),
    ('筦', '管'),
    ('絜', '洁'),
    ('菉', '绿'),
    ('袷', '夹'),
    ('谿', '溪'),
    ('釐', '厘'),
    ('陞', '升'),
    ('龢', '和'),
];

/// A transcript as shown and translated: Chinese in Taiwan Traditional.
pub fn displayed_transcript(text: &str, language: LanguageCode) -> Cow<'_, str> {
    convert(text, language, Variant::ZhTW, TRADITIONAL_FALLBACKS)
}

/// Text as handed to speech synthesis: Chinese in Simplified characters only,
/// keeping the wording.
pub fn spoken_text(text: &str, language: LanguageCode) -> Cow<'_, str> {
    convert(text, language, Variant::ZhHans, SPOKEN_FALLBACKS)
}

fn convert<'a>(
    text: &'a str,
    language: LanguageCode,
    variant: Variant,
    fallbacks: &[(char, char)],
) -> Cow<'a, str> {
    match language {
        LanguageCode::Zh => Cow::Owned(
            zhconv(text, variant)
                .chars()
                .map(|character| {
                    fallbacks
                        .iter()
                        .find(|(from, _)| *from == character)
                        .map_or(character, |(_, to)| *to)
                })
                .collect(),
        ),
        _ => Cow::Borrowed(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_transcripts_are_shown_in_taiwan_traditional() {
        assert_eq!(
            displayed_transcript("他看着我，请问地铁站怎么走？", LanguageCode::Zh),
            "他看著我，請問地鐵站怎麼走？"
        );
        assert_eq!(
            displayed_transcript("广场门票一万块", LanguageCode::Zh),
            "廣場門票一萬塊"
        );
    }

    #[test]
    fn chinese_is_spoken_from_simplified_with_the_same_words() {
        assert_eq!(
            spoken_text("最近的捷運站在哪裡？會議改到三點半。", LanguageCode::Zh),
            "最近的捷运站在哪里？会议改到三点半。"
        );
        assert_eq!(spoken_text("OK 123", LanguageCode::Zh), "OK 123");
    }

    #[test]
    fn characters_the_voice_cannot_read_are_respelled() {
        assert_eq!(
            spoken_text("關於這個夥伴，我瞭解。", LanguageCode::Zh),
            "关于这个伙伴，我了解。"
        );
    }

    #[test]
    fn other_languages_pass_through_unchanged() {
        assert!(matches!(
            spoken_text("請問", LanguageCode::Ja),
            Cow::Borrowed("請問")
        ));
        assert!(matches!(
            displayed_transcript("说", LanguageCode::Ja),
            Cow::Borrowed("说")
        ));
    }
}
