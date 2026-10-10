use crate::language::LanguageCode;

/// Upper bound for one synthesized speech chunk; longer sentences are split
/// at whitespace (or mid-word for CJK).
pub const MAX_SPEECH_CHUNK_CHARS: usize = 180;

const ALWAYS_TERMINATES: [char; 5] = ['。', '！', '？', '؟', '\n'];
const TERMINATES_BEFORE_SPACE: [char; 3] = ['.', '!', '?'];
const CLOSING_MARKS: [char; 8] = ['"', '\'', '”', '’', '」', '』', ')', '）'];
const QUOTE_PAIRS: [(char, char); 5] = [
    ('"', '"'),
    ('\'', '\''),
    ('“', '”'),
    ('「', '」'),
    ('『', '』'),
];
/// Quotes whose other half may sit in a different sentence after splitting.
/// The ASCII apostrophe is left out: a trailing one is usually a possessive.
const SPANNING_QUOTE_PAIRS: [(char, char); 4] =
    [('"', '"'), ('“', '”'), ('「', '」'), ('『', '』')];
const REPLY_LABELS: [&str; 2] = ["translation:", "translated text:"];

pub fn translation_system_prompt(source: LanguageCode, target: LanguageCode) -> String {
    format!(
        "You are a translator. Translate the user's text from {} into {}. \
         Output only the translation, with no explanations, notes, quotes or formatting.",
        source.prompt_name(),
        target.prompt_name()
    )
}

/// Removes wrappers a small model tends to add around a translation:
/// code fences, "Translation:" labels, markdown emphasis and surrounding quotes.
/// Runs per sentence, so a quote opened in one sentence and closed in a later
/// one is removed from both.
pub fn clean_model_reply(reply: &str) -> String {
    let without_fences: Vec<&str> = reply
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect();
    let mut text = without_fences
        .join("\n")
        .replace("**", "")
        .replace("__", "");
    text = text.trim().trim_start_matches('#').trim().to_string();
    for label in REPLY_LABELS {
        let has_label = text
            .get(..label.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(label));
        if has_label {
            text = text[label.len()..].trim().to_string();
        }
    }
    strip_unmatched_quotes(strip_quote_pair(&text).trim())
        .trim()
        .to_string()
}

fn strip_quote_pair(text: &str) -> &str {
    for (open, close) in QUOTE_PAIRS {
        if let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            return inner;
        }
    }
    text
}

/// Drops a quote mark at either end of `text` whose partner is not in `text`.
fn strip_unmatched_quotes(text: &str) -> &str {
    let mut text = text;
    for (open, close) in SPANNING_QUOTE_PAIRS {
        let is_unmatched = |text: &str, partner: char| {
            if open == close {
                text.matches(open).count() % 2 == 1
            } else {
                !text.contains(partner)
            }
        };
        if text.starts_with(open) && is_unmatched(text, close) {
            text = text[open.len_utf8()..].trim_start();
        }
        if text.ends_with(close) && is_unmatched(text, open) {
            text = text[..text.len() - close.len_utf8()].trim_end();
        }
    }
    text
}

/// Turns a stream of text fragments into speakable sentences of at most
/// `MAX_SPEECH_CHUNK_CHARS` characters.
#[derive(Debug, Default)]
pub struct SentenceSplitter {
    pending: String,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a fragment and returns every sentence completed by it.
    pub fn push(&mut self, fragment: &str) -> Vec<String> {
        self.pending.push_str(fragment);
        self.drain(false)
    }

    /// Returns everything still buffered, split to the length limit.
    pub fn finish(&mut self) -> Vec<String> {
        self.drain(true)
    }

    fn drain(&mut self, is_final: bool) -> Vec<String> {
        let mut sentences = Vec::new();
        loop {
            let found = find_sentence_end(&self.pending, is_final)
                .filter(|&end| self.pending[..end].chars().count() <= MAX_SPEECH_CHUNK_CHARS);
            let end = match found {
                Some(end) => end,
                None if self.pending.chars().count() > MAX_SPEECH_CHUNK_CHARS => {
                    forced_break(&self.pending)
                }
                None => break,
            };
            let rest = self.pending.split_off(end);
            let sentence = std::mem::replace(&mut self.pending, rest);
            push_trimmed(&mut sentences, &sentence);
        }
        if is_final {
            let rest = std::mem::take(&mut self.pending);
            push_trimmed(&mut sentences, &rest);
        }
        sentences
    }
}

