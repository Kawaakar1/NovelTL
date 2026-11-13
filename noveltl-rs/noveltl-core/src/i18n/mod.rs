//! Язык интерфейса. Русская строка в коде — это ключ: `t("Настройки")`. Переводы на остальные
//! языки лежат в таблицах `app.rs` (окна, меню, плашка) и `core.rs` (сообщения ядра).
//! Нет перевода — показывается русская строка. Тест `every_key_is_translated` находит в исходниках
//! все вызовы `t(...)`/`tf(...)` и проверяет, что у каждого есть перевод на все языки.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

mod app;
mod core;

/// Язык интерфейса.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLang {
    #[default]
    Ru,
    En,
    Ja,
    Zh,
    Ko,
}

impl UiLang {
    pub const ALL: [UiLang; 5] = [UiLang::Ru, UiLang::En, UiLang::Ja, UiLang::Zh, UiLang::Ko];

    /// Название языка на нём самом — так его узнают в списке при любом языке интерфейса.
    pub fn native_title(self) -> &'static str {
        match self {
            UiLang::Ru => "Русский",
            UiLang::En => "English",
            UiLang::Ja => "日本語",
            UiLang::Zh => "中文",
            UiLang::Ko => "한국어",
        }
    }

    /// Язык интерфейса по коду языка Windows ("ru-RU", "ja-JP"…); незнакомый — английский.
    pub fn from_locale(locale: &str) -> UiLang {
        match locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase().as_str() {
            "ru" | "uk" | "be" | "kk" => UiLang::Ru,
            "ja" => UiLang::Ja,
            "zh" => UiLang::Zh,
            "ko" => UiLang::Ko,
            _ => UiLang::En,
        }
    }

    /// Номер колонки в таблице переводов (у русского колонки нет).
    fn column(self) -> Option<usize> {
        match self {
            UiLang::Ru => None,
            UiLang::En => Some(0),
            UiLang::Ja => Some(1),
            UiLang::Zh => Some(2),
            UiLang::Ko => Some(3),
        }
    }
}

/// Строка таблицы: русский ключ и переводы [English, 日本語, 中文, 한국어].
pub(crate) type Row = (&'static str, [&'static str; 4]);

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(l: UiLang) {
    let i = UiLang::ALL.iter().position(|x| *x == l).unwrap_or(0);
    CURRENT.store(i as u8, Ordering::Relaxed);
}

pub fn lang() -> UiLang {
    UiLang::ALL[CURRENT.load(Ordering::Relaxed) as usize % UiLang::ALL.len()]
}

fn table() -> &'static HashMap<&'static str, &'static [&'static str; 4]> {
    static TABLE: OnceLock<HashMap<&'static str, &'static [&'static str; 4]>> = OnceLock::new();
    TABLE.get_or_init(|| app::ROWS.iter().chain(core::ROWS.iter()).map(|(k, v)| (*k, v)).collect())
}

/// Перевод строки на текущий язык интерфейса.
pub fn t(ru: &'static str) -> &'static str {
    t_in(lang(), ru)
}

/// Перевод строки на заданный язык.
pub fn t_in(l: UiLang, ru: &'static str) -> &'static str {
    match l.column() {
        None => ru,
        Some(c) => table().get(ru).map_or(ru, |v| v[c]),
    }
}

/// Перевод строки с подстановками: каждое `{}` по порядку заменяется на очередной аргумент.
pub fn tf(ru: &'static str, args: &[&dyn Display]) -> String {
    fill(t(ru), args)
}

fn fill(pattern: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut rest = pattern;
    let mut it = args.iter();
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        if let Some(a) = it.next() {
            out.push_str(&a.to_string());
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn switching_language_and_fallback() {
        // глобальный язык не трогаем: тесты идут параллельно и проверяют русские сообщения
        assert_eq!(t_in(UiLang::En, "Настройки"), "Settings");
        assert_eq!(t_in(UiLang::Ja, "Настройки"), "設定");
        assert_eq!(t_in(UiLang::En, "строка без перевода"), "строка без перевода");
        assert_eq!(
            fill(t_in(UiLang::En, "Поймано реплик: {} из {}. Кликайте дальше."), &[&1, &3]),
            "Lines caught: 1 of 3. Keep clicking."
        );
        assert_eq!(t_in(UiLang::Ru, "Настройки"), "Настройки");
        assert_eq!(t("Настройки"), "Настройки");
        assert_eq!(fill("{} и {}", &[&"а"]), "а и ");
    }

    #[test]
    fn locale_mapping() {
        assert_eq!(UiLang::from_locale("ru-RU"), UiLang::Ru);
        assert_eq!(UiLang::from_locale("ja-JP"), UiLang::Ja);
        assert_eq!(UiLang::from_locale("zh-Hans-CN"), UiLang::Zh);
        assert_eq!(UiLang::from_locale("ko_KR"), UiLang::Ko);
        assert_eq!(UiLang::from_locale("de-DE"), UiLang::En);
        assert_eq!(serde_json::to_string(&UiLang::Zh).unwrap(), "\"zh\"");
    }

    #[test]
    fn rows_are_consistent() {
        let mut seen = HashSet::new();
        for (ru, tr) in app::ROWS.iter().chain(core::ROWS.iter()) {
            assert!(seen.insert(*ru), "ключ повторяется: {ru}");
            let n = ru.matches("{}").count();
            for (i, s) in tr.iter().enumerate() {
                assert!(!s.trim().is_empty(), "пустой перевод [{i}] для: {ru}");
                assert_eq!(s.matches("{}").count(), n, "число {{}} не совпадает [{i}] для: {ru}");
            }
        }
    }

    /// Литерал Rust из исходника -> строка (только экранирования, которые встречаются в коде).
    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('u') => {
                    let code: String = it.by_ref().skip(1).take_while(|c| *c != '}').collect();
                    out.extend(u32::from_str_radix(&code, 16).ok().and_then(char::from_u32));
                }
                // перенос строки внутри литерала: пробелы в начале следующей строки отбрасываются
                Some('\n') | Some('\r') => {
                    while it.peek().is_some_and(|c| c.is_whitespace()) {
                        it.next();
                    }
                }
                Some(o) => out.push(o),
                None => {}
            }
        }
        out
    }

    fn scan(dir: &Path, keys: &mut Vec<(String, String)>) {
        let re = regex::Regex::new(r#"\btf?\(\s*"((?:[^"\\]|\\.)*)""#).unwrap();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "i18n") {
                    scan(&p, keys);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                let src = std::fs::read_to_string(&p).unwrap();
                for c in re.captures_iter(&src) {
                    keys.push((unescape(&c[1]), p.display().to_string()));
                }
            }
        }
    }

    #[test]
    fn every_key_is_translated() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let mut keys = Vec::new();
        scan(&root.join("noveltl-core").join("src"), &mut keys);
        scan(&root.join("noveltl").join("src"), &mut keys);
        assert!(keys.len() > 100, "строк для перевода подозрительно мало: {}", keys.len());
        let missing: Vec<String> = keys
            .iter()
            .filter(|(k, _)| !table().contains_key(k.as_str()))
            .map(|(k, f)| format!("{f}: {k:?}"))
            .collect();
        assert!(missing.is_empty(), "нет перевода ({}):\n{}", missing.len(), missing.join("\n"));
    }
}
