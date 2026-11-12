//! Сетевые настройки перевода: прокси, прямое подключение, выбор интерфейса.

use std::error::Error as StdError;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::translators::{Http, HttpError, HttpResponse};

pub const NET_SYSTEM: &str = "system";
pub const NET_DIRECT: &str = "direct";
pub const NET_PROXY: &str = "proxy";

pub fn net_title(mode: &str) -> &'static str {
    match mode {
        NET_SYSTEM => "Как в системе",
        NET_DIRECT => "Напрямую, без прокси",
        NET_PROXY => "Через прокси",
        _ => "",
    }
}

const PROXY_ERROR: &str = "Неверный адрес прокси. Пример: socks5://127.0.0.1:1080 или http://127.0.0.1:8080";
const AUTH_ERROR: &str = "Прокси не принял логин или пароль";
const TIMEOUT: Duration = Duration::from_secs(12);

fn scheme_map(s: &str) -> Option<&'static str> {
    Some(match s {
        "http" => "http",
        "https" => "https",
        "socks5" | "socks5h" => "socks5h",
        "socks4" | "socks4a" => "socks4a",
        _ => return None,
    })
}

/// Разбор адреса как urlsplit в Python: (схема, netloc, остаток после netloc).
fn split_url(text: &str) -> Option<(String, &str, &str)> {
    let (scheme, rest) = text.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some((scheme.to_lowercase(), &rest[..end], &rest[end..]))
}

