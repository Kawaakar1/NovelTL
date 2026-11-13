//! Запуск игры из библиотеки: как двойной щелчок по exe, через Steam или по своей команде.

use noveltl_core::i18n::tf;
use noveltl_core::profiles::Profile;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchHow {
    Exe { path: String, dir: String },
    Steam { appid: String },
    Custom { program: String, args: String, dir: String },
}

/// Первый токен (в кавычках или до пробела) — программа, остальное — аргументы.
fn split_command(cmd: &str) -> (String, String) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        return match rest.find('"') {
            Some(i) => (rest[..i].to_string(), rest[i + 1..].trim().to_string()),
            None => (rest.to_string(), String::new()),
        };
    }
    match cmd.find(char::is_whitespace) {
        Some(i) => (cmd[..i].to_string(), cmd[i..].trim().to_string()),
        None => (cmd.to_string(), String::new()),
    }
}

fn parent_dir(path: &str) -> String {
    path.rfind(['\\', '/']).map(|i| path[..i].to_string()).unwrap_or_default()
}

/// appid Steam-игры, если exe лежит в `...\steamapps\common\<папка>\...`.
fn steam_appid(exe: &str) -> Option<String> {
    let parts: Vec<&str> = exe.split(['\\', '/']).collect();
    let i = parts.iter().position(|p| p.eq_ignore_ascii_case("steamapps"))?;
    if !parts.get(i + 1)?.eq_ignore_ascii_case("common") {
        return None;
    }
    let folder = parts.get(i + 2).filter(|f| !f.is_empty())?;
    // всё до «steamapps» включительно (длины частей + разделители в исходной строке)
    let end: usize = parts[..=i].iter().map(|p| p.len() + 1).sum::<usize>() - 1;
    for e in std::fs::read_dir(Path::new(&exe[..end])).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("appmanifest_").and_then(|n| n.strip_suffix(".acf")) else { continue };
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        for line in text.lines() {
            let mut q = line.split('"').skip(1).step_by(2);
            if q.next().is_some_and(|k| k.eq_ignore_ascii_case("installdir")) && q.next() == Some(folder) {
                return Some(id.to_string());
            }
        }
    }
    None
}

pub fn plan(p: &Profile) -> Result<LaunchHow, String> {
    if !p.launch.trim().is_empty() {
        let (program, args) = split_command(&p.launch);
        if program.is_empty() {
            return Err(tf("Не удалось запустить игру: {}", &[&p.launch]));
        }
        let dir = if p.exe_path.is_empty() { parent_dir(&program) } else { parent_dir(&p.exe_path) };
        return Ok(LaunchHow::Custom { program, args, dir });
    }
    if let Some(appid) = steam_appid(&p.exe_path) {
        return Ok(LaunchHow::Steam { appid });
    }
    if !Path::new(&p.exe_path).is_file() {
        return Err(tf("Файл игры не найден: {}", &[&p.exe_path]));
    }
    Ok(LaunchHow::Exe { path: p.exe_path.clone(), dir: parent_dir(&p.exe_path) })
}

#[cfg(windows)]
pub fn run(how: &LaunchHow) -> Result<(), String> {
    match how {
        LaunchHow::Exe { path, dir } => crate::winutil::shell_open(path, "", dir),
        LaunchHow::Steam { appid } => crate::winutil::shell_open(&format!("steam://rungameid/{appid}"), "", ""),
        LaunchHow::Custom { program, args, dir } => crate::winutil::shell_open(program, args, dir),
    }
}

#[cfg(not(windows))]
pub fn run(how: &LaunchHow) -> Result<(), String> {
    use std::process::Command;
    let spawned = match how {
        LaunchHow::Custom { program, args, dir } => {
            let mut c = Command::new(program);
            // shortcut: аргументы делятся по пробелам без кавычек, при нужде — разбор как в оболочке
            c.args(args.split_whitespace());
            if !dir.is_empty() {
                c.current_dir(dir);
            }
            c.spawn()
        }
        LaunchHow::Exe { path, dir } => Command::new("wine").arg(path).current_dir(dir).spawn(),
        LaunchHow::Steam { appid } => Command::new("xdg-open").arg(format!("steam://rungameid/{appid}")).spawn(),
    };
    spawned.map(|_| ()).map_err(|e| tf("Не удалось запустить игру: {}", &[&e]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prof(exe: &str, launch: &str) -> Profile {
        Profile { exe_path: exe.into(), launch: launch.into(), ..Profile::default() }
    }

    #[test]
    fn custom_quotes_and_spaces() {
        let how = plan(&prof("", r#""C:\Program Files\LE\LEProc.exe" -run "D:\g\a b.exe""#)).unwrap();
        assert_eq!(
            how,
            LaunchHow::Custom {
                program: r"C:\Program Files\LE\LEProc.exe".into(),
                args: r#"-run "D:\g\a b.exe""#.into(),
                dir: r"C:\Program Files\LE".into()
            }
        );
        let how = plan(&prof(r"D:\g\a.exe", "dmm://game  --x")).unwrap();
        assert_eq!(
            how,
            LaunchHow::Custom { program: "dmm://game".into(), args: "--x".into(), dir: r"D:\g".into() }
        );
    }

    #[test]
    fn steam_detected_from_manifest() {
        let root = std::env::temp_dir().join(format!("noveltl_steam_{}", std::process::id()));
        let apps = root.join("steamapps");
        std::fs::create_dir_all(apps.join("common").join("My Game")).unwrap();
        std::fs::write(
            apps.join("appmanifest_12345.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"12345\"\n\t\"installdir\"\t\t\"My Game\"\n}\n",
        )
        .unwrap();
        let exe = apps.join("common").join("My Game").join("game.exe");
        let how = plan(&prof(&exe.to_string_lossy(), "")).unwrap();
        assert_eq!(how, LaunchHow::Steam { appid: "12345".into() });
        // другая папка без манифеста — обычный запуск (файла нет — ошибка)
        let other = apps.join("common").join("Other").join("o.exe");
        assert!(plan(&prof(&other.to_string_lossy(), "")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_exe() {
        let e = plan(&prof("Z:/nope/none.exe", "")).unwrap_err();
        assert!(e.contains("none.exe"), "{e}");
    }
}
