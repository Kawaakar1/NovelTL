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
    /// заметка для ИИ-переводчика: кто это, пол, манера речи (необязательно)
    #[serde(default)]
    pub note: String,
}

/// Сколько прочитано в игре (считается по переведённым репликам).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadStats {
    pub lines: u64,
    pub chars: u64,
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
    /// что убирать из текста реплик этой игры (точные фрагменты)
    pub filters: Vec<String>,
    pub stats: ReadStats,
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
            filters: Vec::new(),
            stats: ReadStats::default(),
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
    LazyLock::new(|| Regex::new(r"[^A-Za-z0-9А-Яа-яЁё_\-]+").expect("regex"));

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
    match serde_json::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(v) => Some(v),
        Err(_) => {
            // битый файл не теряем молча: копия рядом (уже существующую .broken не трогаем)
            let mut bak = path.as_os_str().to_owned();
            bak.push(".broken");
            let bak = PathBuf::from(bak);
            if !bak.exists() {
                let _ = fs::copy(path, &bak);
            }
            None
        }
    }
}

/// Атомарная запись: во .tmp (с sync_all) и rename. Кириллица не экранируется.
fn save_json<T: Serialize>(path: &Path, data: &T) -> std::io::Result<()> {
    use std::io::Write;
    let text = serde_json::to_string_pretty(data).map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = fs::File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
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

    /// Id для нового профиля; занятый (та же программа в другой папке) получает суффикс _2, _3…
    pub fn new_id(&self, exe_name: &str, version: &str) -> String {
        let base = make_profile_id(exe_name, version);
        let mut id = base.clone();
        let mut n = 2;
        while self.path(&id).exists() {
            id = format!("{base}_{n}");
            n += 1;
        }
        id
    }

    pub fn create(&self, name: &str, exe_path: &Path, lang: Lang) -> std::io::Result<Profile> {
        let exe_name = exe_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let version = exe_version(exe_path);
        let profile = Profile {
            id: self.new_id(&exe_name, &version),
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
        self.find_in(&self.all(), exe_path)
    }

    /// Как `find_for_exe`, но по уже прочитанному списку профилей (`all()`).
    /// Игра узнаётся по имени exe И папке: у разных игр бывает одинаковый exe (например, Game.exe у RPG Maker).
    pub fn find_in(&self, profiles: &[Profile], exe_path: &Path) -> Option<Profile> {
        let exe_name = exe_path
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let dir = exe_path.parent().map(|d| d.to_string_lossy().to_lowercase());
        let same_game: Vec<&Profile> = profiles
            .iter()
            .filter(|p| {
                p.exe_name.to_lowercase() == exe_name
                    && dir.is_some()
                    && Path::new(&p.exe_path).parent().map(|d| d.to_string_lossy().to_lowercase()) == dir
            })
            .collect();
        if same_game.is_empty() {
            return None;
        }
        let version = exe_version(exe_path);
        let exe_path_str = exe_path.to_string_lossy().into_owned();
        if let Some(p) = same_game.iter().find(|p| p.version == version) {
            let mut p = (*p).clone();
            if p.exe_path != exe_path_str {
                p.exe_path = exe_path_str;
                let _ = self.save(&p);
            }
            return Some(p);
        }
        // база для клона — профиль с самым поздним временем изменения файла (при равенстве/без времени — последний)
        let base = same_game
            .iter()
            .max_by_key(|p| fs::metadata(self.path(&p.id)).and_then(|m| m.modified()).ok())?;
        let mut clone = (*base).clone();
        clone.id = self.new_id(&base.exe_name, &version);
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
    /// Язык перевода.
    pub target_lang: Lang,
    /// Переводчик: auto / deepl / google
    pub engine_mode: String,
    /// Язык оригинала по умолчанию для новых игр.
    pub default_source_lang: Lang,
    /// Ник пользователя в главном окне (пусто — показывается «NovelTL»).
    pub nickname: String,
    /// Имя файла аватара в папке data (пусто — логотип программы).
    pub avatar: String,
    /// Цвета своей темы (тема "custom"): фон, панели/карточки, текст, акцент — "#RRGGBB".
    pub custom_bg: String,
    pub custom_card: String,
    pub custom_text: String,
    pub custom_accent: String,
    /// Ключ и регион Microsoft Translator (ключ на диске шифруется).
    pub ms_key: String,
    pub ms_region: String,
    /// Локальный ИИ (Ollama / LM Studio): адрес сервера и имя модели.
    pub local_ai_url: String,
    pub local_ai_model: String,
    /// Второй перевод мелко под основным: "" (нет), google, deepl, microsoft, local_ai.
    pub second_engine: String,
    /// Язык интерфейса; None — по языку Windows.
    pub ui_lang: Option<crate::i18n::UiLang>,
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
            target_lang: Lang::Ru,
            engine_mode: "auto".into(),
            default_source_lang: Lang::Ja,
            nickname: String::new(),
            avatar: String::new(),
            custom_bg: "#101012".into(),
            custom_card: "#1C1C20".into(),
            custom_text: "#F2F2F2".into(),
            custom_accent: "#FFFFFF".into(),
            ms_key: String::new(),
            ms_region: String::new(),
            local_ai_url: "http://localhost:11434".into(),
            local_ai_model: String::new(),
            second_engine: String::new(),
            ui_lang: None,
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
        s.ms_key = secret::unprotect(&s.ms_key);
        s
    }

    /// Ключ DeepL и пароль прокси на диск пишутся зашифрованными (DPAPI).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        self.save_checked(path).map(|_| ())
    }

    /// Как `save`, но сообщает `true`, если шифрование не удалось и ключ DeepL или пароль прокси
    /// пришлось записать открытым текстом (чтобы не потерять их) — об этом надо предупредить.
    pub fn save_checked(&self, path: &Path) -> std::io::Result<bool> {
        let mut copy = self.clone();
        copy.deepl_key = secret::protect(&self.deepl_key);
        copy.proxy_pass = secret::protect(&self.proxy_pass);
        copy.ms_key = secret::protect(&self.ms_key);
        let plain = |v: &str| !v.is_empty() && !secret::is_protected(v);
        let fallback = plain(&copy.deepl_key) || plain(&copy.proxy_pass) || plain(&copy.ms_key);
        save_json(path, &copy)?;
        Ok(fallback)
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
        p.glossary.push(GlossaryEntry { src: "芳乃".into(), dst: "Ёсино".into(), note: String::new() });
        store.save(&p).unwrap();

        let found = store.find_for_exe(&exe).unwrap();
        assert_eq!(found.id, p.id);
        assert_eq!(
            found.hook,
            Some(HookRef { code: "HKIRI".into(), name: "KiriKiriZ".into(), ctx_lo: 0x1234, ctx2_lo: 0x0005 })
        );
        assert_eq!(found.glossary, vec![GlossaryEntry { src: "芳乃".into(), dst: "Ёсино".into(), note: String::new() }]);
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
    fn clone_base_is_latest_modified() {
        use std::time::{Duration, SystemTime};
        let dir = tmp_dir("latest");
        let exe = dir.join("game.exe");
        fs::write(&exe, vec![b'x'; 7]).unwrap();
        let store = ProfileStore::new(dir.join("profiles"));
        let base = SystemTime::now() - Duration::from_secs(1000);
        // по алфавиту последний — Game_20, но свежее всех Game_100
        for (ver, age) in [("100", 0u64), ("20", 100), ("200", 200)] {
            let p = Profile {
                id: format!("Game_{ver}"),
                name: format!("v{ver}"),
                exe_name: "game.exe".into(),
                exe_path: exe.to_string_lossy().into_owned(),
                version: ver.into(),
                ..Profile::default()
            };
            store.save(&p).unwrap();
            let f = fs::OpenOptions::new().write(true).open(store.path(&p.id)).unwrap();
            f.set_modified(base + Duration::from_secs(500 - age)).unwrap();
        }
        let cloned = store.find_for_exe(&exe).unwrap();
        assert_eq!(cloned.name, "v100");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_exe_name_in_other_folder_is_other_game() {
        // две разные игры на RPG Maker: у обеих Game.exe, у второй — того же и другого размера
        let dir = tmp_dir("rpgm");
        let (a, b) = (dir.join("A"), dir.join("B"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("Game.exe"), vec![b'x'; 100]).unwrap();
        let store = ProfileStore::new(dir.join("profiles"));
        let mut p = store.create("Игра A", &a.join("Game.exe"), Lang::Ja).unwrap();
        p.set_hook("H", "n", 1, 2);
        store.save(&p).unwrap();
        for size in [100usize, 200] {
            fs::write(b.join("Game.exe"), vec![b'x'; size]).unwrap();
            assert!(store.find_for_exe(&b.join("Game.exe")).is_none(), "чужая игра размера {size}");
        }
        assert_eq!(store.all().len(), 1, "профиль чужой игры не должен появиться");
        // тот же размер: новый профиль получает свой id, старый не перезаписывается
        fs::write(b.join("Game.exe"), vec![b'x'; 100]).unwrap();
        let q = store.create("Игра B", &b.join("Game.exe"), Lang::Ja).unwrap();
        assert_ne!(q.id, p.id);
        assert_eq!(store.get(&p.id).unwrap().name, "Игра A");
        assert_eq!(store.find_for_exe(&b.join("Game.exe")).unwrap().id, q.id);
        // регистр имени файла не важен только в Windows; в Linux GAME.EXE — другой файл
        if cfg!(windows) {
            assert_eq!(store.find_for_exe(&a.join("GAME.EXE")).unwrap().id, p.id);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_settings_are_backed_up() {
        let dir = tmp_dir("broken");
        let path = dir.join("settings.json");
        fs::write(&path, "{не json").unwrap();
        assert_eq!(AppSettings::load(&path), AppSettings::default());
        let bak = dir.join("settings.json.broken");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "{не json");
        // существующую копию не перезаписываем
        fs::write(&path, "{ещё один").unwrap();
        let _ = AppSettings::load(&path);
        assert_eq!(fs::read_to_string(&bak).unwrap(), "{не json");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn slug_keeps_yo() {
        assert_eq!(make_profile_id("Ёлка.exe", "1"), "Ёлка_1");
        assert_eq!(make_profile_id("ёж.exe", "1"), "ёж_1");
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

    #[cfg(windows)]
    #[test]
    fn settings_ms_key_encrypted_on_disk() {
        let dir = tmp_dir("ms");
        let path = dir.join("settings.json");
        let mut s = AppSettings::default();
        s.ms_key = "ms-секрет-ключ".into();
        s.ms_region = "westeurope".into();
        s.save(&path).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("ms-секрет-ключ"));
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(v["ms_key"].as_str().unwrap().starts_with("dpapi:"));
        let s2 = AppSettings::load(&path);
        assert_eq!((s2.ms_key.as_str(), s2.ms_region.as_str()), ("ms-секрет-ключ", "westeurope"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_old_file_without_new_fields() {
        let dir = tmp_dir("old");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{"deepl_key": "k", "font_size": 30}"#).unwrap();
        let s = AppSettings::load(&path);
        assert_eq!(s.deepl_key, "k");
        assert_eq!((s.ms_key.as_str(), s.ms_region.as_str()), ("", ""));
        assert_eq!((s.local_ai_url.as_str(), s.local_ai_model.as_str()), ("http://localhost:11434", ""));
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
    fn settings_old_file_has_lang_defaults() {
        let dir = tmp_dir("set");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{"deepl_key": "k"}"#).unwrap();
        let s = AppSettings::load(&path);
        assert_eq!(s.target_lang, Lang::Ru);
        assert_eq!(s.engine_mode, "auto");
        assert_eq!(s.default_source_lang, Lang::Ja);
        // новые значения читаются из файла
        fs::write(&path, r#"{"target_lang": "en", "engine_mode": "google", "default_source_lang": "ko"}"#).unwrap();
        let s = AppSettings::load(&path);
        assert_eq!((s.target_lang, s.engine_mode.as_str(), s.default_source_lang), (Lang::En, "google", Lang::Ko));
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