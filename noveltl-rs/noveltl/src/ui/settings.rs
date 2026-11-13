//! Окно «Настройки — NovelTL» (перенос `SettingsDialog` и `PlatePreview` из `noveltl/ui/dialogs.py`).
//! Отдельное окно (immediate viewport). Тема и вид плашки применяются сразу (предпросмотр),
//! «Отмена» и крестик откатывают их; остальное сохраняется только по «Сохранить».

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, CursorIcon, FontId, Frame, Layout, Margin, Painter, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId,
};
use noveltl_core::i18n::{t, tf, UiLang};
use noveltl_core::langdetect::Lang;
use noveltl_core::net::{self, NET_DIRECT, NET_PROXY, NET_SYSTEM};
use noveltl_core::profiles::AppSettings;
use noveltl_core::translators::ENGINE_MODES;

use super::theme::{self, button, pal, BtnKind};
use super::{avatar, lock_ctl, Shared};
use crate::hotkeys::parse_hotkey;
use crate::overlay::parse_color;

/// Заголовок окна настроек.
pub fn settings_title() -> &'static str {
    t("Настройки — NovelTL")
}

/// Разделы окна (порядок = порядок в меню слева): значок и название.
fn sections() -> [(theme::Icon, &'static str); 7] {
    use theme::Icon;
    [
        (Icon::Palette, t("Оформление")),
        (Icon::Translate, t("Перевод")),
        (Icon::Overlay, t("Плашка")),
        (Icon::Network, t("Сеть")),
        (Icon::Keyboard, t("Клавиши")),
        (Icon::Book, t("Японский")),
        (Icon::Settings, t("Общие")),
    ]
}

const PROJECT_URL: &str = "https://github.com/Kawaakar1/NovelTL";

/// Названия цветов плашки (они же подсказки и заголовки строк).
fn colors() -> [&'static str; 4] {
    [t("Цвет текста"), t("Цвет имени"), t("Цвет контура"), t("Цвет фона")]
}

/// Цвета своей темы — слоты 4..8 (после цветов плашки): (название, подсказка).
fn custom_colors() -> [(&'static str, &'static str); 4] {
    [
        (t("Фон"), t("Цвет фона окон")),
        (t("Карточки"), t("Цвет панелей и карточек")),
        (t("Текст"), t("Цвет текста")),
        (t("Акцент"), t("Акцентный цвет")),
    ]
}

/// Подсказка для окна выбора цвета по номеру слота.
fn slot_hint(i: usize) -> &'static str {
    if i < 4 {
        colors()[i]
    } else {
        custom_colors()[(i - 4).min(3)].1
    }
}

/// Семейства шрифтов: имя -> файлы в `C:\Windows\Fonts` (в список попадают те, у которых файл есть).
const FONT_FILES: &[(&str, &[&str])] = &[
    ("Segoe UI", &["segoeui.ttf"]),
    ("Arial", &["arial.ttf"]),
    ("Calibri", &["calibri.ttf"]),
    ("Cambria", &["cambria.ttc"]),
    ("Candara", &["candara.ttf"]),
    ("Consolas", &["consola.ttf"]),
    ("Constantia", &["constan.ttf"]),
    ("Corbel", &["corbel.ttf"]),
    ("Georgia", &["georgia.ttf"]),
    ("Meiryo", &["meiryo.ttc"]),
    ("MS Gothic", &["msgothic.ttc"]),
    ("Tahoma", &["tahoma.ttf"]),
    ("Times New Roman", &["times.ttf"]),
    ("Trebuchet MS", &["trebuc.ttf"]),
    ("Verdana", &["verdana.ttf"]),
    ("Yu Gothic", &["YuGothR.ttc", "YuGothM.ttc", "yugothic.ttf"]),
];

