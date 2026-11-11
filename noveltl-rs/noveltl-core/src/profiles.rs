//! Профили игр (JSON в data/profiles) и общие настройки (data/settings.json).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::langdetect::Lang;
use crate::scorer::HookRef;
use crate::secret;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayFrac {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Default for OverlayFrac {
    fn default() -> Self {
        OverlayFrac { x: 0.08, y: 0.70, w: 0.84, h: 0.24 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub src: String,
    pub dst: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub exe_name: String,
    pub exe_path: String,
    /// размер exe: новая версия игры = новый профиль
    pub version: String,
    pub lang: Lang,
    pub hook: Option<HookRef>,
    /// поток с именами говорящих, если имя идёт отдельно от реплики
    pub name_hook: Option<HookRef>,
    pub overlay: OverlayFrac,
    pub glossary: Vec<GlossaryEntry>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            id: String::new(),
            name: String::new(),
            exe_name: String::new(),
            exe_path: String::new(),
            version: String::new(),
            lang: Lang::Ja,
            hook: None,
            name_hook: None,
            overlay: OverlayFrac::default(),
            glossary: Vec::new(),
        }
    }
}

impl Profile {
    pub fn set_hook(&mut self, code: &str, name: &str, ctx: u64, ctx2: u64) {
        self.hook = Some(HookRef::from_thread(code, name, ctx, ctx2));
    }
}

/// Размер файла в hex строчными, при ошибке "0".
pub fn exe_version(exe_path: &Path) -> String {
    match fs::metadata(exe_path) {
        Ok(m) => format!("{:x}", m.len()),
        Err(_) => "0".to_string(),
    }
}

/// Аналог os.path.splitext: точка в начале имени расширением не считается.
fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if name[..i].chars().any(|c| c != '.') => &name[..i],
        _ => name,
    }
}

static RE_SLUG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^A-Za-z0-9А-Яа-я_\-]+").expect("regex"));

pub fn make_profile_id(exe_name: &str, version: &str) -> String {
    let stem = file_stem(exe_name);
    let replaced = RE_SLUG.replace_all(stem, "_");
    let trimmed = replaced.trim_matches('_');
    let slug = if trimmed.is_empty() { "game" } else { trimmed };
    format!("{slug}_{version}")
}

/// Читает JSON-объект; нет файла / битый JSON / не объект -> None.
fn load_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    // BOM на всякий случай
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// Атомарная запись: во .tmp и rename. Кириллица не экранируется.
fn save_json<T: Serialize>(path: &Path, data: &T) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(data).map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

pub struct ProfileStore {
    dir: PathBuf,
}

