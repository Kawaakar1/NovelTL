//! Определение языка по кане и иероглифам.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ja,
    Zh,
    En,
    Auto,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::Ja, Lang::Zh, Lang::En, Lang::Auto];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::Zh => "zh",
            Lang::En => "en",
            Lang::Auto => "auto",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Lang::Ja => "Японский",
            Lang::Zh => "Китайский",
            Lang::En => "Английский",
            Lang::Auto => "Авто",
        }
    }

    /// Кодовая страница для LunaHook.
    pub fn codepage(self) -> u32 {
        match self {
            Lang::Zh => 936,
            _ => 932,
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

pub fn is_cjk(c: char) -> bool {
    is_kana(c) || is_han(c)
}

pub fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

/// Возвращает Ja / Zh / En или None, если понять нельзя.
pub fn detect_language(s: &str) -> Option<Lang> {
    let (mut kana, mut han, mut latin) = (false, false, false);
    for c in s.chars() {
        if is_kana(c) {
            kana = true;
        } else if is_han(c) {
            han = true;
        } else if c.is_ascii_alphabetic() {
            latin = true;
        }
    }
    if kana {
        Some(Lang::Ja)
    } else if han {
        Some(Lang::Zh)
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
    fn test_resolve_auto_han_only() {
        assert_eq!(resolve_language(Lang::Auto, "大丈夫？"), None);
        assert_eq!(resolve_language(Lang::Zh, "大丈夫"), Some(Lang::Zh));
        assert_eq!(resolve_language(Lang::Auto, "おはよう"), Some(Lang::Ja));
    }

    #[test]
    fn test_lang_meta() {
        assert_eq!(Lang::ALL.map(|l| l.code()), ["ja", "zh", "en", "auto"]);
        assert_eq!(Lang::Zh.codepage(), 936);
        assert_eq!(Lang::En.codepage(), 932);
        assert_eq!(Lang::Auto.title(), "Авто");
        assert_eq!(serde_json::to_string(&Lang::Auto).unwrap(), "\"auto\"");
        assert_eq!(serde_json::from_str::<Lang>("\"zh\"").unwrap(), Lang::Zh);
    }
}
