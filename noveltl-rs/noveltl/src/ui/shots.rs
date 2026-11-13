//! Снимки окон для проверки (`--shots ПАПКА`): главное окно средствами egui, окно игры с плашкой — с экрана.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, UserData, ViewportCommand};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, ROP_CODE, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowThreadProcessId, SetWindowPos, ShowWindow, WindowFromPoint, GA_ROOT, HWND_TOP,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
};

use super::demo_game::DemoGame;
use super::glossary::GlossaryWindow;
use super::settings::SettingsWindow;
use super::wizard::Wizard;
use super::{lock_ctl, Shared, WizardMode, Windows};
use crate::{tray, winutil};
use noveltl_core::net::NET_PROXY;
use noveltl_core::profiles::OverlayFrac;

/// Профиль для снимков: ник «Reader» и аватар-заглушка, нарисованная здесь же
/// (чёрно-белые кольца) — никаких картинок пользователя. Пишется только в демо-папку данных.
fn demo_profile(shared: &Shared) {
    const N: u32 = 256;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - 128.0, y as f32 - 100.0);
            let r = (dx * dx + dy * dy).sqrt();
            let v: u8 = if r < 46.0 {
                235
            } else if (dx.abs() < 70.0) && y > 160 {
                200
            } else if ((r / 18.0) as i32) % 2 == 0 {
                40
            } else {
                70
            };
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let src = std::env::temp_dir().join(format!("noveltl_demo_avatar_{}.png", std::process::id()));
    if save_png(&src, N, N, &rgba).is_err() {
        return;
    }
    let stored = super::avatar::store_avatar(&src);
    let _ = std::fs::remove_file(&src);
    let mut c = lock_ctl(&shared.controller);
    let mut s = c.settings.clone();
    s.nickname = "Reader".to_string();
    if let Ok(name) = stored {
        s.avatar = name;
    }
    c.save_settings(s);
}

/// Сохраняет RGBA-картинку в PNG.
pub fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Снимок viewport'а, пришедший как `egui::Event::Screenshot`.
pub fn save_color_image(path: &Path, img: &ColorImage) -> Result<(), String> {
    let mut rgba = Vec::with_capacity(img.pixels.len() * 4);
    for px in &img.pixels {
        rgba.extend_from_slice(&px.to_array());
    }
    save_png(path, img.size[0] as u32, img.size[1] as u32, &rgba)
}

/// Снимок области экрана (x, y, w, h) через GDI; захватывает и отдельные окна поверх (плашку).
pub fn capture_screen_region(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(HWND(std::ptr::null_mut()));
        let dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp);
        let blit = BitBlt(dc, 0, 0, w, h, screen, x, y, ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0));
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(dc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(dc);
        ReleaseDC(HWND(std::ptr::null_mut()), screen);
        if blit.is_err() || lines == 0 {
            return None;
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
            px[3] = 255;
        }
        Some(buf)
    }
}

