use crate::language::LanguageCode;

/// Upper bound for one synthesized speech chunk, as in the old UI.
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
const REPLY_LABELS: [&str; 2] = ["translation:", "translated text:"];

pub fn translation_system_prompt(source: LanguageCode, target: LanguageCode) -> String {
    format!(
        "You are a translator. Translate the user's text from {} into {}. \
         Output only the translation, with no explanations, notes, quotes or formatting.",
        source.display_name(),
        target.display_name()
    )
}

/// Removes wrappers a small model tends to add around a translation:
/// code fences, "Translation:" labels, markdown emphasis and surrounding quotes.
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
    strip_quote_pair(&text).trim().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_names_both_languages_and_forbids_extras() {
        let prompt = translation_system_prompt(LanguageCode::En, LanguageCode::Ja);
        assert!(prompt.contains("from English into Japanese"));
        assert!(prompt.contains("Output only the translation"));
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
