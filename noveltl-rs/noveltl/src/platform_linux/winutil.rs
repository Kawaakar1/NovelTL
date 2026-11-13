//! Linux: замена winutil.rs. Чужих окон не перечисляем; свои окна — условные номера.
#![allow(dead_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    pub exe_path: String,
}

impl WindowInfo {
    pub fn exe_name(&self) -> String {
        self.exe_path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
    }
}

/// Условные номера своих окон (настоящих HWND в Linux нет).
pub const MAIN_HWND: isize = 1;
pub const DEMO_HWND: isize = 2;

/// Только для проверок разработчиком: область экрана «x,y,ширина,высота» из переменной
/// NOVELTL_TEST_AREA (например, второй монитор), где открывать окна программы.
pub fn test_area() -> Option<[f32; 4]> {
    let v = std::env::var("NOVELTL_TEST_AREA").ok()?;
    let n: Vec<f32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (n.len() == 4 && n[2] > 0.0 && n[3] > 0.0).then(|| [n[0], n[1], n[2], n[3]])
}

pub fn is_window(hwnd: isize) -> bool {
    hwnd == MAIN_HWND || hwnd == DEMO_HWND
}

pub fn is_minimized(_hwnd: isize) -> bool {
    false
}

pub fn is_process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

pub fn is_process_64bit(_pid: u32) -> bool {
    true
}

pub fn list_windows(own_allowed: &[isize]) -> Vec<WindowInfo> {
    if !own_allowed.contains(&DEMO_HWND) {
        return Vec::new();
    }
    vec![WindowInfo {
        hwnd: DEMO_HWND,
        pid: std::process::id(),
        title: crate::demo::DEMO_TITLE.to_string(),
        exe_path: std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    }]
}

/// Выбор сетевого интерфейса — только в Windows.
pub fn list_interfaces() -> Vec<(String, String)> {
    Vec::new()
}

fn autostart_file() -> Option<std::path::PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(v) => std::path::PathBuf::from(v),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("autostart").join("noveltl.desktop"))
}

pub fn autostart_enabled() -> bool {
    autostart_file().is_some_and(|p| p.exists())
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    let path = autostart_file().ok_or_else(|| "HOME".to_string())?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=NovelTL\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
            exe.to_string_lossy()
        );
        std::fs::write(&path, text).map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Открывает папку, где лежит файл.
pub fn reveal_in_explorer(path: &str) {
    let p = std::path::Path::new(path);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(p);
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// Открывает папку или ссылку программой по умолчанию.
pub fn open_url(target: &str) {
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

pub fn system_ui_lang() -> noveltl_core::i18n::UiLang {
    use noveltl_core::i18n::UiLang;
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return UiLang::from_locale(&v);
            }
        }
    }
    UiLang::En
}
