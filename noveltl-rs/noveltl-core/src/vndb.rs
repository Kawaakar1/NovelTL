//! VNDB (api.vndb.org/kana): поиск игры по названию, обложка, скриншоты, персонажи. Только чтение.
//! Берутся только безопасные картинки: sexual < 0.5 и violence < 1.0.

use regex::Regex;
use serde_json::{json, Value};

use crate::translators::Http;

const API: &str = "https://api.vndb.org/kana";

#[derive(Clone, Debug, PartialEq)]
pub struct VnHit {
    pub id: String,
    pub title: String,
    pub alttitle: String,
    pub cover: Option<String>,
    pub screens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VnChar {
    pub id: String,
    pub name: String,
    pub original: String,
    pub image: Option<String>,
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// URL картинки, если она безопасна; нет оценок — считаем небезопасной.
fn safe_url(img: &Value) -> Option<String> {
    let sexual = img.get("sexual")?.as_f64()?;
    let violence = img.get("violence")?.as_f64()?;
    let url = img.get("url")?.as_str()?;
    (sexual < 0.5 && violence < 1.0 && url.starts_with("https://t.vndb.org/")).then(|| url.to_string())
}

fn query(http: &dyn Http, path: &str, body: Value) -> Result<Vec<Value>, String> {
    let r = http.post_json(&format!("{API}/{path}"), &[], &body).map_err(|e| e.name())?;
    if r.status != 200 {
        return Err(format!("HTTP {}", r.status));
    }
    let v: Value = serde_json::from_str(&r.body).map_err(|e| e.to_string())?;
    Ok(v.get("results").and_then(Value::as_array).cloned().unwrap_or_default())
}

fn parse_hit(v: &Value) -> VnHit {
    VnHit {
        id: str_of(v, "id"),
        title: str_of(v, "title"),
        alttitle: str_of(v, "alttitle"),
        cover: v.get("image").and_then(safe_url),
        screens: v
            .get("screenshots")
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |a| a.iter().filter_map(safe_url).collect()),
    }
}

fn parse_char(v: &Value) -> VnChar {
    VnChar {
        id: str_of(v, "id"),
        name: str_of(v, "name"),
        original: str_of(v, "original"),
        image: v.get("image").and_then(safe_url),
    }
}

pub fn search(http: &dyn Http, query_text: &str) -> Result<Vec<VnHit>, String> {
    let body = json!({
        "filters": ["search", "=", query_text],
        "fields": "id,title,alttitle,image.url,image.sexual,image.violence,screenshots.url,screenshots.sexual,screenshots.violence",
        "results": 10
    });
    Ok(query(http, "vn", body)?.iter().map(parse_hit).filter(|h| !h.id.is_empty()).collect())
}

pub fn characters(http: &dyn Http, vid: &str) -> Result<Vec<VnChar>, String> {
    let body = json!({
        "filters": ["vn", "=", ["id", "=", vid]],
        "fields": "id,name,original,image.url,image.sexual,image.violence",
        "results": 100
    });
    Ok(query(http, "character", body)?.iter().map(parse_char).filter(|c| !c.id.is_empty()).collect())
}

/// Строка для сравнения: без пробелов, «・» и знаков, в нижнем регистре.
pub fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn parts(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| c.is_whitespace() || c == '・').map(norm).filter(|p| !p.is_empty())
}

/// Персонаж по имени из игры: полное имя важнее совпадения с одной из частей.
pub fn match_character<'a>(name: &str, chars: &'a [VnChar]) -> Option<&'a VnChar> {
    let n = norm(name);
    if n.is_empty() {
        return None;
    }
    chars
        .iter()
        .find(|c| norm(&c.original) == n || norm(&c.name) == n)
        .or_else(|| chars.iter().find(|c| parts(&c.original).any(|p| p == n) || parts(&c.name).any(|p| p == n)))
}