impl ProfileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = fs::create_dir_all(&dir);
        ProfileStore { dir }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn load_profile(path: &Path) -> Option<Profile> {
        load_json::<Profile>(path).filter(|p| !p.id.is_empty())
    }

    pub fn all(&self) -> Vec<Profile> {
        let mut names: Vec<String> = match fs::read_dir(&self.dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".json"))
                .collect(),
            Err(_) => return Vec::new(),
        };
        names.sort();
        names
            .iter()
            .filter_map(|n| Self::load_profile(&self.dir.join(n)))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Profile> {
        Self::load_profile(&self.path(id))
    }

    pub fn save(&self, p: &Profile) -> std::io::Result<()> {
        save_json(&self.path(&p.id), p)
    }

    pub fn delete(&self, id: &str) {
        let _ = fs::remove_file(self.path(id));
    }

    pub fn create(&self, name: &str, exe_path: &Path, lang: Lang) -> std::io::Result<Profile> {
        let exe_name = exe_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let version = exe_version(exe_path);
        let profile = Profile {
            id: make_profile_id(&exe_name, &version),
            name: name.to_string(),
            exe_name,
            exe_path: exe_path.to_string_lossy().into_owned(),
            version,
            lang,
            ..Profile::default()
        };
        self.save(&profile)?;
        Ok(profile)
    }

    /// Профиль для запущенного exe. Для новой версии игры копирует профиль старой.
    pub fn find_for_exe(&self, exe_path: &Path) -> Option<Profile> {
        let exe_name = exe_path
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let version = exe_version(exe_path);
        let exe_path_str = exe_path.to_string_lossy().into_owned();
        let same_name: Vec<Profile> = self
            .all()
            .into_iter()
            .filter(|p| p.exe_name.to_lowercase() == exe_name)
            .collect();
        if same_name.is_empty() {
            return None;
        }
        for mut p in same_name.iter().cloned() {
            if p.version == version {
                if p.exe_path != exe_path_str {
                    p.exe_path = exe_path_str;
                    let _ = self.save(&p);
                }
                return Some(p);
            }
        }
        let base = same_name.last()?;
        let mut clone = base.clone();
        clone.id = make_profile_id(&base.exe_name, &version);
        clone.version = version;
        clone.exe_path = exe_path_str;
        let _ = self.save(&clone);
        Some(clone)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub deepl_key: String,
    /// system / dark / light
    pub theme: String,
    /// system / direct / proxy
    pub net_mode: String,
    pub proxy_url: String,
    pub proxy_user: String,
    pub proxy_pass: String,
    /// пусто = автоматически
    pub bind_ip: String,
    pub font_family: String,
    pub font_size: u32,
    pub text_color: String,
    pub name_color: String,
    pub outline_color: String,
    pub bg_color: String,
    /// проценты
    pub bg_opacity: u32,
    pub hotkey_toggle: String,
    pub hotkey_retranslate: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            deepl_key: String::new(),
            theme: "system".into(),
            net_mode: "system".into(),
            proxy_url: String::new(),
            proxy_user: String::new(),
            proxy_pass: String::new(),
            bind_ip: String::new(),
            font_family: "Segoe UI".into(),
            font_size: 26,
            text_color: "#FFFFFF".into(),
            name_color: "#FFD27F".into(),
            outline_color: "#000000".into(),
            bg_color: "#0E0F1A".into(),
            bg_opacity: 85,
            hotkey_toggle: "Ctrl+Alt+H".into(),
            hotkey_retranslate: "Ctrl+Alt+R".into(),
        }
    }
}

impl AppSettings {
    /// Нет файла / битый JSON -> значения по умолчанию. Поле неверного типа заменяется
    /// значением по умолчанию только само — остальные настройки (ключ DeepL и т.п.) сохраняются.
    pub fn load(path: &Path) -> Self {
        let Some(serde_json::Value::Object(file)) = load_json::<serde_json::Value>(path) else {
            return Self::default();
        };
        let Ok(serde_json::Value::Object(mut merged)) = serde_json::to_value(Self::default()) else {
            return Self::default();
        };
        for (key, value) in file {
            if let Some(default) = merged.get(&key) {
                let mut probe = merged.clone();
                probe.insert(key.clone(), value.clone());
                let ok = serde_json::from_value::<Self>(serde_json::Value::Object(probe)).is_ok();
                if ok && std::mem::discriminant(default) == std::mem::discriminant(&value) {
                    merged.insert(key, value);
                }
            }
        }
        let mut s: Self = serde_json::from_value(serde_json::Value::Object(merged)).unwrap_or_default();
        s.deepl_key = secret::unprotect(&s.deepl_key);
        s.proxy_pass = secret::unprotect(&s.proxy_pass);
        s
    }

