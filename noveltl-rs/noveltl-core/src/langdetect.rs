//! Определение языка по кане и иероглифам.

use crate::i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ja,
    Zh,
    En,
    Ko,
    Ru,
    Auto,
}

impl Lang {
    /// Языки оригинала (порядок для списков).
    pub const ALL: [Lang; 6] = [Lang::Ja, Lang::Zh, Lang::En, Lang::Ko, Lang::Ru, Lang::Auto];
    /// Языки перевода.
    pub const TARGETS: [Lang; 5] = [Lang::Ru, Lang::En, Lang::Ja, Lang::Zh, Lang::Ko];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh",
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::Ru => "ru",
            Lang::Auto => "auto",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Lang::Ja => t("Японский"),
            Lang::Zh => t("Китайский"),
            Lang::En => t("Английский"),
            Lang::Ko => t("Корейский"),
            Lang::Ru => t("Русский"),
            Lang::Auto => t("Авто"),
        }
    }

    /// Название внутри фразы: по-русски со строчной («→ русский»), на других языках — как есть.
    pub fn title_in_text(self) -> String {
        if crate::i18n::lang() == crate::i18n::UiLang::Ru {
            self.title().to_lowercase()
        } else {
            self.title().to_string()
        }
    }

    /// Название на самом языке.
    pub fn native_title(self) -> &'static str {
        match self {
            Lang::Ja => "日本語",
            Lang::Zh => "中文（简体）",
            Lang::En => "English",
            Lang::Ko => "한국어",
            Lang::Ru => "Русский",
            Lang::Auto => "Авто",
        }
    }

    /// Кодовая страница для LunaHook.
    pub fn codepage(self) -> u32 {
        match self {
            Lang::Zh => 936,
            Lang::Ko => 949,
            Lang::Ru => 1251,
            _ => 932,
        }
    }

    /// Код языка для Google.
    pub fn google_code(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh-CN",
            Lang::En => "en",
            Lang::Ko => "ko",
            Lang::Ru => "ru",
            Lang::Auto => "auto",
        }
    }

    /// Исходный язык для DeepL (None — пусть определит сам).
    pub fn deepl_source(self) -> Option<&'static str> {
        match self {
            Lang::Ja => Some("JA"),
            Lang::Zh => Some("ZH"),
            Lang::En => Some("EN"),
            Lang::Ko => Some("KO"),
            Lang::Ru => Some("RU"),
            Lang::Auto => None,
        }
    }

    /// Целевой язык для DeepL.
    pub fn deepl_target(self) -> &'static str {
        match self {
            Lang::Ja => "JA",
            Lang::Zh => "ZH-HANS",
            Lang::En => "EN-US",
            Lang::Ko => "KO",
            Lang::Ru | Lang::Auto => "RU",
        }
    }
}

pub fn is_kana(c: char) -> bool {
    let o = c as u32;
    (0x3040..=0x30FF).contains(&o) || (0x31F0..=0x31FF).contains(&o) || (0xFF66..=0xFF9D).contains(&o)
}

pub fn is_han(c: char) -> bool {
    let o = c as u32;
    (0x4E00..=0x9FFF).contains(&o) || (0x3400..=0x4DBF).contains(&o) || (0xF900..=0xFAFF).contains(&o)
}

pub fn is_hangul(c: char) -> bool {
    let o = c as u32;
    (0xAC00..=0xD7AF).contains(&o) || (0x1100..=0x11FF).contains(&o) || (0x3130..=0x318F).contains(&o)
}

pub fn is_cyrillic(c: char) -> bool {
    (0x0400..=0x04FF).contains(&(c as u32))
}

pub fn is_cjk(c: char) -> bool {
    is_kana(c) || is_han(c)
}

pub fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

/// Возвращает Ja / Ko / Zh / Ru / En или None, если понять нельзя.
pub fn detect_language(s: &str) -> Option<Lang> {
    let (mut kana, mut hangul, mut han, mut cyr, mut latin) = (false, false, false, false, false);
    for c in s.chars() {
        if is_kana(c) {
            kana = true;
        } else if is_hangul(c) {
            hangul = true;
        } else if is_han(c) {
            han = true;
        } else if is_cyrillic(c) {
            cyr = true;
        } else if c.is_ascii_alphabetic() {
            latin = true;
        }
    }
    if kana {
        Some(Lang::Ja)
    } else if hangul {
        Some(Lang::Ko)
    } else if han {
        Some(Lang::Zh)
    } else if cyr {
        Some(Lang::Ru)
    } else if latin {
        Some(Lang::En)
    } else {
        None
    }
}

