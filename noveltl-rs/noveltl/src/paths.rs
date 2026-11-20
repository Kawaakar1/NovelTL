//! Пути приложения: всё хранится рядом с exe (перенос `noveltl/paths.py`).
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Mutex;

static DATA_DIR_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Папка с exe.
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `app_dir()\files`; при разработке (запуск из `target\`) — папка files Python-версии.
pub fn files_dir() -> PathBuf {
    let near = app_dir().join("files");
    // путь машины сборки — только в отладочной сборке, в релиз он не попадает
    #[cfg(debug_assertions)]
    if !near.is_dir() {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("files");
    }
    near
}

/// Системная папка шрифтов: `%SystemRoot%\Fonts` (Windows может стоять не на C:).
pub fn windows_fonts_dir() -> PathBuf {
    fonts_dir_from(std::env::var_os("SystemRoot"))
}

fn fonts_dir_from(system_root: Option<std::ffi::OsString>) -> PathBuf {
    let root = system_root.filter(|r| !r.is_empty()).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join("Fonts")
}

/// Переопределить папку данных (для демо). Глобально.
pub fn set_data_dir_override(path: PathBuf) {
    *DATA_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// Папка данных; создаётся при обращении.
pub fn data_dir() -> PathBuf {
    let over = DATA_DIR_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let path = over.unwrap_or_else(|| app_dir().join("data"));
    let _ = std::fs::create_dir_all(&path);
    path
}

/// Подпапка папки данных; создаётся при обращении.
pub fn sub_data_dir(name: &str) -> PathBuf {
    let path = data_dir().join(name);
    let _ = std::fs::create_dir_all(&path);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_dir_follows_system_root() {
        assert_eq!(fonts_dir_from(Some(r"D:\WINDOWS".into())), PathBuf::from(r"D:\WINDOWS").join("Fonts"));
        assert_eq!(fonts_dir_from(None), PathBuf::from(r"C:\Windows").join("Fonts"));
        assert_eq!(fonts_dir_from(Some("".into())), PathBuf::from(r"C:\Windows").join("Fonts"));
    }
}
