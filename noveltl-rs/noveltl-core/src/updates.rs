//! Проверка обновлений: спрашивает GitHub о последнем релизе.

use crate::translators::Http;

pub const RELEASES_API: &str = "https://api.github.com/repos/Kawaakar1/NovelTL/releases/latest";

/// (версия без «v», ссылка на релиз) из ответа GitHub.
pub fn parse_latest(json: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v.get("tag_name")?.as_str()?.trim().trim_start_matches('v');
    let url = v.get("html_url")?.as_str()?;
    if tag.is_empty() || url.is_empty() {
        return None;
    }
    Some((tag.to_string(), url.to_string()))
}

fn parse_ver(s: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = s.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// Версия `latest` строго новее `current` (сравнение чисел x.y.z; мусор — false).
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_ver(latest), parse_ver(current)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => false,
    }
}

/// Some((версия, ссылка)) — если вышла версия новее `current`.
pub fn check(http: &dyn Http, current: &str) -> Result<Option<(String, String)>, String> {
    let resp = http.get(RELEASES_API, &[]).map_err(|e| format!("{e:?}"))?;
    if resp.status != 200 {
        return Err(format!("HTTP {}", resp.status));
    }
    let (ver, url) = parse_latest(&resp.body).ok_or("bad response")?;
    Ok(is_newer(&ver, current).then_some((ver, url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer() {
        assert!(is_newer("0.6.10", "0.6.9"));
        assert!(is_newer("0.7.0", "0.6.1"));
        assert!(is_newer("v0.7.0", "0.6.1"));
        assert!(!is_newer("0.6.1", "0.6.1"));
        assert!(!is_newer("0.6.0", "0.6.1"));
        assert!(!is_newer("abc", "0.6.1"));
        assert!(!is_newer("0.7.0", ""));
    }

    #[test]
    fn parse() {
        let j = r#"{"tag_name":"v0.7.0","html_url":"https://github.com/x/y/releases/tag/v0.7.0"}"#;
        assert_eq!(parse_latest(j), Some(("0.7.0".into(), "https://github.com/x/y/releases/tag/v0.7.0".into())));
        assert_eq!(parse_latest("{}"), None);
        assert_eq!(parse_latest("xx"), None);
    }
}
