//! Переводчики: DeepL (основной, с ключом) и Google без ключа (запасной).

use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::cache::TranslationCache;
use crate::langdetect::Lang;

pub const ENGINE_DEEPL: &str = "deepl";
pub const ENGINE_GOOGLE: &str = "google";

pub fn engine_title(engine: &str) -> &'static str {
    match engine {
        ENGINE_DEEPL => "DeepL",
        ENGINE_GOOGLE => "Google",
        _ => "",
    }
}

// ---------- HTTP-абстракция (в тестах подменяется, как FakeSession в Python) ----------

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HttpError {
    /// Не удалось подключиться к прокси.
    ProxyUnreachable,
    /// Прокси не принял логин/пароль (407 при CONNECT, отказ SOCKS-авторизации).
    ProxyAuth,
    /// ОС запретила выход через интерфейс (WSAEACCES 10013).
    AccessDenied,
    /// Привязка к несуществующему адресу (WSAEADDRNOTAVAIL 10049).
    AddressUnavailable,
    /// Прочие ошибки соединения (в строке — тип, без адреса прокси).
    Connect(String),
    Timeout,
    Other(String),
}

impl HttpError {
    /// Короткое имя ошибки для сообщений пользователю (аналог `type(e).__name__`).
    pub fn name(&self) -> String {
        match self {
            HttpError::ProxyUnreachable | HttpError::ProxyAuth => "ProxyError".to_string(),
            HttpError::AccessDenied | HttpError::AddressUnavailable => "ConnectionError".to_string(),
            HttpError::Connect(s) | HttpError::Other(s) => s.clone(),
            HttpError::Timeout => "Timeout".to_string(),
        }
    }
}

pub trait Http: Send + Sync {
    fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &serde_json::Value,
    ) -> Result<HttpResponse, HttpError>;
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<HttpResponse, HttpError>;
}

// ---------- ошибки и результат ----------

#[derive(Clone, Debug, PartialEq)]
pub enum TranslateError {
    /// 403 (неверный ключ) или 456 (лимит исчерпан) — DeepL отключается до смены ключа.
    Denied(u16),
    Failed(String),
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranslateError::Denied(status) => {
                let reason = match status {
                    403 => "ключ не принят",
                    456 => "лимит символов исчерпан",
                    _ => "доступ запрещён",
                };
                write!(f, "ошибка {status}: {reason}")
            }
            TranslateError::Failed(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for TranslateError {}

#[derive(Clone, Debug, PartialEq)]
pub struct TranslateResult {
    pub text: String,
    pub engine: &'static str,
    pub cached: bool,
    /// Сообщение для пользователя (например, о переключении на Google).
    pub notice: String,
}

// ---------- DeepL ----------

pub struct DeepLTranslator {
    pub api_key: String,
    http: Arc<dyn Http>,
}

impl DeepLTranslator {
    pub fn new(api_key: &str, http: Arc<dyn Http>) -> Self {
        DeepLTranslator { api_key: api_key.trim().to_string(), http }
    }

    pub fn url(&self) -> String {
        let host = if self.api_key.ends_with(":fx") { "api-free.deepl.com" } else { "api.deepl.com" };
        format!("https://{host}/v2/translate")
    }

    fn lang_code(lang: Option<Lang>) -> Option<&'static str> {
        match lang {
            Some(Lang::Ja) => Some("JA"),
            Some(Lang::Zh) => Some("ZH"),
            Some(Lang::En) => Some("EN"),
            _ => None,
        }
    }

    pub fn translate(&self, text: &str, src_lang: Option<Lang>, context: &str) -> Result<String, TranslateError> {
        let mut payload = serde_json::json!({"text": [text], "target_lang": "RU"});
        if let Some(code) = Self::lang_code(src_lang) {
            payload["source_lang"] = Value::from(code);
        }
        if !context.is_empty() {
            payload["context"] = Value::from(context);
        }
        let auth = format!("DeepL-Auth-Key {}", self.api_key);
        let resp = self
            .http
            .post_json(&self.url(), &[("Authorization", auth.as_str())], &payload)
            .map_err(|e| TranslateError::Failed(format!("нет связи с DeepL ({})", e.name())))?;
        if resp.status == 403 || resp.status == 456 {
            return Err(TranslateError::Denied(resp.status));
        }
        if resp.status != 200 {
            return Err(TranslateError::Failed(format!("DeepL ответил кодом {}", resp.status)));
        }
        let bad = || TranslateError::Failed("непонятный ответ DeepL".to_string());
        let v: Value = serde_json::from_str(&resp.body).map_err(|_| bad())?;
        v.get("translations")
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("text"))
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .ok_or_else(bad)
    }
}