fn installed_fonts() -> Vec<String> {
    let dir = std::path::PathBuf::from(r"C:\Windows\Fonts");
    FONT_FILES
        .iter()
        .filter(|(_, files)| files.iter().any(|f| dir.join(f).is_file()))
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Высота рабочей области экрана в пикселях (в Linux — обычная высота экрана ноутбука).
#[cfg(not(windows))]
fn work_area_height_px() -> f32 {
    1000.0
}

/// Высота рабочей области экрана в пикселях.
#[cfg(windows)]
fn work_area_height_px() -> f32 {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut r = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut std::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() && r.bottom > r.top {
        (r.bottom - r.top) as f32
    } else {
        1040.0
    }
}

/// То, что в настройках видно сразу (предпросмотр).
#[derive(Clone, PartialEq)]
struct PreviewKey {
    theme: String,
    font_family: String,
    font_size: u32,
    text_color: String,
    name_color: String,
    outline_color: String,
    bg_color: String,
    bg_opacity: u32,
    custom_bg: String,
    custom_card: String,
    custom_text: String,
    custom_accent: String,
    accent: String,
}

impl PreviewKey {
    fn of(s: &AppSettings) -> PreviewKey {
        PreviewKey {
            custom_bg: s.custom_bg.clone(),
            custom_card: s.custom_card.clone(),
            custom_text: s.custom_text.clone(),
            custom_accent: s.custom_accent.clone(),
            accent: s.accent.clone(),
            theme: s.theme.clone(),
            font_family: s.font_family.clone(),
            font_size: s.font_size,
            text_color: s.text_color.clone(),
            name_color: s.name_color.clone(),
            outline_color: s.outline_color.clone(),
            bg_color: s.bg_color.clone(),
            bg_opacity: s.bg_opacity,
        }
    }

    fn put(&self, s: &mut AppSettings) {
        s.custom_bg = self.custom_bg.clone();
        s.custom_card = self.custom_card.clone();
        s.custom_text = self.custom_text.clone();
        s.custom_accent = self.custom_accent.clone();
        s.accent = self.accent.clone();
        s.theme = self.theme.clone();
        s.font_family = self.font_family.clone();
        s.font_size = self.font_size;
        s.text_color = self.text_color.clone();
        s.name_color = self.name_color.clone();
        s.outline_color = self.outline_color.clone();
        s.bg_color = self.bg_color.clone();
        s.bg_opacity = self.bg_opacity;
    }
}

/// Переносит правки из `d` и сочетания клавиш `hk` в копию `base`.
fn draft_to_settings(d: &AppSettings, hk: &[String; 3], base: &AppSettings) -> AppSettings {
    let mut new = base.clone();
    PreviewKey::of(d).put(&mut new);
    new.nickname = d.nickname.trim().to_string();
    new.avatar = d.avatar.clone();
    new.deepl_key = d.deepl_key.trim().to_string();
    new.ms_key = d.ms_key.trim().to_string();
    new.ms_region = d.ms_region.trim().to_string();
    new.local_ai_url = d.local_ai_url.trim().trim_end_matches('/').to_string();
    new.local_ai_model = d.local_ai_model.trim().to_string();
    new.hotkey_toggle = hk[0].trim().to_string();
    new.hotkey_retranslate = hk[1].trim().to_string();
    new.hotkey_settings = hk[2].trim().to_string();
    new.engine_mode = d.engine_mode.clone();
    new.second_engine = d.second_engine.clone();
    new.learn_mode = d.learn_mode;
    new.target_lang = d.target_lang;
    new.default_source_lang = d.default_source_lang;
    new.ui_lang = d.ui_lang;
    new.overlay_show_original = d.overlay_show_original;
    new.overlay_toolbar_dim = d.overlay_toolbar_dim;
    new
}

enum Act {
    Save,
    Cancel,
    Check,
    DownloadDict,
}

pub struct SettingsWindow {
    /// состояние словаря: (есть, идёт скачивание, текст) — обновляется перед отрисовкой
    dict_info: (bool, bool, String),
    id: ViewportId,
    icon: Arc<egui::IconData>,
    pos: Option<[f32; 2]>,
    /// настройки, какими они были при открытии окна
    orig: AppSettings,
    /// то, что правится в окне
    d: AppSettings,
    /// что уже отправлено в плашку и тему
    applied: PreviewKey,
    status: String,
    /// выбранный раздел (индекс в SECTIONS)
    section: usize,
    /// автозапуск с Windows (состояние в окне)
    autostart: bool,
    key_show: bool,
    net_mode: String,
    proxy_addr: String,
    proxy_user: String,
    proxy_pass: String,
    pass_show: bool,
    bind_ip: String,
    ifaces: Vec<(String, String)>,
    fonts: Vec<String>,
    hk: [String; 3],
    capturing: Option<usize>,
    picking: Option<(usize, Color32)>,
    /// текст в поле HEX окна выбора цвета
    hex_buf: String,
    /// текстура превью аватара: (имя файла, текстура)
    avatar_tex: Option<(String, TextureHandle)>,
    check_slot: Arc<Mutex<Option<(bool, String)>>>,
    checking: bool,
    check_result: Option<(bool, String)>,
    /// список моделей локального ИИ: результат фонового запроса
    models_slot: Arc<Mutex<Option<Result<Vec<String>, String>>>>,
    models_loading: bool,
    models: Vec<String>,
    models_error: String,
    /// показать ключ Microsoft
    ms_key_show: bool,
    messages: Vec<(String, String)>,
    close_after_message: bool,
    saved: bool,
    closed: bool,
    frames: u32,
    limit: f32,
    content_h: f32,
    sent_h: f32,
    bg_tex: Option<TextureHandle>,
}

impl SettingsWindow {
    pub fn new(shared: &Shared, pos: Option<[f32; 2]>) -> SettingsWindow {
        let (orig, status) = {
            let c = lock_ctl(&shared.controller);
            (c.settings.clone(), c.translator_status())
        };
        let mut proxy_addr = orig.proxy_url.clone();
        let mut proxy_user = orig.proxy_user.clone();
        let mut proxy_pass = orig.proxy_pass.clone();
        if proxy_user.is_empty() {
            let (addr, u, p) = net::split_proxy_credentials(&proxy_addr);
            proxy_addr = addr;
            if !u.is_empty() {
                proxy_user = u;
                proxy_pass = p;
            }
        }
        let ifaces: Vec<(String, String)> = crate::winutil::list_interfaces()
            .into_iter()
            .map(|(name, ip)| (format!("{name} — {ip}"), ip))
            .collect();
        let mut ifaces_all = ifaces;
        if !orig.bind_ip.is_empty() && !ifaces_all.iter().any(|(_, ip)| *ip == orig.bind_ip) {
            ifaces_all.push((tf("{} (сейчас недоступен)", &[&orig.bind_ip]), orig.bind_ip.clone()));
        }
        let mut fonts = installed_fonts();
        if !fonts.iter().any(|f| *f == orig.font_family) {
            fonts.insert(0, orig.font_family.clone());
        }
        let ppp = shared.ctx.pixels_per_point().max(0.5);
        let limit = (work_area_height_px() / ppp - 60.0).max(420.0);
        SettingsWindow {
            dict_info: (false, false, String::new()),
            id: ViewportId::from_hash_of("noveltl_settings"),
            icon: Arc::new(theme::icon_data(64)),
            pos,
            applied: PreviewKey::of(&orig),
            d: orig.clone(),
            hk: [orig.hotkey_toggle.clone(), orig.hotkey_retranslate.clone(), orig.hotkey_settings.clone()],
            net_mode: orig.net_mode.clone(),
            bind_ip: orig.bind_ip.clone(),
            orig,
            status,
            section: 0,
            autostart: crate::winutil::autostart_enabled(),
            key_show: false,
            proxy_addr,
            proxy_user,
            proxy_pass,
            pass_show: false,
            ifaces: ifaces_all,
            fonts,
            capturing: None,
            picking: None,
            hex_buf: String::new(),
            avatar_tex: None,
            check_slot: Arc::new(Mutex::new(None)),
            checking: false,
            check_result: None,
            models_slot: Arc::new(Mutex::new(None)),
            models_loading: false,
            models: Vec::new(),
            models_error: String::new(),
            ms_key_show: false,
            messages: Vec::new(),
            close_after_message: false,
            saved: false,
            closed: false,
            frames: 0,
            limit,
            content_h: 0.0,
            sent_h: 0.0,
            bg_tex: None,
        }
    }

    pub fn viewport_id(&self) -> ViewportId {
        self.id
    }

    /// Заголовок окна (по нему окно находят для снимка с экрана).
    pub fn window_title(&self) -> &'static str {
        settings_title()
    }

    /// Сколько раз окно уже отрисовано.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Закрыть как «Отмена» (для `--shots`).
    pub fn cancel(&mut self) {
        self.closed = true;
    }

    /// Для `--shots`: подставить сетевые поля (только в окне, без сохранения).
    pub fn fill_net_for_shots(&mut self, mode: &str, addr: &str, user: &str, pass: &str) {
        self.net_mode = mode.to_string();
        self.proxy_addr = addr.to_string();
        self.proxy_user = user.to_string();
        self.proxy_pass = pass.to_string();
    }

    /// Для `--shots`: выбрать раздел по номеру.
    pub fn select_section_for_shots(&mut self, i: usize) {
        self.section = i.min(sections().len() - 1);
        // высота окна подбирается заново под этот раздел
        self.frames = 0;
    }

    /// Для `--shots`: подставить ник-заглушку (аватар — демо-заглушка из профиля снимков).
    pub fn fill_profile_for_shots(&mut self, nick: &str) {
        self.d.nickname = nick.to_string();
    }

    /// Для `--shots`: выбрать «Свою» тему с заданными цветами (только в окне).
    pub fn fill_custom_theme_for_shots(&mut self, bg: &str, card: &str, text: &str, accent: &str) {
        self.d.theme = "custom".to_string();
        self.d.custom_bg = bg.to_string();
        self.d.custom_card = card.to_string();
        self.d.custom_text = text.to_string();
        self.d.custom_accent = accent.to_string();
        self.d.accent = accent.to_string();
    }

    /// Для `--shots`: выбрать «Локальный ИИ» с адресом и моделью-заглушкой (сеть не трогаем).
    pub fn fill_local_ai_for_shots(&mut self, url: &str, model: &str) {
        self.d.engine_mode = "local_ai".to_string();
        self.d.local_ai_url = url.to_string();
        self.d.local_ai_model = model.to_string();
        self.models = vec![model.to_string()];
    }

    /// Рисует окно; false — окно нужно закрыть (после этого вызвать `close`).
    pub fn show(&mut self, ctx: &egui::Context, shared: &Shared) -> bool {
        self.frames += 1;
        if self.models_loading {
            let got = self.models_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            match got {
                Some(Ok(list)) => {
                    self.models = list;
                    self.models_error.clear();
                    self.models_loading = false;
                }
                Some(Err(e)) => {
                    self.models_error = e;
                    self.models_loading = false;
                }
                None => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if self.checking {
            let got = self.check_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            match got {
                Some(r) => {
                    self.check_result = Some(r);
                    self.checking = false;
                }
                None => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        let h0 = 900.0_f32.min(self.limit);
        let mut builder = ViewportBuilder::default()
            .with_title(settings_title())
            .with_inner_size([880.0, h0])
            .with_min_inner_size([660.0, 420.0])
            .with_active(!crate::winutil::test_area().is_some())
            .with_always_on_top()
            .with_icon(self.icon.clone());
        if let Some(pos) = self.pos {
            builder = builder.with_position(pos);
        }
        if self.sent_h == 0.0 {
            self.sent_h = h0;
        }
        {
            let c = lock_ctl(&shared.controller);
            self.dict_info = (c.dict.is_some(), c.dict_busy, c.dict_status.clone());
        }
        let mut acts: Vec<Act> = Vec::new();
        let id = self.id;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            self.draw(ctx, &mut acts);
        });
        for a in acts {
            match a {
                Act::Save => self.save(shared),
                Act::Cancel => self.closed = true,
                Act::Check => self.start_check(ctx),
                Act::DownloadDict => lock_ctl(&shared.controller).download_dictionary(),
            }
        }
        self.push_preview(ctx, shared);
        !self.closed
    }

    /// Окно закрыто: без сохранения тема и вид плашки откатываются.
    pub fn close(&mut self, shared: &Shared) {
        if self.saved {
            return;
        }
        // выбранные, но не сохранённые картинки удаляем
        avatar::cleanup(&self.orig.avatar);
        let key = PreviewKey::of(&self.orig);
        {
            let mut c = lock_ctl(&shared.controller);
            key.put(&mut c.settings);
            c.apply_overlay_style();
        }
        shared.ctx.request_repaint();
    }

    /// Тема и вид плашки из окна — сразу в приложение и на плашку (на копии настроек окна, не сохраняя).
    fn push_preview(&mut self, ctx: &egui::Context, shared: &Shared) {
        let key = PreviewKey::of(&self.d);
        if key == self.applied {
            return;
        }
        {
            let mut c = lock_ctl(&shared.controller);
            key.put(&mut c.settings);
            c.apply_overlay_style();
        }
        let mut tmp = self.orig.clone();
        key.put(&mut tmp);
        theme::apply(ctx, &tmp);
        ctx.request_repaint();
        self.applied = key;
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.messages.push(("NovelTL".to_string(), text.into()));
    }

    fn start_check(&mut self, ctx: &egui::Context) {
        let http = match net::make_http(&self.net_mode, &self.proxy_addr, &self.bind_ip, &self.proxy_user, &self.proxy_pass)
        {
            Ok(h) => h,
            Err(e) => {
                self.check_result = Some((false, e));
                return;
            }
        };
        self.checking = true;
        self.check_result = None;
        let slot = self.check_slot.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = net::check_google(&http);
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            ctx.request_repaint();
        });
    }

    /// Запрашивает список моделей локального сервера в фоне.
    fn start_models(&mut self, ctx: &egui::Context) {
        self.models_loading = true;
        self.models_error.clear();
        let url = self.d.local_ai_url.trim().to_string();
        let slot = self.models_slot.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let http = net::make_local_http();
            let r = noveltl_core::translators::list_local_models(http.as_ref(), &url);
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            ctx.request_repaint();
        });
    }

    /// Черновик окна (без сети и прокси) поверх `base`.
    fn apply_draft(&self, base: &AppSettings) -> AppSettings {
        draft_to_settings(&self.d, &self.hk, base)
    }

    fn save(&mut self, shared: &Shared) {
        let combos = self.hk.clone().map(|c| c.trim().to_string());
        for combo in &combos {
            if !combo.is_empty() && parse_hotkey(combo).is_none() {
                self.warn(tf("Сочетание «{}» не подходит. Нужна клавиша с Ctrl, Alt или Shift.", &[combo]));
                return;
            }
        }
        let dup = (0..combos.len()).any(|i| !combos[i].is_empty() && combos[i + 1..].contains(&combos[i]));
        if dup {
            self.warn(t("Для двух действий выбрано одно и то же сочетание."));
            return;
        }
        if self.d.engine_mode == "deepl" && self.d.deepl_key.trim().is_empty() {
            self.warn(t("Для режима «Только DeepL» нужен ключ."));
            return;
        }
        if self.d.engine_mode == "microsoft" && self.d.ms_key.trim().is_empty() {
            self.warn(t("Для Microsoft Translator нужен ключ."));
            return;
        }
        if self.d.engine_mode == "local_ai" {
            if self.d.local_ai_model.trim().is_empty() {
                self.warn(t("Выберите или впишите модель локального ИИ."));
                return;
            }
            let u = self.d.local_ai_url.trim();
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                self.warn(t("Адрес сервера локального ИИ должен начинаться с http:// или https://."));
                return;
            }
        }
        let mode = self.net_mode.clone();
        let mut proxy_text = self.proxy_addr.trim().to_string();
        let mut user = self.proxy_user.clone();
        let mut pass = self.proxy_pass.clone();
        if mode == NET_PROXY {
            if let Err(e) = net::build_proxy_url(&proxy_text, &user, &pass) {
                self.warn(e);
                return;
            }
            match net::normalize_proxy(&proxy_text) {
                Ok(n) => proxy_text = n,
                Err(e) => {
                    self.warn(e);
                    return;
                }
            }
            let (clean, u, p) = net::split_proxy_credentials(&proxy_text);
            proxy_text = clean;
            if !u.is_empty() && user.trim().is_empty() {
                user = u;
                pass = p;
            }
        }
        let mut new = self.apply_draft(&self.orig);
        new.net_mode = mode;
        new.proxy_url = proxy_text;
        new.proxy_user = user;
        new.proxy_pass = pass;
        new.bind_ip = self.bind_ip.clone();
        self.saved = true;
        avatar::cleanup(&new.avatar);
        let errors = {
            let mut c = lock_ctl(&shared.controller);
            c.save_settings(new);
            c.hotkey_errors.clone()
        };
        // автозапуск: ошибка не мешает остальным настройкам
        let auto_err = crate::winutil::set_autostart(self.autostart).err();
        if let Some(e) = &auto_err {
            self.warn(tf("Не удалось изменить автозапуск: {}\nОстальные настройки сохранены.", &[e]));
        }
        if errors.is_empty() {
            if auto_err.is_some() {
                self.close_after_message = true;
            } else {
                self.closed = true;
            }
        } else {
            self.warn(tf(
                "Не удалось занять сочетание: {}.\nСкорее всего, его уже использует другая программа — выберите другое.\nОстальные настройки сохранены.",
                &[&errors.join(", ")],
            ));
            // окно остаётся открытым — можно сразу выбрать другое сочетание
        }
    }

    // ---------- рисование ----------

    fn draw(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if ctx.input(|i| i.viewport().close_requested()) {
            acts.push(Act::Cancel);
        }
        self.handle_capture(ctx);
        self.load_avatar_tex(ctx);
        let p = pal();
        let foot = egui::TopBottomPanel::bottom("set_foot")
            .show_separator_line(true)
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 24, right: 24, top: 10, bottom: 18 }))
            .show(ctx, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, t("Сохранить"), BtnKind::Primary, true, None).clicked() {
                        acts.push(Act::Save);
                    }
                    if button(ui, t("Отмена"), BtnKind::Normal, true, None).clicked() {
                        acts.push(Act::Cancel);
                    }
                });
            });
        // меню разделов слева: профиль и пункты
        egui::SidePanel::left("set_nav")
            .exact_width(200.0)
            .resizable(false)
            .show_separator_line(true)
            .frame(Frame::new().fill(p.panel).inner_margin(Margin { left: 12, right: 12, top: 18, bottom: 8 }))
            .show(ctx, |ui| {
                self.draw_profile_head(ui);
                ui.add_space(10.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                for (i, (icon, title)) in sections().iter().enumerate() {
                    if nav_item(ui, *icon, title, self.section == i).clicked() {
                        self.section = i;
                    }
                }
            });
        let central = egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ctx, |ui| {
            // полоса прокрутки всегда на виду (а не плавающая)
            ui.style_mut().spacing.scroll.floating = false;
            egui::ScrollArea::vertical().id_salt("settings_scroll").auto_shrink([false, false]).show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: 24, right: 16, top: 20, bottom: 12 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.draw_section(ui, acts);
                });
            })
        });
        self.content_h = central.inner.content_size.y;
        // высота окна — по содержимому, но не больше рабочей области экрана минус 60 px
        let want = (self.content_h + foot.response.rect.height()).clamp(420.0_f32.min(self.limit), self.limit);
        if self.frames <= 12 && self.content_h > 0.0 && (want - self.sent_h).abs() > 1.5 {
            let w = ctx.input(|i| i.viewport().inner_rect).map_or(880.0, |r| r.width());
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(w, want)));
            self.sent_h = want;
        }
        self.color_window(ctx);
        self.modal(ctx);
    }

    fn modal(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = self.messages.first().cloned() else { return };
        let mut ok = false;
        let m = egui::Modal::new(egui::Id::new("settings_message")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(theme::h2(title));
            ui.add_space(4.0);
            wrap(ui, theme::body(text));
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, "OK", BtnKind::Primary, true, Some(80.0)).clicked() {
                    ok = true;
                }
            });
        });
        if ok || m.should_close() {
            self.messages.remove(0);
            if self.messages.is_empty() && self.close_after_message {
                self.closed = true;
            }
        }
    }

    /// Ждёт сочетание клавиш для поля, по которому щёлкнули.
    fn handle_capture(&mut self, ctx: &egui::Context) {
        let Some(i) = self.capturing else { return };
        let events = ctx.input(|inp| inp.events.clone());
        for ev in events {
            let egui::Event::Key { key, pressed: true, modifiers, repeat: false, .. } = ev else { continue };
            if key == egui::Key::Escape {
                self.capturing = None;
                return;
            }
            let Some(name) = key_name(key) else { continue };
            let is_f = name.len() > 1 && name.starts_with('F') && name[1..].chars().all(|c| c.is_ascii_digit());
            let ctrl = modifiers.ctrl || modifiers.command;
            if !(ctrl || modifiers.alt || modifiers.shift || is_f) {
                continue; // одиночная клавиша глобальной не бывает
            }
            let mut parts: Vec<&str> = Vec::new();
            if ctrl {
                parts.push("Ctrl");
            }
            if modifiers.alt {
                parts.push("Alt");
            }
            if modifiers.shift {
                parts.push("Shift");
            }
            parts.push(&name);
            self.hk[i] = parts.join("+");
            self.capturing = None;
            return;
        }
    }

    fn color_slot(&mut self, i: usize) -> &mut String {
        match i {
            0 => &mut self.d.text_color,
            1 => &mut self.d.name_color,
            2 => &mut self.d.outline_color,
            3 => &mut self.d.bg_color,
            4 => &mut self.d.custom_bg,
            5 => &mut self.d.custom_card,
            6 => &mut self.d.custom_text,
            _ => &mut self.d.accent,
        }
    }

    fn color_value(&self, i: usize) -> Color32 {
        if i >= 4 {
            let s = match i {
                4 => &self.d.custom_bg,
                5 => &self.d.custom_card,
                6 => &self.d.custom_text,
                _ => &self.d.accent,
            };
            let fb = theme::parse_hex(&self.d.custom_accent, Color32::GRAY);
            return theme::parse_hex(s, fb);
        }
        let (s, fb) = match i {
            0 => (&self.d.text_color, 0xFFFFFF),
            1 => (&self.d.name_color, 0xFFD27F),
            2 => (&self.d.outline_color, 0x000000),
            _ => (&self.d.bg_color, 0x0E0F1A),
        };
        to_color(s, fb)
    }

    /// Выбор цвета: встроенный `color_picker` egui во всплывающем окне.
    fn color_window(&mut self, ctx: &egui::Context) {
        let Some((i, mut c)) = self.picking else { return };
        let mut open = true;
        let mut done = false;
        let mut changed = false;
        egui::Window::new(t("Выберите цвет"))
            .id(egui::Id::new("settings_color_window"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(theme::body(slot_hint(i)));
                changed = egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque);
                if changed {
                    self.hex_buf = theme::to_hex(c);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(theme::body("HEX"));
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.hex_buf)
                            .char_limit(7)
                            .margin(Margin::symmetric(8, 6))
                            .desired_width(100.0),
                    );
                    if r.changed() {
                        let t = self.hex_buf.trim().trim_start_matches('#');
                        if t.len() == 6 && t.chars().all(|ch| ch.is_ascii_hexdigit()) {
                            c = theme::parse_hex(&format!("#{t}"), c);
                            changed = true;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, t("Готово"), BtnKind::Primary, true, Some(90.0)).clicked() {
                        done = true;
                    }
                });
            });
        if changed {
            *self.color_slot(i) = theme::to_hex(c);
            self.picking = Some((i, c));
        }
        if done || !open {
            self.picking = None;
        }
    }

    /// Содержимое выбранного раздела.
    fn draw_section(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(theme::h1(sections()[self.section].1));
        match self.section {
            0 => self.draw_look(ui),
            1 => self.draw_translate(ui),
            2 => self.draw_plate(ui),
            3 => self.draw_network(ui, acts),
            4 => self.draw_hotkeys(ui),
            5 => self.draw_japanese(ui, acts),
            _ => self.draw_general(ui),
        }
    }

    /// Открыть окно выбора цвета для слота.
    fn open_picker(&mut self, i: usize) {
        let c = self.color_value(i);
        self.hex_buf = theme::to_hex(c);
        self.picking = Some((i, c));
    }

    /// Текстура превью аватара (перезагружается при смене файла).
    fn load_avatar_tex(&mut self, ctx: &egui::Context) {
        let name = self.d.avatar.clone();
        if name.is_empty() {
            self.avatar_tex = None;
        } else if self.avatar_tex.as_ref().map(|(n, _)| n.as_str()) != Some(name.as_str()) {
            self.avatar_tex = avatar::load_rgba(&name).map(|(w, h, rgba)| {
                let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                (name.clone(), ctx.load_texture(format!("settings_avatar_{name}"), img, TextureOptions::LINEAR))
            });
        }
    }

    /// Шапка боковой панели: круглый аватар, ник и подпись «Профиль».
    fn draw_profile_head(&self, ui: &mut Ui) {
        let p = pal();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let tex = self.avatar_tex.as_ref().map(|(_, t)| t);
        paint_avatar(&painter, Pos2::new(rect.left() + 18.0, rect.center().y), 18.0, tex, &self.d.nickname);
        let nick = self.d.nickname.trim();
        let nick = if nick.is_empty() { "NovelTL" } else { nick };
        let x = rect.left() + 46.0;
        painter.text(Pos2::new(x, rect.center().y - 8.0), Align2::LEFT_CENTER, nick, FontId::new(15.0, theme::bold_family()), p.text);
        painter.text(Pos2::new(x, rect.center().y + 10.0), Align2::LEFT_CENTER, t("Профиль"), FontId::proportional(12.0), p.dim);
    }

    /// Строка с образцом цвета слота `i`: щелчок открывает выбор цвета.
    fn color_row(&mut self, ui: &mut Ui, title: &str, desc: Option<&str>, i: usize) {
        theme::row_w(ui, title, desc, 140.0, |ui| {
            let c = self.color_value(i);
            if swatch(ui, c).on_hover_text(slot_hint(i)).clicked() {
                self.open_picker(i);
            }
            ui.label(theme::dim(theme::to_hex(c)));
        });
    }

    fn draw_look(&mut self, ui: &mut Ui) {
        theme::section_label(ui, t("Профиль"));
        theme::card(ui, |ui| {
            theme::row_w(ui, t("Ник"), Some(t("Как вас называть — показывается в главном окне")), 240.0, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.d.nickname)
                        .char_limit(32)
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(ui.available_width()),
                );
            });
            theme::row_w(ui, t("Аватар"), None, 280.0, |ui| {
                if button(ui, t("Выбрать картинку…"), BtnKind::Normal, true, None).clicked() {
                    if let Some(path) = avatar::pick_image() {
                        match avatar::store_avatar(&path) {
                            Ok(n) => self.d.avatar = n,
                            Err(e) => self.warn(tf("Не удалось загрузить картинку: {}", &[&e])),
                        }
                    }
                }
                if !self.d.avatar.is_empty() && button(ui, t("Убрать"), BtnKind::Normal, true, None).clicked() {
                    self.d.avatar = String::new();
                }
            });
        });
        theme::section_label(ui, t("Тема"));
        theme::card(ui, |ui| {
            let labels: Vec<&str> = theme::MODES.iter().map(|(_, n)| t(n)).collect();
            let cur = theme::MODES.iter().position(|(id, _)| *id == self.d.theme).unwrap_or(0);
            let w = theme::segmented_width(ui, &labels);
            theme::row_w(ui, t("Режим"), Some(t("Светлая или тёмная тема окон")), w, |ui| {
                if let Some(i) = theme::segmented(ui, &labels, cur) {
                    self.d.theme = theme::MODES[i].0.to_string();
                }
            });
            theme::row_w(ui, t("Акцентный цвет"), Some(t("Цвет кнопок и выделения")), 200.0, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut chosen = None;
                for (hex, name) in theme::ACCENTS {
                    let sel = self.d.accent.eq_ignore_ascii_case(hex);
                    if accent_dot(ui, Some(hex), sel).on_hover_text(t(name)).clicked() {
                        chosen = Some(hex.to_string());
                    }
                }
                let own = !self.d.accent.is_empty() && !theme::ACCENTS.iter().any(|(h, _)| self.d.accent.eq_ignore_ascii_case(h));
                if accent_dot(ui, None, own).on_hover_text(t("Свой цвет")).clicked() {
                    self.open_picker(7);
                }
                if let Some(h) = chosen {
                    self.d.accent = h;
                }
            });
            if self.d.theme == "custom" {
                let cc = custom_colors();
                for k in 0..3 {
                    self.color_row(ui, cc[k].0, Some(cc[k].1), 4 + k);
                }
                theme::row(ui, t("Сбросить цвета"), Some(t("Вернуть цвета своей темы по умолчанию")), |ui| {
                    if button(ui, t("Сбросить"), BtnKind::Normal, true, None).clicked() {
                        let def = AppSettings::default();
                        self.d.custom_bg = def.custom_bg;
                        self.d.custom_card = def.custom_card;
                        self.d.custom_text = def.custom_text;
                        self.d.custom_accent = def.custom_accent;
                        self.d.accent = def.accent;
                    }
                });
            }
        });
        theme::section_label(ui, t("Интерфейс"));
        theme::card(ui, |ui| {
            // подсказка на английском — найти строку можно, даже если интерфейс на незнакомом языке
            let saved = t("Язык меняется после нажатия «Сохранить».");
            let desc = if t("Язык интерфейса") == "Interface language" {
                saved.to_string()
            } else {
                format!("Interface language. {saved}")
            };
            theme::row(ui, t("Язык интерфейса"), Some(&desc), |ui| {
                let current = self.d.ui_lang.unwrap_or_else(noveltl_core::i18n::lang);
                egui::ComboBox::from_id_salt("set_ui_lang")
                    .width(ui.available_width().max(80.0))
                    .selected_text(theme::body(current.native_title()))
                    .show_ui(ui, |ui| {
                        for l in UiLang::ALL {
                            if ui.selectable_label(current == l, l.native_title()).clicked() {
                                self.d.ui_lang = Some(l);
                            }
                        }
                    });
            });
            theme::row_w(ui, t("Запускать вместе с Windows"), None, 36.0, |ui| {
                theme::toggle(ui, &mut self.autostart);
            });
        });
    }

    fn draw_translate(&mut self, ui: &mut Ui) {
        theme::section_label(ui, t("Переводчик"));
        theme::card(ui, |ui| {
            theme::row_w(ui, t("Основной переводчик"), None, 260.0, |ui| {
                let cur = ENGINE_MODES.iter().find(|(c, _)| *c == self.d.engine_mode).map_or(ENGINE_MODES[0].1, |m| m.1);
                egui::ComboBox::from_id_salt("set_engine")
                    .width(ui.available_width().max(80.0))
                    .truncate()
                    .selected_text(theme::body(t(cur)))
                    .show_ui(ui, |ui| {
                        for (code, title) in ENGINE_MODES {
                            ui.selectable_value(&mut self.d.engine_mode, code.to_string(), t(title));
                        }
                    });
            });
            match self.d.engine_mode.as_str() {
                "microsoft" => {
                    theme::row_w(ui, t("Ключ Microsoft"), None, 300.0, |ui| {
                        let w = (ui.available_width() - 110.0).max(80.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.d.ms_key)
                                .password(!self.ms_key_show)
                                .margin(Margin::symmetric(8, 6))
                                .desired_width(w),
                        );
                        if button(ui, t("Показать"), BtnKind::Normal, true, None).clicked() {
                            self.ms_key_show = !self.ms_key_show;
                        }
                    });
                    theme::row_w(
                        ui,
                        t("Регион"),
                        Some(t("Регион, например, westeurope; если ресурс глобальный — оставьте пустым")),
                        240.0,
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.d.ms_region)
                                    .margin(Margin::symmetric(8, 6))
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    theme::note(
                        ui,
                        theme::small(tf(
                            "Бесплатный тариф F0 — 2 млн символов в месяц. Ключ: портал Azure → создать ресурс \
                             «Translator» → «Ключи и конечная точка». Сейчас: {}.",
                            &[&self.status],
                        )),
                    );
                }
                "local_ai" => {
                    theme::row_w(
                        ui,
                        t("Адрес сервера"),
                        Some(t("Адрес сервера: Ollama — http://localhost:11434, LM Studio — http://localhost:1234")),
                        260.0,
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.d.local_ai_url)
                                    .margin(Margin::symmetric(8, 6))
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    theme::row_w(ui, t("Модель"), None, 360.0, |ui| {
                        let cur = if self.d.local_ai_model.is_empty() {
                            t("Выберите модель").to_string()
                        } else {
                            self.d.local_ai_model.clone()
                        };
                        egui::ComboBox::from_id_salt("set_local_model")
                            .width(140.0)
                            .selected_text(theme::body(cur))
                            .show_ui(ui, |ui| {
                                for m in &self.models {
                                    ui.selectable_value(&mut self.d.local_ai_model, m.clone(), m);
                                }
                            });
                        if button(ui, t("Обновить список"), BtnKind::Normal, !self.models_loading, None).clicked() {
                            self.start_models(ui.ctx());
                        }
                        if self.models_loading {
                            ui.label(theme::body(t("Загрузка…")));
                        }
                    });
                    theme::row_w(ui, t("Имя модели"), Some(t("или впишите имя модели вручную")), 260.0, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.d.local_ai_model)
                                .margin(Margin::symmetric(8, 6))
                                .desired_width(ui.available_width()),
                        );
                    });
                    if !self.models_error.is_empty() {
                        theme::note(ui, theme::small(self.models_error.clone()).color(pal().bad));
                    }
                    theme::note(
                        ui,
                        theme::small(tf(
                            "Бесплатно и без интернета. Установите Ollama (ollama.com) и скачайте модель командой \
                             `ollama pull <модель>`; для перевода нужна видеокарта примерно от 6–8 ГБ. Первая реплика \
                             может переводиться дольше — модель загружается в память. Сейчас: {}.",
                            &[&self.status],
                        )),
                    );
                }
                _ => {
                    let key_on = self.d.engine_mode != "google";
                    theme::row_w(ui, t("Ключ DeepL API (необязательно)"), None, 300.0, |ui| {
                        let w = (ui.available_width() - 110.0).max(80.0);
                        ui.add_enabled(
                            key_on,
                            egui::TextEdit::singleline(&mut self.d.deepl_key)
                                .password(!self.key_show)
                                .margin(Margin::symmetric(8, 6))
                                .desired_width(w),
                        );
                        if button(ui, t("Показать"), BtnKind::Normal, key_on, None).clicked() {
                            self.key_show = !self.key_show;
                        }
                    });
                    theme::note(
                        ui,
                        theme::small(tf(
                            "С ключом переводит DeepL (ключ на «:fx» — бесплатный тариф). Без ключа или при ошибке \
                             DeepL перевод идёт через Google. Сейчас: {}.",
                            &[&self.status],
                        )),
                    );
                }
            }
            theme::row_w(
                ui,
                t("Второй перевод"),
                Some(t(
                    "Показывается мелко под основным переводом. Каждая реплика переводится дважды — расходуются лимиты обоих переводчиков.",
                )),
                260.0,
                |ui| {
                    let opts = [
                        ("", t("Нет")),
                        ("google", "Google"),
                        ("deepl", "DeepL"),
                        ("microsoft", "Microsoft Translator"),
                        ("local_ai", t("Локальный ИИ")),
                    ];
                    let cur = opts.iter().find(|(c, _)| *c == self.d.second_engine).map_or(opts[0].1, |o| o.1);
                    egui::ComboBox::from_id_salt("set_second_engine")
                        .width(ui.available_width().max(80.0))
                        .selected_text(theme::body(cur))
                        .show_ui(ui, |ui| {
                            for (code, title) in opts {
                                ui.selectable_value(&mut self.d.second_engine, code.to_string(), title);
                            }
                        });
                },
            );
        });
        theme::section_label(ui, t("Языки"));
        theme::card(ui, |ui| {
            theme::row_w(ui, t("Переводить на"), None, 220.0, |ui| {
                egui::ComboBox::from_id_salt("set_target_lang")
                    .width(ui.available_width().max(80.0))
                    .selected_text(theme::body(lang_label(self.d.target_lang)))
                    .show_ui(ui, |ui| {
                        for l in Lang::TARGETS {
                            ui.selectable_value(&mut self.d.target_lang, l, lang_label(l));
                        }
                    });
            });
            theme::row_w(
                ui,
                t("Язык игр по умолчанию"),
                Some(t("Подставляется в мастере для новых игр. Язык уже добавленной игры меняется в главном окне.")),
                220.0,
                |ui| {
                    egui::ComboBox::from_id_salt("set_source_lang")
                        .width(ui.available_width().max(80.0))
                        .selected_text(theme::body(lang_label(self.d.default_source_lang)))
                        .show_ui(ui, |ui| {
                            for l in Lang::ALL {
                                ui.selectable_value(&mut self.d.default_source_lang, l, lang_label(l));
                            }
                        });
                },
            );
        });
    }

    fn draw_network(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let p = pal();
        let proxy_on = self.net_mode == NET_PROXY;
        theme::section_label(ui, t("Подключение"));
        theme::card(ui, |ui| {
            theme::row_w(ui, t("Подключение"), None, 240.0, |ui| {
                egui::ComboBox::from_id_salt("set_net_mode")
                    .width(ui.available_width().max(80.0))
                    .selected_text(theme::body(net::net_title(&self.net_mode)))
                    .show_ui(ui, |ui| {
                        for code in [NET_SYSTEM, NET_DIRECT, NET_PROXY] {
                            ui.selectable_value(&mut self.net_mode, code.to_string(), net::net_title(code));
                        }
                    });
            });
            theme::row_w(ui, t("Адрес прокси"), Some(t("socks5://127.0.0.1:1080 или http://127.0.0.1:8080")), 260.0, |ui| {
                ui.add_enabled(
                    proxy_on,
                    egui::TextEdit::singleline(&mut self.proxy_addr)
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(ui.available_width()),
                );
            });
            theme::row_w(ui, t("Логин"), Some(t("если прокси требует")), 260.0, |ui| {
                ui.add_enabled(
                    proxy_on,
                    egui::TextEdit::singleline(&mut self.proxy_user)
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(ui.available_width()),
                );
            });
            theme::row_w(ui, t("Пароль"), Some(t("если прокси требует")), 300.0, |ui| {
                let w = (ui.available_width() - 110.0).max(80.0);
                ui.add_enabled(
                    proxy_on,
                    egui::TextEdit::singleline(&mut self.proxy_pass)
                        .password(!self.pass_show)
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(w),
                );
                if button(ui, t("Показать"), BtnKind::Normal, proxy_on, None).clicked() {
                    self.pass_show = !self.pass_show;
                }
            });
            theme::row_w(ui, t("Сетевой интерфейс"), None, 260.0, |ui| {
                let cur = if self.bind_ip.is_empty() {
                    t("Автоматически").to_string()
                } else {
                    self.ifaces
                        .iter()
                        .find(|(_, ip)| *ip == self.bind_ip)
                        .map_or_else(|| self.bind_ip.clone(), |(t, _)| t.clone())
                };
                egui::ComboBox::from_id_salt("set_iface")
                    .width(ui.available_width().max(80.0))
                    .selected_text(theme::body(cur))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.bind_ip, String::new(), t("Автоматически"));
                        for (title, ip) in &self.ifaces {
                            ui.selectable_value(&mut self.bind_ip, ip.clone(), title.as_str());
                        }
                    });
            });
            if proxy_on && !self.proxy_addr.trim().is_empty() {
                if let Err(e) = net::build_proxy_url(&self.proxy_addr, &self.proxy_user, &self.proxy_pass) {
                    theme::note(ui, RichText::new(e).size(13.0).color(p.bad));
                }
            }
            theme::note(
                ui,
                theme::small(t(
                    "Если Google пишет «код 429», выберите другой интерфейс (например, обычное подключение вместо VPN) \
                     или прокси. Настройка влияет только на перевод. Логин и пароль нужны, только если их требует прокси.",
                )),
            );
        });
        theme::section_label(ui, t("Проверка связи"));
        theme::card(ui, |ui| {
            let desc = if self.checking { Some(t("Проверяю…").to_string()) } else { self.check_result.as_ref().map(|(_, s)| s.clone()) };
            theme::row_w(ui, t("Связь с Google"), desc.as_deref(), 230.0, |ui| {
                if button(ui, t("Проверить связь с Google"), BtnKind::Normal, !self.checking, None).clicked() {
                    acts.push(Act::Check);
                }
            });
        });
    }

    fn draw_plate(&mut self, ui: &mut Ui) {
        self.draw_preview(ui);
        theme::section_label(ui, t("Вид плашки"));
        theme::card(ui, |ui| {
            theme::row_w(ui, t("Шрифт"), None, 220.0, |ui| {
                egui::ComboBox::from_id_salt("set_font")
                    .width(ui.available_width().max(80.0))
                    .selected_text(theme::body(self.d.font_family.clone()))
                    .show_ui(ui, |ui| {
                        for f in &self.fonts {
                            ui.selectable_value(&mut self.d.font_family, f.clone(), f.as_str());
                        }
                    });
            });
            theme::row_w(ui, t("Размер текста"), None, 260.0, |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(80.0);
                ui.visuals_mut().widgets.inactive.bg_fill = pal().border; // иначе дорожка сливается с карточкой
                ui.add(egui::Slider::new(&mut self.d.font_size, 14..=60).show_value(false));
                ui.label(theme::dim(format!("{} px", self.d.font_size)));
            });
            for (i, hint) in colors().into_iter().enumerate() {
                self.color_row(ui, hint, None, i);
            }
            theme::row_w(ui, t("Непрозрачность фона"), None, 260.0, |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(80.0);
                ui.visuals_mut().widgets.inactive.bg_fill = pal().border; // иначе дорожка сливается с карточкой
                ui.add(egui::Slider::new(&mut self.d.bg_opacity, 0..=100).show_value(false));
                ui.label(theme::dim(format!("{}%", self.d.bg_opacity)));
            });
            theme::row_w(ui, t("Показывать оригинал под переводом"), Some(t("Мелким шрифтом под текстом перевода")), 36.0, |ui| {
                theme::toggle(ui, &mut self.d.overlay_show_original);
            });
            theme::row_w(
                ui,
                t("Приглушённые кнопки на плашке"),
                Some(t("Панель кнопок почти прозрачная и становится яркой, только когда наводишь на неё курсор")),
                36.0,
                |ui| {
                    theme::toggle(ui, &mut self.d.overlay_toolbar_dim);
                },
            );
        });
    }

    fn draw_hotkeys(&mut self, ui: &mut Ui) {
        theme::section_label(ui, t("Сочетания клавиш"));
        theme::card(ui, |ui| {
            let titles = [t("Скрыть / показать плашку"), t("Перевести заново без кэша"), t("Открыть настройки")];
            for (i, title) in titles.into_iter().enumerate() {
                theme::row_w(ui, title, None, 200.0, |ui| {
                    let resp = hotkey_field(ui, &self.hk[i], self.capturing == Some(i), ui.available_width());
                    if resp.clicked() {
                        self.capturing = Some(i);
                    } else if resp.clicked_elsewhere() && self.capturing == Some(i) {
                        self.capturing = None;
                    }
                });
            }
            theme::note(ui, theme::small(t("Щёлкните по полю и нажмите новое сочетание. Работают в любой программе.")));
        });
    }

    fn draw_japanese(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        theme::section_label(ui, t("Изучение японского"));
        let (has, busy, status) = self.dict_info.clone();
        theme::card(ui, |ui| {
            theme::row_w(
                ui,
                t("Фуригана и словарь в истории диалога"),
                Some(t(
                    "Над иероглифами в истории — чтение каной, при наведении на слово — его значения, правая кнопка мыши — «Добавить в Anki».",
                )),
                36.0,
                |ui| {
                    theme::toggle(ui, &mut self.d.learn_mode);
                },
            );
            let line = if !status.is_empty() {
                status
            } else if has {
                t("Словарь установлен.").to_string()
            } else {
                t("Словарь не скачан.").to_string()
            };
            let show_btn = !has && !busy;
            theme::row_w(ui, "JMdict", Some(&line), if show_btn { 200.0 } else { 0.0 }, |ui| {
                if show_btn && button(ui, t("Скачать (~22 МБ)"), BtnKind::Normal, true, None).clicked() {
                    acts.push(Act::DownloadDict);
                }
            });
            theme::note(ui, theme::small(noveltl_core::dict::ATTRIBUTION));
            theme::note(ui, theme::small(t("Для кнопки «Добавить в Anki» нужна программа Anki с дополнением AnkiConnect.")));
        });
    }

    fn draw_general(&mut self, ui: &mut Ui) {
        theme::section_label(ui, t("Данные"));
        theme::card(ui, |ui| {
            theme::row_w(ui, t("Папка с данными"), Some(t("Настройки, профили игр и кэш перевода")), 230.0, |ui| {
                if button(ui, t("Открыть папку с данными"), BtnKind::Normal, true, None).clicked() {
                    let _ = std::process::Command::new("explorer.exe").arg(crate::paths::data_dir()).spawn();
                }
            });
        });
        theme::section_label(ui, t("О программе"));
        theme::card(ui, |ui| {
            let title = format!("NovelTL {}", env!("CARGO_PKG_VERSION"));
            theme::row_w(
                ui,
                &title,
                Some(t("Перехват текста — LunaHook (GPL-3.0). Лицензии — в папке files\\licenses.")),
                140.0,
                |ui| {
                    if button(ui, t("Страница проекта"), BtnKind::Normal, true, None).clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(PROJECT_URL).spawn();
                    }
                },
            );
        });
    }

    /// Предпросмотр плашки: картинка-градиент и плашка поверх (как `PlatePreview`).
    fn draw_preview(&mut self, ui: &mut Ui) {
        if self.bg_tex.is_none() {
            self.bg_tex =
                Some(ui.ctx().load_texture("settings_plate_bg", gradient_image(1024, 240, 20.0), TextureOptions::LINEAR));
        }
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 120.0), Sense::hover());
        let painter = ui.painter_at(rect);
        if let Some(t) = &self.bg_tex {
            painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        let plate = Rect::from_min_max(rect.min + Vec2::new(16.0, 14.0), rect.max - Vec2::new(16.0, 14.0));
        let bg = self.color_value(3);
        let alpha = (self.d.bg_opacity.min(100) as f32 * 2.55).round() as u8;
        painter.rect_filled(plate, CornerRadius::same(10), Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), alpha));

        let painter = painter.with_clip_rect(plate);
        let (text_c, name_c, outline_c) = (self.color_value(0), self.color_value(1), self.color_value(2));
        let pad = Vec2::new(14.0, 8.0);
        let wrap_w = (plate.width() - 2.0 * pad.x).max(40.0);
        let avail_h = plate.height() - 2.0 * pad.y;
        let (name, line) = (t("Акари"), t("Доброе утро. Вы сегодня снова рано."));
        // размер подгоняется, чтобы всё поместилось в плашку
        let mut scale = 1.0_f32;
        let (name_font, text_font) = loop {
            let ts = (self.d.font_size as f32 * scale).max(8.0);
            let ns = (ts * 0.62).max(9.0);
            let nf = FontId::new(ns, theme::bold_family());
            let tf = FontId::proportional(ts);
            let h = painter.layout(name.to_string(), nf.clone(), name_c, wrap_w).size().y
                + 2.0
                + painter.layout(line.to_string(), tf.clone(), text_c, wrap_w).size().y;
            if h <= avail_h || scale < 0.3 {
                break (nf, tf);
            }
            scale *= 0.92;
        };
        let mut y = plate.top() + pad.y;
        let name_h = paint_outlined(&painter, Pos2::new(plate.left() + pad.x, y), name, name_font, name_c, outline_c, wrap_w);
        y += name_h + 2.0;
        paint_outlined(&painter, Pos2::new(plate.left() + pad.x, y), line, text_font, text_c, outline_c, wrap_w);
    }
}