/// Byte offset just past the first complete sentence, if any. A terminator
/// at the very end of a non-final buffer waits, because a closing quote or a
/// digit may still follow.
fn find_sentence_end(text: &str, is_final: bool) -> Option<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (index, &(_, c)) in chars.iter().enumerate() {
        let is_always = ALWAYS_TERMINATES.contains(&c);
        if !is_always && !TERMINATES_BEFORE_SPACE.contains(&c) {
            continue;
        }
        let mut after = index + 1;
        while after < chars.len() && is_trailing_mark(chars[after].1) {
            after += 1;
        }
        match chars.get(after) {
            Some(&(byte, next)) if is_always || next.is_whitespace() => return Some(byte),
            Some(_) => continue,
            None if is_final => return Some(text.len()),
            None => return None,
        }
    }
    None
}

fn is_trailing_mark(c: char) -> bool {
    CLOSING_MARKS.contains(&c)
        || ALWAYS_TERMINATES.contains(&c)
        || TERMINATES_BEFORE_SPACE.contains(&c)
}

/// Break point for text over the limit: the last whitespace within the
/// limit, or exactly at the limit when there is none (CJK).
fn forced_break(text: &str) -> usize {
    let limit = text
        .char_indices()
        .nth(MAX_SPEECH_CHUNK_CHARS)
        .map_or(text.len(), |(byte, _)| byte);
    match text[..limit].rfind(char::is_whitespace) {
        Some(byte) if byte > 0 => byte,
        _ => limit,
    }
}

fn push_trimmed(sentences: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
}

/// Tidies a recognizer transcript. For Chinese and Japanese the spaces
/// between characters go first, so the repetition check counts characters
/// rather than characters and spaces.
pub fn clean_transcript(text: &str, language: LanguageCode) -> String {
    if language.is_unspaced_cjk() {
        drop_trailing_repetition(&remove_cjk_spaces(text)).to_string()
    } else {
        drop_trailing_repetition(text).to_string()
    }
}

/// Removes the spaces Moonshine sometimes puts between Chinese or Japanese
/// characters ("请 问" for "请问"). A space with a Latin letter or digit on
/// either side is kept. Not for Korean, which separates words with spaces.
pub fn remove_cjk_spaces(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| {
            !(c == ' '
                && i > 0
                && chars.get(i + 1).is_some_and(|&next| is_cjk(next))
                && is_cjk(chars[i - 1]))
        })
        .map(|(_, &c)| c)
        .collect()
}

/// Han, kana, and CJK or full-width punctuation.
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3000}'..='\u{30FF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{FF00}'..='\u{FFEF}')
}

/// Shortest phrase, in characters, that counts as a repetition loop; shorter
/// repeats ("no, no") are left alone.
const MIN_REPEATED_PHRASE_CHARS: usize = 8;

