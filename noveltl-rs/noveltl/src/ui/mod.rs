//! Интерфейс на egui: общее состояние, приложение `App` и заглушки для окон, которые будут позже.

pub mod avatar;
pub mod demo_game;
pub mod glossary;
pub mod main_window;
pub mod settings;
pub mod shots;
pub mod theme;
pub mod wizard;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use tray_icon::TrayIcon;

use noveltl_core::langdetect::{is_han, is_hangul, is_kana, Lang};

use crate::controller::Controller;
use crate::FontNeeds;
use crate::demo::{DemoSource, DEMO_TITLE};
use crate::tray;

pub const APP_NAME: &str = "NovelTL";
const TRAY_HINT: &str = "NovelTL продолжает работать в трее. Выход — через меню значка.";

/// Замок контроллера: короткое обращение; отравленный замок (паника в другом потоке) не мешает.
pub fn lock_ctl(c: &Arc<Mutex<Controller>>) -> MutexGuard<'_, Controller> {
    c.lock().unwrap_or_else(|e| e.into_inner())
}

/// Просьбы к интерфейсу из других потоков (например, из меню трея).
pub enum Request {
    OpenSettings,
}

/// Состояние, общее для интерфейса, трея и фонового потока-движка.
pub struct Shared {
    pub controller: Arc<Mutex<Controller>>,
    pub ctx: egui::Context,
    /// HWND главного окна (0 — ещё не найден)
    pub main_hwnd: AtomicIsize,
    /// главное окно показано (не спрятано в трей)
    pub visible: AtomicBool,
    /// пользователь выбрал «Выход»
    pub quit: AtomicBool,
    /// остановить поток-движок
    pub engine_stop: AtomicBool,
    requests: Mutex<Vec<Request>>,
    inbox: Mutex<Vec<String>>,
}

impl Shared {
    pub fn new(controller: Arc<Mutex<Controller>>, ctx: egui::Context) -> Shared {
        Shared {
            controller,
            ctx,
            main_hwnd: AtomicIsize::new(0),
            visible: AtomicBool::new(true),
            quit: AtomicBool::new(false),
            engine_stop: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
            inbox: Mutex::new(Vec::new()),
        }
    }

    pub fn push_request(&self, r: Request) {
        self.requests.lock().unwrap_or_else(|e| e.into_inner()).push(r);
        self.ctx.request_repaint();
    }

    fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut *self.requests.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Уведомления контроллера (кладёт поток-движок, забирает интерфейс).
    pub fn push_notices(&self, notices: Vec<String>) {
        let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        inbox.extend(notices);
        let extra = inbox.len().saturating_sub(50);
        inbox.drain(..extra);
    }