/// Пункт меню разделов: значок и название, выбранный подсвечен.
fn nav_item(ui: &mut Ui, icon: theme::Icon, text: &str, selected: bool) -> egui::Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.card);
    }
    let painter = ui.painter_at(rect);
    let ic = if selected { p.accent } else { p.dim };
    painter.text(rect.left_center() + Vec2::new(22.0, 0.0), Align2::CENTER_CENTER, theme::icon(icon), FontId::new(17.0, theme::icon_family()), ic);
    painter.text(rect.left_center() + Vec2::new(40.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(15.0), p.text);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Круглый аватар: картинка по кругу (сетка-веер) или буква ника на сером круге.
fn paint_avatar(painter: &Painter, c: Pos2, r: f32, tex: Option<&TextureHandle>, nick: &str) {
    let p = pal();
    match tex {
        Some(t) => {
            let mut mesh = egui::Mesh::with_texture(t.id());
            let n = 48;
            mesh.vertices.push(egui::epaint::Vertex { pos: c, uv: Pos2::new(0.5, 0.5), color: Color32::WHITE });
            for k in 0..=n {
                let a = k as f32 / n as f32 * std::f32::consts::TAU;
                let (s, co) = a.sin_cos();
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: c + Vec2::new(co, s) * r,
                    uv: Pos2::new(0.5 + co * 0.5, 0.5 + s * 0.5),
                    color: Color32::WHITE,
                });
            }
            for k in 0..n {
                mesh.indices.extend_from_slice(&[0, k + 1, k + 2]);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
        None => {
            painter.circle_filled(c, r, p.border);
            let letter = nick.trim().chars().next().map_or("N".to_string(), |ch| ch.to_uppercase().to_string());
            painter.text(c, Align2::CENTER_CENTER, letter, FontId::proportional(r * 0.9), p.text);
        }
    }
    painter.circle_stroke(c, r, Stroke::new(1.0_f32, p.border));
}

/// Квадратный образец цвета.
fn swatch(ui: &mut Ui, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(44.0, 24.0), Sense::click());
    ui.painter().rect(rect, CornerRadius::same(6), color, Stroke::new(1.0_f32, pal().dim), StrokeKind::Inside);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Кружок акцента 20 px (слот 24 px): `hex` пустой — монохром (половинки чёрная/белая), None — «+» со своим цветом.
fn accent_dot(ui: &mut Ui, hex: Option<&str>, selected: bool) -> egui::Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let (c, r) = (rect.center(), 10.0);
    let painter = ui.painter();
    match hex {
        Some("") => {
            painter.circle_filled(c, r, Color32::WHITE);
            let half: Vec<Pos2> = (0..=16)
                .map(|k| {
                    let a = std::f32::consts::FRAC_PI_2 + k as f32 / 16.0 * std::f32::consts::PI;
                    c + Vec2::new(a.cos(), a.sin()) * r
                })
                .collect();
            painter.add(egui::Shape::convex_polygon(half, Color32::BLACK, Stroke::NONE));
            painter.circle_stroke(c, r, Stroke::new(1.0_f32, p.border));
        }
        Some(h) => {
            painter.circle_filled(c, r, theme::parse_hex(h, Color32::GRAY));
        }
        None => {
            if selected {
                painter.circle_filled(c, r, p.accent);
            } else {
                // пунктирная окружность и плюс
                for k in 0..12 {
                    let (a0, a1) = (k as f32 * 30.0_f32.to_radians(), (k as f32 * 30.0 + 18.0).to_radians());
                    painter.line_segment(
                        [c + Vec2::new(a0.cos(), a0.sin()) * r, c + Vec2::new(a1.cos(), a1.sin()) * r],
                        Stroke::new(1.0_f32, p.dim),
                    );
                }
                let s = Stroke::new(1.5_f32, p.dim);
                painter.line_segment([c - Vec2::new(4.0, 0.0), c + Vec2::new(4.0, 0.0)], s);
                painter.line_segment([c - Vec2::new(0.0, 4.0), c + Vec2::new(0.0, 4.0)], s);
            }
        }
    }
    if selected {
        painter.circle_stroke(c, r + 2.0, Stroke::new(2.0_f32, p.text));
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Название языка по-русски. Родные названия (日本語, 한국어) не показываем: шрифты для них
/// грузятся лениво, и в окне настроек они превращались в квадратики.
fn lang_label(l: Lang) -> String {
    l.title().to_string()
}

fn wrap(ui: &mut Ui, text: RichText) {
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Wrap));
}