// ---------- Google ----------

/// Максимум символов в одном GET-запросе Google.
const GOOGLE_MAX_CHARS: usize = 1000;

/// Делит текст на куски не длиннее `limit` символов. Возвращает (кусок, разделитель после него).
/// Режем по `\n`, слишком длинную строку — по концу предложения, в крайнем случае жёстко.
fn split_for_google(text: &str, limit: usize) -> Vec<(String, &'static str)> {
    // элементарные части: (текст, разделитель после)
    let mut units: Vec<(String, &'static str)> = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let line_sep = if i + 1 < lines.len() { "\n" } else { "" };
        if line.chars().count() <= limit {
            units.push((line.to_string(), line_sep));
            continue;
        }
        let mut cur = String::new();
        let mut cur_len = 0;
        for c in line.chars() {
            cur.push(c);
            cur_len += 1;
            if cur_len == limit || "。！？!?.".contains(c) {
                units.push((std::mem::take(&mut cur), ""));
                cur_len = 0;
            }
        }
        if !cur.is_empty() {
            units.push((cur, ""));
        }
        if let Some(last) = units.last_mut() {
            last.1 = line_sep;
        }
    }
    // упаковка частей в куски
    let mut chunks: Vec<(String, &'static str)> = Vec::new();
    let mut cur = String::new();
    let mut cur_len = 0;
    let mut last_sep = "";
    let mut started = false;
    for (u, sep) in units {
        let ulen = u.chars().count();
        if !started {
            cur = u;
            cur_len = ulen;
            started = true;
        } else if cur_len + last_sep.len() + ulen <= limit {
            cur.push_str(last_sep);
            cur.push_str(&u);
            cur_len += last_sep.len() + ulen;
        } else {
            chunks.push((std::mem::take(&mut cur), last_sep));
            cur = u;
            cur_len = ulen;
        }
        last_sep = sep;
    }
    if started {
        chunks.push((cur, last_sep));
    }
    chunks
}

pub struct GoogleTranslator {
    http: Arc<dyn Http>,
}

impl GoogleTranslator {
    pub const URL: &'static str = "https://translate.googleapis.com/translate_a/single";

    pub fn new(http: Arc<dyn Http>) -> Self {
        GoogleTranslator { http }
    }

    fn lang_code(lang: Option<Lang>) -> &'static str {
        match lang {
            Some(Lang::Ja) => "ja",
            Some(Lang::Zh) => "zh-CN",
            Some(Lang::En) => "en",
            _ => "auto",
        }
    }

    /// Длинный текст режем на куски (GET с q раздувает URL), короткий — один запрос.
    pub fn translate(&self, text: &str, src_lang: Option<Lang>) -> Result<String, TranslateError> {
        if text.chars().count() <= GOOGLE_MAX_CHARS {
            return self.translate_one(text, src_lang);
        }
        let mut out = String::new();
        for (chunk, sep) in split_for_google(text, GOOGLE_MAX_CHARS) {
            if chunk.trim().is_empty() {
                out.push_str(&chunk);
            } else {
                out.push_str(&self.translate_one(&chunk, src_lang)?);
            }
            out.push_str(sep);
        }
        Ok(out)
    }

    fn translate_one(&self, text: &str, src_lang: Option<Lang>) -> Result<String, TranslateError> {
        let query = [
            ("client", "gtx"),
            ("sl", Self::lang_code(src_lang)),
            ("tl", "ru"),
            ("dt", "t"),
            ("q", text),
        ];
        let resp = self
            .http
            .get(Self::URL, &query)
            .map_err(|e| TranslateError::Failed(format!("нет связи с Google ({})", e.name())))?;
        if resp.status == 429 {
            return Err(TranslateError::Failed(
                "Google временно ограничил запросы с вашего адреса (код 429). \
                 Укажите ключ DeepL или смените сеть/прокси в настройках"
                    .to_string(),
            ));
        }
        if resp.status != 200 {
            return Err(TranslateError::Failed(format!("Google ответил кодом {}", resp.status)));
        }
        let bad = || TranslateError::Failed("непонятный ответ Google".to_string());
        let v: Value = serde_json::from_str(&resp.body).map_err(|_| bad())?;
        let segs = v.get(0).and_then(|s| s.as_array()).ok_or_else(bad)?;
        let mut out = String::new();
        for seg in segs {
            match seg.get(0) {
                None | Some(Value::Null) => {}
                Some(Value::String(s)) => out.push_str(s),
                Some(_) => return Err(bad()),
            }
        }
        Ok(out)
    }
}