/// Хост (в нижнем регистре, без скобок IPv6) и порт из `host:port` (после последней `@`).
/// Err — порт не число или вне диапазона u16.
fn host_port(netloc: &str) -> Result<(String, Option<u32>), ()> {
    let hostport = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
    let (host, port_s) = if let Some(rest) = hostport.strip_prefix('[') {
        match rest.split_once(']') {
            Some((h, tail)) => (h, tail.strip_prefix(':')),
            None => (rest, None),
        }
    } else {
        match hostport.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    // последний ':' для порта, как у Python (rpartition) — у обычных адресов он единственный
    let port = match port_s {
        None | Some("") => None,
        Some(p) => {
            if !p.chars().all(|c| c.is_ascii_digit()) {
                return Err(());
            }
            let n: u32 = p.parse().map_err(|_| ())?;
            if n > 65535 {
                return Err(());
            }
            Some(n)
        }
    };
    Ok((host.to_lowercase(), port))
}

pub fn normalize_proxy(text: &str) -> Result<String, String> {
    let mut text = text.trim().to_string();
    if text.is_empty() {
        return Err("Укажите адрес прокси".to_string());
    }
    if !text.contains("://") {
        text = format!("http://{text}");
    }
    let err = || PROXY_ERROR.to_string();
    let (scheme, netloc, _) = split_url(&text).ok_or_else(err)?;
    let (host, port) = host_port(netloc).map_err(|_| err())?;
    let scheme = scheme_map(&scheme).ok_or_else(err)?;
    if scheme == "socks4a" {
        // reqwest умеет только SOCKS5 и HTTP(S)
        return Err("SOCKS4 не поддерживается — используйте SOCKS5 или HTTP".to_string());
    }
    match port {
        Some(p) if !host.is_empty() && (1..=65535).contains(&p) => Ok(format!("{scheme}://{netloc}")),
        _ => Err(err()),
    }
}

/// Как urllib.parse.quote(s, safe="").
fn quote(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Как urllib.parse.unquote (некорректный UTF-8 заменяется).
fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|c| (*c as char).to_digit(16)),
                b.get(i + 2).and_then(|c| (*c as char).to_digit(16)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn build_proxy_url(address: &str, user: &str, password: &str) -> Result<String, String> {
    let base = normalize_proxy(address)?;
    let user = user.trim();
    if user.is_empty() && !password.is_empty() {
        return Err("Укажите логин прокси".to_string());
    }
    if user.is_empty() {
        return Ok(base);
    }
    let (scheme, netloc, _) = split_url(&base).ok_or_else(|| PROXY_ERROR.to_string())?;
    if scheme == "socks4a" && !password.is_empty() {
        return Err("SOCKS4 не поддерживает пароль — используйте SOCKS5".to_string());
    }
    let (host, port) = host_port(netloc).map_err(|_| PROXY_ERROR.to_string())?;
    let host = if host.contains(':') { format!("[{host}]") } else { host };
    let mut cred = quote(user);
    if !password.is_empty() {
        cred.push(':');
        cred.push_str(&quote(password));
    }
    Ok(format!("{scheme}://{cred}@{host}:{}", port.unwrap_or(0)))
}

/// Разделяет адрес с логином на (адрес без логина, логин, пароль); логин и пароль раскодированы.
pub fn split_proxy_credentials(url: &str) -> (String, String, String) {
    let text = url.trim();
    if let Some((scheme, netloc, rest)) = split_url(text) {
        if let Some((userinfo, hostport)) = netloc.rsplit_once('@') {
            let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            return (format!("{scheme}://{hostport}{rest}"), unquote(user), unquote(password));
        }
    }
    (url.to_string(), String::new(), String::new())
}

// ---------- HTTP через reqwest ----------

pub struct ReqwestHttp {
    client: reqwest::blocking::Client,
    uses_proxy: bool,
}

/// Собирает тексты всей цепочки ошибок и ищет коды ОС.
fn map_error(e: &reqwest::Error, uses_proxy: bool) -> HttpError {
    if e.is_timeout() {
        return HttpError::Timeout;
    }
    let mut texts: Vec<String> = vec![e.to_string().to_lowercase()];
    let mut os_codes: Vec<i32> = vec![];
    let mut cur: Option<&(dyn StdError + 'static)> = e.source();
    while let Some(err) = cur {
        texts.push(err.to_string().to_lowercase());
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if let Some(code) = io.raw_os_error() {
                os_codes.push(code);
            }
        }
        cur = err.source();
    }
    let all = texts.join(" | ");
    // HTTP-прокси: 407 на CONNECT -> "proxy authorization required";
    // SOCKS5: отказ логина/пароля -> "credentials not accepted"
    if all.contains("proxy authorization required")
        || all.contains("proxy authentication required")
        || all.contains("credentials not accepted")
        || all.contains("authentication failed")
        || all.contains("auth failure")
    {
        return HttpError::ProxyAuth;
    }
    // код ОС бывает только в тексте (ошибка TLS-обёртки не отдаёт io::Error), поэтому смотрим и туда
    if os_codes.contains(&10013) || all.contains("(os error 10013)") {
        return HttpError::AccessDenied;
    }
    if os_codes.contains(&10049) || all.contains("(os error 10049)") {
        return HttpError::AddressUnavailable;
    }
    if e.is_connect() {
        return if uses_proxy { HttpError::ProxyUnreachable } else { HttpError::Connect("ConnectError".to_string()) };
    }
    if e.is_request() || e.is_body() || e.is_decode() {
        return HttpError::Other("RequestError".to_string());
    }
    HttpError::Other("Error".to_string())
}

impl ReqwestHttp {
    fn finish(&self, r: Result<reqwest::blocking::Response, reqwest::Error>) -> Result<HttpResponse, HttpError> {
        let resp = r.map_err(|e| map_error(&e, self.uses_proxy))?;
        let status = resp.status().as_u16();
        let body = resp.text().map_err(|e| map_error(&e, self.uses_proxy))?;
        Ok(HttpResponse { status, body })
    }

}

impl Http for ReqwestHttp {
    fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &Value) -> Result<HttpResponse, HttpError> {
        let mut req = self.client.post(url).json(body);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        self.finish(req.send())
    }

    fn post_json_timeout(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout_secs: u64,
    ) -> Result<HttpResponse, HttpError> {
        let mut req = self.client.post(url).json(body).timeout(Duration::from_secs(timeout_secs));
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        self.finish(req.send())
    }

    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
        self.finish(self.client.get(url).query(query).send())
    }
}

pub fn make_http(
    mode: &str,
    proxy_url: &str,
    bind_ip: &str,
    proxy_user: &str,
    proxy_pass: &str,
) -> Result<ReqwestHttp, String> {
    let mut b = reqwest::blocking::Client::builder().timeout(TIMEOUT);
    let mut uses_proxy = false;
    if mode == NET_DIRECT {
        b = b.no_proxy();
    } else if mode == NET_PROXY {
        let url = build_proxy_url(proxy_url, proxy_user, proxy_pass)?;
        // логин и пароль отдаём отдельно (раскодированными): так они уходят на сервер ровно как введены
        let (clean, user, pass) = split_proxy_credentials(&url);
        let mut proxy = reqwest::Proxy::all(clean.as_str()).map_err(|_| PROXY_ERROR.to_string())?;
        if !user.is_empty() {
            proxy = proxy.basic_auth(&user, &pass);
        }
        b = b.no_proxy().proxy(proxy);
        uses_proxy = true;
    }
    let bind_ip = bind_ip.trim();
    if !bind_ip.is_empty() {
        let ip: IpAddr = bind_ip.parse().map_err(|_| format!("Неверный сетевой адрес: {bind_ip}"))?;
        b = b.local_address(ip);
    }
    let client = b.build().map_err(|e| format!("Не удалось настроить соединение: {e}"))?;
    Ok(ReqwestHttp { client, uses_proxy })
}