/// Язык из профиля, а для «авто» — по тексту.
pub fn resolve_language(profile: Lang, text: &str) -> Option<Lang> {
    match profile {
        // одни иероглифы без каны — может быть японский; пусть решает переводчик
        Lang::Auto => match detect_language(text) {
            Some(Lang::Zh) => None,
            other => other,
        },
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("おはよう、世界"), Some(Lang::Ja));
        assert_eq!(detect_language("你好，世界"), Some(Lang::Zh));
        assert_eq!(detect_language("Hello"), Some(Lang::En));
        assert_eq!(detect_language("123 ..."), None);
        assert_eq!(resolve_language(Lang::Zh, "おはよう"), Some(Lang::Zh));
        assert_eq!(resolve_language(Lang::Auto, "おはよう"), Some(Lang::Ja));
    }

    #[test]
    fn test_detect_hangul_cyrillic() {
        assert_eq!(detect_language("안녕하세요"), Some(Lang::Ko));
        assert_eq!(detect_language("Привет, мир"), Some(Lang::Ru));
        assert_eq!(detect_language("ㅋㅋㅋ"), Some(Lang::Ko));
        assert!(is_hangul('한') && !is_hangul('a'));
        assert!(is_cyrillic('Ж') && !is_cyrillic('a'));
        assert_eq!(resolve_language(Lang::Auto, "안녕"), Some(Lang::Ko));
        assert_eq!(resolve_language(Lang::Auto, "Привет"), Some(Lang::Ru));
        assert_eq!(resolve_language(Lang::Ko, "x"), Some(Lang::Ko));
    }

    #[test]
    fn test_resolve_auto_han_only() {
        assert_eq!(resolve_language(Lang::Auto, "大丈夫？"), None);
        assert_eq!(resolve_language(Lang::Zh, "大丈夫"), Some(Lang::Zh));
        assert_eq!(resolve_language(Lang::Auto, "おはよう"), Some(Lang::Ja));
    }

    #[test]
    fn test_lang_meta() {
        assert_eq!(Lang::ALL.map(|l| l.code()), ["ja", "zh", "en", "ko", "ru", "auto"]);
        assert_eq!(Lang::TARGETS.map(|l| l.code()), ["ru", "en", "ja", "zh", "ko"]);
        assert_eq!(Lang::Zh.codepage(), 936);
        assert_eq!(Lang::En.codepage(), 932);
        assert_eq!(Lang::Ko.codepage(), 949);
        assert_eq!(Lang::Ru.codepage(), 1251);
        assert_eq!(Lang::Ja.codepage(), 932);
        assert_eq!(Lang::Ko.title(), "Корейский");
        assert_eq!(Lang::Ru.title(), "Русский");
        assert_eq!(Lang::Zh.native_title(), "中文（简体）");
        assert_eq!(Lang::Ko.native_title(), "한국어");
        assert_eq!(Lang::ALL.map(|l| l.google_code()), ["ja", "zh-CN", "en", "ko", "ru", "auto"]);
        assert_eq!(
            Lang::ALL.map(|l| l.deepl_source()),
            [Some("JA"), Some("ZH"), Some("EN"), Some("KO"), Some("RU"), None]
        );
        assert_eq!(Lang::ALL.map(|l| l.deepl_target()), ["JA", "ZH-HANS", "EN-US", "KO", "RU", "RU"]);
        assert_eq!(serde_json::from_str::<Lang>("\"ko\"").unwrap(), Lang::Ko);
        assert_eq!(serde_json::to_string(&Lang::Ru).unwrap(), "\"ru\"");
        assert_eq!(Lang::Auto.title(), "Авто");
        assert_eq!(serde_json::to_string(&Lang::Auto).unwrap(), "\"auto\"");
        assert_eq!(serde_json::from_str::<Lang>("\"zh\"").unwrap(), Lang::Zh);
    }
}
