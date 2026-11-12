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
use noveltl_core::langdetect::Lang;
use noveltl_core::net::{self, NET_DIRECT, NET_PROXY, NET_SYSTEM};
use noveltl_core::profiles::AppSettings;
use noveltl_core::translators::ENGINE_MODES;

use super::theme::{self, button, hline, pal, BtnKind};
use super::{avatar, lock_ctl, Shared};
use crate::hotkeys::parse_hotkey;
use crate::overlay::parse_color;

pub const SETTINGS_TITLE: &str = "Настройки — NovelTL";

/// Разделы окна (порядок = порядок в меню слева).
const SECTIONS: [&str; 5] = ["Персонализация", "Перевод", "Сеть", "Горячие клавиши", "Общие"];

const PROJECT_URL: &str = "https://github.com/Kawaakar1/NovelTL";

/// (подпись под цветом, подсказка)
const COLORS: [(&str, &str); 4] =
    [("Текста", "Цвет текста"), ("Имени", "Цвет имени"), ("Контура", "Цвет контура"), ("Фона", "Цвет фона")];

/// Цвета своей темы — слоты 4..8 (после цветов плашки).
const CUSTOM_COLORS: [(&str, &str); 4] =
    [("Фон", "Цвет фона окон"), ("Панели", "Цвет панелей и карточек"), ("Текст", "Цвет текста"), ("Акцент", "Акцентный цвет")];

/// Подсказка для окна выбора цвета по номеру слота.
fn slot_hint(i: usize) -> &'static str {
    if i < 4 {
        COLORS[i].1
    } else {
        CUSTOM_COLORS[(i - 4).min(3)].1
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

/// Высота рабочей области экрана в пикселях.
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
}