/// Клиент для локального ИИ (Ollama / LM Studio): без прокси, без привязки к интерфейсу, таймаут 120 с.
pub fn make_local_http() -> Arc<dyn Http> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("не удалось создать HTTP-клиент");
    Arc::new(ReqwestHttp { client, uses_proxy: false })
}

pub fn check_google(http: &dyn Http) -> (bool, String) {
    let query = [("client", "gtx"), ("sl", "en"), ("tl", "ru"), ("dt", "t"), ("q", "test")];
    let resp = match http.get("https://translate.googleapis.com/translate_a/single", &query) {
        Ok(r) => r,
        Err(HttpError::ProxyAuth) => return (false, AUTH_ERROR.to_string()),
        Err(HttpError::ProxyUnreachable) => {
            return (false, "Не удалось подключиться к прокси. Проверьте адрес и что прокси запущен".to_string())
        }
        Err(HttpError::AccessDenied) => {
            return (
                false,
                "Выход через этот интерфейс запрещён системой. Обычно так делает VPN \
                 с защитой от утечек (kill switch) — выключите её в настройках VPN или используйте прокси"
                    .to_string(),
            )
        }
        Err(HttpError::AddressUnavailable) => {
            return (false, "Этого сетевого адреса сейчас нет на компьютере — выберите другой интерфейс".to_string())
        }
        Err(HttpError::Connect(_)) | Err(HttpError::Timeout) => {
            return (false, "Нет соединения с Google. Проверьте интернет, прокси или выбранный интерфейс".to_string())
        }
        Err(HttpError::Other(name)) => return (false, format!("Ошибка: {name}")),
    };
    match resp.status {
        407 => (false, AUTH_ERROR.to_string()),
        200 => (true, "Google отвечает, перевод работает".to_string()),
        429 => (
            false,
            "Google ограничил запросы с этого адреса (код 429). Попробуйте другой интерфейс или прокси".to_string(),
        ),
        s => (false, format!("Google ответил кодом {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    #[test]
    fn normalize_proxy_ok() {
        for (src, exp) in [
            ("127.0.0.1:8080", "http://127.0.0.1:8080"),
            ("socks5://127.0.0.1:1080", "socks5h://127.0.0.1:1080"),
            ("  http://user:pass@host:3128 ", "http://user:pass@host:3128"),
        ] {
            assert_eq!(normalize_proxy(src).unwrap(), exp);
        }
    }

    #[test]
    fn normalize_proxy_errors() {
        for src in ["", "ftp://h:1", "socks5://h", "http://:80", "http://h:99999"] {
            assert!(normalize_proxy(src).is_err(), "{src}");
        }
        assert_eq!(normalize_proxy("").unwrap_err(), "Укажите адрес прокси");
        assert!(normalize_proxy("ftp://h:1").unwrap_err().starts_with("Неверный адрес прокси"));
    }

    #[test]
    fn titles() {
        assert_eq!(net_title(NET_SYSTEM), "Как в системе");
        assert_eq!(net_title(NET_DIRECT), "Напрямую, без прокси");
        assert_eq!(net_title(NET_PROXY), "Через прокси");
    }

    #[test]
    fn make_http_modes() {
        assert!(make_http(NET_SYSTEM, "", "", "", "").is_ok());
        assert!(make_http(NET_DIRECT, "", "", "", "").is_ok());
        assert!(make_http(NET_PROXY, "socks5://127.0.0.1:1080", "", "", "").is_ok());
        assert!(make_http(NET_PROXY, "bad", "", "", "").is_err());
        assert!(make_http(NET_DIRECT, "", "127.0.0.1", "", "").is_ok());
        assert!(make_http(NET_DIRECT, "", "не-адрес", "", "").is_err());
    }

    #[test]
    fn build_proxy_url_basic() {
        assert_eq!(build_proxy_url("127.0.0.1:1080", "", "").unwrap(), "http://127.0.0.1:1080");
        assert_eq!(
            build_proxy_url("socks5://h:1080", "user", "p@ss:w/rd").unwrap(),
            "socks5h://user:p%40ss%3Aw%2Frd@h:1080"
        );
        assert_eq!(build_proxy_url("socks5://h:1080", "user", "").unwrap(), "socks5h://user@h:1080");
    }

    #[test]
    fn build_proxy_url_fields_replace_address_data() {
        assert_eq!(build_proxy_url("http://old:x@h:3128", "new", "y").unwrap(), "http://new:y@h:3128");
        assert_eq!(build_proxy_url("http://old:x@h:3128", "", "").unwrap(), "http://old:x@h:3128");
    }

    #[test]
    fn build_proxy_url_errors() {
        assert!(build_proxy_url("http://h:1", "", "secret").is_err());
        assert!(build_proxy_url("socks4://h:1080", "user", "secret").is_err());
    }

    #[test]
    fn build_proxy_url_cyrillic_roundtrip() {
        let url = build_proxy_url("socks5://h:1080", "юзер", "пароль:1 @").unwrap();
        assert!(!url.contains("юзер"));
        assert_eq!(
            split_proxy_credentials(&url),
            ("socks5h://h:1080".to_string(), "юзер".to_string(), "пароль:1 @".to_string())
        );
    }

    #[test]
    fn split_credentials() {
        let s = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(split_proxy_credentials("http://h:1"), s("http://h:1", "", ""));
        assert_eq!(split_proxy_credentials("http://u:p@h:1"), s("http://h:1", "u", "p"));
        assert_eq!(split_proxy_credentials("http://u%40x:p%3A%2F@h:1"), s("http://h:1", "u@x", "p:/"));
        assert_eq!(split_proxy_credentials(""), s("", "", ""));
    }

    // ---------- живые тесты на локальных серверах (без интернета) ----------

    /// Запускает сервер: каждое соединение отдаёт обработчику, потом закрывает.
    fn serve(handler: impl Fn(&mut TcpStream) + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { return };
                let _ = conn.set_read_timeout(Some(Duration::from_secs(3)));
                handler(&mut conn);
            }
        });
        port
    }

    fn recvn(c: &mut TcpStream, n: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = vec![0u8; n];
        c.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn socks_server(got: Arc<Mutex<Vec<(Vec<u8>, Vec<u8>)>>>, expected: (Vec<u8>, Vec<u8>)) -> u16 {
        serve(move |c| {
            let run = |c: &mut TcpStream| -> std::io::Result<()> {
                let h = recvn(c, 2)?;
                recvn(c, h[1] as usize)?;
                c.write_all(&[5, 2])?;
                recvn(c, 1)?;
                let n = recvn(c, 1)?[0] as usize;
                let user = recvn(c, n)?;
                let n = recvn(c, 1)?[0] as usize;
                let pw = recvn(c, n)?;
                let ok = (user.clone(), pw.clone()) == expected;
                got.lock().unwrap().push((user, pw));
                if ok {
                    c.write_all(&[1, 0])?;
                    let mut b = [0u8; 256];
                    let _ = c.read(&mut b);
                } else {
                    c.write_all(&[1, 1])?;
                }
                Ok(())
            };
            let _ = run(c);
        })
    }

    #[test]
    fn socks5_credentials_reach_server_decoded() {
        let got = Arc::new(Mutex::new(vec![]));
        let expected = ("юзер".as_bytes().to_vec(), "p@ss:w/rd".as_bytes().to_vec());
        let port = socks_server(got.clone(), expected.clone());
        let url = format!("socks5://127.0.0.1:{port}");

        let http = make_http(NET_PROXY, &url, "", "юзер", "p@ss:w/rd").unwrap();
        let (_, text) = check_google(&http);
        assert_eq!(got.lock().unwrap().last().unwrap(), &expected);
        assert!(!text.contains("p@ss") && !text.contains("%40"), "{text}");
        assert_ne!(text, AUTH_ERROR);

        let http = make_http(NET_PROXY, &url, "", "юзер", "wrong").unwrap();
        assert_eq!(check_google(&http), (false, AUTH_ERROR.to_string()));
    }

    #[test]
    fn http_proxy_basic_auth() {
        let got: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let g2 = got.clone();
        let port = serve(move |c| {
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            while !data.windows(4).any(|w| w == b"\r\n\r\n") {
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => data.extend_from_slice(&buf[..n]),
                }
            }
            let mut creds = String::new();
            for line in data.split(|b| *b == b'\n') {
                let line = String::from_utf8_lossy(line).trim().to_string();
                if line.to_lowercase().starts_with("proxy-authorization:") {
                    let token = line.splitn(3, ' ').nth(2).unwrap_or("");
                    use base64::Engine;
                    if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(token) {
                        creds = String::from_utf8_lossy(&raw).into_owned();
                    }
                }
            }
            g2.lock().unwrap().push(creds.clone());
            let reply: &[u8] = if creds == "user:p@ss:w/rd" {
                b"HTTP/1.1 200 Connection established\r\n\r\n"
            } else {
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\nContent-Length: 0\r\n\r\n"
            };
            let _ = c.write_all(reply);
        });
        let url = format!("http://127.0.0.1:{port}");

        let http = make_http(NET_PROXY, &url, "", "user", "p@ss:w/rd").unwrap();
        let (_, text) = check_google(&http);
        assert_eq!(got.lock().unwrap().last().unwrap(), "user:p@ss:w/rd");
        assert!(!text.contains("p@ss"), "{text}");
        assert_ne!(text, AUTH_ERROR);

        let http = make_http(NET_PROXY, &url, "", "user", "wrong").unwrap();
        assert_eq!(check_google(&http), (false, AUTH_ERROR.to_string()));

        // кириллица в логине и пароле уходит в UTF-8, а не роняет запрос
        let http = make_http(NET_PROXY, &url, "", "юзер", "пароль@1").unwrap();
        assert_eq!(check_google(&http), (false, AUTH_ERROR.to_string()));
        assert_eq!(got.lock().unwrap().last().unwrap(), "юзер:пароль@1");
    }

    #[test]
    fn local_ai_over_real_http() {
        use crate::langdetect::Lang;
        use crate::translators::LocalAiTranslator;
        let got: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let g2 = got.clone();
        let port = serve(move |c| {
            // читаем заголовки, затем тело по Content-Length
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            let mut need: Option<usize> = None;
            loop {
                if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&data[..pos]).to_lowercase();
                    let len = *need.get_or_insert_with(|| {
                        head.lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                            .unwrap_or(0)
                    });
                    if data.len() >= pos + 4 + len {
                        break;
                    }
                }
                match c.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => data.extend_from_slice(&buf[..n]),
                }
            }
            g2.lock().unwrap().push(String::from_utf8_lossy(&data).into_owned());
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"«Привет, мир»"}}]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = c.write_all(reply.as_bytes());
        });
        let tr = LocalAiTranslator::new(&format!("http://127.0.0.1:{port}/"), "test-model", make_local_http());
        let out = tr.translate("こんにちは", Some(Lang::Ja), Lang::Ru, "前の文").unwrap();
        assert_eq!(out, "Привет, мир");
        let req = got.lock().unwrap().last().unwrap().clone();
        assert!(req.starts_with("POST /v1/chat/completions HTTP/1.1"), "{req}");
        assert!(req.contains("\"model\":\"test-model\"") && req.contains("前の文"));
    }

    #[test]
    fn local_ai_server_down_message() {
        use crate::langdetect::Lang;
        use crate::translators::LocalAiTranslator;
        let url = "http://127.0.0.1:9";
        let e = LocalAiTranslator::new(url, "m", make_local_http()).translate("a", None, Lang::Ru, "").unwrap_err();
        assert_eq!(e.to_string(), format!("Локальный ИИ не отвечает по адресу {url}. Запустите Ollama или LM Studio."));
    }

    #[test]
    fn proxy_is_really_used() {
        let http = make_http(NET_PROXY, "http://127.0.0.1:9", "", "", "").unwrap();
        let (ok, text) = check_google(&http);
        assert!(!ok && text.contains("прокси"), "{text}");
    }

    #[test]
    fn bind_to_missing_address_does_not_crash() {
        let http = make_http(NET_DIRECT, "", "10.255.255.254", "", "").unwrap();
        let (ok, text) = check_google(&http);
        assert!(!ok && !text.is_empty());
        assert_eq!(text, "Этого сетевого адреса сейчас нет на компьютере — выберите другой интерфейс");
    }

    #[test]
    fn check_google_texts() {
        struct Fixed(Result<HttpResponse, HttpError>);
        impl Http for Fixed {
            fn post_json(&self, _: &str, _: &[(&str, &str)], _: &Value) -> Result<HttpResponse, HttpError> {
                self.0.clone()
            }
            fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
                self.0.clone()
            }
        }
        let r = |s: u16| Fixed(Ok(HttpResponse { status: s, body: String::new() }));
        assert_eq!(check_google(&r(200)), (true, "Google отвечает, перевод работает".to_string()));
        assert_eq!(check_google(&r(407)), (false, AUTH_ERROR.to_string()));
        assert!(check_google(&r(429)).1.contains("код 429"));
        assert_eq!(check_google(&r(500)).1, "Google ответил кодом 500");
        assert!(check_google(&Fixed(Err(HttpError::AccessDenied))).1.contains("kill switch"));
        assert!(check_google(&Fixed(Err(HttpError::Connect("x".into())))).1.starts_with("Нет соединения с Google"));
        assert_eq!(check_google(&Fixed(Err(HttpError::Other("Boom".into())))).1, "Ошибка: Boom");
    }
}