    fn take_notices(&self) -> Vec<String> {
        std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Поток-движок: каждые 30 мс вызывает `poll()` контроллера, пока не поднят флаг остановки.
pub fn start_engine(shared: Arc<Shared>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            while !shared.engine_stop.load(Ordering::SeqCst) {
                let notices = {
                    let mut c = lock_ctl(&shared.controller);
                    c.poll();
                    c.take_notices()
                };
                if !notices.is_empty() {
                    // окно спрятано или свёрнуто — показываем сообщение у значка в трее
                    let hwnd = shared.main_hwnd.load(Ordering::SeqCst);
                    if !shared.visible.load(Ordering::SeqCst) || (hwnd != 0 && crate::winutil::is_minimized(hwnd)) {
                        for n in &notices {
                            tray::balloon(APP_NAME, n);
                        }
                    }
                    shared.push_notices(notices);
                    shared.ctx.request_repaint();
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        })
        .expect("поток движка")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WizardMode {
    New,
    Hook,
    Position,
}

/// Какие окна просят открыть. Сами окна (мастер, настройки, глоссарий) будут сделаны позже и
/// станут читать эти флаги.
#[derive(Default)]
#[allow(dead_code)]
pub struct Windows {
    pub wizard: Option<WizardMode>,
    pub settings: bool,
    /// id профиля, для которого открыть глоссарий
    pub glossary: Option<String>,
}

impl Windows {
    pub fn open_wizard(&mut self, mode: WizardMode) {
        self.wizard = Some(mode);
    }

    pub fn open_settings(&mut self) {
        self.settings = true;
    }

    pub fn open_glossary(&mut self, profile_id: String) {
        self.glossary = Some(profile_id);
    }
}

pub struct Config {
    pub demo: bool,
    pub shots: Option<PathBuf>,
}

pub struct App {
    shared: Arc<Shared>,
    windows: Windows,
    main: main_window::MainWindow,
    game: Option<demo_game::DemoGame>,
    shots: Option<shots::Shots>,
    wizard: Option<wizard::Wizard>,
    settings: Option<settings::SettingsWindow>,
    glossary: Option<glossary::GlossaryWindow>,
    // значок держим живым до конца работы
    _tray: Option<TrayIcon>,
    engine: Option<JoinHandle<()>>,
    title: String,
    tray_hint_shown: bool,
    close_sent: bool,
    /// какие восточные шрифты уже загружены
    fonts_loaded: FontNeeds,
    /// текстура аватара: (имя файла, текстура или None, если нет/не загрузилась)
    avatar_tex: Option<(String, Option<egui::TextureHandle>)>,
}

impl App {
    pub fn new(
        shared: Arc<Shared>,
        engine: JoinHandle<()>,
        cfg: Config,
        demo_source: Option<DemoSource>,
        start_error: String,
    ) -> App {
        theme::install(&shared.ctx);
        let rgba = theme::icon_rgba(32);
        let tray = tray::create(shared.clone(), rgba, 32);
        let title = if cfg.demo { format!("{APP_NAME} — демо-режим") } else { APP_NAME.to_string() };
        let mut main = main_window::MainWindow::new();
        if !start_error.is_empty() {
            main.add_message(APP_NAME, &format!("{start_error}\n\nПроверьте, что папка files лежит рядом с программой."));
        }
        let game = demo_source.map(demo_game::DemoGame::new);
        let shots = cfg.shots.map(shots::Shots::new);
        App {
            shared,
            windows: Windows::default(),
            main,
            game,
            shots,
            wizard: None,
            settings: None,
            glossary: None,
            _tray: tray,
            engine: Some(engine),
            title,
            tray_hint_shown: false,
            close_sent: false,
            fonts_loaded: FontNeeds::default(),
            avatar_tex: None,
        }
    }

    fn find_windows(&mut self) {
        if self.shared.main_hwnd.load(Ordering::SeqCst) == 0 {
            let h = tray::find_own_window(&self.title);
            if h != 0 {
                self.shared.main_hwnd.store(h, Ordering::SeqCst);
            }
        }
        if let Some(game) = self.game.as_mut() {
            if game.hwnd == 0 {
                let h = tray::find_own_window(DEMO_TITLE);
                if h != 0 {
                    game.hwnd = h;
                    // окно «игры» должно попасть в список игр и стать целью плашки
                    lock_ctl(&self.shared.controller).set_demo_hwnd(h);
                }
            }
        }
    }

    /// Какие шрифты нужны сейчас: язык оригинала игры, язык перевода и сам показанный текст.
    fn font_needs(&self) -> FontNeeds {
        let c = lock_ctl(&self.shared.controller);
        let wizard_open = self.wizard.is_some() || self.windows.wizard.is_some();
        // японский — как раньше: демо, мастер, глоссарий или любая игра
        let mut n = FontNeeds {
            ja: self.game.is_some() || wizard_open || self.glossary.is_some() || c.session.is_some(),
            ..FontNeeds::default()
        };
        let mut langs = vec![c.settings.target_lang];
        if wizard_open {
            langs.push(c.settings.default_source_lang);
        }
        let mut texts: Vec<&str> = Vec::new();
        if let Some(s) = c.session.as_ref() {
            langs.push(s.profile.lang);
            if let Some(r) = s.last_result.as_ref() {
                texts.extend([r.src_name.as_str(), r.src_text.as_str(), r.name.as_str(), r.text.as_str()]);
            }
        }
        for l in langs {
            match l {
                Lang::Ja => n.ja = true,
                Lang::Ko => n.ko = true,
                Lang::Zh => n.zh = true,
                _ => {}
            }
        }
        let zh_game = c.session.as_ref().is_some_and(|s| s.profile.lang == Lang::Zh);
        for t in texts {
            for ch in t.chars() {
                if is_hangul(ch) {
                    n.ko = true;
                } else if is_kana(ch) {
                    n.ja = true;
                } else if is_han(ch) {
                    if zh_game {
                        n.zh = true;
                    } else {
                        n.ja = true;
                    }
                }
            }
        }
        n.zh_first = n.zh && zh_game;
        n
    }

    /// Выход: контроллер и поток-движок останавливаются (вызывается и при обычном закрытии окна).
    fn stop_engine(&mut self) {
        lock_ctl(&self.shared.controller).shutdown();
        self.shared.engine_stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.engine.take() {
            let _ = h.join();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.find_windows();

        // восточные шрифты — только когда на экране может появиться текст этого письма
        let want = self.fonts_loaded.union(self.font_needs());
        if want != self.fonts_loaded {
            crate::install_fonts(ctx, want);
            self.fonts_loaded = want;
        }

        let settings = lock_ctl(&self.shared.controller).settings.clone();
        theme::apply(ctx, &settings);

        // аватар перезагружаем только при смене имени файла
        if self.avatar_tex.as_ref().map(|(n, _)| n.as_str()) != Some(settings.avatar.as_str()) {
            let tex = avatar::load_rgba(&settings.avatar).map(|(w, h, px)| {
                let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                ctx.load_texture("avatar", img, egui::TextureOptions::LINEAR)
            });
            self.avatar_tex = Some((settings.avatar.clone(), tex));
        }

        for r in self.shared.take_requests() {
            match r {
                Request::OpenSettings => self.windows.open_settings(),
            }
        }
        if let Some(text) = self.shared.take_notices().pop() {
            self.main.show_notice(&text);
        }
        if let Some(mode) = self.windows.wizard.take() {
            match &self.wizard {
                Some(w) => ctx.send_viewport_cmd_to(w.viewport_id(), egui::ViewportCommand::Focus),
                None => {
                    // в режиме снимков окно мастера ставим справа, чтобы оно не закрывало «игру»
                    let pos = self.shots.as_ref().map(|_| {
                        let w = ctx.input(|i| i.viewport().monitor_size).map_or(1920.0, |s| s.x);
                        [(w - 590.0).max(0.0), 20.0]
                    });
                    self.wizard = Some(wizard::Wizard::new(mode, &self.shared, pos));
                }
            }
        }

        // в режиме снимков окна настроек и глоссария ставим справа, чтобы они не закрывали «игру»
        let side_pos = |w: f32| {
            self.shots.as_ref().map(|_| {
                let sw = ctx.input(|i| i.viewport().monitor_size).map_or(1920.0, |s| s.x);
                [(sw - w).max(0.0), 10.0]
            })
        };
        if std::mem::take(&mut self.windows.settings) {
            match &self.settings {
                Some(w) => ctx.send_viewport_cmd_to(w.viewport_id(), egui::ViewportCommand::Focus),
                None => self.settings = Some(settings::SettingsWindow::new(&self.shared, side_pos(800.0))),
            }
        }
        if let Some(id) = self.windows.glossary.take() {
            match &self.glossary {
                Some(w) if w.profile_id() == id => {
                    ctx.send_viewport_cmd_to(w.viewport_id(), egui::ViewportCommand::Focus)
                }
                _ => {
                    if let Some(w) = glossary::GlossaryWindow::new(&self.shared, &id, side_pos(560.0)) {
                        self.glossary = Some(w);
                    }
                }
            }
        }

        // крестик — в трей, а не выход
        if ctx.input(|i| i.viewport().close_requested()) && !self.shared.quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.shared.visible.store(false, Ordering::SeqCst);
            if !self.tray_hint_shown {
                self.tray_hint_shown = true;
                tray::balloon(APP_NAME, TRAY_HINT);
            }
        }
        if self.shared.quit.load(Ordering::SeqCst) && !self.close_sent {
            self.close_sent = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let avatar_tex = self.avatar_tex.as_ref().and_then(|(_, t)| t.as_ref());
        self.main.show(ctx, &self.shared, &mut self.windows, avatar_tex);

        if let Some(game) = self.game.as_mut() {
            game.show(ctx);
        }
        if let Some(w) = self.wizard.as_mut() {
            let keep = w.show(ctx, &self.shared);
            if !keep {
                w.close(&self.shared);
                self.wizard = None;
            }
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if let Some(w) = self.settings.as_mut() {
            let keep = w.show(ctx, &self.shared);
            if !keep {
                w.close(&self.shared);
                self.settings = None;
            }
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if let Some(w) = self.glossary.as_mut() {
            let keep = w.show(ctx, &self.shared);
            if !keep {
                self.glossary = None;
            }
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if let (Some(shots), Some(game)) = (self.shots.as_mut(), self.game.as_mut()) {
            shots.step(
                ctx,
                &self.shared,
                game,
                &mut self.wizard,
                &mut self.windows,
                &mut self.settings,
                &mut self.glossary,
            );
        }

        ctx.request_repaint_after(Duration::from_millis(300));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_engine();
        tray::remove_icon();
    }
}