impl PreviewKey {
    fn of(s: &AppSettings) -> PreviewKey {
        PreviewKey {
            custom_bg: s.custom_bg.clone(),
            custom_card: s.custom_card.clone(),
            custom_text: s.custom_text.clone(),
            custom_accent: s.custom_accent.clone(),
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

enum Act {
    Save,
    Cancel,
    Check,
}

pub struct SettingsWindow {
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
    hk: [String; 2],
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
            ifaces_all.push((format!("{} (сейчас недоступен)", orig.bind_ip), orig.bind_ip.clone()));
        }
        let mut fonts = installed_fonts();
        if !fonts.iter().any(|f| *f == orig.font_family) {
            fonts.insert(0, orig.font_family.clone());
        }
        let ppp = shared.ctx.pixels_per_point().max(0.5);
        let limit = (work_area_height_px() / ppp - 60.0).max(420.0);
        SettingsWindow {
            id: ViewportId::from_hash_of("noveltl_settings"),
            icon: Arc::new(theme::icon_data(64)),
            pos,
            applied: PreviewKey::of(&orig),
            d: orig.clone(),
            hk: [orig.hotkey_toggle.clone(), orig.hotkey_retranslate.clone()],
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
        SETTINGS_TITLE
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
        self.section = i.min(SECTIONS.len() - 1);
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
            .with_title(SETTINGS_TITLE)
            .with_inner_size([780.0, h0])
            .with_min_inner_size([660.0, 320.0])
            .with_always_on_top()
            .with_icon(self.icon.clone());
        if let Some(pos) = self.pos {
            builder = builder.with_position(pos);
        }
        if self.sent_h == 0.0 {
            self.sent_h = h0;
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

    fn save(&mut self, shared: &Shared) {
        let toggle = self.hk[0].trim().to_string();
        let retr = self.hk[1].trim().to_string();
        for combo in [&toggle, &retr] {
            if !combo.is_empty() && parse_hotkey(combo).is_none() {
                self.warn(format!("Сочетание «{combo}» не подходит. Нужна клавиша с Ctrl, Alt или Shift."));
                return;
            }
        }
        if !toggle.is_empty() && toggle == retr {
            self.warn("Для двух действий выбрано одно и то же сочетание.");
            return;
        }
        if self.d.engine_mode == "deepl" && self.d.deepl_key.trim().is_empty() {
            self.warn("Для режима «Только DeepL» нужен ключ.");
            return;
        }
        if self.d.engine_mode == "microsoft" && self.d.ms_key.trim().is_empty() {
            self.warn("Для Microsoft Translator нужен ключ.");
            return;
        }
        if self.d.engine_mode == "local_ai" {
            if self.d.local_ai_model.trim().is_empty() {
                self.warn("Выберите или впишите модель локального ИИ.");
                return;
            }
            let u = self.d.local_ai_url.trim();
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                self.warn("Адрес сервера локального ИИ должен начинаться с http:// или https://.");
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
        let mut new = self.orig.clone();
        PreviewKey::of(&self.d).put(&mut new);
        new.nickname = self.d.nickname.trim().to_string();
        new.avatar = self.d.avatar.clone();
        new.net_mode = mode;
        new.proxy_url = proxy_text;
        new.proxy_user = user;
        new.proxy_pass = pass;
        new.bind_ip = self.bind_ip.clone();
        new.deepl_key = self.d.deepl_key.trim().to_string();
        new.ms_key = self.d.ms_key.trim().to_string();
        new.ms_region = self.d.ms_region.trim().to_string();
        new.local_ai_url = self.d.local_ai_url.trim().trim_end_matches('/').to_string();
        new.local_ai_model = self.d.local_ai_model.trim().to_string();
        new.hotkey_toggle = toggle;
        new.hotkey_retranslate = retr;
        new.engine_mode = self.d.engine_mode.clone();
        new.target_lang = self.d.target_lang;
        new.default_source_lang = self.d.default_source_lang;
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
            self.warn(format!("Не удалось изменить автозапуск: {e}\nОстальные настройки сохранены."));
        }
        if errors.is_empty() {
            if auto_err.is_some() {
                self.close_after_message = true;
            } else {
                self.closed = true;
            }
        } else {
            self.warn(format!(
                "Не удалось занять сочетание: {}.\nСкорее всего, его уже использует другая программа — выберите другое.\nОстальные настройки сохранены.",
                errors.join(", ")
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
        let p = pal();
        let foot = egui::TopBottomPanel::bottom("set_foot")
            .show_separator_line(true)
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 24, right: 24, top: 10, bottom: 18 }))
            .show(ctx, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Сохранить", BtnKind::Primary, true, None).clicked() {
                        acts.push(Act::Save);
                    }
                    if button(ui, "Отмена", BtnKind::Normal, true, None).clicked() {
                        acts.push(Act::Cancel);
                    }
                });
            });
        // меню разделов слева
        egui::SidePanel::left("set_nav")
            .exact_width(190.0)
            .resizable(false)
            .show_separator_line(true)
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 12, right: 12, top: 20, bottom: 8 }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (i, title) in SECTIONS.iter().enumerate() {
                    if nav_item(ui, title, self.section == i).clicked() {
                        self.section = i;
                    }
                }
            });
        let central = egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ctx, |ui| {
            egui::ScrollArea::vertical().id_salt("settings_scroll").auto_shrink([false, false]).show(ui, |ui| {
                Frame::new().inner_margin(Margin { left: 24, right: 24, top: 20, bottom: 8 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.draw_section(ui, acts);
                });
            })
        });
        self.content_h = central.inner.content_size.y;
        // высота окна — по содержимому, но не больше рабочей области экрана минус 60 px
        let want = (self.content_h + foot.response.rect.height()).clamp(420.0_f32.min(self.limit), self.limit);
        if self.frames <= 12 && self.content_h > 0.0 && (want - self.sent_h).abs() > 1.5 {
            let w = ctx.input(|i| i.viewport().inner_rect).map_or(780.0, |r| r.width());
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
            _ => &mut self.d.custom_accent,
        }
    }

    fn color_value(&self, i: usize) -> Color32 {
        if i >= 4 {
            let s = match i {
                4 => &self.d.custom_bg,
                5 => &self.d.custom_card,
                6 => &self.d.custom_text,
                _ => &self.d.custom_accent,
            };
            return theme::parse_hex(s, Color32::GRAY);
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
        egui::Window::new("Выберите цвет")
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
                    if button(ui, "Готово", BtnKind::Primary, true, Some(90.0)).clicked() {
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
        let label_w = 190.0;
        let gap = 18.0;
        let ctl_w = (ui.available_width() - label_w - gap).max(140.0);
        ui.spacing_mut().item_spacing.y = 12.0;
        ui.label(theme::h1(SECTIONS[self.section]));
        match self.section {
            0 => self.draw_personal(ui, label_w, gap, ctl_w),
            1 => self.draw_translate(ui, label_w, gap, ctl_w),
            2 => self.draw_network(ui, acts, label_w, gap, ctl_w),
            3 => self.draw_hotkeys(ui, label_w, gap, ctl_w),
            _ => self.draw_general(ui),
        }
    }

    /// Открыть окно выбора цвета для слота.
    fn open_picker(&mut self, i: usize) {
        let c = self.color_value(i);
        self.hex_buf = theme::to_hex(c);
        self.picking = Some((i, c));
    }

    fn draw_personal(&mut self, ui: &mut Ui, label_w: f32, gap: f32, ctl_w: f32) {
        self.draw_profile(ui);
        hline(ui);
        self.draw_themes(ui);
        hline(ui);
        self.draw_plate(ui, label_w, gap, ctl_w);
    }

    /// Профиль: ник и аватар.
    fn draw_profile(&mut self, ui: &mut Ui) {
        let p = pal();
        ui.label(theme::h2("Профиль"));
        ui.horizontal(|ui| {
            ui.label(theme::body("Ник"));
            ui.add(
                egui::TextEdit::singleline(&mut self.d.nickname)
                    .char_limit(32)
                    .hint_text("Как вас называть — показывается в главном окне")
                    .margin(Margin::symmetric(8, 6))
                    .desired_width((ui.available_width() - 20.0).clamp(160.0, 420.0)),
            );
        });
        // текстура превью (перезагружается при смене файла)
        let name = self.d.avatar.clone();
        if name.is_empty() {
            self.avatar_tex = None;
        } else if self.avatar_tex.as_ref().map(|(n, _)| n.as_str()) != Some(name.as_str()) {
            self.avatar_tex = avatar::load_rgba(&name).map(|(w, h, rgba)| {
                let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                (name.clone(), ui.ctx().load_texture(format!("settings_avatar_{name}"), img, TextureOptions::LINEAR))
            });
        }
        ui.horizontal(|ui| {
            ui.label(theme::body("Аватар"));
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            let painter = ui.painter();
            match &self.avatar_tex {
                Some((_, t)) => {
                    // круглая обрезка: картинка по кругу через сетку-веер
                    let mut mesh = egui::Mesh::with_texture(t.id());
                    let n = 48;
                    let c = rect.center();
                    let r = 32.0;
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
                    painter.circle_filled(rect.center(), 32.0, p.border);
                    let letter = self.d.nickname.trim().chars().next().map_or("N".to_string(), |ch| ch.to_uppercase().to_string());
                    painter.text(rect.center(), Align2::CENTER_CENTER, letter, FontId::proportional(28.0), p.text);
                }
            }
            painter.circle_stroke(rect.center(), 32.0, Stroke::new(1.0_f32, p.border));
            if button(ui, "Выбрать картинку…", BtnKind::Normal, true, None).clicked() {
                if let Some(path) = avatar::pick_image() {
                    match avatar::store_avatar(&path) {
                        Ok(n) => self.d.avatar = n,
                        Err(e) => self.warn(format!("Не удалось загрузить картинку: {e}")),
                    }
                }
            }
            if !self.d.avatar.is_empty() && button(ui, "Убрать", BtnKind::Normal, true, None).clicked() {
                self.d.avatar = String::new();
            }
        });
    }

    /// Сетка карточек-пресетов и, для «Своей», цвета темы.
    fn draw_themes(&mut self, ui: &mut Ui) {
        ui.label(theme::h2("Тема оформления"));
        let sys_dark = ui.ctx().system_theme() != Some(egui::Theme::Light);
        let cur_accent = pal().accent;
        let (cw, ch) = (130.0_f32, 64.0_f32);
        let (title_h, sp) = (20.0_f32, 10.0_f32);
        let mut chosen: Option<String> = None;
        for row in theme::PRESETS.chunks(4) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = sp;
                for (id, title) in row {
                    let pp = theme::palette_for(id, &self.d, sys_dark);
                    let sel = self.d.theme == *id;
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, ch), Sense::click());
                        let painter = ui.painter();
                        painter.rect_filled(rect, CornerRadius::same(8), pp.bg);
                        let card = Rect::from_min_size(rect.min + Vec2::new(10.0, 10.0), Vec2::new(cw - 20.0, 28.0));
                        painter.rect(card, CornerRadius::same(5), pp.card, Stroke::new(1.0_f32, pp.border), StrokeKind::Inside);
                        let bar = Rect::from_min_size(card.min + Vec2::new(8.0, 8.0), Vec2::new(40.0, 6.0));
                        painter.rect_filled(bar, CornerRadius::same(3), pp.accent);
                        let line = Rect::from_min_size(card.min + Vec2::new(8.0, 18.0), Vec2::new(64.0, 4.0));
                        painter.rect_filled(line, CornerRadius::same(2), pp.text);
                        let stroke = if sel {
                            Stroke::new(2.0_f32, cur_accent)
                        } else {
                            Stroke::new(1.0_f32, pp.border)
                        };
                        painter.rect_stroke(rect, CornerRadius::same(8), stroke, StrokeKind::Inside);
                        let (tr, _) = ui.allocate_exact_size(Vec2::new(cw, title_h), Sense::hover());
                        ui.painter().text(
                            tr.left_center() + Vec2::new(2.0, 0.0),
                            Align2::LEFT_CENTER,
                            *title,
                            FontId::proportional(13.0),
                            if sel { cur_accent } else { pal().text },
                        );
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            chosen = Some(id.to_string());
                        }
                    });
                }
            });
        }
        if let Some(id) = chosen {
            self.d.theme = id;
        }
        if self.d.theme == "custom" {
            ui.add_space(4.0);
            ui.label(theme::h2("Цвета своей темы"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (k, (title, hint)) in CUSTOM_COLORS.iter().enumerate() {
                    let i = 4 + k;
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(44.0, 28.0), Sense::click());
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(6),
                            self.color_value(i),
                            Stroke::new(1.0_f32, pal().dim),
                            StrokeKind::Inside,
                        );
                        let resp = resp.on_hover_text(*hint).on_hover_cursor(CursorIcon::PointingHand);
                        if resp.clicked() {
                            self.open_picker(i);
                        }
                        ui.label(theme::small(*title));
                    });
                }
                if button(ui, "Сбросить", BtnKind::Normal, true, None).clicked() {
                    let def = AppSettings::default();
                    self.d.custom_bg = def.custom_bg;
                    self.d.custom_card = def.custom_card;
                    self.d.custom_text = def.custom_text;
                    self.d.custom_accent = def.custom_accent;
                }
            });
        }
    }

    fn draw_translate(&mut self, ui: &mut Ui, label_w: f32, gap: f32, ctl_w: f32) {
        // --- переводчик ---
        ui.label(theme::h2("Переводчик"));
        let cur = ENGINE_MODES.iter().find(|(c, _)| *c == self.d.engine_mode).map_or(ENGINE_MODES[0].1, |m| m.1);
        egui::ComboBox::from_id_salt("set_engine").width(ui.available_width().min(420.0)).selected_text(theme::body(cur)).show_ui(
            ui,
            |ui| {
                for (code, title) in ENGINE_MODES {
                    ui.selectable_value(&mut self.d.engine_mode, code.to_string(), title);
                }
            },
        );
        match self.d.engine_mode.as_str() {
            "microsoft" => {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 110.0;
                    ui.add(
                        egui::TextEdit::singleline(&mut self.d.ms_key)
                            .password(!self.ms_key_show)
                            .hint_text("Ключ Microsoft")
                            .margin(Margin::symmetric(8, 6))
                            .desired_width(w),
                    );
                    if button(ui, "Показать", BtnKind::Normal, true, None).clicked() {
                        self.ms_key_show = !self.ms_key_show;
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.d.ms_region)
                        .hint_text("Регион, например, westeurope; если ресурс глобальный — оставьте пустым")
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(ui.available_width().min(420.0)),
                );
                wrap(
                    ui,
                    theme::small(format!(
                        "Бесплатный тариф F0 — 2 млн символов в месяц. Ключ: портал Azure → создать ресурс \
                         «Translator» → «Ключи и конечная точка». Сейчас: {}.",
                        self.status
                    )),
                );
            }
            "local_ai" => {
                ui.add(
                    egui::TextEdit::singleline(&mut self.d.local_ai_url)
                        .hint_text("Адрес сервера: Ollama — http://localhost:11434, LM Studio — http://localhost:1234")
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(ui.available_width().min(420.0)),
                );
                ui.horizontal(|ui| {
                    let cur = if self.d.local_ai_model.is_empty() {
                        "Выберите модель".to_string()
                    } else {
                        self.d.local_ai_model.clone()
                    };
                    egui::ComboBox::from_id_salt("set_local_model")
                        .width(220.0)
                        .selected_text(theme::body(cur))
                        .show_ui(ui, |ui| {
                            for m in &self.models {
                                ui.selectable_value(&mut self.d.local_ai_model, m.clone(), m);
                            }
                        });
                    if button(ui, "Обновить список", BtnKind::Normal, !self.models_loading, None).clicked() {
                        self.start_models(ui.ctx());
                    }
                    if self.models_loading {
                        ui.label(theme::body("Загрузка…"));
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.d.local_ai_model)
                        .hint_text("или впишите имя модели вручную")
                        .margin(Margin::symmetric(8, 6))
                        .desired_width(ui.available_width().min(420.0)),
                );
                if !self.models_error.is_empty() {
                    wrap(ui, theme::small(self.models_error.clone()).color(pal().bad));
                }
                wrap(
                    ui,
                    theme::small(format!(
                        "Бесплатно и без интернета. Установите Ollama (ollama.com) и скачайте модель командой \
                         `ollama pull <модель>`; для перевода нужна видеокарта примерно от 6–8 ГБ. Первая реплика \
                         может переводиться дольше — модель загружается в память. Сейчас: {}.",
                        self.status
                    )),
                );
            }
            _ => {
                let key_on = self.d.engine_mode != "google";
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 110.0;
                    ui.add_enabled(
                        key_on,
                        egui::TextEdit::singleline(&mut self.d.deepl_key)
                            .password(!self.key_show)
                            .hint_text("Ключ DeepL API (необязательно)")
                            .margin(Margin::symmetric(8, 6))
                            .desired_width(w),
                    );
                    if button(ui, "Показать", BtnKind::Normal, key_on, None).clicked() {
                        self.key_show = !self.key_show;
                    }
                });
                wrap(
                    ui,
                    theme::small(format!(
                        "С ключом переводит DeepL (ключ на «:fx» — бесплатный тариф). Без ключа или при ошибке \
                         DeepL перевод идёт через Google. Сейчас: {}.",
                        self.status
                    )),
                );
            }
        }
        hline(ui);

        // --- языки ---
        egui::Grid::new("set_lang_grid").num_columns(2).spacing([gap, 10.0]).min_col_width(label_w).show(ui, |ui| {
            ui.label(theme::body("Переводить на"));
            egui::ComboBox::from_id_salt("set_target_lang")
                .width(ctl_w)
                .selected_text(theme::body(lang_label(self.d.target_lang)))
                .show_ui(ui, |ui| {
                    for l in Lang::TARGETS {
                        ui.selectable_value(&mut self.d.target_lang, l, lang_label(l));
                    }
                });
            ui.end_row();

            ui.label(theme::body("Язык игр по умолчанию"));
            egui::ComboBox::from_id_salt("set_source_lang")
                .width(ctl_w)
                .selected_text(theme::body(lang_label(self.d.default_source_lang)))
                .show_ui(ui, |ui| {
                    for l in Lang::ALL {
                        ui.selectable_value(&mut self.d.default_source_lang, l, lang_label(l));
                    }
                });
            ui.end_row();
        });
        wrap(
            ui,
            theme::small(
                "Подставляется в мастере для новых игр. Язык уже добавленной игры меняется в главном окне.",
            ),
        );
    }

    fn draw_network(&mut self, ui: &mut Ui, acts: &mut Vec<Act>, label_w: f32, gap: f32, ctl_w: f32) {
        let p = pal();
        let proxy_on = self.net_mode == NET_PROXY;
        egui::Grid::new("set_net_grid").num_columns(2).spacing([gap, 10.0]).min_col_width(label_w).show(ui, |ui| {
            ui.label(theme::body("Подключение"));
            egui::ComboBox::from_id_salt("set_net_mode")
                .width(ctl_w)
                .selected_text(theme::body(net::net_title(&self.net_mode)))
                .show_ui(ui, |ui| {
                    for code in [NET_SYSTEM, NET_DIRECT, NET_PROXY] {
                        ui.selectable_value(&mut self.net_mode, code.to_string(), net::net_title(code));
                    }
                });
            ui.end_row();

            ui.label(theme::body("Адрес прокси"));
            ui.add_enabled(
                proxy_on,
                egui::TextEdit::singleline(&mut self.proxy_addr)
                    .hint_text("socks5://127.0.0.1:1080 или http://127.0.0.1:8080")
                    .margin(Margin::symmetric(8, 6))
                    .desired_width(ctl_w),
            );
            ui.end_row();

            ui.label(theme::body("Логин"));
            ui.add_enabled(
                proxy_on,
                egui::TextEdit::singleline(&mut self.proxy_user)
                    .hint_text("если прокси требует")
                    .margin(Margin::symmetric(8, 6))
                    .desired_width(ctl_w),
            );
            ui.end_row();

            ui.label(theme::body("Пароль"));
            ui.horizontal(|ui| {
                ui.add_enabled(
                    proxy_on,
                    egui::TextEdit::singleline(&mut self.proxy_pass)
                        .password(!self.pass_show)
                        .hint_text("если прокси требует")
                        .margin(Margin::symmetric(8, 6))
                        .desired_width((ctl_w - 112.0).max(80.0)),
                );
                if button(ui, "Показать", BtnKind::Normal, proxy_on, None).clicked() {
                    self.pass_show = !self.pass_show;
                }
            });
            ui.end_row();

            ui.label(theme::body("Сетевой интерфейс"));
            let cur = if self.bind_ip.is_empty() {
                "Автоматически".to_string()
            } else {
                self.ifaces
                    .iter()
                    .find(|(_, ip)| *ip == self.bind_ip)
                    .map_or_else(|| self.bind_ip.clone(), |(t, _)| t.clone())
            };
            egui::ComboBox::from_id_salt("set_iface").width(ctl_w).selected_text(theme::body(cur)).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.bind_ip, String::new(), "Автоматически");
                for (title, ip) in &self.ifaces {
                    ui.selectable_value(&mut self.bind_ip, ip.clone(), title.as_str());
                }
            });
            ui.end_row();
        });
        if proxy_on && !self.proxy_addr.trim().is_empty() {
            if let Err(e) = net::build_proxy_url(&self.proxy_addr, &self.proxy_user, &self.proxy_pass) {
                wrap(ui, RichText::new(e).size(13.0).color(p.bad));
            }
        }
        wrap(
            ui,
            theme::small(
                "Если Google пишет «код 429», выберите другой интерфейс (например, обычное подключение вместо VPN) \
                 или прокси. Настройка влияет только на перевод. Логин и пароль нужны, только если их требует прокси.",
            ),
        );
        ui.horizontal(|ui| {
            if button(ui, "Проверить связь с Google", BtnKind::Normal, !self.checking, None).clicked() {
                acts.push(Act::Check);
            }
            let rich = if self.checking {
                theme::body("Проверяю…")
            } else {
                match &self.check_result {
                    Some((ok, text)) => RichText::new(text.clone()).size(14.0).color(if *ok { p.ok } else { p.bad }),
                    None => theme::body(""),
                }
            };
            wrap(ui, rich);
        });
    }

    fn draw_plate(&mut self, ui: &mut Ui, label_w: f32, gap: f32, ctl_w: f32) {
        let p = pal();
        // --- плашка ---
        ui.label(theme::h2("Плашка с переводом"));
        self.draw_preview(ui);
        egui::Grid::new("set_look_grid").num_columns(2).spacing([gap, 10.0]).min_col_width(label_w).show(ui, |ui| {
            ui.label(theme::body("Шрифт"));
            egui::ComboBox::from_id_salt("set_font")
                .width(ctl_w)
                .selected_text(theme::body(self.d.font_family.clone()))
                .show_ui(ui, |ui| {
                    for f in &self.fonts {
                        ui.selectable_value(&mut self.d.font_family, f.clone(), f.as_str());
                    }
                });
            ui.end_row();

            ui.label(theme::body("Размер текста"));
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ctl_w - 64.0).max(80.0);
                ui.add(egui::Slider::new(&mut self.d.font_size, 14..=60).show_value(false));
                ui.label(theme::dim(format!("{} px", self.d.font_size)));
            });
            ui.end_row();

            ui.label(theme::body("Цвета"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (i, (title, hint)) in COLORS.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(44.0, 28.0), Sense::click());
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(6),
                            self.color_value(i),
                            Stroke::new(1.0_f32, p.dim),
                            StrokeKind::Inside,
                        );
                        let resp = resp.on_hover_text(*hint).on_hover_cursor(CursorIcon::PointingHand);
                        if resp.clicked() {
                            self.open_picker(i);
                        }
                        ui.label(theme::small(*title));
                    });
                }
            });
            ui.end_row();

            ui.label(theme::body("Непрозрачность фона"));
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = (ctl_w - 64.0).max(80.0);
                ui.add(egui::Slider::new(&mut self.d.bg_opacity, 0..=100).show_value(false));
                ui.label(theme::dim(format!("{}%", self.d.bg_opacity)));
            });
            ui.end_row();
        });
    }

    fn draw_hotkeys(&mut self, ui: &mut Ui, label_w: f32, gap: f32, ctl_w: f32) {
        egui::Grid::new("set_keys_grid").num_columns(2).spacing([gap, 10.0]).min_col_width(label_w).show(ui, |ui| {
            for (i, title) in ["Скрыть / показать плашку", "Перевести заново без кэша"].into_iter().enumerate() {
                ui.label(theme::body(title));
                let resp = hotkey_field(ui, &self.hk[i], self.capturing == Some(i), ctl_w);
                if resp.clicked() {
                    self.capturing = Some(i);
                } else if resp.clicked_elsewhere() && self.capturing == Some(i) {
                    self.capturing = None;
                }
                ui.end_row();
            }
        });
        wrap(ui, theme::small("Щёлкните по полю и нажмите новое сочетание. Работают в любой программе."));
    }

    fn draw_general(&mut self, ui: &mut Ui) {
        ui.checkbox(&mut self.autostart, theme::body("Запускать вместе с Windows"));
        hline(ui);
        if button(ui, "Открыть папку с данными", BtnKind::Normal, true, None).clicked() {
            let _ = std::process::Command::new("explorer.exe").arg(crate::paths::data_dir()).spawn();
        }
        hline(ui);
        ui.label(theme::h2("О программе"));
        ui.label(theme::body(format!("NovelTL {}", env!("CARGO_PKG_VERSION"))));
        wrap(ui, theme::small("Перехват текста — LunaHook (GPL-3.0). Лицензии — в папке files\\licenses."));
        if button(ui, "Страница проекта", BtnKind::Link, true, None).clicked() {
            let _ = std::process::Command::new("explorer.exe").arg(PROJECT_URL).spawn();
        }
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
        let (name, line) = ("Акари", "Доброе утро. Вы сегодня снова рано.");
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

/// Пункт меню разделов: выбранный подсвечен.
fn nav_item(ui: &mut Ui, text: &str, selected: bool) -> egui::Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.card);
    }
    let color = if selected { p.accent } else { p.text };
    ui.painter().text(rect.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(15.0), color);
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
        ("Нажмите сочетание…".to_string(), p.dim)
    } else if text.is_empty() {
        ("не задано".to_string(), p.dim)
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
    fn gradient_corners_transparent() {
        let img = gradient_image(100, 40, 10.0);
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[20 * 100 + 50].a(), 255);
    }
}
