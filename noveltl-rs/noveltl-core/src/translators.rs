//! Переводчики: DeepL (основной, с ключом) и Google без ключа (запасной).

use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::cache::TranslationCache;
use crate::langdetect::Lang;

pub const ENGINE_DEEPL: &str = "deepl";
pub const ENGINE_GOOGLE: &str = "google";
pub const ENGINE_MICROSOFT: &str = "microsoft";
pub const ENGINE_LOCAL_AI: &str = "local_ai";

/// Перевод не нужен: язык оригинала совпал с языком перевода.
pub const ENGINE_NONE: &str = "none";

/// Режимы выбора переводчика: (код, подпись).
pub const ENGINE_MODES: [(&str, &str); 5] = [
    ("auto", "Авто: DeepL, если указан ключ, иначе Google"),
    ("deepl", "Только DeepL"),
    ("google", "Только Google"),
    ("microsoft", "Microsoft Translator (бесплатно, по ключу)"),
    ("local_ai", "Локальный ИИ (Ollama / LM Studio, бесплатно)"),
];

pub fn engine_title(engine: &str) -> &'static str {
    match engine {
        ENGINE_DEEPL => "DeepL",
        ENGINE_GOOGLE => "Google",
        ENGINE_MICROSOFT => "Microsoft",
        ENGINE_LOCAL_AI => "Локальный ИИ",
        ENGINE_NONE => "Без перевода (тот же язык)",
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
    /// POST с своим таймаутом (секунды); по умолчанию — обычный post_json.
    fn post_json_timeout(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout_secs: u64,
    ) -> Result<HttpResponse, HttpError> {
        let _ = timeout_secs;
        self.post_json(url, headers, body)
    }
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

    pub fn translate(
        &self,
        text: &str,
        src_lang: Option<Lang>,
        target: Lang,
        context: &str,
    ) -> Result<String, TranslateError> {
        let mut payload = serde_json::json!({"text": [text], "target_lang": target.deepl_target()});
        if let Some(code) = src_lang.and_then(Lang::deepl_source) {
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
        lang.map(Lang::google_code).unwrap_or("auto")
    }

    /// Длинный текст режем на куски (GET с q раздувает URL), короткий — один запрос.
    pub fn translate(&self, text: &str, src_lang: Option<Lang>, target: Lang) -> Result<String, TranslateError> {
        if text.chars().count() <= GOOGLE_MAX_CHARS {
            return self.translate_one(text, src_lang, target);
        }
        let mut out = String::new();
        for (chunk, sep) in split_for_google(text, GOOGLE_MAX_CHARS) {
            if chunk.trim().is_empty() {
                out.push_str(&chunk);
            } else {
                out.push_str(&self.translate_one(&chunk, src_lang, target)?);
            }
            out.push_str(sep);
        }
        Ok(out)
    }

    fn translate_one(&self, text: &str, src_lang: Option<Lang>, target: Lang) -> Result<String, TranslateError> {
        let query = [
            ("client", "gtx"),
            ("sl", Self::lang_code(src_lang)),
            ("tl", target.google_code()),
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

// ---------- Microsoft Translator ----------

/// Код языка для Microsoft Translator.
fn ms_code(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "ja",
        Lang::Zh => "zh-Hans",
        Lang::En => "en",
        Lang::Ko => "ko",
        Lang::Ru => "ru",
        Lang::Auto => "auto",
    }
}

/// Английское название языка (для подсказки локальному ИИ).
fn lang_name_en(lang: Lang) -> &'static str {
    match lang {
        Lang::Ja => "Japanese",
        Lang::Zh => "Chinese (Simplified)",
        Lang::En => "English",
        Lang::Ko => "Korean",
        Lang::Ru => "Russian",
        Lang::Auto => "the original language",
    }
}

/// Ключ или регион Microsoft не приняты (401/403).
const MS_KEY_ERROR: &str = "Microsoft: ключ не подходит (проверьте ключ и регион)";

pub struct MicrosoftTranslator {
    pub key: String,
    pub region: String,
    http: Arc<dyn Http>,
}

impl MicrosoftTranslator {
    pub const URL: &'static str = "https://api.cognitive.microsofttranslator.com/translate";

    pub fn new(key: &str, region: &str, http: Arc<dyn Http>) -> Self {
        MicrosoftTranslator { key: key.trim().to_string(), region: region.trim().to_string(), http }
    }

    pub fn translate(&self, text: &str, src_lang: Option<Lang>, target: Lang) -> Result<String, TranslateError> {
        let mut url = format!("{}?api-version=3.0&to={}", Self::URL, ms_code(target));
        if let Some(l) = src_lang.filter(|l| *l != Lang::Auto) {
            url.push_str(&format!("&from={}", ms_code(l)));
        }
        let mut headers: Vec<(&str, &str)> = vec![("Ocp-Apim-Subscription-Key", self.key.as_str())];
        if !self.region.is_empty() {
            headers.push(("Ocp-Apim-Subscription-Region", self.region.as_str()));
        }
        let body = serde_json::json!([{ "Text": text }]);
        let resp = self
            .http
            .post_json(&url, &headers, &body)
            .map_err(|e| TranslateError::Failed(format!("нет связи с Microsoft ({})", e.name())))?;
        match resp.status {
            200 => {}
            401 | 403 => return Err(TranslateError::Failed(MS_KEY_ERROR.to_string())),
            429 => return Err(TranslateError::Failed("Microsoft: исчерпан лимит запросов".to_string())),
            s => return Err(TranslateError::Failed(format!("Microsoft ответил кодом {s}"))),
        }
        let bad = || TranslateError::Failed("непонятный ответ Microsoft".to_string());
        let v: Value = serde_json::from_str(&resp.body).map_err(|_| bad())?;
        v.get(0)
            .and_then(|t| t.get("translations"))
            .and_then(|t| t.get(0))
            .and_then(|t| t.get("text"))
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .ok_or_else(bad)
    }
}

// ---------- локальный ИИ (OpenAI-совместимый сервер) ----------

/// Таймаут ответа локальной модели, секунды.
const LOCAL_AI_TIMEOUT: u64 = 120;

/// Убирает блоки рассуждений `<think>…</think>` (у «думающих» моделей).
fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(start) = out.find("<think>") {
        match out[start..].find("</think>") {
            Some(end) => out.replace_range(start..start + end + "</think>".len(), ""),
            None => {
                // блок не закрыт — отбрасываем хвост
                out.truncate(start);
                break;
            }
        }
    }
    // закрывающий тег без открывающего: рассуждение шло до него
    if let Some(p) = out.rfind("</think>") {
        out = out[p + "</think>".len()..].to_string();
    }
    out
}

/// Снимает кавычки, если ими обёрнут весь ответ.
fn strip_quotes(s: &str) -> String {
    for (open, close) in [('«', '»'), ('"', '"'), ('「', '」'), ('『', '』'), ('“', '”')] {
        if s.chars().count() >= 2 && s.starts_with(open) && s.ends_with(close) {
            let inner = &s[open.len_utf8()..s.len() - close.len_utf8()];
            if !inner.contains(close) && !inner.contains(open) {
                return inner.trim().to_string();
            }
        }
    }
    s.to_string()
}

/// Очистка ответа модели: рассуждения, пробелы, внешние кавычки.
fn clean_ai_reply(s: &str) -> String {
    strip_quotes(strip_think(s).trim())
}

pub struct LocalAiTranslator {
    pub base_url: String,
    pub model: String,
    http: Arc<dyn Http>,
}

impl LocalAiTranslator {
    pub fn new(base_url: &str, model: &str, http: Arc<dyn Http>) -> Self {
        LocalAiTranslator {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            model: model.trim().to_string(),
            http,
        }
    }

    pub fn translate(
        &self,
        text: &str,
        src_lang: Option<Lang>,
        target: Lang,
        context: &str,
    ) -> Result<String, TranslateError> {
        if self.model.is_empty() {
            return Err(TranslateError::Failed("Укажите модель локального ИИ в настройках".to_string()));
        }
        let source = src_lang.map(lang_name_en).unwrap_or("the original language");
        let system = format!(
            "You are a professional translator of visual novels. Translate the user's line from {source} into {}. \
             Keep the meaning, tone and speech style; keep character names consistent; \
             do not add explanations, notes, quotes or the original text. Output only the translation.",
            lang_name_en(target)
        );
        let user = if context.is_empty() {
            text.to_string()
        } else {
            format!("Previous lines (for context, do not translate):\n{context}\n\nTranslate this line:\n{text}")
        };
        let body = serde_json::json!({
            "model": self.model,
            "temperature": 0.2,
            "stream": false,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        let url = format!("{}/v1/chat/completions", self.base_url);
        let resp = self.http.post_json_timeout(&url, &[], &body, LOCAL_AI_TIMEOUT).map_err(|e| match e {
            HttpError::Timeout => TranslateError::Failed(format!(
                "Локальный ИИ не ответил за {LOCAL_AI_TIMEOUT} с (модель слишком медленная?)"
            )),
            _ => TranslateError::Failed(format!(
                "Локальный ИИ не отвечает по адресу {}. Запустите Ollama или LM Studio.",
                self.base_url
            )),
        })?;
        if resp.status != 200 {
            return Err(TranslateError::Failed(format!("Локальный ИИ ответил кодом {}", resp.status)));
        }
        let bad = || TranslateError::Failed("непонятный ответ локального ИИ".to_string());
        let v: Value = serde_json::from_str(&resp.body).map_err(|_| bad())?;
        let raw = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(bad)?;
        let out = clean_ai_reply(raw);
        if out.is_empty() {
            return Err(TranslateError::Failed("Локальный ИИ вернул пустой ответ".to_string()));
        }
        Ok(out)
    }
}

/// Список моделей локального сервера: сначала `/v1/models` (OpenAI), затем `/api/tags` (Ollama).
pub fn list_local_models(http: &dyn Http, base_url: &str) -> Result<Vec<String>, String> {
    let base = base_url.trim().trim_end_matches('/');
    let fetch = |path: &str, list: &str, field: &str| -> Option<Vec<String>> {
        let resp = http.get(&format!("{base}{path}"), &[]).ok()?;
        if resp.status != 200 {
            return None;
        }
        let v: Value = serde_json::from_str(&resp.body).ok()?;
        let names: Vec<String> = v
            .get(list)?
            .as_array()?
            .iter()
            .filter_map(|m| m.get(field).and_then(|n| n.as_str()).map(str::to_string))
            .collect();
        Some(names)
    };
    let found = fetch("/v1/models", "data", "id")
        .filter(|l| !l.is_empty())
        .or_else(|| fetch("/api/tags", "models", "name").filter(|l| !l.is_empty()));
    found.ok_or_else(|| {
        format!("Не удалось получить список моделей с {base}. Запустите Ollama или LM Studio и загрузите модель.")
    })
}

// ---------- менеджер ----------

/// DeepL -> при 403/456 автоматически Google. Кэш — по тексту и переводчику.
pub struct TranslatorManager {
    http: Arc<dyn Http>,
    cache: Option<Arc<TranslationCache>>,
    google: GoogleTranslator,
    deepl: Option<DeepLTranslator>,
    deepl_disabled_reason: String,
    /// последняя ошибка Microsoft / локального ИИ: сообщение о переходе на Google показываем один раз
    selected_failure: String,
    microsoft: Option<MicrosoftTranslator>,
    /// Microsoft не принял ключ: до смены ключа в сеть не ходим, сразу Google
    microsoft_denied: bool,
    local_url: String,
    local_model: String,
    /// Отдельный клиент для локального ИИ (None — общий http).
    local_http: Option<Arc<dyn Http>>,
    target: Lang,
    mode: &'static str,
}

impl TranslatorManager {
    pub fn new(deepl_key: &str, cache: Option<Arc<TranslationCache>>, http: Arc<dyn Http>) -> Self {
        let mut m = TranslatorManager {
            google: GoogleTranslator::new(http.clone()),
            http,
            cache,
            deepl: None,
            deepl_disabled_reason: String::new(),
            selected_failure: String::new(),
            microsoft: None,
            microsoft_denied: false,
            local_url: String::new(),
            local_model: String::new(),
            local_http: None,
            target: Lang::Ru,
            mode: "auto",
        };
        m.set_deepl_key(deepl_key);
        m
    }

    /// Язык перевода.
    pub fn set_target(&mut self, lang: Lang) {
        self.target = lang;
    }

    pub fn target(&self) -> Lang {
        self.target
    }

    /// Режим выбора переводчика; неизвестное значение -> "auto".
    pub fn set_mode(&mut self, mode: &str) {
        self.mode = ENGINE_MODES.iter().map(|(m, _)| *m).find(|m| *m == mode).unwrap_or("auto");
        self.selected_failure.clear();
    }

    pub fn mode(&self) -> &str {
        self.mode
    }

    pub fn set_deepl_key(&mut self, key: &str) {
        let key = key.trim();
        self.deepl = if key.is_empty() { None } else { Some(DeepLTranslator::new(key, self.http.clone())) };
        self.deepl_disabled_reason.clear();
    }

    /// Ключ и регион Microsoft Translator (пустой ключ — выключен).
    pub fn set_microsoft(&mut self, key: &str, region: &str) {
        let key = key.trim();
        self.microsoft =
            if key.is_empty() { None } else { Some(MicrosoftTranslator::new(key, region, self.http.clone())) };
        self.microsoft_denied = false;
        self.selected_failure.clear();
    }

    /// Адрес сервера и модель локального ИИ.
    pub fn set_local_ai(&mut self, base_url: &str, model: &str) {
        self.local_url = base_url.trim().to_string();
        self.local_model = model.trim().to_string();
        self.selected_failure.clear();
    }

    /// Отдельный клиент для локального ИИ (без прокси и привязки к интерфейсу).
    pub fn set_local_http(&mut self, http: Arc<dyn Http>) {
        self.local_http = Some(http);
    }

    /// Аналог set_session: новая сеть (прокси, интерфейс).
    pub fn set_http(&mut self, http: Arc<dyn Http>) {
        self.google = GoogleTranslator::new(http.clone());
        if let Some(d) = self.deepl.as_mut() {
            d.http = http.clone();
        }
        if let Some(m) = self.microsoft.as_mut() {
            m.http = http.clone();
        }
        self.http = http;
    }

    pub fn set_cache(&mut self, cache: Option<Arc<TranslationCache>>) {
        self.cache = cache;
    }

    pub fn active_engine(&self) -> &'static str {
        match self.mode {
            "google" => ENGINE_GOOGLE,
            "deepl" => ENGINE_DEEPL,
            "microsoft" => ENGINE_MICROSOFT,
            "local_ai" => ENGINE_LOCAL_AI,
            _ if self.deepl.is_some() && self.deepl_disabled_reason.is_empty() => ENGINE_DEEPL,
            _ => ENGINE_GOOGLE,
        }
    }

    pub fn status_text(&self) -> String {
        match self.mode {
            "google" => "Google (выбран в настройках)".to_string(),
            "microsoft" => {
                if self.microsoft.is_none() {
                    "Microsoft недоступен: ключ не задан (используется Google)".to_string()
                } else if self.microsoft_denied {
                    "Microsoft недоступен: ключ не подходит (используется Google)".to_string()
                } else {
                    "Microsoft Translator".to_string()
                }
            }
            "local_ai" => {
                if self.local_model.is_empty() {
                    "Локальный ИИ: модель не указана (используется Google)".to_string()
                } else {
                    format!("Локальный ИИ ({})", self.local_model)
                }
            }
            "deepl" => {
                if self.deepl.is_none() {
                    "DeepL недоступен: ключ не задан".to_string()
                } else if !self.deepl_disabled_reason.is_empty() {
                    format!("DeepL недоступен: {}", self.deepl_disabled_reason)
                } else {
                    "DeepL".to_string()
                }
            }
            _ => {
                if self.active_engine() == ENGINE_DEEPL {
                    "DeepL".to_string()
                } else if !self.deepl_disabled_reason.is_empty() {
                    format!("Google (DeepL недоступен: {})", self.deepl_disabled_reason)
                } else {
                    "Google (ключ DeepL не задан)".to_string()
                }
            }
        }
    }

    fn cached(&self, text: &str, engine: &str, use_cache: bool) -> Option<String> {
        if use_cache {
            self.cache.as_ref().and_then(|c| c.get(text, engine, self.target.code()))
        } else {
            None
        }
    }

    fn store(&self, text: &str, engine: &str, result: &str) {
        if let Some(c) = &self.cache {
            if !result.is_empty() {
                c.put(text, engine, self.target.code(), result);
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
        // тот же язык: переводить нечего
        if src_lang == Some(self.target) {
            return Ok(TranslateResult {
                text: text.to_string(),
                engine: ENGINE_NONE,
                cached: false,
                notice: String::new(),
            });
        }
        if self.mode == "deepl" {
            return self.translate_deepl_only(text, src_lang, context, use_cache);
        }
        let mut notice = String::new();
        if self.mode == "microsoft" || self.mode == "local_ai" {
            let engine = self.active_engine();
            match self.translate_selected(engine, text, src_lang, context, use_cache) {
                Ok(res) => {
                    self.selected_failure.clear();
                    return Ok(res);
                }
                Err(e) => {
                    let msg = format!("{} не сработал ({e}) — перевод через Google.", engine_title(engine));
                    // одно и то же сообщение на каждой реплике не повторяем
                    if msg != self.selected_failure {
                        notice = msg.clone();
                        self.selected_failure = msg;
                    }
                }
            }
        }
        if self.mode == "auto" && self.active_engine() == ENGINE_DEEPL {
            if let Some(hit) = self.cached(text, ENGINE_DEEPL, use_cache) {
                return Ok(TranslateResult { text: hit, engine: ENGINE_DEEPL, cached: true, notice });
            }
            let attempt = match &self.deepl {
                Some(d) => d.translate(text, src_lang, self.target, context),
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
        let out = self.google.translate(text, src_lang, self.target)?;
        self.store(text, ENGINE_GOOGLE, &out);
        Ok(TranslateResult { text: out, engine: ENGINE_GOOGLE, cached: false, notice })
    }

    /// Режимы Microsoft / локальный ИИ: кэш, затем сам переводчик. Ошибку обрабатывает вызывающий (Google).
    fn translate_selected(
        &mut self,
        engine: &'static str,
        text: &str,
        src_lang: Option<Lang>,
        context: &str,
        use_cache: bool,
    ) -> Result<TranslateResult, TranslateError> {
        // в ключ кэша для локального ИИ входит модель
        let cache_key =
            if engine == ENGINE_LOCAL_AI { format!("{ENGINE_LOCAL_AI}:{}", self.local_model) } else { engine.to_string() };
        if let Some(hit) = self.cached(text, &cache_key, use_cache) {
            return Ok(TranslateResult { text: hit, engine, cached: true, notice: String::new() });
        }
        let out = if engine == ENGINE_MICROSOFT {
            if self.microsoft_denied {
                return Err(TranslateError::Failed(MS_KEY_ERROR.to_string()));
            }
            let res = match &self.microsoft {
                Some(m) => m.translate(text, src_lang, self.target),
                None => {
                    return Err(TranslateError::Failed("ключ Microsoft не задан".to_string()));
                }
            };
            if res.as_ref().is_err_and(|e| e.to_string() == MS_KEY_ERROR) {
                self.microsoft_denied = true;
            }
            res?
        } else {
            let http = self.local_http.clone().unwrap_or_else(|| self.http.clone());
            LocalAiTranslator::new(&self.local_url, &self.local_model, http).translate(
                text,
                src_lang,
                self.target,
                context,
            )?
        };
        self.store(text, &cache_key, &out);
        Ok(TranslateResult { text: out, engine, cached: false, notice: String::new() })
    }

    /// Режим «только DeepL»: без ключа и при любой ошибке — ошибка, без перехода на Google.
    fn translate_deepl_only(
        &mut self,
        text: &str,
        src_lang: Option<Lang>,
        context: &str,
        use_cache: bool,
    ) -> Result<TranslateResult, TranslateError> {
        let Some(d) = &self.deepl else {
            return Err(TranslateError::Failed(
                "Ключ DeepL не задан — укажите его в настройках или выберите другой переводчик.".to_string(),
            ));
        };
        if let Some(hit) = self.cached(text, ENGINE_DEEPL, use_cache) {
            return Ok(TranslateResult { text: hit, engine: ENGINE_DEEPL, cached: true, notice: String::new() });
        }
        match d.translate(text, src_lang, self.target, context) {
            Ok(out) => {
                self.deepl_disabled_reason.clear();
                self.store(text, ENGINE_DEEPL, &out);
                Ok(TranslateResult { text: out, engine: ENGINE_DEEPL, cached: false, notice: String::new() })
            }
            Err(e) => {
                if matches!(e, TranslateError::Denied(_)) {
                    self.deepl_disabled_reason = e.to_string();
                }
                Err(e)
            }
        }
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
            .translate("こんにちは", Some(Lang::Ja), Lang::Ru, "前の文")
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
    fn target_codes_in_requests() {
        // (цель, DeepL target_lang, Google tl)
        for (t, d, g) in [(Lang::En, "EN-US", "en"), (Lang::Ko, "KO", "ko"), (Lang::Zh, "ZH-HANS", "zh-CN")] {
            let s = FakeHttp::new(200);
            let mut m = TranslatorManager::new("key", None, http(&s));
            m.set_target(t);
            assert_eq!(m.target(), t);
            m.translate("こんにちは", Some(Lang::Ja), "", true).unwrap();
            assert_eq!(s.posts()[0].json["target_lang"], d);
            assert_eq!(s.posts()[0].json["source_lang"], "JA");
            let s = FakeHttp::new(200);
            let mut m = TranslatorManager::new("", None, http(&s));
            m.set_target(t);
            m.translate("こんにちは", Some(Lang::Ja), "", true).unwrap();
            assert_eq!(param(&s.gets()[0], "tl"), g);
            assert_eq!(param(&s.gets()[0], "sl"), "ja");
        }
        // источники: корейский, китайский, авто
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("key", None, http(&s));
        m.translate("안녕", Some(Lang::Ko), "", true).unwrap();
        m.translate("你好", None, "", true).unwrap();
        assert_eq!(s.posts()[0].json["source_lang"], "KO");
        assert_eq!(s.posts()[0].json["target_lang"], "RU");
        assert!(s.posts()[1].json.get("source_lang").is_none());
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("", None, http(&s));
        m.translate("你好", Some(Lang::Zh), "", true).unwrap();
        m.translate("안녕", Some(Lang::Ko), "", true).unwrap();
        assert_eq!((param(&s.gets()[0], "sl"), param(&s.gets()[1], "sl")), ("zh-CN", "ko"));
        assert_eq!(param(&s.gets()[0], "tl"), "ru");
    }

    #[test]
    fn mode_google_ignores_deepl() {
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("key", None, http(&s));
        m.set_mode("google");
        assert_eq!(m.mode(), "google");
        let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.engine, res.text.as_str()), ("google", "G:テスト"));
        assert!(s.posts().is_empty() && s.gets().len() == 1);
        assert_eq!(m.status_text(), "Google (выбран в настройках)");
        assert_eq!(m.active_engine(), "google");
    }

    #[test]
    fn mode_deepl_only() {
        // без ключа — ошибка, в сеть не ходим
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("", None, http(&s));
        m.set_mode("deepl");
        let e = m.translate("テスト", Some(Lang::Ja), "", true).unwrap_err();
        assert_eq!(
            e.to_string(),
            "Ключ DeepL не задан — укажите его в настройках или выберите другой переводчик."
        );
        assert!(s.posts().is_empty() && s.gets().is_empty());
        // с ключом работает через DeepL
        m.set_deepl_key("key");
        let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.engine, res.text.as_str()), ("deepl", "D:テスト"));
        assert_eq!(m.status_text(), "DeepL");
        // ошибка DeepL — ошибка, без Google
        for status in [403u16, 500] {
            let s = FakeHttp::new(status);
            let mut m = TranslatorManager::new("key", None, http(&s));
            m.set_mode("deepl");
            assert!(m.translate("テスト", Some(Lang::Ja), "", true).is_err());
            assert!(s.gets().is_empty());
            if status == 403 {
                assert!(m.status_text().starts_with("DeepL недоступен: ошибка 403"));
            }
        }
    }

    #[test]
    fn mode_unknown_is_auto() {
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("", None, http(&s));
        assert_eq!(m.mode(), "auto");
        m.set_mode("google");
        m.set_mode("bogus");
        assert_eq!(m.mode(), "auto");
        assert_eq!(ENGINE_MODES.map(|(c, _)| c), ["auto", "deepl", "google", "microsoft", "local_ai"]);
    }

    #[test]
    fn same_language_not_translated() {
        let s = FakeHttp::new(200);
        let mut m = TranslatorManager::new("key", None, http(&s));
        m.set_target(Lang::En);
        let res = m.translate("Hello", Some(Lang::En), "", true).unwrap();
        assert_eq!((res.text.as_str(), res.engine, res.cached), ("Hello", "none", false));
        assert!(s.posts().is_empty() && s.gets().is_empty());
        assert_eq!(engine_title("none"), "Без перевода (тот же язык)");
        // неизвестный язык оригинала — переводим
        m.translate("Hello", None, "", true).unwrap();
        assert_eq!(s.posts().len(), 1);
        // даже в режиме deepl без ключа тот же язык не даёт ошибки
        let mut m = TranslatorManager::new("", None, http(&s));
        m.set_mode("deepl");
        assert_eq!(m.translate("Привет", Some(Lang::Ru), "", true).unwrap().engine, "none");
    }

    #[test]
    fn cache_depends_on_target() {
        let s = FakeHttp::new(200);
        let cache = Arc::new(TranslationCache::in_memory().unwrap());
        let mut m = TranslatorManager::new("", Some(cache.clone()), http(&s));
        assert!(!m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        m.set_target(Lang::En);
        // другая цель — кэш не подходит
        assert!(!m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert!(m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        m.set_target(Lang::Ru);
        assert!(m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert_eq!(s.gets().len(), 2);
        assert_eq!(cache.count(), 2);
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
        assert_eq!((engine_title("microsoft"), engine_title("local_ai")), ("Microsoft", "Локальный ИИ"));
    }

    // ---------- Microsoft и локальный ИИ ----------

    type PostRec = (String, Vec<(String, String)>, Value, Option<u64>);
    type Responder = Box<dyn Fn(&str) -> Result<HttpResponse, HttpError> + Send + Sync>;

    /// Своя сеть для тестов: ответ POST и GET задаётся функцией от URL; Google-запросы отвечают «G:<q>».
    struct Script {
        post: Responder,
        get: Responder,
        posts: std::sync::Mutex<Vec<PostRec>>,
        gets: std::sync::Mutex<Vec<String>>,
    }

    fn resp(status: u16, body: &str) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse { status, body: body.to_string() })
    }

    impl Script {
        fn new(post: Responder, get: Responder) -> Arc<Self> {
            Arc::new(Script { post, get, posts: Default::default(), gets: Default::default() })
        }
        fn record(&self, url: &str, h: &[(&str, &str)], b: &Value, t: Option<u64>) {
            let h = h.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
            self.posts.lock().unwrap().push((url.to_string(), h, b.clone(), t));
        }
        fn posts(&self) -> Vec<PostRec> {
            self.posts.lock().unwrap().clone()
        }
    }

    impl Http for Script {
        fn post_json(&self, url: &str, h: &[(&str, &str)], b: &Value) -> Result<HttpResponse, HttpError> {
            self.record(url, h, b, None);
            (self.post)(url)
        }
        fn post_json_timeout(&self, url: &str, h: &[(&str, &str)], b: &Value, t: u64) -> Result<HttpResponse, HttpError> {
            self.record(url, h, b, Some(t));
            (self.post)(url)
        }
        fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
            self.gets.lock().unwrap().push(url.to_string());
            if url.contains("translate_a") {
                let q = query.iter().find(|(k, _)| *k == "q").map(|(_, v)| *v).unwrap_or("");
                return resp(200, &serde_json::json!([[[format!("G:{q}"), q, null]], null, "ja"]).to_string());
            }
            (self.get)(url)
        }
    }

    fn no_get() -> Responder {
        Box::new(|_| Err(HttpError::Other("нет".into())))
    }

    fn ms_ok() -> Responder {
        Box::new(|_| resp(200, r#"[{"translations":[{"text":"Привет"}]}]"#))
    }

    fn ai_reply(content: &str) -> Responder {
        let body = serde_json::json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string();
        Box::new(move |_| resp(200, &body))
    }

    fn hdr<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
        h.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn microsoft_request_and_response() {
        let s = Script::new(ms_ok(), no_get());
        let mut m = TranslatorManager::new("", None, s.clone());
        m.set_mode("microsoft");
        m.set_microsoft(" ms-key ", "westeurope");
        let res = m.translate("こんにちは", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.text.as_str(), res.engine, res.notice.as_str()), ("Привет", "microsoft", ""));
        let (url, h, body, t) = &s.posts()[0];
        assert_eq!(url, "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=ru&from=ja");
        assert_eq!(hdr(h, "Ocp-Apim-Subscription-Key"), Some("ms-key"));
        assert_eq!(hdr(h, "Ocp-Apim-Subscription-Region"), Some("westeurope"));
        assert_eq!(body, &serde_json::json!([{"Text": "こんにちは"}]));
        assert!(t.is_none());
        assert_eq!(m.status_text(), "Microsoft Translator");
        assert_eq!(m.active_engine(), "microsoft");
    }

    #[test]
    fn microsoft_no_region_no_source_and_target_codes() {
        let s = Script::new(ms_ok(), no_get());
        let mut m = TranslatorManager::new("", None, s.clone());
        m.set_mode("microsoft");
        m.set_microsoft("k", "");
        m.set_target(Lang::Zh);
        m.translate("x", None, "", true).unwrap();
        let (url, h, _, _) = &s.posts()[0];
        assert_eq!(url, "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=zh-Hans");
        assert!(hdr(h, "Ocp-Apim-Subscription-Region").is_none());
        assert_eq!(ms_code(Lang::Ko), "ko");
        assert_eq!(ms_code(Lang::En), "en");
    }

    #[test]
    fn microsoft_error_texts() {
        for status in [401u16, 403] {
            let s = Script::new(Box::new(move |_| resp(status, "")), no_get());
            let e = MicrosoftTranslator::new("k", "", s).translate("x", None, Lang::Ru).unwrap_err();
            assert_eq!(e.to_string(), "Microsoft: ключ не подходит (проверьте ключ и регион)");
        }
        let s = Script::new(Box::new(|_| resp(429, "")), no_get());
        let e = MicrosoftTranslator::new("k", "", s).translate("x", None, Lang::Ru).unwrap_err();
        assert_eq!(e.to_string(), "Microsoft: исчерпан лимит запросов");
        let s = Script::new(Box::new(|_| resp(500, "")), no_get());
        let e = MicrosoftTranslator::new("k", "", s).translate("x", None, Lang::Ru).unwrap_err();
        assert_eq!(e.to_string(), "Microsoft ответил кодом 500");
    }

    #[test]
    fn microsoft_failure_falls_back_to_google() {
        let s = Script::new(Box::new(|_| resp(401, "")), no_get());
        let mut m = TranslatorManager::new("", None, s.clone());
        m.set_mode("microsoft");
        m.set_microsoft("bad", "");
        let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.text.as_str(), res.engine), ("G:テスト", "google"));
        assert_eq!(
            res.notice,
            "Microsoft не сработал (Microsoft: ключ не подходит (проверьте ключ и регион)) — перевод через Google."
        );
        // ключ не принят: дальше Microsoft не дёргаем, сообщение не повторяем; новый ключ — снова пробуем
        let res = m.translate("次", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.engine, res.notice.as_str()), ("google", ""));
        assert_eq!(s.posts().len(), 1);
        assert_eq!(m.status_text(), "Microsoft недоступен: ключ не подходит (используется Google)");
        m.set_microsoft("new", "");
        m.translate("三", Some(Lang::Ja), "", true).unwrap();
        assert_eq!(s.posts().len(), 2);
        // без ключа тоже Google с пояснением
        let mut m = TranslatorManager::new("", None, Script::new(ms_ok(), no_get()));
        m.set_mode("microsoft");
        let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
        assert!(res.engine == "google" && res.notice.starts_with("Microsoft не сработал"));
        assert!(m.status_text().starts_with("Microsoft недоступен"));
    }

    #[test]
    fn local_ai_request_shape_and_context() {
        let s = Script::new(ai_reply("Привет, мир"), no_get());
        let mut m = TranslatorManager::new("", None, s.clone());
        m.set_mode("local_ai");
        m.set_local_ai("http://localhost:11434/", "qwen2.5");
        let res = m.translate("こんにちは", Some(Lang::Ja), "前の文", true).unwrap();
        assert_eq!((res.text.as_str(), res.engine), ("Привет, мир", "local_ai"));
        let (url, _, body, t) = &s.posts()[0];
        assert_eq!(url, "http://localhost:11434/v1/chat/completions");
        assert_eq!(*t, Some(120));
        assert_eq!(body["model"], "qwen2.5");
        assert_eq!(body["stream"], false);
        assert_eq!(body["temperature"], 0.2);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        let sys = msgs[0]["content"].as_str().unwrap();
        assert!(sys.contains("from Japanese into Russian") && sys.contains("Output only the translation."));
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(
            msgs[1]["content"],
            "Previous lines (for context, do not translate):\n前の文\n\nTranslate this line:\nこんにちは"
        );
        // без контекста и без языка оригинала
        m.translate("你好", None, "", false).unwrap();
        let (_, _, body, _) = &s.posts()[1];
        assert_eq!(body["messages"][1]["content"], "你好");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("from the original language"));
        assert_eq!(m.status_text(), "Локальный ИИ (qwen2.5)");
    }

    #[test]
    fn local_ai_reply_cleanup() {
        for (raw, exp) in [
            ("  Привет  ", "Привет"),
            ("«Привет»", "Привет"),
            ("\"Привет\"", "Привет"),
            ("「Привет」", "Привет"),
            ("<think>Хм,\nнадо перевести.</think>\n\nПривет", "Привет"),
            ("<think>рассуждение</think>«Привет»", "Привет"),
            ("«А» и «Б»", "«А» и «Б»"),
        ] {
            assert_eq!(clean_ai_reply(raw), exp, "{raw}");
        }
    }

    #[test]
    fn local_ai_empty_and_errors() {
        let ai = |http: Arc<Script>| LocalAiTranslator::new("http://h:1", "m", http);
        let e = ai(Script::new(ai_reply("  <think>x</think> "), no_get())).translate("a", None, Lang::Ru, "").unwrap_err();
        assert_eq!(e.to_string(), "Локальный ИИ вернул пустой ответ");
        let s = Script::new(Box::new(|_| Err(HttpError::Connect("ConnectError".into()))), no_get());
        let e = ai(s).translate("a", None, Lang::Ru, "").unwrap_err();
        assert_eq!(e.to_string(), "Локальный ИИ не отвечает по адресу http://h:1. Запустите Ollama или LM Studio.");
        let e = LocalAiTranslator::new("http://h:1", " ", Script::new(ai_reply("x"), no_get()))
            .translate("a", None, Lang::Ru, "")
            .unwrap_err();
        assert_eq!(e.to_string(), "Укажите модель локального ИИ в настройках");
    }

    #[test]
    fn local_ai_failure_falls_back_to_google() {
        let s = Script::new(Box::new(|_| Err(HttpError::Connect("ConnectError".into()))), no_get());
        let mut m = TranslatorManager::new("", None, s);
        m.set_mode("local_ai");
        m.set_local_ai("http://localhost:11434", "m");
        let res = m.translate("テスト", Some(Lang::Ja), "", true).unwrap();
        assert_eq!((res.text.as_str(), res.engine), ("G:テスト", "google"));
        assert_eq!(
            res.notice,
            "Локальный ИИ не сработал (Локальный ИИ не отвечает по адресу http://localhost:11434. \
             Запустите Ollama или LM Studio.) — перевод через Google."
        );
        // та же ошибка на следующей реплике — без повторного сообщения
        let res2 = m.translate("テスト2", Some(Lang::Ja), "", true).unwrap();
        assert!(res2.engine == "google" && res2.notice.is_empty());
        // после смены настроек сообщение снова покажется
        m.set_local_ai("http://localhost:11434", "m");
        assert!(!m.translate("テスト3", Some(Lang::Ja), "", true).unwrap().notice.is_empty());
    }

    #[test]
    fn local_ai_uses_separate_http() {
        let general = Script::new(Box::new(|_| Err(HttpError::Other("не туда".into()))), no_get());
        let local = Script::new(ai_reply("Привет"), no_get());
        let mut m = TranslatorManager::new("", None, general.clone());
        m.set_mode("local_ai");
        m.set_local_ai("http://localhost:1234", "m");
        m.set_local_http(local.clone());
        m.set_http(general.clone());
        assert_eq!(m.translate("a", None, "", true).unwrap().engine, "local_ai");
        assert!(general.posts().is_empty() && local.posts().len() == 1);
    }

    #[test]
    fn local_ai_cache_depends_on_model() {
        let s = Script::new(ai_reply("Привет"), no_get());
        let cache = Arc::new(TranslationCache::in_memory().unwrap());
        let mut m = TranslatorManager::new("", Some(cache.clone()), s.clone());
        m.set_mode("local_ai");
        m.set_local_ai("http://h:1", "model-a");
        assert!(!m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert!(m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        m.set_local_ai("http://h:1", "model-b");
        assert!(!m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        m.set_local_ai("http://h:1", "model-a");
        assert!(m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert_eq!((s.posts().len(), cache.count()), (2, 2));
    }

    #[test]
    fn microsoft_cached() {
        let s = Script::new(ms_ok(), no_get());
        let cache = Arc::new(TranslationCache::in_memory().unwrap());
        let mut m = TranslatorManager::new("", Some(cache), s.clone());
        m.set_mode("microsoft");
        m.set_microsoft("k", "");
        assert!(!m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert!(m.translate("テスト", Some(Lang::Ja), "", true).unwrap().cached);
        assert_eq!(s.posts().len(), 1);
    }

    #[test]
    fn list_models_openai_and_ollama() {
        let s = Script::new(
            no_get(),
            Box::new(|url| {
                if url == "http://h:1/v1/models" {
                    resp(200, r#"{"data":[{"id":"a"},{"id":"b"}]}"#)
                } else {
                    resp(404, "")
                }
            }),
        );
        assert_eq!(list_local_models(&*s, "http://h:1/").unwrap(), vec!["a", "b"]);
        // запасной вариант — Ollama
        let s = Script::new(
            no_get(),
            Box::new(|url| {
                if url == "http://h:1/api/tags" {
                    resp(200, r#"{"models":[{"name":"llama3:8b"}]}"#)
                } else {
                    resp(404, "")
                }
            }),
        );
        assert_eq!(list_local_models(&*s, "http://h:1").unwrap(), vec!["llama3:8b"]);
        assert_eq!(s.gets.lock().unwrap().as_slice(), ["http://h:1/v1/models", "http://h:1/api/tags"]);
        // сервер не запущен
        let s = Script::new(no_get(), Box::new(|_| Err(HttpError::Connect("c".into()))));
        let e = list_local_models(&*s, "http://h:1").unwrap_err();
        assert!(e.contains("Не удалось получить список моделей") && e.contains("http://h:1"));
    }

    #[test]
    fn default_post_json_timeout_delegates() {
        struct Plain(std::sync::Mutex<u32>);
        impl Http for Plain {
            fn post_json(&self, _: &str, _: &[(&str, &str)], _: &Value) -> Result<HttpResponse, HttpError> {
                *self.0.lock().unwrap() += 1;
                resp(200, "ok")
            }
            fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
                Err(HttpError::Timeout)
            }
        }
        let p = Plain(Default::default());
        assert_eq!(p.post_json_timeout("u", &[], &Value::Null, 5).unwrap().body, "ok");
        assert_eq!(*p.0.lock().unwrap(), 1);
    }
}
