use serde::{Deserialize, Serialize};

/// A language offered on the lane revolvers. Serialized as its lowercase code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguageCode {
    Ar,
    En,
    Es,
    Ja,
    Zh,
    Ko,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotateDirection {
    Previous,
    Next,
}

impl LanguageCode {
    /// Revolver order, identical to `AVAILABLE_LANGUAGES` in the old React UI.
    pub const ALL: [LanguageCode; 6] = [
        LanguageCode::Ar,
        LanguageCode::En,
        LanguageCode::Es,
        LanguageCode::Ja,
        LanguageCode::Zh,
        LanguageCode::Ko,
    ];

    pub fn code(self) -> &'static str {
        match self {
            LanguageCode::Ar => "ar",
            LanguageCode::En => "en",
            LanguageCode::Es => "es",
            LanguageCode::Ja => "ja",
            LanguageCode::Zh => "zh",
            LanguageCode::Ko => "ko",
        }
    }

    pub fn from_code(code: &str) -> Option<LanguageCode> {
        LanguageCode::ALL
            .into_iter()
            .find(|language| language.code() == code)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            LanguageCode::Ar => "Arabic",
            LanguageCode::En => "English",
            LanguageCode::Es => "Spanish",
            LanguageCode::Ja => "Japanese",
            LanguageCode::Zh => "Chinese",
            LanguageCode::Ko => "Korean",
        }
    }

    pub fn is_right_to_left(self) -> bool {
        matches!(self, LanguageCode::Ar)
    }

    fn index(self) -> usize {
        LanguageCode::ALL
            .iter()
            .position(|language| *language == self)
            .expect("ALL lists every variant")
    }
}

/// Next language on a lane's revolver, skipping the slot held by the other lane
/// (the two lanes may never show the same language).
pub fn rotate_language(
    current: LanguageCode,
    other: LanguageCode,
    direction: RotateDirection,
) -> LanguageCode {
    let count = LanguageCode::ALL.len();
    let step = match direction {
        RotateDirection::Previous => count - 1,
        RotateDirection::Next => 1,
    };
    let mut index = (current.index() + step) % count;
    if LanguageCode::ALL[index] == other {
        index = (index + step) % count;
    }
    LanguageCode::ALL[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for language in LanguageCode::ALL {
            assert_eq!(LanguageCode::from_code(language.code()), Some(language));
        }
        assert_eq!(LanguageCode::from_code("fr"), None);
    }

    #[test]
    fn revolver_order_matches_old_ui() {
        let codes: Vec<&str> = LanguageCode::ALL.iter().map(|l| l.code()).collect();
        assert_eq!(codes, ["ar", "en", "es", "ja", "zh", "ko"]);
    }

    #[test]
    fn rotate_next_skips_language_of_other_lane() {
        let rotated = rotate_language(LanguageCode::Ar, LanguageCode::En, RotateDirection::Next);
        assert_eq!(rotated, LanguageCode::Es);
    }

    #[test]
    fn rotate_previous_wraps_and_skips() {
        let rotated = rotate_language(
            LanguageCode::Ar,
            LanguageCode::Ko,
            RotateDirection::Previous,
        );
        assert_eq!(rotated, LanguageCode::Zh);
        let rotated = rotate_language(
            LanguageCode::Ar,
            LanguageCode::En,
            RotateDirection::Previous,
        );
        assert_eq!(rotated, LanguageCode::Ko);
    }

    #[test]
    fn only_arabic_is_right_to_left() {
        let rtl: Vec<LanguageCode> = LanguageCode::ALL
            .into_iter()
            .filter(|l| l.is_right_to_left())
            .collect();
        assert_eq!(rtl, [LanguageCode::Ar]);
    }

    #[test]
    fn display_names_are_english() {
        assert_eq!(LanguageCode::Zh.display_name(), "Chinese");
        assert_eq!(LanguageCode::Ar.display_name(), "Arabic");
    }
}
