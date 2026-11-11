//! Очистка текста из хука: теги KiriKiri, удвоения, повторы, имя говорящего.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::langdetect::has_cjk;

// %f$ll; %p-1; %fSourceHanSans; и т. п.
static TAG_PERCENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"%[^;%\s「」『』（）。、]{1,40};").unwrap());
// [r] [l] [p] [np] [ruby text="..."] [/b]
static TAG_BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[/?[a-z_][a-z0-9_]*(?:[\s=,][^\[\]]*)?\]").unwrap());
static TAG_ANGLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?[a-zA-Z][a-zA-Z0-9]*(?:\s[^<>]*)?>").unwrap());
static CONTROL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f\x{200B}-\x{200F}\x{FEFF}]").unwrap()
});
static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s{2,}").unwrap());
static JOIN_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{3000}]{2,}").unwrap());

static RE_BRACKET_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^\s*【([^【】\n]{1,24})】\s*(.+)$").unwrap());
static RE_QUOTE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^([^\s「」『』（）()]{1,12})\s*([「『].+[」』])\s*$").unwrap());
static RE_COLON_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)^([A-Z][A-Za-z'.\-]*(?: [A-Z?][A-Za-z'.\-]*){0,2}|[^\s:：]{1,10})\s*[:：]\s*(\S.*)$",
    )
    .unwrap()
});

const SENTENCE_END: &str = "。！？!?…」』）)\"”.~〜♪—―";
const OPEN_QUOTES: &str = "「『（(“\"";
const NAME_FORBIDDEN: &str = "。、！？!?…「」『』（）()【】,.:;：；\"“”";