fn to_color(s: &str, fallback: u32) -> Color32 {
    let v = parse_color(s, fallback);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Текст с контуром: сначала цветом контура со сдвигами по кругу, потом основным. Возвращает высоту текста.
fn paint_outlined(
    painter: &Painter,
    pos: Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    outline: Color32,
    wrap_w: f32,
) -> f32 {
    let main = painter.layout(text.to_string(), font.clone(), color, wrap_w);
    let out = painter.layout(text.to_string(), font.clone(), outline, wrap_w);
    let r = (font.size / 18.0).clamp(1.0, 3.0);
    let d = r * 0.7;
    for (dx, dy) in [(r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r), (d, d), (-d, d), (d, -d), (-d, -d)] {
        painter.galley(pos + Vec2::new(dx, dy), out.clone(), outline);
    }
    let h = main.size().y;
    painter.galley(pos, main, color);
    h
}

/// Скруглённая картинка-градиент как у `PlatePreview` (диагональ, три цвета).
fn gradient_image(w: usize, h: usize, radius: f32) -> ColorImage {
    let stops: [(f32, [f32; 3]); 3] = [
        (0.0, [0xf6 as f32, 0xc7 as f32, 0xa8 as f32]),
        (0.5, [0xe7 as f32, 0x9a as f32, 0xa5 as f32]),
        (1.0, [0x5b as f32, 0x4a as f32, 0x7a as f32]),
    ];
    let (wf, hf) = (w as f32, h as f32);
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = ((px * wf + py * hf) / (wf * wf + hf * hf)).clamp(0.0, 1.0);
            let (a, b) = if t <= 0.5 { (stops[0], stops[1]) } else { (stops[1], stops[2]) };
            let k = ((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
            let c = |i: usize| (a.1[i] + (b.1[i] - a.1[i]) * k).round() as u8;
            let dx = (radius - px).max(px - (wf - radius)).max(0.0);
            let dy = (radius - py).max(py - (hf - radius)).max(0.0);
            let cover = (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            pixels.push(Color32::from_rgba_unmultiplied(c(0), c(1), c(2), (cover * 255.0).round() as u8));
        }
    }
    ColorImage { size: [w, h], pixels }
}

/// Поле сочетания: по щелчку ждёт нажатия.
fn hotkey_field(ui: &mut Ui, text: &str, capturing: bool, width: f32) -> egui::Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let (fill, stroke) = if capturing {
        (p.selected, Stroke::new(1.0_f32, p.accent))
    } else if resp.hovered() {
        (p.card, Stroke::new(1.0_f32, p.accent))
    } else {
        (p.card, Stroke::new(1.0_f32, p.border))
    };
    ui.painter().rect(rect, CornerRadius::same(8), fill, stroke, StrokeKind::Inside);
    let (shown, color) = if capturing {
        (t("Нажмите сочетание…").to_string(), p.dim)
    } else if text.is_empty() {
        (t("не задано").to_string(), p.dim)
    } else {
        (text.to_string(), p.text)
    };
    ui.painter().text(rect.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, shown, FontId::proportional(14.0), color);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Имя клавиши в том виде, как его понимает `parse_hotkey`.
fn key_name(k: egui::Key) -> Option<String> {
    use egui::Key;
    let s = match k {
        Key::Space => "Space",
        Key::Tab => "Tab",
        Key::PageUp => "PgUp",
        Key::PageDown => "PgDown",
        Key::End => "End",
        Key::Home => "Home",
        Key::ArrowLeft => "Left",
        Key::ArrowUp => "Up",
        Key::ArrowRight => "Right",
        Key::ArrowDown => "Down",
        Key::Insert => "Ins",
        Key::Delete => "Del",
        Key::Backtick => "`",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Backslash => "\\",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",
        _ => {
            let n = k.name();
            let mut chars = n.chars();
            let single = matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_alphanumeric());
            let f_key = n.len() > 1
                && n.starts_with('F')
                && n[1..].parse::<u32>().is_ok_and(|v| (1..=24).contains(&v));
            if single || f_key {
                return Some(n.to_string());
            }
            return None;
        }
    };
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_parse_back() {
        for k in [egui::Key::A, egui::Key::Num5, egui::Key::F9, egui::Key::Space, egui::Key::Minus, egui::Key::ArrowLeft] {
            let n = key_name(k).expect("имя");
            assert!(parse_hotkey(&format!("Ctrl+Alt+{n}")).is_some(), "{n}");
        }
    }

    #[test]
    fn preview_key_roundtrip() {
        let mut s = AppSettings::default();
        let mut other = AppSettings::default();
        other.font_size = 40;
        other.theme = "light".into();
        PreviewKey::of(&other).put(&mut s);
        assert_eq!(s.font_size, 40);
        assert_eq!(s.theme, "light");
    }

    #[test]
    fn ui_lang_saved() {
        let base = AppSettings::default();
        let mut d = base.clone();
        d.ui_lang = Some(UiLang::En);
        d.accent = "#2FBF8A".into();
        d.overlay_show_original = true;
        d.overlay_toolbar_dim = true;
        let hk = ["Ctrl+Alt+H".to_string(), "Ctrl+Alt+R".to_string(), " Ctrl+Alt+O ".to_string()];
        let new = draft_to_settings(&d, &hk, &base);
        assert_eq!(new.ui_lang, Some(UiLang::En));
        assert_eq!(new.accent, "#2FBF8A");
        assert!(new.overlay_show_original);
        assert!(new.overlay_toolbar_dim);
        assert_eq!(new.hotkey_settings, "Ctrl+Alt+O");
    }

    #[test]
    fn gradient_corners_transparent() {
        let img = gradient_image(100, 40, 10.0);
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[20 * 100 + 50].a(), 255);
    }
}