// ---------- менеджер ----------

/// DeepL -> при 403/456 автоматически Google. Кэш — по тексту и переводчику.
pub struct TranslatorManager {
    http: Arc<dyn Http>,
    cache: Option<Arc<TranslationCache>>,
    google: GoogleTranslator,
    deepl: Option<DeepLTranslator>,
    deepl_disabled_reason: String,
}

impl TranslatorManager {
    pub fn new(deepl_key: &str, cache: Option<Arc<TranslationCache>>, http: Arc<dyn Http>) -> Self {
        let mut m = TranslatorManager {
            google: GoogleTranslator::new(http.clone()),
            http,
            cache,
            deepl: None,
            deepl_disabled_reason: String::new(),
        };
        m.set_deepl_key(deepl_key);
        m
    }

    pub fn set_deepl_key(&mut self, key: &str) {
        let key = key.trim();
        self.deepl = if key.is_empty() { None } else { Some(DeepLTranslator::new(key, self.http.clone())) };
        self.deepl_disabled_reason.clear();
    }

    /// Аналог set_session: новая сеть (прокси, интерфейс).
    pub fn set_http(&mut self, http: Arc<dyn Http>) {
        self.google = GoogleTranslator::new(http.clone());
        if let Some(d) = self.deepl.as_mut() {
            d.http = http.clone();
        }
        self.http = http;
    }

    pub fn set_cache(&mut self, cache: Option<Arc<TranslationCache>>) {
        self.cache = cache;
    }

