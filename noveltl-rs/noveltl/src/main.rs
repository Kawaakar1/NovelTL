//! NovelTL — точка входа (перенос `main.py`).
//!
//!   noveltl.exe                 обычный запуск
//!   noveltl.exe --demo          демо-режим: имитация игры, настоящая игра не нужна
//!   noveltl.exe --shots ПАПКА   (для разработки) снять снимки окон и выйти
//!   noveltl.exe --selfcheck     проверить загрузку LunaHook и выйти
//!   noveltl.exe --theme system|dark|light
#![windows_subsystem = "windows"]

#[allow(dead_code)]
mod controller;
#[allow(dead_code)]
mod demo;
#[allow(dead_code)]
mod hotkeys;
#[allow(dead_code)]
mod lunahost;
#[allow(dead_code)]
mod overlay;
#[allow(dead_code)]
mod paths;
#[allow(dead_code)]
mod source;
mod tray;
mod ui;
#[allow(dead_code)]
mod winutil;

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

use controller::Controller;
use demo::DemoSource;
use lunahost::LunaHost;
use source::{HookSource, Waker};
use ui::{theme, App, Config, Shared};

struct Args {
    demo: bool,
    shots: Option<PathBuf>,
    selfcheck: bool,
    theme: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args { demo: false, shots: None, selfcheck: false, theme: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => a.demo = true,
            "--selfcheck" => a.selfcheck = true,
            "--shots" => a.shots = it.next().map(PathBuf::from),
            "--theme" => a.theme = it.next().filter(|t| matches!(t.as_str(), "system" | "dark" | "light")),
            other => {
                if let Some(v) = other.strip_prefix("--shots=") {
                    a.shots = Some(PathBuf::from(v));
                } else if let Some(v) = other.strip_prefix("--theme=") {
                    a.theme = Some(v.to_string()).filter(|t| matches!(t.as_str(), "system" | "dark" | "light"));
                }
            }
        }
    }
    a
}

fn message_box(text: &str) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(ui::APP_NAME), MB_OK | MB_ICONINFORMATION);
    }
}

/// Какие письма нужны на экране (шрифты грузятся лениво, чтобы не раздувать память).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FontNeeds {
    pub ja: bool,
    pub ko: bool,
    pub zh: bool,
    /// китайский впереди японского (общие иероглифы рисуются китайскими начертаниями)
    pub zh_first: bool,
}

impl FontNeeds {
    /// Объединение: уже загруженное остаётся, новое добавляется.
    pub fn union(self, o: FontNeeds) -> FontNeeds {
        FontNeeds { ja: self.ja || o.ja, ko: self.ko || o.ko, zh: self.zh || o.zh, zh_first: self.zh_first || o.zh_first }
    }
}