/// Название игры из заголовка окна: без версий, скобок и хвостов вида « - Steam».
pub fn clean_title(window_title: &str) -> String {
    let brackets = Regex::new(r"\[[^\]]*\]|\([^)]*\)|（[^）]*）|【[^】]*】|［[^］]*］").unwrap();
    let version = Regex::new(r"(?i)\b(ver\.?\s*\d[\w.]*|v\d+(\.\d+)+\w*)").unwrap();
    let tail = Regex::new(r"(?i)\s[-–—|]\s*(steam|dmm[\w ]*|dlsite[\w ]*|fanza[\w ]*|gog[\w ]*)$").unwrap();
    let s = brackets.replace_all(window_title, " ");
    let s = version.replace_all(&s, " ");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = tail.replace(&s, "");
    s.trim_matches(|c: char| c.is_whitespace() || "-–—|:".contains(c)).to_string()
}

/// Варианты запроса для автопоиска: заголовок целиком, затем без подзаголовка
/// («Senren*Banka - A Thousand Colors Of Love -» → «Senren*Banka»).
pub fn title_variants(window_title: &str) -> Vec<String> {
    let full = clean_title(window_title);
    let mut out = vec![full.clone()];
    for sep in [" - ", " – ", " — ", ":", "：", "～", " ~ "] {
        if let Some((head, _)) = full.split_once(sep) {
            out.push(head.trim().to_string());
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|v| norm(v).chars().count() >= 2 && seen.insert(norm(v)));
    out
}

/// Подходит ли найденная игра к запросу: одно название содержит другое.
/// Короткое название VNDB (меньше 4 знаков) засчитываем только целиком — иначе «Air» нашёлся бы где угодно.
pub fn hit_matches(hit: &VnHit, query: &str) -> bool {
    let q = norm(query);
    !q.is_empty()
        && [&hit.title, &hit.alttitle].iter().map(|t| norm(t)).filter(|t| !t.is_empty()).any(|t| {
            t == q || t.contains(&q) || (t.chars().count() >= 4 && q.contains(&t))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translators::{HttpError, HttpResponse};

    const VN: &str = r#"{"more":false,"results":[{"alttitle":"千恋＊万花","id":"v19073","image":{"sexual":0,"url":"https://t.vndb.org/cv/46/80746.jpg","violence":0},"screenshots":[{"sexual":0.07,"url":"https://t.vndb.org/sf/37/96737.jpg","violence":0},{"sexual":0.25,"url":"https://t.vndb.org/sf/38/96738.jpg","violence":0},{"sexual":2,"url":"https://t.vndb.org/sf/44/96744.jpg","violence":0},{"sexual":0,"url":"https://t.vndb.org/sf/45/96745.jpg","violence":1}],"title":"Senren * Banka"}]}"#;
    const CH: &str = r#"{"more":true,"results":[{"id":"c44768","image":{"sexual":0,"url":"https://t.vndb.org/ch/07/95007.jpg","violence":0},"name":"Tomotake Yoshino","original":"朝武 芳乃"},{"id":"c44769","image":{"sexual":1,"url":"https://t.vndb.org/ch/02/95002.jpg","violence":0},"name":"Hitachi Mako","original":"常陸 茉子"},{"id":"c44771","image":null,"name":"Lena Liechtenauer","original":"レナ・リヒテナウアー"}]}"#;

    struct Fixed(String);
    impl Http for Fixed {
        fn post_json(&self, _: &str, _: &[(&str, &str)], _: &Value) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse { status: 200, body: self.0.clone() })
        }
        fn get(&self, _: &str, _: &[(&str, &str)]) -> Result<HttpResponse, HttpError> {
            Err(HttpError::Other("get".into()))
        }
    }

    #[test]
    fn parses_vn_and_filters_unsafe_images() {
        let hits = search(&Fixed(VN.into()), "Senren").unwrap();
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!((h.id.as_str(), h.title.as_str(), h.alttitle.as_str()), ("v19073", "Senren * Banka", "千恋＊万花"));
        assert_eq!(h.cover.as_deref(), Some("https://t.vndb.org/cv/46/80746.jpg"));
        assert_eq!(h.screens, ["https://t.vndb.org/sf/37/96737.jpg", "https://t.vndb.org/sf/38/96738.jpg"]);
    }

    #[test]
    fn parses_characters_and_filters_unsafe() {
        let ch = characters(&Fixed(CH.into()), "v19073").unwrap();
        assert_eq!(ch.len(), 3);
        assert!(ch[0].image.is_some());
        assert!(ch[1].image.is_none(), "sexual 1 — небезопасно");
        assert!(ch[2].image.is_none());
        assert!(search(&Fixed("не json".into()), "x").is_err());
    }

    #[test]
    fn matches_characters() {
        let ch = characters(&Fixed(CH.into()), "v").unwrap();
        assert_eq!(match_character("芳乃", &ch).unwrap().id, "c44768");
        assert_eq!(match_character("朝武芳乃", &ch).unwrap().id, "c44768");
        assert_eq!(match_character("Mako", &ch).unwrap().id, "c44769");
        assert_eq!(match_character("hitachi mako", &ch).unwrap().id, "c44769");
        assert_eq!(match_character("レナ", &ch).unwrap().id, "c44771");
        assert!(match_character("将臣", &ch).is_none());
        assert!(match_character("", &ch).is_none());
    }

    #[test]
    fn cleans_titles() {
        assert_eq!(clean_title("千恋＊万花 - ver1.00"), "千恋＊万花");
        assert_eq!(clean_title("Senren Banka [Ver.1.2.0]"), "Senren Banka");
        assert_eq!(clean_title("Some Game (Steam) v1.05"), "Some Game");
        assert_eq!(clean_title("Some Game - Steam"), "Some Game");
        assert_eq!(clean_title("  Plain   Title "), "Plain Title");
        assert_eq!(clean_title("素晴らしき日々【体験版】"), "素晴らしき日々");
    }

    /// Живой запрос (только чтение): cargo test -p noveltl-core vndb_live -- --ignored
    #[test]
    fn variants_and_matching() {
        let v = title_variants("Senren*Banka - A Thousand Colors Of Love -");
        assert_eq!(v, ["Senren*Banka - A Thousand Colors Of Love", "Senren*Banka"]);
        assert_eq!(title_variants("千恋＊万花"), ["千恋＊万花"]);
        let hit = |t: &str, a: &str| VnHit { id: "v1".into(), title: t.into(), alttitle: a.into(), cover: None, screens: vec![] };
        // заголовок окна длиннее названия на VNDB — раньше это не засчитывалось
        assert!(hit_matches(&hit("Senren * Banka", "千恋＊万花"), "Senren*Banka - A Thousand Colors Of Love"));
        assert!(hit_matches(&hit("Senren * Banka", "千恋＊万花"), "千恋＊万花"));
        assert!(hit_matches(&hit("Riddle Joker", ""), "Riddle"));
        // короткое название — только целиком
        assert!(!hit_matches(&hit("Air", ""), "Fairy Tale Air Edition"));
        assert!(hit_matches(&hit("Air", ""), "AIR"));
        assert!(!hit_matches(&hit("Sabbat of the Witch", ""), "Senren Banka"));
    }

    #[test]
    #[ignore]
    fn vndb_live() {
        let http = crate::net::make_http("system", "", "", "", "").unwrap();
        let hits = search(&http, "Senren Banka").unwrap();
        assert!(hits.iter().any(|h| h.id == "v19073"));
        // заголовок окна Steam-версии целиком: первый же вариант должен найти игру
        let q = &title_variants("Senren*Banka - A Thousand Colors Of Love -")[0];
        let found = search(&http, q).unwrap().into_iter().find(|h| hit_matches(h, q));
        assert_eq!(found.map(|h| h.id).as_deref(), Some("v19073"));
        let ch = characters(&http, "v19073").unwrap();
        assert!(match_character("芳乃", &ch).is_some());
    }
}