/// Поднять окно над соседними, НЕ делая его активным: фокус остаётся у того, что открыл
/// пользователь (полноэкранная игра на другом мониторе не свернётся).
pub fn bring_to_front(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as _);
        if winutil::is_minimized(hwnd) {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
        let _ = SetWindowPos(h, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// Во всех контрольных точках снимаемой области сверху лежат только окна самой программы
/// (демо-игра, мастер, настройки, плашка) — чужие окна в снимок не попадут.
fn game_window_visible(hwnd: isize) -> bool {
    let Some((x, y, w, h)) = winutil::client_rect_on_screen(hwnd) else {
        return false;
    };
    let me = std::process::id();
    unsafe {
        for fx in [0.02, 0.25, 0.5, 0.75, 0.98] {
            for fy in [0.02, 0.25, 0.5, 0.75, 0.98] {
                let pt = POINT { x: x + (w as f64 * fx) as i32, y: y + (h as f64 * fy) as i32 };
                let root = GetAncestor(WindowFromPoint(pt), GA_ROOT);
                let mut pid = 0u32;
                GetWindowThreadProcessId(root, Some(&mut pid));
                if root.0.is_null() || pid != me {
                    return false;
                }
            }
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Start,
    /// ждём снимок (имя файла в `awaiting`), потом идём в `after`
    WaitShot,
    OpenWizard,
    WizardWait,
    StartSearch,
    Connect,
    Lines(i32),
    Found,
    OthersOpen,
    OthersWait,
    Confirm,
    Step3Wait,
    EditShot,
    AfterFinish,
    Translate,
    Settle,
    InGame,
    SettingsOpen,
    SettingsWait,
    /// снимок раздела настроек с этим номером
    SettingsSection(usize),
    /// снимок «Персонализации» со своей темой
    SettingsCustom,
    /// раздел «Плашка» со стилем «Как в игре»
    SettingsPlateGame,
    SettingsLocalAi,
    SettingsClose,
    GlossaryOpen,
    GlossaryWait,
    GlossaryShot,
    GlossaryClose,
    Finish,
    Done,
}

pub struct Shots {
    dir: PathBuf,
    stage: Stage,
    after: Stage,
    since: Instant,
    started: Instant,
    frames: u32,
    awaiting: Option<String>,
    fronted: bool,
    log: Vec<String>,
}

impl Shots {
    pub fn new(dir: PathBuf) -> Shots {
        let _ = std::fs::create_dir_all(&dir);
        let now = Instant::now();
        Shots {
            dir,
            stage: Stage::Start,
            after: Stage::Done,
            since: now,
            started: now,
            frames: 0,
            awaiting: None,
            fronted: false,
            log: Vec::new(),
        }
    }

    fn go(&mut self, stage: Stage) {
        self.stage = stage;
        self.since = Instant::now();
        self.fronted = false;
    }

    fn note(&mut self, text: impl Into<String>) {
        self.log.push(text.into());
        let _ = std::fs::write(self.dir.join("shots.log"), self.log.join("\n"));
    }

    /// Снимок главного окна; после его прихода сценарий идёт в `after`.
    fn request(&mut self, ctx: &egui::Context, name: &str, after: Stage) {
        self.awaiting = Some(name.to_string());
        self.after = after;
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
        self.go(Stage::WaitShot);
    }

    /// Снимок окна мастера: у вложенных окон egui снимков нет, поэтому снимаем с экрана (окно всегда сверху).
    fn shot_wizard(&mut self, wizard: &Wizard, name: &str, after: Stage) {
        let hwnd = tray::find_own_window(wizard.window_title());
        self.capture_game(hwnd, name);
        self.go(after);
    }

    /// Снимок отдельного окна по заголовку (настройки, глоссарий) — с экрана, окно всегда сверху.
    fn shot_window(&mut self, title: impl AsRef<str>, name: &str, after: Stage) {
        let hwnd = tray::find_own_window(title.as_ref());
        self.capture_game(hwnd, name);
        self.go(after);
    }

    /// Снимок области окна игры с экрана (вместе с плашкой).
    fn capture_game(&mut self, hwnd: isize, name: &str) {
        // снимается кусок экрана — если окно игры чем-то закрыто, в снимок попадут чужие окна
        if !game_window_visible(hwnd) {
            self.note(format!("окно игры закрыто другим окном — {name} не сохранён"));
            return;
        }
        match winutil::client_rect_on_screen(hwnd)
            .and_then(|(x, y, w, h)| capture_screen_region(x, y, w, h).map(|b| (w, h, b)))
        {
            Some((w, h, buf)) => match save_png(&self.dir.join(name), w as u32, h as u32, &buf) {
                Ok(()) => self.note(format!("сохранён {name}")),
                Err(e) => self.note(format!("ошибка {name}: {e}")),
            },
            None => self.note(format!("не удалось снять экран с игрой ({name})")),
        }
    }

    fn finish(&mut self, shared: &Shared, why: &str) {
        self.note(why);
        shared.quit.store(true, Ordering::SeqCst);
        shared.ctx.request_repaint();
        self.go(Stage::Done);
    }

    /// Вызывается каждый кадр; управляет сценарием: мастер проходится через его же методы.
    pub fn step(
        &mut self,
        ctx: &egui::Context,
        shared: &Shared,
        game: &mut DemoGame,
        wizard: &mut Option<Wizard>,
        windows: &mut Windows,
        settings: &mut Option<SettingsWindow>,
        glossary: &mut Option<GlossaryWindow>,
    ) {
        self.frames += 1;
        ctx.request_repaint_after(Duration::from_millis(50));
        if self.stage == Stage::Done {
            return;
        }
        if self.started.elapsed() > Duration::from_secs(80) {
            self.finish(shared, "таймаут сценария");
            return;
        }
        // пришедшие снимки: главного окна и окна мастера
        let shots: Vec<Arc<ColorImage>> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        for img in shots {
            if let Some(name) = self.awaiting.take() {
                match save_color_image(&self.dir.join(&name), &img) {
                    Ok(()) => self.note(format!("сохранён {name}")),
                    Err(e) => self.note(format!("ошибка {name}: {e}")),
                }
                let next = self.after;
                self.go(next);
            }
        }
        let waited = self.since.elapsed();
        match self.stage {
            Stage::Start => {
                if self.frames == 1 {
                    demo_profile(shared);
                }
                if self.frames > 6 && waited > Duration::from_millis(1200) {
                    self.request(ctx, "01_main_empty.png", Stage::OpenWizard);
                }
            }
            Stage::WaitShot => {
                if waited > Duration::from_secs(5) {
                    let name = self.awaiting.clone().unwrap_or_default();
                    self.finish(shared, &format!("снимок не пришёл: {name}"));
                }
            }
            Stage::OpenWizard => {
                if game.hwnd == 0 {
                    if waited > Duration::from_secs(10) {
                        self.finish(shared, "окно игры не найдено");
                    }
                    return;
                }
                windows.open_wizard(WizardMode::New);
                self.go(Stage::WizardWait);
            }
            Stage::WizardWait => {
                if let Some(w) = wizard.as_ref() {
                    if w.frames() > 3 && waited > Duration::from_millis(700) {
                        self.shot_wizard(w, "02_wizard_step1.png", Stage::StartSearch);
                    }
                } else if waited > Duration::from_secs(5) {
                    self.finish(shared, "окно мастера не открылось");
                }
            }
            Stage::StartSearch => {
                let ok = wizard.as_mut().is_some_and(|w| w.select_and_start(shared, game.hwnd));
                if ok {
                    self.go(Stage::Connect);
                } else if waited > Duration::from_secs(5) {
                    self.finish(shared, "демо-окно не найдено в списке мастера");
                }
            }
            Stage::Connect => {
                let connected = lock_ctl(&shared.controller).session.as_ref().is_some_and(|s| s.connected);
                if connected && waited > Duration::from_millis(700) {
                    if let Some(w) = wizard.as_ref() {
                        self.shot_wizard(w, "03_wizard_step2_waiting.png", Stage::Lines(0));
                    }
                } else if waited > Duration::from_secs(10) {
                    self.finish(shared, "нет подключения к демо-источнику");
                }
            }
            Stage::Lines(i) => {
                if i == 0 || waited > Duration::from_millis(900) {
                    game.advance();
                    self.go(if i < 3 { Stage::Lines(i + 1) } else { Stage::Found });
                }
            }
            Stage::Found => {
                if waited > Duration::from_millis(900) {
                    if let Some(w) = wizard.as_ref() {
                        self.shot_wizard(w, "04_wizard_step2_found.png", Stage::OthersOpen);
                    }
                }
            }
            Stage::OthersOpen => {
                if let Some(w) = wizard.as_mut() {
                    w.set_others(true);
                }
                self.go(Stage::OthersWait);
            }
            Stage::OthersWait => {
                if waited > Duration::from_millis(500) {
                    if let Some(w) = wizard.as_ref() {
                        self.shot_wizard(w, "05_wizard_step2_others.png", Stage::Confirm);
                    }
                }
            }
            Stage::Confirm => {
                let ok = wizard.as_mut().is_some_and(|w| {
                    w.set_others(false);
                    w.confirm_best(shared)
                });
                if ok {
                    bring_to_front(game.hwnd);
                    self.go(Stage::Step3Wait);
                } else if waited > Duration::from_secs(10) {
                    self.finish(shared, "мастер не нашёл текст диалога");
                }
            }
            Stage::Step3Wait => {
                if waited > Duration::from_millis(700) {
                    if let Some(w) = wizard.as_ref() {
                        self.shot_wizard(w, "06_wizard_step3.png", Stage::EditShot);
                    }
                }
            }
            Stage::EditShot => {
                if !self.fronted {
                    // плашка чуть выше и меньше окна диалога, чтобы на снимке был виден и оригинал
                    lock_ctl(&shared.controller).set_overlay_position(OverlayFrac { x: 0.14, y: 0.50, w: 0.72, h: 0.17 });
                    bring_to_front(game.hwnd);
                    self.fronted = true;
                    self.note("игра на переднем плане");
                }
                if waited > Duration::from_millis(1500) {
                    self.capture_game(game.hwnd, "07_overlay_edit_mode.png");
                    if let Some(w) = wizard.as_mut() {
                        w.finish(shared);
                    }
                    self.go(Stage::AfterFinish);
                }
            }
            Stage::AfterFinish => {
                if waited > Duration::from_millis(600) {
                    game.advance();
                    self.go(Stage::Translate);
                }
            }
            Stage::Translate => {
                let has = lock_ctl(&shared.controller).session.as_ref().is_some_and(|s| s.last_result.is_some());
                if has {
                    self.go(Stage::Settle);
                } else if waited > Duration::from_secs(15) {
                    self.finish(shared, "перевод не пришёл");
                }
            }
            Stage::Settle => {
                if waited > Duration::from_millis(1200) {
                    self.go(Stage::InGame);
                }
            }
            Stage::InGame => {
                if !self.fronted {
                    bring_to_front(game.hwnd);
                    self.fronted = true;
                }
                if waited > Duration::from_millis(1500) {
                    self.capture_game(game.hwnd, "08_overlay_in_game.png");
                    self.request(ctx, "09_main_running.png", Stage::SettingsOpen);
                }
            }
            Stage::SettingsOpen => {
                windows.open_settings();
                self.go(Stage::SettingsWait);
            }
            Stage::SettingsWait => match settings.as_ref() {
                Some(w) => {
                    if w.frames() > 14 && waited > Duration::from_millis(1800) {
                        self.go(Stage::SettingsSection(0));
                    }
                }
                None => {
                    if waited > Duration::from_secs(5) {
                        self.finish(shared, "окно настроек не открылось");
                    }
                }
            },
            Stage::SettingsSection(i) => {
                const NAMES: [&str; 7] = [
                    "10_settings_look.png",
                    "11_settings_translate.png",
                    "12_settings_plate.png",
                    "13_settings_network.png",
                    "14_settings_hotkeys.png",
                    "15_settings_japanese.png",
                    "16_settings_general.png",
                ];
                if !self.fronted {
                    // выбрать раздел (для сети — ещё и подставить прокси-заглушки)
                    if let Some(w) = settings.as_mut() {
                        w.select_section_for_shots(i);
                        if i == 0 {
                            w.fill_profile_for_shots("Reader");
                        }
                        if i == 3 {
                            w.fill_net_for_shots(NET_PROXY, "socks5h://127.0.0.1:1080", "user", "secret");
                        }
                    }
                    self.fronted = true;
                }
                if waited > Duration::from_millis(700) {
                    // локальный ИИ снимаем до «своей» темы, чтобы он был в обычной теме
                    let next = if i + 1 < NAMES.len() { Stage::SettingsSection(i + 1) } else { Stage::SettingsLocalAi };
                    if let Some(w) = settings.as_ref() {
                        self.shot_window(w.window_title(), NAMES[i], next);
                    } else {
                        self.go(Stage::SettingsClose);
                    }
                }
            }
            Stage::SettingsCustom => {
                if !self.fronted {
                    if let Some(w) = settings.as_mut() {
                        w.select_section_for_shots(0);
                        w.fill_profile_for_shots("Reader");
                        w.fill_custom_theme_for_shots("#1B1020", "#2A1830", "#F5EAF8", "#FF7AC6");
                    }
                    self.fronted = true;
                }
                // тема применяется через предпросмотр — даём время на перерисовку
                if waited > Duration::from_millis(1200) {
                    if let Some(w) = settings.as_ref() {
                        self.shot_window(w.window_title(), "17_settings_theme_custom.png", Stage::SettingsPlateGame);
                    } else {
                        self.go(Stage::SettingsClose);
                    }
                }
            }
            Stage::SettingsPlateGame => {
                if !self.fronted {
                    if let Some(w) = settings.as_mut() {
                        w.select_section_for_shots(2);
                        w.apply_game_preset_for_shots();
                    }
                    self.fronted = true;
                }
                if waited > Duration::from_millis(1200) {
                    if let Some(w) = settings.as_ref() {
                        self.shot_window(w.window_title(), "12b_settings_plate_game.png", Stage::SettingsClose);
                    } else {
                        self.go(Stage::SettingsClose);
                    }
                }
            }
            Stage::SettingsLocalAi => {
                if !self.fronted {
                    if let Some(w) = settings.as_mut() {
                        w.select_section_for_shots(1);
                        w.fill_local_ai_for_shots("http://localhost:11434", "qwen2.5:7b");
                    }
                    self.fronted = true;
                }
                if waited > Duration::from_millis(1200) {
                    if let Some(w) = settings.as_ref() {
                        self.shot_window(w.window_title(), "18_settings_local_ai.png", Stage::SettingsCustom);
                    } else {
                        self.go(Stage::SettingsClose);
                    }
                }
            }
            Stage::SettingsClose => match settings.as_mut() {
                Some(w) => {
                    w.cancel();
                    if waited > Duration::from_secs(5) {
                        self.finish(shared, "окно настроек не закрылось");
                    }
                }
                None => self.go(Stage::GlossaryOpen),
            },
            Stage::GlossaryOpen => {
                let id = lock_ctl(&shared.controller).session.as_ref().map(|s| s.profile.id.clone());
                match id {
                    Some(id) => {
                        windows.open_glossary(id);
                        self.go(Stage::GlossaryWait);
                    }
                    None => self.finish(shared, "нет игры для глоссария"),
                }
            }
            Stage::GlossaryWait => match glossary.as_mut() {
                Some(g) => {
                    if g.frames() > 4 {
                        g.fill_for_shots(&[("灯里", "Акари"), ("悠斗", "Юто"), ("小春", "Кохару")]);
                        self.go(Stage::GlossaryShot);
                    }
                }
                None => {
                    if waited > Duration::from_secs(5) {
                        self.finish(shared, "окно глоссария не открылось");
                    }
                }
            },
            Stage::GlossaryShot => {
                if waited > Duration::from_millis(900) {
                    if let Some(g) = glossary.as_ref() {
                        self.shot_window(g.window_title(), "19_glossary.png", Stage::GlossaryClose);
                    } else {
                        self.go(Stage::GlossaryClose);
                    }
                }
            }
            Stage::GlossaryClose => match glossary.as_mut() {
                Some(g) => {
                    g.cancel();
                    if waited > Duration::from_secs(5) {
                        self.finish(shared, "окно глоссария не закрылось");
                    }
                }
                None => self.go(Stage::Finish),
            },
            Stage::Finish => self.finish(shared, "готово"),
            Stage::Done => {}
        }
    }
}