    /// Ключ DeepL и пароль прокси на диск пишутся зашифрованными (DPAPI).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut copy = self.clone();
        copy.deepl_key = secret::protect(&self.deepl_key);
        copy.proxy_pass = secret::protect(&self.proxy_pass);
        save_json(path, &copy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static N: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let d = std::env::temp_dir().join(format!(
            "noveltl_{}_{}_{}_{}",
            tag,
            std::process::id(),
            nanos,
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn profiles_roundtrip() {
        let dir = tmp_dir("prof");
        let exe = dir.join("SenrenBanka.exe");
        fs::write(&exe, vec![b'x'; 100]).unwrap();
        let store = ProfileStore::new(dir.join("profiles"));
        let mut p = store.create("Senren*Banka", &exe, Lang::Ja).unwrap();
        p.set_hook("HKIRI", "KiriKiriZ", 0x7FF600001234, 0x10005);
        p.glossary.push(GlossaryEntry { src: "芳乃".into(), dst: "Ёсино".into() });
        store.save(&p).unwrap();

        let found = store.find_for_exe(&exe).unwrap();
        assert_eq!(found.id, p.id);
        assert_eq!(
            found.hook,
            Some(HookRef { code: "HKIRI".into(), name: "KiriKiriZ".into(), ctx_lo: 0x1234, ctx2_lo: 0x0005 })
        );
        assert_eq!(found.glossary, vec![GlossaryEntry { src: "芳乃".into(), dst: "Ёсино".into() }]);
        assert!(found.overlay.w > 0.0 && found.overlay.w <= 1.0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_new_version_is_cloned() {
        let dir = tmp_dir("prof");
        let exe = dir.join("game.exe");
        fs::write(&exe, vec![b'x'; 100]).unwrap();
        let store = ProfileStore::new(dir.join("profiles"));
        let mut p = store.create("Game", &exe, Lang::Ja).unwrap();
        p.set_hook("H", "n", 1, 2);
        store.save(&p).unwrap();
        fs::write(&exe, vec![b'x'; 200]).unwrap(); // игра обновилась
        let p2 = store.find_for_exe(&exe).unwrap();
        assert_ne!(p2.id, p.id);
        assert_eq!(p2.hook, p.hook);
        assert_eq!(store.all().len(), 2);
        assert!(store.find_for_exe(&dir.join("other.exe")).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_roundtrip() {
        let dir = tmp_dir("set");
        let path = dir.join("settings.json");
        let mut s = AppSettings::load(&path);
        assert_eq!(s.hotkey_toggle, "Ctrl+Alt+H");
        assert_eq!(s.hotkey_retranslate, "Ctrl+Alt+R");
        s.deepl_key = "k:fx".into();
        s.font_size = 30;
        s.save(&path).unwrap();
        let s2 = AppSettings::load(&path);
        assert_eq!((s2.deepl_key.as_str(), s2.font_size), ("k:fx", 30));
        assert_eq!(s2, s);
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn settings_secrets_encrypted_on_disk() {
        let dir = tmp_dir("sec");
        let path = dir.join("settings.json");
        let mut s = AppSettings::default();
        s.deepl_key = "секрет-ключ:fx".into();
        s.proxy_pass = "пароль123".into();
        s.save(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("секрет-ключ:fx") && !raw.contains("пароль123"));
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(v["deepl_key"].as_str().unwrap().starts_with("dpapi:"));
        assert!(v["proxy_pass"].as_str().unwrap().starts_with("dpapi:"));
        let s2 = AppSettings::load(&path);
        assert_eq!(s2.deepl_key, "секрет-ключ:fx");
        assert_eq!(s2.proxy_pass, "пароль123");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_undecryptable_secret_is_empty() {
        let dir = tmp_dir("sec");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{"deepl_key": "dpapi:AAAA", "font_size": 30}"#).unwrap();
        let s = AppSettings::load(&path);
        assert_eq!(s.deepl_key, "");
        assert_eq!(s.font_size, 30);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_have_theme_and_survive_old_file() {
        let dir = tmp_dir("set");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{"deepl_key": "k:fx", "font_size": 30}"#).unwrap();
        let s = AppSettings::load(&path);
        assert_eq!(s.theme, "system");
        assert_eq!(s.deepl_key, "k:fx");
        assert_eq!(s.font_size, 30);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_old_file_has_net_defaults() {
        let dir = tmp_dir("set");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{"deepl_key": "k"}"#).unwrap();
        let s = AppSettings::load(&path);
        assert!(s.net_mode == "system" && s.bind_ip.is_empty() && s.proxy_url.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_old_file_has_no_proxy_credentials() {
        let dir = tmp_dir("set");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{"deepl_key": "k", "proxy_url": "http://h:1"}"#).unwrap();
        let s = AppSettings::load(&path);
        assert!(s.proxy_user.is_empty() && s.proxy_pass.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_broken_or_unknown() {
        let dir = tmp_dir("set");
        let path = dir.join("settings.json");
        fs::write(&path, "{не json").unwrap();
        assert_eq!(AppSettings::load(&path), AppSettings::default());
        fs::write(&path, r#"{"font_size": 31, "unknown_field": [1, 2]}"#).unwrap();
        assert_eq!(AppSettings::load(&path).font_size, 31);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_id_like_python() {
        assert_eq!(make_profile_id("SenrenBanka.exe", "64"), "SenrenBanka_64");
        assert_eq!(make_profile_id("Мой Квест!.exe", "a"), "Мой_Квест_a");
        assert_eq!(make_profile_id("???.exe", "0"), "game_0");
        assert_eq!(make_profile_id("a.b.exe", "1"), "a_b_1");
    }

    #[test]
    fn exe_version_hex() {
        let dir = tmp_dir("ver");
        let exe = dir.join("a.exe");
        fs::write(&exe, vec![0u8; 255]).unwrap();
        assert_eq!(exe_version(&exe), "ff");
        assert_eq!(exe_version(&dir.join("missing.exe")), "0");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn python_profile_json_compat() {
        let dir = tmp_dir("compat");
        let store = ProfileStore::new(&dir);
        let json = r#"{
  "id": "Game_64",
  "name": "Сэнрэн Банка",
  "exe_name": "Game.exe",
  "exe_path": "C:\\Games\\Game.exe",
  "version": "64",
  "lang": "ja",
  "hook": {"code": "HKIRI", "name": "KiriKiriZ", "ctx_lo": 4660, "ctx2_lo": 0},
  "name_hook": null,
  "overlay": {"x": 0.08, "y": 0.7, "w": 0.84, "h": 0.24},
  "glossary": [{"src": "芳乃", "dst": "Ёсино"}],
  "extra": 1
}"#;
        fs::write(dir.join("Game_64.json"), json).unwrap();
        let p = store.get("Game_64").unwrap();
        assert_eq!(p.name, "Сэнрэн Банка");
        assert_eq!(p.lang, Lang::Ja);
        assert_eq!(p.hook.as_ref().map(|h| (h.ctx_lo, h.ctx2_lo)), Some((4660, 0)));
        assert!(p.name_hook.is_none());
        assert_eq!(p.overlay, OverlayFrac::default());
        store.save(&p).unwrap();
        // кириллица не экранируется
        let saved = fs::read_to_string(dir.join("Game_64.json")).unwrap();
        assert!(saved.contains("Сэнрэн Банка"));
        let p2 = store.get("Game_64").unwrap();
        assert_eq!(serde_json::to_value(&p).unwrap(), serde_json::to_value(&p2).unwrap());
        // файл без id и битый файл пропускаются
        fs::write(dir.join("noid.json"), r#"{"name": "x"}"#).unwrap();
        fs::write(dir.join("broken.json"), "{").unwrap();
        assert_eq!(store.all().len(), 1);
        store.delete("Game_64");
        assert!(store.get("Game_64").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod lenient_settings_tests {
    use super::*;

    #[test]
    fn wrong_type_field_keeps_others() {
        let dir = std::env::temp_dir().join(format!("ntl_lenient_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"deepl_key": "k:fx", "font_size": "огромный", "bg_opacity": 40, "unknown": 1}"#).unwrap();
        let s = AppSettings::load(&path);
        assert_eq!(s.deepl_key, "k:fx");
        assert_eq!(s.bg_opacity, 40);
        assert_eq!(s.font_size, AppSettings::default().font_size);
        let _ = std::fs::remove_dir_all(&dir);
    }
}