    pub fn active_engine(&self) -> &'static str {
        if self.deepl.is_some() && self.deepl_disabled_reason.is_empty() {
            ENGINE_DEEPL
        } else {
            ENGINE_GOOGLE
        }
    }

    pub fn status_text(&self) -> String {
        if self.active_engine() == ENGINE_DEEPL {
            "DeepL".to_string()
        } else if !self.deepl_disabled_reason.is_empty() {
            format!("Google (DeepL недоступен: {})", self.deepl_disabled_reason)
        } else {
            "Google (ключ DeepL не задан)".to_string()
        }
    }

    fn cached(&self, text: &str, engine: &str, use_cache: bool) -> Option<String> {
        if use_cache {
            self.cache.as_ref().and_then(|c| c.get(text, engine))
        } else {
            None
        }
    }

    fn store(&self, text: &str, engine: &str, result: &str) {
        if let Some(c) = &self.cache {
            if !result.is_empty() {
                c.put(text, engine, result);
            }
        }
    }

    pub fn translate(
        &mut self,
        text: &str,
        src_lang: Option<Lang>,
        context: &str,
        use_cache: bool,
    ) -> Result<TranslateResult, TranslateError> {
        let mut notice = String::new();
        if self.active_engine() == ENGINE_DEEPL {
            if let Some(hit) = self.cached(text, ENGINE_DEEPL, use_cache) {
                return Ok(TranslateResult { text: hit, engine: ENGINE_DEEPL, cached: true, notice });
            }
            let attempt = match &self.deepl {
                Some(d) => d.translate(text, src_lang, context),
                None => Err(TranslateError::Failed(String::new())),
            };
            match attempt {
                Ok(out) => {
                    self.store(text, ENGINE_DEEPL, &out);
                    return Ok(TranslateResult { text: out, engine: ENGINE_DEEPL, cached: false, notice });
                }
                Err(e @ TranslateError::Denied(_)) => {
                    self.deepl_disabled_reason = e.to_string();
                    notice = format!("DeepL недоступен ({e}). Переключился на Google.");
                }
                Err(e) => notice = format!("DeepL: {e}. Эта реплика переведена через Google."),
            }
        }

        if let Some(hit) = self.cached(text, ENGINE_GOOGLE, use_cache) {
            return Ok(TranslateResult { text: hit, engine: ENGINE_GOOGLE, cached: true, notice });
        }
        let out = self.google.translate(text, src_lang)?;
        self.store(text, ENGINE_GOOGLE, &out);
        Ok(TranslateResult { text: out, engine: ENGINE_GOOGLE, cached: false, notice })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Debug)]
    pub struct Post {
        pub url: String,
        pub json: Value,
        pub headers: Vec<(String, String)>,
    }

    /// Имитация сети: DeepL через post_json, Google через get.
    pub struct FakeHttp {
        pub deepl_status: u16,
        pub broken: bool,
        pub posts: Mutex<Vec<Post>>,
        pub gets: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl FakeHttp {
        pub fn new(deepl_status: u16) -> Arc<Self> {
            Arc::new(FakeHttp {
                deepl_status,
                broken: false,
                posts: Mutex::new(vec![]),
                gets: Mutex::new(vec![]),
            })
        }
        pub fn broken() -> Arc<Self> {
            Arc::new(FakeHttp {
                deepl_status: 200,
                broken: true,
                posts: Mutex::new(vec![]),
                gets: Mutex::new(vec![]),
            })
        }
        pub fn posts(&self) -> Vec<Post> {
            self.posts.lock().unwrap().clone()
        }
        pub fn gets(&self) -> Vec<Vec<(String, String)>> {
            self.gets.lock().unwrap().clone()
        }
    }

    impl Http for FakeHttp {
        fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &Value) -> Result<HttpResponse, HttpError> {
            self.posts.lock().unwrap().push(Post {
                url: url.to_string(),
                json: body.clone(),
                headers: headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            });
            if self.deepl_status != 200 {
                return Ok(HttpResponse { status: self.deepl_status, body: String::new() });
            }
            let t = body["text"][0].as_str().unwrap_or("");
            Ok(HttpResponse {
                status: 200,
                body: serde_json::json!({"translations": [{"text": format!("D:{t}")}]}).to_string(),
            })
        }

        fn get(&self, _url: &str, query: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
            if self.broken {
                return Err(HttpError::Other("OSError".to_string()));
            }
            self.gets
                .lock()
                .unwrap()
                .push(query.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect());
            let q = query.iter().find(|(k, _)| *k == "q").map(|(_, v)| *v).unwrap_or("");
            Ok(HttpResponse {
                status: 200,
                body: serde_json::json!([[[format!("G:{q}"), q, null]], null, "ja"]).to_string(),
            })
        }
    }

    pub fn param<'a>(q: &'a [(String, String)], key: &str) -> &'a str {
        q.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn http(h: &Arc<FakeHttp>) -> Arc<dyn Http> {
        h.clone()
    }

    #[test]
    fn deepl_free_and_pro_urls() {
        let s = FakeHttp::new(200);
        assert!(DeepLTranslator::new("abc:fx", http(&s)).url().contains("api-free.deepl.com"));
        assert!(DeepLTranslator::new("abc", http(&s)).url().starts_with("https://api.deepl.com"));
    }

    #[test]
    fn deepl_request_shape() {
        let s = FakeHttp::new(200);
        let out = DeepLTranslator::new("key:fx", http(&s))
            .translate("こんにちは", Some(Lang::Ja), "前の文")
            .unwrap();
        assert_eq!(out, "D:こんにちは");
        let req = &s.posts()[0];
        assert_eq!(req.url, "https://api-free.deepl.com/v2/translate");
        assert_eq!(
            req.json,
            serde_json::json!({"text": ["こんにちは"], "target_lang": "RU", "source_lang": "JA", "context": "前の文"})
        );
        assert!(req.headers.contains(&("Authorization".to_string(), "DeepL-Auth-Key key:fx".to_string())));
    }

    #[test]
    fn google_request_shape() {
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("", None, http(&s));
        let res = m.translate("こんにちは", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.text.as_str(), res.engine), ("G:こんにちは", "google"));
        let g = s.gets();
        assert_eq!(param(&g[0], "client"), "gtx");
        assert_eq!(param(&g[0], "sl"), "ja");
        assert_eq!(param(&g[0], "tl"), "ru");
        assert_eq!(param(&g[0], "dt"), "t");
    }

    #[test]
    fn deepl_denied_falls_back_to_google() {
        for status in [403u16, 456] {
            let s = FakeHttp::new(status);
            let mut m = TranslatorManager::new("key", None, http(&s));
            let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
            assert!(res.engine == "google" && res.text == "G:テスト");
            assert!(res.notice.contains("Переключился на Google") && res.notice.contains(&status.to_string()));
            assert_eq!(m.active_engine(), "google");
            // дальше DeepL уже не дёргаем
            let res2 = m.translate("次", Some(Lang::Ja), "", true).unwrap();
            assert!(s.posts().len() == 1 && res2.notice.is_empty());
            // новый ключ снова включает DeepL
            m.set_deepl_key("other");
            assert_eq!(m.active_engine(), "deepl");
        }
    }

    #[test]
    fn denied_texts() {
        assert_eq!(TranslateError::Denied(403).to_string(), "ошибка 403: ключ не принят");
        assert_eq!(TranslateError::Denied(456).to_string(), "ошибка 456: лимит символов исчерпан");
    }

    #[test]
    fn deepl_temporary_error_does_not_disable() {
        let s = FakeHttp::new(500);
        let mut m = TranslatorManager::new("key", None, http(&s));
        let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
        assert!(res.engine == "google" && !res.notice.is_empty());
        assert_eq!(res.notice, "DeepL: DeepL ответил кодом 500. Эта реплика переведена через Google.");
        assert_eq!(m.active_engine(), "deepl");
    }

    #[test]
    fn cache_hit_and_bypass() {
        let s = FakeHttp::new(200);
        let cache = Arc::new(TranslationCache::in_memory().unwrap());
        let mut m = TranslatorManager::new("key", Some(cache.clone()), http(&s));
        assert!(!m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert!(m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert_eq!(s.posts().len(), 1);
        assert!(!m.translate("テスト", Some(Lang::Ja), "", false).unwrap().cached);
        assert_eq!(s.posts().len(), 2);
        assert_eq!(cache.count(), 1);
    }

    #[test]
    fn google_long_text_is_split() {
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("", None, http(&s));
        // 25 строк по 99 символов + переводы строк = 2499 символов
        let line = "а".repeat(99);
        let text = vec![line.as_str(); 25].join("\n");
        let res = m.translate(&text, Some(Lang::Ja), "", true).unwrap();
        let g = s.gets();
        assert!(g.len() >= 3);
        for q in &g {
            assert!(param(q, "q").chars().count() <= 1000);
        }
        // фейк добавляет «G:» к каждому куску — убираем и сверяем склейку
        let mut restored = res.text.clone();
        for _ in 0..g.len() {
            restored = restored.replacen("G:", "", 1);
        }
        assert_eq!(restored, text);
        // одна очень длинная строка без разделителей режется по предложениям/жёстко
        let long = "あ。".repeat(1200);
        let s2 = FakeHttp::new(200);
        let mut m2 = TranslatorManager::new("", None, http(&s2));
        m2.translate(&long, Some(Lang::Ja), "", true).unwrap();
        assert!(s2.gets().len() >= 3 && s2.gets().iter().all(|q| param(q, "q").chars().count() <= 1000));
    }

    #[test]
    fn google_failure_raises() {
        let mut m = TranslatorManager::new("", None, FakeHttp::broken());
        let e = m.translate("x", Some(Lang::Ja), "", true).unwrap_err();
        assert_eq!(e, TranslateError::Failed("нет связи с Google (OSError)".to_string()));
    }

    #[test]
    fn google_429_text() {
        struct Limited;
        impl Http for Limited {
            fn post_json(&self, _: &str, _: &[(&str, &str)], _: &Value) -> Result<HttpResponse, HttpError> {
                Err(HttpError::Timeout)
            }
            fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
                Ok(HttpResponse { status: 429, body: String::new() })
            }
        }
        let mut m = TranslatorManager::new("", None, Arc::new(Limited));
        let e = m.translate("x", None, "", true).unwrap_err();
        assert_eq!(
            e.to_string(),
            "Google временно ограничил запросы с вашего адреса (код 429). \
             Укажите ключ DeepL или смените сеть/прокси в настройках"
        );
        // нет связи с DeepL: реплика уходит в Google, DeepL не отключается
        let mut m = TranslatorManager::new("k", None, Arc::new(Limited));
        let _ = m.translate("x", None, "", true);
        assert_eq!(m.active_engine(), "deepl");
    }

    #[test]
    fn manager_set_http() {
        let first = FakeHttp::new(200);
        let mut m = TranslatorManager::new("key", None, http(&first));
        let new = FakeHttp::new(200);
        m.set_http(http(&new));
        m.translate("a", Some(Lang::Ja), "", true).unwrap();
        assert!(first.posts().is_empty() && new.posts().len() == 1);
        m.set_deepl_key("");
        m.translate("b", Some(Lang::Ja), "", true).unwrap();
        assert!(first.gets().is_empty() && new.gets().len() == 1);
    }

    #[test]
    fn status_text_and_titles() {
        let s = FakeHttp::new(403);
        let mut m = TranslatorManager::new("", None, http(&s));
        assert_eq!(m.status_text(), "Google (ключ DeepL не задан)");
        m.set_deepl_key("k");
        assert_eq!(m.status_text(), "DeepL");
        m.translate("a", None, "", true).unwrap();
        assert_eq!(m.status_text(), "Google (DeepL недоступен: ошибка 403: ключ не принят)");
        assert_eq!((engine_title(ENGINE_DEEPL), engine_title(ENGINE_GOOGLE)), ("DeepL", "Google"));
    }
}