/// Cuts a recognizer's repetition loop at the end of `text`: when a phrase is
/// followed by one or more copies of it, only the first copy is kept. The loop
/// may end in a partial copy or in a stray tail shorter than the phrase, which
/// goes too. Moonshine falls into such loops when a recording holds several
/// seconds of silence or steady noise.
pub fn drop_trailing_repetition(text: &str) -> &str {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut cut = len;
    let mut run = vec![0usize; len];
    for period in MIN_REPEATED_PHRASE_CHARS..=len / 2 {
        // run[k]: how many characters up to k each equal the one a period earlier.
        for k in 0..len {
            run[k] = if k >= period && chars[k] == chars[k - period] {
                run[k - 1] + 1
            } else {
                0
            };
        }
        // The loop must reach to within one period of the end, and hold a
        // whole second copy; that copy starts at `end - run`.
        for end in (len + 1).saturating_sub(period).max(1)..=len {
            let length = run[end - 1];
            if length >= period {
                cut = cut.min(end - length);
            }
        }
    }
    if cut == len {
        return text;
    }
    let byte = text.char_indices().nth(cut).map_or(text.len(), |(i, _)| i);
    text[..byte].trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_between_cjk_characters_are_removed() {
        assert_eq!(
            remove_cjk_spaces("请 问 最 近 的 火 车 站 在 哪 里 ？"),
            "请问最近的火车站在哪里？"
        );
        assert_eq!(remove_cjk_spaces("駅 は どこ です か"), "駅はどこですか");
        assert_eq!(
            remove_cjk_spaces("我 想 买 iPhone 16 和 AirPods"),
            "我想买 iPhone 16 和 AirPods"
        );
        assert_eq!(remove_cjk_spaces("Good morning."), "Good morning.");
    }

    #[test]
    fn spaced_cjk_loops_are_judged_by_characters() {
        // Four characters repeat: too short to count as a loop, spaced or not.
        assert_eq!(
            clean_transcript("我 不 知 道 我 不 知 道 我 不 知 道", LanguageCode::Zh),
            "我不知道我不知道我不知道"
        );
        let looped = "请 问 车 站 在 哪 里 我 想 买 两 张 到 台 北 的 车 票 \
                      我 想 买 两 张 到 台 北 的 车 票 我 想 买";
        assert_eq!(
            clean_transcript(looped, LanguageCode::Zh),
            "请问车站在哪里我想买两张到台北的车票"
        );
        // Korean keeps its spaces.
        assert_eq!(
            clean_transcript("안녕 하세요", LanguageCode::Ko),
            "안녕 하세요"
        );
    }

    #[test]
    fn repetition_loop_keeps_the_first_copy() {
        let looped = "请 问 车 站 在 哪 里 我 想 买 两 张 车 票 我 想 买 两 张 车 票 我 想 买 两 张 车 票 我 想";
        assert_eq!(
            drop_trailing_repetition(looped),
            "请 问 车 站 在 哪 里 我 想 买 两 张 车 票"
        );
        let whole = "请问最近的车站在哪里？请问最近的车站在哪里？请问最近";
        assert_eq!(drop_trailing_repetition(whole), "请问最近的车站在哪里？");
        // The last copy breaks off into a stray tail shorter than the phrase.
        let stray =
            "请问车站在哪里我想买两张到台北的车票请问车站在哪里我想买两张到台北的车票我想买两张";
        assert_eq!(
            drop_trailing_repetition(stray),
            "请问车站在哪里我想买两张到台北的车票"
        );
    }

    #[test]
    fn text_without_a_loop_is_unchanged() {
        for text in [
            "",
            "Good morning.",
            "No, no, no.",
            "请问最近的火车站在哪里？我想买两张到台北的车票。",
            "I would like to buy two tickets to Tokyo.",
        ] {
            assert_eq!(drop_trailing_repetition(text), text);
        }
    }

    #[test]
    fn prompt_names_both_languages_and_forbids_extras() {
        let prompt = translation_system_prompt(LanguageCode::En, LanguageCode::Ja);
        assert!(prompt.contains("from English into Japanese"));
        assert!(prompt.contains("Output only the translation"));
    }

    #[test]
    fn prompt_asks_for_traditional_chinese() {
        let prompt = translation_system_prompt(LanguageCode::Zh, LanguageCode::En);
        assert!(prompt.contains("from Traditional Chinese into English"));
        let prompt = translation_system_prompt(LanguageCode::Ko, LanguageCode::Zh);
        assert!(prompt.contains("from Korean into Traditional Chinese"));
    }

    #[test]
    fn cleaning_strips_fences_labels_quotes_and_markdown() {
        assert_eq!(clean_model_reply("```\nHola\n```"), "Hola");
        assert_eq!(clean_model_reply("Translation: Bonjour"), "Bonjour");
        assert_eq!(clean_model_reply("TRANSLATED TEXT:  Hallo"), "Hallo");
        assert_eq!(clean_model_reply("\"Hello there.\""), "Hello there.");
        assert_eq!(clean_model_reply("「こんにちは」"), "こんにちは");
        assert_eq!(clean_model_reply("**Bold** move"), "Bold move");
        assert_eq!(clean_model_reply("# Title"), "Title");
        assert_eq!(clean_model_reply("It's fine."), "It's fine.");
        assert_eq!(clean_model_reply("   "), "");
    }

    #[test]
    fn cleaning_drops_quote_halves_left_by_sentence_splitting() {
        assert_eq!(clean_model_reply("\"Hola."), "Hola.");
        assert_eq!(clean_model_reply("¿Qué tal?\""), "¿Qué tal?");
        assert_eq!(clean_model_reply("Translation: \"Hola."), "Hola.");
        assert_eq!(clean_model_reply("“Hola."), "Hola.");
        assert_eq!(clean_model_reply("Adiós.”"), "Adiós.");
        assert_eq!(clean_model_reply("「你好。"), "你好。");
        assert_eq!(clean_model_reply("再见。」"), "再见。");
        assert_eq!(clean_model_reply("『はい。"), "はい。");
    }

    #[test]
    fn cleaning_keeps_quotes_that_are_matched_inside_the_sentence() {
        assert_eq!(
            clean_model_reply("\"Hola\", dijo ella."),
            "\"Hola\", dijo ella."
        );
        assert_eq!(clean_model_reply("Dijo “sí” ayer."), "Dijo “sí” ayer.");
        assert_eq!(clean_model_reply("他说「好」。"), "他说「好」。");
        assert_eq!(clean_model_reply("The dogs' toys."), "The dogs' toys.");
        assert_eq!(clean_model_reply("The dogs'"), "The dogs'");
    }

    #[test]
    fn splits_english_sentences_while_streaming() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("Hello world. How"), ["Hello world."]);
        assert!(splitter.push(" are you?").is_empty());
        assert_eq!(splitter.finish(), ["How are you?"]);
    }

    #[test]
    fn decimal_points_do_not_split() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("Pi is 3.14 today. "), ["Pi is 3.14 today."]);
    }

    #[test]
    fn ellipsis_and_mixed_marks_stay_in_one_sentence() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(
            splitter.push("Wait... Really?! Yes"),
            ["Wait...", "Really?!"]
        );
        assert_eq!(splitter.finish(), ["Yes"]);
    }

    #[test]
    fn cjk_terminators_split_without_spaces() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("你好。今天"), ["你好。"]);
        assert_eq!(splitter.finish(), ["今天"]);
    }

    #[test]
    fn closing_quote_after_terminator_waits_for_more_text() {
        let mut splitter = SentenceSplitter::new();
        assert!(splitter.push("「你好。").is_empty());
        assert_eq!(splitter.push("」再见"), ["「你好。」"]);
        assert_eq!(splitter.finish(), ["再见"]);
    }

    #[test]
    fn arabic_question_mark_splits() {
        let mut splitter = SentenceSplitter::new();
        assert_eq!(splitter.push("كيف حالك؟ "), ["كيف حالك؟"]);
    }

    #[test]
    fn long_english_without_punctuation_breaks_at_spaces() {
        let words: Vec<String> = (0..80).map(|i| format!("word{i}")).collect();
        let text = words.join(" ");
        let mut splitter = SentenceSplitter::new();
        let mut chunks = splitter.push(&text);
        chunks.extend(splitter.finish());
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|c| c.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert_eq!(chunks.join(" "), text);
    }

    #[test]
    fn long_cjk_without_punctuation_breaks_at_the_limit() {
        let text = "字".repeat(400);
        let mut splitter = SentenceSplitter::new();
        let mut chunks = splitter.push(&text);
        chunks.extend(splitter.finish());
        let lengths: Vec<usize> = chunks.iter().map(|c| c.chars().count()).collect();
        assert_eq!(lengths, [180, 180, 40]);
    }

    #[test]
    fn long_sentence_with_terminator_is_still_capped() {
        let text = format!("{}。", "字".repeat(200));
        let mut splitter = SentenceSplitter::new();
        let mut chunks = splitter.push(&text);
        chunks.extend(splitter.finish());
        assert!(chunks
            .iter()
            .all(|c| c.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn finish_on_empty_input_returns_nothing() {
        assert!(SentenceSplitter::new().finish().is_empty());
    }
}
