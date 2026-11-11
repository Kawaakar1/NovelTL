//! Реплика из игры -> глоссарий -> кэш/переводчик -> готовый перевод с именем.

use std::collections::VecDeque;

use crate::langdetect::{resolve_language, Lang};
use crate::profiles::GlossaryEntry;
use crate::translators::{TranslateError, TranslatorManager};

pub const CONTEXT_LINES: usize = 3;

/// Подставляет в исходный текст готовые переводы имён и терминов (длинные — первыми).
pub fn apply_glossary(text: &str, glossary: &[GlossaryEntry]) -> String {
    let mut entries: Vec<&GlossaryEntry> =
        glossary.iter().filter(|e| !e.src.is_empty() && !e.dst.is_empty()).collect();
    // сортировка устойчивая: при равной длине порядок как в глоссарии
    entries.sort_by(|a, b| b.src.chars().count().cmp(&a.src.chars().count()));
    // один проход слева направо: результат замены повторно не сканируется
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if let Some(e) = entries.iter().find(|e| rest.starts_with(e.src.as_str())) {
            out.push_str(&e.dst);
            i += e.src.len();
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub fn glossary_exact(text: &str, glossary: &[GlossaryEntry]) -> Option<String> {
    glossary.iter().find(|e| e.src == text && !e.dst.is_empty()).map(|e| e.dst.clone())
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineResult {
    /// Переведённое имя.
    pub name: String,
    /// Переведённый текст.
    pub text: String,
    pub src_name: String,
    pub src_text: String,
    pub engine: &'static str,
    pub cached: bool,
    pub notice: String,
}

pub struct TranslationPipeline {
    pub manager: TranslatorManager,
    pub lang: Lang,
    pub glossary: Vec<GlossaryEntry>,
    context: VecDeque<String>,
}

impl TranslationPipeline {
    pub fn new(manager: TranslatorManager, lang: Lang, glossary: Vec<GlossaryEntry>) -> Self {
        TranslationPipeline { manager, lang, glossary, context: VecDeque::new() }
    }

    pub fn reset_context(&mut self) {
        self.context.clear();
    }

    fn translate_name(
        &mut self,
        name: &str,
        lang: Option<Lang>,
        use_cache: bool,
    ) -> Result<(String, String), TranslateError> {
        if let Some(exact) = glossary_exact(name, &self.glossary) {
            return Ok((exact, String::new()));
        }
        let prepared = apply_glossary(name, &self.glossary);
        let res = self.manager.translate(&prepared, lang, "", use_cache)?;
        let text = res.text.trim_matches(|c| c == ' ' || c == '.' || c == '。').to_string();
        Ok((text, res.notice))
    }

    pub fn process(&mut self, name: &str, text: &str, use_cache: bool) -> Result<LineResult, TranslateError> {
        let lang = resolve_language(self.lang, text);
        let context = self.context.iter().filter(|c| c.as_str() != text).cloned().collect::<Vec<_>>().join("\n");
        let prepared = apply_glossary(text, &self.glossary);
        let res = self.manager.translate(&prepared, lang, &context, use_cache)?;
        let mut notice = res.notice.clone();
        let mut name_tr = String::new();
        if !name.is_empty() {
            // имя переводим всегда через кэш: оно не меняется от реплики к реплике
            match self.translate_name(name, lang, true) {
                Ok((n, name_notice)) => {
                    name_tr = n;
                    if notice.is_empty() {
                        notice = name_notice;
                    }
                }
                Err(_) => {
                    // ошибка имени не должна губить готовый перевод реплики
                    name_tr = name.to_string();
                    if notice.is_empty() {
                        notice = "Имя не удалось перевести — показано как есть.".to_string();
                    }
                }
            }
        }
        if self.context.back().map(String::as_str) != Some(text) {
            if self.context.len() == CONTEXT_LINES {
                self.context.pop_front();
            }
            self.context.push_back(text.to_string());
        }
        Ok(LineResult {
            name: name_tr,
            text: res.text,
            src_name: name.to_string(),
            src_text: text.to_string(),
            engine: res.engine,
            cached: res.cached,
            notice,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::TranslationCache;
    use crate::translators::fake::{param, FakeHttp};
    use std::sync::Arc;

    fn g(src: &str, dst: &str) -> GlossaryEntry {
        GlossaryEntry { src: src.to_string(), dst: dst.to_string() }
    }

    #[test]
    fn glossary() {
        let gl = vec![g("芳乃", "Ёсино"), g("朝武芳乃", "Томотакэ Ёсино")];
        assert_eq!(apply_glossary("朝武芳乃と芳乃", &gl), "Томотакэ ЁсиноとЁсино");
        assert_eq!(glossary_exact("芳乃", &gl).as_deref(), Some("Ёсино"));
        assert_eq!(glossary_exact("х", &gl), None);
    }

    #[test]
    fn pipeline_name_context_glossary() {
        let s = FakeHttp::new(200);
        let cache = Arc::new(TranslationCache::in_memory().unwrap());
        let m = TranslatorManager::new("key", Some(cache), s.clone());
        let mut p = TranslationPipeline::new(m, Lang::Ja, vec![g("芳乃", "Ёсино")]);
        let r1 = p.process("芳乃", "「おはよう」", true).unwrap();
        assert_eq!(r1.name, "Ёсино"); // имя из глоссария, без запроса
        assert_eq!(r1.text, "D:「おはよう」");
        let r2 = p.process("将臣", "「芳乃、おはよう」", true).unwrap();
        assert_eq!(r2.name, "D:将臣");
        assert_eq!(r2.text, "D:「Ёсино、おはよう」");
        // в DeepL ушёл контекст с предыдущей репликой
        let line_reqs: Vec<_> =
            s.posts().into_iter().filter(|q| q.json["text"][0].as_str().unwrap_or("").contains("おはよう")).collect();
        assert!(line_reqs[0].json.get("context").is_none());
        assert_eq!(line_reqs[1].json["context"], "「おはよう」");
    }

    #[test]
    fn pipeline_context_is_limited() {
        let s = FakeHttp::new(200);
        let mut p = TranslationPipeline::new(TranslatorManager::new("key", None, s.clone()), Lang::Ja, vec![]);
        for i in 0..6 {
            p.process("", &format!("реплика{i}"), true).unwrap();
        }
        assert_eq!(s.posts().last().unwrap().json["context"], "реплика2\nреплика3\nреплика4");
        p.reset_context();
        p.process("", "новая", true).unwrap();
        assert!(s.posts().last().unwrap().json.get("context").is_none());
    }

    #[test]
    fn pipeline_auto_language() {
        let s = FakeHttp::new(200);
        let mut p = TranslationPipeline::new(TranslatorManager::new("", None, s.clone()), Lang::Auto, vec![]);
        p.process("", "你好", true).unwrap();
        p.process("", "おはよう", true).unwrap();
        p.process("", "Hello", true).unwrap();
        let sl: Vec<String> = s.gets().iter().map(|q| param(q, "sl").to_string()).collect();
        // только иероглифы без каны — язык определяет сам переводчик
        assert_eq!(sl, ["auto", "ja", "en"]);
    }

    #[test]
    fn glossary_no_cascade() {
        let gl = vec![g("Kevin", "Кевин"), g("Кевин", "Кэвин")];
        assert_eq!(apply_glossary("Kevin", &gl), "Кевин");
    }

    #[test]
    fn pipeline_name_error_keeps_line() {
        use crate::translators::{Http, HttpError, HttpResponse};
        use std::sync::atomic::{AtomicUsize, Ordering};
        // первый запрос (реплика) успешен, второй (имя) — ошибка
        struct SecondFails(AtomicUsize);
        impl Http for SecondFails {
            fn post_json(
                &self,
                _: &str,
                _: &[(&str, &str)],
                body: &serde_json::Value,
            ) -> Result<HttpResponse, HttpError> {
                if self.0.fetch_add(1, Ordering::SeqCst) >= 1 {
                    return Err(HttpError::Timeout);
                }
                let t = body["text"][0].as_str().unwrap_or("");
                Ok(HttpResponse {
                    status: 200,
                    body: serde_json::json!({"translations": [{"text": format!("D:{t}")}]}).to_string(),
                })
            }
            fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
                Err(HttpError::Timeout)
            }
        }
        let m = TranslatorManager::new("key", None, Arc::new(SecondFails(AtomicUsize::new(0))));
        let mut p = TranslationPipeline::new(m, Lang::Ja, vec![]);
        let r = p.process("将臣", "「おはよう」", true).unwrap();
        assert_eq!(r.text, "D:「おはよう」");
        assert_eq!(r.name, "将臣");
        assert_eq!(r.notice, "Имя не удалось перевести — показано как есть.");
    }

    #[test]
    fn pipeline_error_propagates() {
        let mut p =
            TranslationPipeline::new(TranslatorManager::new("", None, FakeHttp::broken()), Lang::Ja, vec![]);
        assert!(p.process("", "x", true).is_err());
    }
}