/// Служебные слова перед двоеточием, которые не являются именами.
const COLON_STOP_WORDS: [&str; 33] = [
    "note", "notes", "warning", "hint", "tip", "tips", "info", "chapter", "day", "time", "date",
    "location", "place", "objective", "quest", "task", "mission", "tutorial", "example", "update",
    "result", "status", "rule", "rules", "q", "a", "ps", "p.s", "nb", "n.b", "ok",
    "error", "system",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanResult {
    pub name: String,
    pub text: String,
}

impl CleanResult {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

pub fn strip_tags(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.replace("\\n", "\n");
    let text = TAG_PERCENT.replace_all(&text, "");
    // [r] — перенос строки в KiriKiri
    let text = text.replace("[r]", "\n");
    let text = TAG_BRACKET.replace_all(&text, "");
    let text = TAG_ANGLE.replace_all(&text, "");
    CONTROL.replace_all(&text, "").into_owned()
}

/// «ここんんにに» -> «こんに». Строку из одного символа («ああああ») не трогаем.
pub fn collapse_repeated_chars(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    for k in [4usize, 3, 2] {
        if n >= 2 * k && n % k == 0 {
            let all_same = chars.chunks(k).all(|ch| ch.iter().all(|&c| c == ch[0]));
            if all_same {
                let out: Vec<char> = chars.iter().step_by(k).copied().collect();
                let uniq: HashSet<char> = out.iter().copied().collect();
                if uniq.len() > 1 {
                    return out.into_iter().collect();
                }
            }
        }
    }
    text.to_string()
}

/// «Привет.Привет.» -> «Привет.». Короткие слова-повторы (ドキドキ) не трогаем.
pub fn collapse_repeated_phrase(text: &str) -> String {
    let s = text.trim();
    let lines: Vec<&str> = s.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() >= 2 && lines.iter().all(|l| *l == lines[0]) && lines[0].chars().count() >= 2 {
        return lines[0].to_string();
    }
    // «Masaomi  Masaomi  Masaomi» — повтор через пробелы (так KiriKiri отдаёт имена)
    let tokens: Vec<&str> = MULTI_SPACE.split(s).filter(|t| !t.is_empty()).collect();
    if tokens.len() >= 2 && tokens.iter().all(|t| *t == tokens[0]) {
        return tokens[0].to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    for period in 2..=n / 2 {
        if n % period != 0 {
            continue;
        }
        let unit = &chars[..period];
        if !chars.chunks(period).all(|ch| ch == unit) {
            continue;
        }
        let uniq: HashSet<char> = unit.iter().copied().collect();
        if uniq.len() < 2 {
            return s.to_string();
        }
        if period >= 6 || SENTENCE_END.contains(unit[period - 1]) {
            return unit.iter().collect();
        }
        return s.to_string();
    }
    s.to_string()
}

fn looks_like_name(name: &str, limit: usize) -> bool {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > limit {
        return false;
    }
    !name.chars().any(|c| NAME_FORBIDDEN.contains(c))
}

/// Отделяет имя говорящего. Возвращает (имя, текст); имя может быть пустым.
pub fn split_name(text: &str) -> (String, String) {
    let s = text.trim();
    if let Some(m) = RE_BRACKET_NAME.captures(s) {
        return (m[1].trim().to_string(), m[2].trim().to_string());
    }

    if let Some((first, rest)) = s.split_once('\n') {
        let rest = rest.trim();
        if let Some(c0) = rest.chars().next() {
            if OPEN_QUOTES.contains(c0) && looks_like_name(first, 16) {
                return (first.trim().to_string(), rest.to_string());
            }
        }
    }

    if let Some(m) = RE_QUOTE_NAME.captures(s) {
        if looks_like_name(&m[1], 12) {
            return (m[1].trim().to_string(), m[2].trim().to_string());
        }
    }

    if let Some(m) = RE_COLON_NAME.captures(s) {
        let name = &m[1];
        // «http://», «12:30» и подобное именем не считаем
        // латинское имя не берём: служебное слово («Note:») или текст со строчной буквы
        let latin_reject = !has_cjk(name)
            && (m[2].chars().next().is_some_and(|c| c.is_ascii_lowercase())
                || COLON_STOP_WORDS.contains(&name.trim().to_lowercase().as_str()));
        if looks_like_name(name, 25)
            && !latin_reject
            && !name.chars().all(|c| c.is_numeric())
            && !m[2].starts_with("//")
        {
            return (name.trim().to_string(), m[2].trim().to_string());
        }
    }
    (String::new(), s.to_string())
}

pub fn join_lines(text: &str) -> String {
    let parts: Vec<&str> = text.split('\n').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    let sep = if has_cjk(text) { "" } else { " " };
    JOIN_SPACES.replace_all(&parts.join(sep), " ").trim().to_string()
}

/// Полная очистка одной реплики из хука.
pub fn clean(raw: &str) -> CleanResult {
    if raw.is_empty() {
        return CleanResult::default();
    }
    let text = strip_tags(raw);
    let text = text.trim();
    if text.is_empty() {
        return CleanResult::default();
    }
    let text = collapse_repeated_chars(text);
    let text = text
        .split('\n')
        .map(collapse_repeated_chars)
        .collect::<Vec<_>>()
        .join("\n");
    let text = collapse_repeated_phrase(&text);
    let (name, body) = split_name(&text);
    let body = collapse_repeated_phrase(&join_lines(&body));
    let name = join_lines(&name);
    if body.is_empty() {
        return CleanResult { name: String::new(), text: name };
    }
    CleanResult { name, text: body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_kirikiri_tags() {
        assert_eq!(strip_tags("こんにちは[l][r]世界[p]"), "こんにちは\n世界");
        assert_eq!(strip_tags("%f$ll;おはよう%p-1;"), "おはよう");
        assert_eq!(strip_tags("[ruby text=\"よしの\"]芳乃[np]"), "芳乃");
        assert_eq!(strip_tags("100% sure; really"), "100% sure; really");
    }

    #[test]
    fn test_collapse_repeated_chars() {
        let cases = [
            ("ここんんににちちはは", "こんにちは"),
            ("こここんんんにににちちちははは", "こんにちは"),
            ("ああああ", "ああああ"), // одиночный повтор не трогаем
            ("こんにちは", "こんにちは"),
            ("HHeelllloo", "Hello"),
        ];
        for (src, expected) in cases {
            assert_eq!(collapse_repeated_chars(src), expected, "{src}");
        }
    }

    #[test]
    fn test_collapse_repeated_phrase() {
        let cases = [
            ("おはよう。おはよう。", "おはよう。"),
            ("今日はいい天気今日はいい天気", "今日はいい天気"),
            ("おはよう\nおはよう", "おはよう"),
            ("ドキドキ", "ドキドキ"), // настоящее слово
            ("ははは", "ははは"),
            ("「うん」「うん」「うん」", "「うん」"),
        ];
        for (src, expected) in cases {
            assert_eq!(collapse_repeated_phrase(src), expected, "{src}");
        }
    }

    #[test]
    fn test_split_name() {
        let cases = [
            ("【芳乃】「おはようございます」", "芳乃", "「おはようございます」"),
            ("芳乃「おはようございます」", "芳乃", "「おはようございます」"),
            ("芳乃\n「おはようございます」", "芳乃", "「おはようございます」"),
            ("Yoshino: Good morning.", "Yoshino", "Good morning."),
            ("彼は「そう」と言った。", "", "彼は「そう」と言った。"),
            ("ただの地の文です。", "", "ただの地の文です。"),
            ("It was 12:30 already.", "", "It was 12:30 already."),
            ("Note: there is a door.", "", "Note: there is a door."),
            ("Warning: Do not enter.", "", "Warning: Do not enter."),
            ("Akari: hello there", "", "Akari: hello there"),
            ("芳乃：「おはよう」", "芳乃", "「おはよう」"),
        ];
        for (src, name, text) in cases {
            assert_eq!(split_name(src), (name.to_string(), text.to_string()), "{src}");
        }
    }

    #[test]
    fn test_clean_full() {
        let res = clean("【芳乃】%f$ll;「おはよう[r]ございます」[p]");
        assert_eq!((res.name.as_str(), res.text.as_str()), ("芳乃", "「おはようございます」"));
    }

    #[test]
    fn test_clean_joins_lines() {
        assert_eq!(clean("今日は\nいい天気").text, "今日はいい天気");
        assert_eq!(clean("It is\na good day.").text, "It is a good day.");
    }

    #[test]
    fn test_clean_doubled_with_name() {
        let res = clean("【【芳芳乃乃】】「「ははいい」」");
        assert_eq!((res.name.as_str(), res.text.as_str()), ("芳乃", "「はい」"));
    }

    #[test]
    fn test_clean_empty() {
        assert!(clean("").is_empty());
        assert!(clean("[p][l]").is_empty());
    }

    #[test]
    fn test_whitespace_repeated_name() {
        assert_eq!(clean("Masaomi  Masaomi  Masaomi  ").text, "Masaomi");
        assert_eq!(clean("Titty Ninja  Titty Ninja  ").text, "Titty Ninja");
        assert_eq!(clean("Mako  Mako  ").text, "Mako");
    }
}