/// Шрифты: Segoe UI основной, Yu Gothic и Meiryo — для японского, Malgun Gothic — для корейского,
/// Microsoft YaHei — для китайского, Segoe UI Bold — семейство «bold».
/// Восточные шрифты занимают десятки МБ памяти, поэтому грузятся только когда нужны (`needs`);
/// они стоят в конце цепочки, так что латиница и кириллица остаются Segoe UI.
fn install_fonts(ctx: &egui::Context, needs: FontNeeds) {
    let dir = PathBuf::from(r"C:\Windows\Fonts");
    let mut fonts = FontDefinitions::default();
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut add = |key: &str, file: &str| -> bool {
        match std::fs::read(dir.join(file)) {
            Ok(bytes) => {
                fonts.font_data.insert(key.to_string(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let segoe = add("segoe", "segoeui.ttf");
    // «жирное» семейство — полужирный Segoe UI Semibold: обычный Bold выглядел слишком тяжёлым
    let segoe_bold = add("segoe_bold", "seguisb.ttf") || add("segoe_bold", "segoeuib.ttf");
    let yu = needs.ja && add("yugothic", "YuGothM.ttc");
    let meiryo = needs.ja && !yu && add("meiryo", "meiryo.ttc");
    let malgun = needs.ko && add("malgun", "malgun.ttf");
    let gulim = needs.ko && !malgun && add("gulim", "gulim.ttc");
    let yahei = needs.zh && add("msyh", "msyh.ttc");
    let simsun = needs.zh && !yahei && add("simsun", "simsun.ttc");
    let pick = |names: &[(&str, bool)]| -> Vec<String> {
        names.iter().filter(|(_, ok)| *ok).map(|(n, _)| n.to_string()).collect()
    };
    let compose = |head: Vec<String>| -> Vec<String> {
        let mut v = head;
        v.extend(defaults.iter().cloned());
        v
    };
    // восточные шрифты: порядок зависит от того, чей язык главный (zh_first — китайский раньше японского)
    let east = |first: bool| -> Vec<(&str, bool)> {
        let ja = [("yugothic", yu), ("meiryo", meiryo)];
        let zh = [("msyh", yahei), ("simsun", simsun)];
        let ko = [("malgun", malgun), ("gulim", gulim)];
        let mut v: Vec<(&str, bool)> = Vec::new();
        if first {
            v.extend(zh);
            v.extend(ja);
        } else {
            v.extend(ja);
            v.extend(zh);
        }
        v.extend(ko);
        v
    };
    let mut prop_names = vec![("segoe", segoe)];
    prop_names.extend(east(needs.zh_first));
    let prop = compose(pick(&prop_names));
    let mut bold_names = vec![(if segoe_bold { "segoe_bold" } else { "segoe" }, segoe_bold || segoe)];
    bold_names.extend(east(needs.zh_first));
    let bold = compose(pick(&bold_names));
    let mut jp_names = east(needs.zh_first);
    jp_names.push(("segoe", segoe));
    let jp = compose(pick(&jp_names));
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(theme::bold_family(), bold);
    fonts.families.insert(theme::jp_family(), jp);
    ctx.set_fonts(fonts);
}

fn selfcheck(data_dir: &std::path::Path) -> i32 {
    let waker: Waker = Arc::new(|| {});
    let (tx, rx) = channel();
    let luna = LunaHost::start(tx, waker.clone());
    let error = luna.error();
    let mut controller = Controller::new(Box::new(luna), rx, false, 0, waker);
    let loaded = error.is_empty();
    let result = serde_json::json!({
        "frozen": true,
        "files_dir": paths::files_dir().is_dir(),
        "lunahost_loaded": loaded,
        "error": error,
        "hotkey_errors": controller.hotkey_errors,
        "version": env!("CARGO_PKG_VERSION"),
    });
    let text = serde_json::to_string_pretty(&result).unwrap_or_default();
    let _ = std::fs::write(data_dir.join("selfcheck.json"), text);
    controller.shutdown();
    if loaded {
        0
    } else {
        2
    }
}

fn main() {
    let args = parse_args();
    let demo = args.demo || args.shots.is_some();

    if demo {
        // демо не трогает настоящие профили
        let mut dir = paths::app_dir().join("data").join(if args.shots.is_some() { "demo_shots" } else { "demo" });
        if args.shots.is_some() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(abs) = dir.canonicalize() {
            dir = abs;
        }
        paths::set_data_dir_override(dir);
    }

    // один экземпляр: именованный мьютекс держим до конца работы (по нему установщик понимает, что программа запущена)
    let mutex = unsafe { CreateMutexW(None, false, w!("NovelTL_Running")) };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already && !demo {
        message_box("NovelTL уже запущен — посмотрите значок в трее.");
        return;
    }
    let _mutex = mutex;

    if args.selfcheck && !demo {
        std::process::exit(selfcheck(&paths::data_dir()));
    }

    let icon = theme::icon_data(64);
    let title = if demo { format!("{} — демо-режим", ui::APP_NAME) } else { ui::APP_NAME.to_string() };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([900.0, 600.0])
        .with_min_inner_size([780.0, 520.0])
        .with_icon(Arc::new(icon));
    // проверки разработчиком: окно в заданной области (например, на втором мониторе)
    if let Some([x, y, _, _]) = winutil::test_area() {
        viewport = viewport.with_position([x + 20.0, y + 20.0]);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let cfg = Config { demo, shots: args.shots.clone() };
    let theme_arg = args.theme.clone();
    let result = eframe::run_native(
        ui::APP_NAME,
        options,
        Box::new(move |cc| {
            install_fonts(&cc.egui_ctx, FontNeeds::default());
            let ctx = cc.egui_ctx.clone();
            let waker: Waker = {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
            let (tx, rx) = channel();
            let (source, demo_source, start_error): (Box<dyn HookSource>, Option<DemoSource>, String) = if cfg.demo {
                let src = DemoSource::new(tx, waker.clone());
                (Box::new(src.clone()), Some(src), String::new())
            } else {
                let luna = LunaHost::start(tx, waker.clone());
                let err = luna.error();
                (Box::new(luna), None, err)
            };
            let mut controller = Controller::new(source, rx, cfg.demo, 0, waker);
            if let Some(t) = &theme_arg {
                controller.settings.theme = t.clone();
            }
            let controller = Arc::new(Mutex::new(controller));
            let shared = Arc::new(Shared::new(controller, ctx));
            let engine = ui::start_engine(shared.clone());
            Ok(Box::new(App::new(shared, engine, cfg, demo_source, start_error)))
        }),
    );
    if let Err(e) = result {
        message_box(&format!("Не удалось запустить окно: {e}"));
    }
}
