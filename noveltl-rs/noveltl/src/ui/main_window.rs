//! Главное окно (перенос `noveltl/ui/main_window.py`): список игр слева, страница игры справа.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, text::LayoutJob, Align, Color32, ColorImage, CornerRadius, FontId, Frame, Layout, Margin, RichText,
    Sense, TextureHandle, TextureOptions, Vec2,
};
use noveltl_core::achievements;
use noveltl_core::dict::{Dictionary, Segment};
use noveltl_core::i18n::{t, tf};
use noveltl_core::langdetect::Lang;
use noveltl_core::pipeline::LineResult;
use noveltl_core::profiles::{Profile, ProfileStore, ReadStats};
use noveltl_core::vndb::{self, VnHit};

use super::theme::{self, button, hline, pal, BtnKind};
use super::{lock_ctl, Shared, WizardMode, Windows};
use crate::paths::sub_data_dir;
use crate::winutil;

const NOTICE_SECS: u64 = 15;
const PROFILES_REFRESH: Duration = Duration::from_secs(1);

/// Что нужно знать о сессии, чтобы нарисовать страницу (копия, чтобы не держать замок).
struct SessSnap {
    profile: Profile,
    pid: u32,
    saved: bool,
    connected: bool,
    setup: bool,
    has_selected: bool,
    error: String,
    /// история диалога, самая новая реплика — первая
    history: Vec<LineResult>,
    /// цвет имени из глоссария для каждой реплики истории
    name_colors: Vec<Option<Color32>>,
}

struct Snap {
    session: Option<SessSnap>,
    user_hidden: bool,
    hotkey_toggle: String,
    hotkey_retranslate: String,
    translator: String,
    /// язык перевода со строчной буквы
    target_title: String,
    nickname: String,
    /// новая версия: (номер, ссылка)
    update: Option<(String, String)>,
    /// словарь для фуриганы (только при включённом режиме изучения)
    learn: Option<Arc<Mutex<Dictionary>>>,
    demo: bool,
    /// согласие на VNDB: None — ещё не спрашивали
    vndb: Option<bool>,
    /// ревизия картинок по играм (заполняется после загрузки списка профилей)
    art_revs: HashMap<String, u64>,
    art_status: String,
    art_pending: usize,
    vndb_results: Option<Result<Vec<VnHit>, String>>,
    /// сегодняшний день "YYYY-MM-DD" по местному времени
    today: String,
    /// слов добавлено в Anki (для достижений)
    anki_added: u64,
}

enum Action {
    Retry,
    ToggleOverlay,
    AskDelete(String, String),
    Report,
    Wizard(WizardMode),
    Glossary(String),
    Settings,
    SetLang(String, Lang),
    Play(Profile),
    LaunchSettings(Profile),
    Library,
    OpenLog(String),
    Anki(String, String),
    OpenUrl(String),
    DismissUpdate,
    OpenGame(String),
    VndbConsent(bool),
    /// окно «Обложка и фон»: (id игры, название)
    ArtOpen(String, String),
    /// поиск на VNDB: (id игры, запрос)
    VndbSearch(String, String),
    VndbPick(String, VnHit),
    /// выбрать файл: (id игры, true — фон, false — обложка)
    PickArt(String, bool),
    CaptureBackdrop(String),
    ClearArt(String),
    /// загрузить миниатюры результатов поиска
    Thumbs(Vec<String>),
}

fn is_japanese(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{FF66}'..='\u{FF9F}')
}

/// Оригинал реплики с фуриганой; наведение — значения из словаря, правая кнопка — в Anki.
fn furigana(
    ui: &mut egui::Ui,
    dict: &Arc<Mutex<Dictionary>>,
    cache: &mut HashMap<String, Vec<Segment>>,
    res: &LineResult,
    actions: &mut Vec<Action>,
) {
    let p = pal();
    let prefer_ru = noveltl_core::i18n::lang() == noveltl_core::i18n::UiLang::Ru;
    let text = res.src_text.as_str();
    if cache.len() > 200 {
        cache.clear();
    }
    let segs = cache
        .entry(text.to_string())
        .or_insert_with(|| dict.lock().unwrap_or_else(|e| e.into_inner()).segments(text))
        .clone();
    let lookup = |pos: usize| dict.lock().unwrap_or_else(|e| e.into_inner()).lookup_at(text, pos, prefer_ru);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(0.0, 0.0);
        for seg in &segs {
            let resp = ui
                .vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let rt = match &seg.reading {
                        Some(r) => RichText::new(r.as_str()).size(10.5).color(p.dim),
                        None => RichText::new(" ").size(10.5),
                    };
                    ui.add(egui::Label::new(rt).selectable(false));
                    ui.add(egui::Label::new(theme::jp(seg.text.as_str())).selectable(false).sense(Sense::click()))
                })
                .inner;
            if !seg.text.chars().any(is_japanese) {
                continue;
            }
            resp.on_hover_ui(|ui| match lookup(seg.start) {
                Some(m) => {
                    ui.label(theme::h2(m.form.as_str()));
                    if let Some(r) = m.entry.readings.first().filter(|r| **r != m.form) {
                        ui.label(theme::body(r.as_str()));
                    }
                    for g in m.entry.glosses.iter().take(5) {
                        ui.label(theme::body(format!("• {g}")));
                    }
                    ui.label(theme::small(t("Правая кнопка мыши — добавить в Anki")));
                }
                None => {
                    ui.label(theme::body(t("Нет в словаре")));
                }
            })
            .context_menu(|ui| {
                if let Some(m) = lookup(seg.start) {
                    if ui.button(t("Добавить в Anki")).clicked() {
                        let mut front = m.form.clone();
                        if let Some(r) = m.entry.readings.first().filter(|r| **r != m.form) {
                            front.push_str(&format!("【{r}】"));
                        }
                        let back = format!(
                            "{}<br><br>{}<br>{}",
                            m.entry.glosses.iter().take(5).cloned().collect::<Vec<_>>().join("<br>"),
                            text,
                            res.text
                        );
                        actions.push(Action::Anki(front, back));
                        ui.close_menu();
                    }
                } else {
                    ui.label(theme::body(t("Нет в словаре")));
                }
            });
        }
    });
}

struct LaunchEdit {
    id: String,
    name: String,
    exe: String,
    cmd: String,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// «Играли: вчера» и подобное.
fn played_text(last: u64, now: u64) -> String {
    let when = if last == 0 {
        t("ещё не запускали").to_string()
    } else {
        match now.saturating_sub(last) / 86400 {
            0 => t("сегодня").to_string(),
            1 => t("вчера").to_string(),
            n => tf("{} дн. назад", &[&n]),
        }
    };
    tf("Играли: {}", &[&when])
}

/// Значок игры: картинка из exe или первая буква названия на цветном квадрате.
fn paint_game_icon(painter: &egui::Painter, rect: egui::Rect, tex: Option<egui::TextureId>, name: &str) {
    if let Some(id) = tex {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(id, rect, uv, Color32::WHITE);
        return;
    }
    let hue = (name.chars().map(|c| c as u32).sum::<u32>() % 360) as f32 / 360.0;
    let fill: Color32 = egui::ecolor::Hsva::new(hue, 0.45, 0.55, 1.0).into();
    painter.rect_filled(rect, CornerRadius::same((rect.height() / 5.0) as u8), fill);
    let letter = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().to_string();
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::new(rect.height() * 0.5, theme::bold_family()),
        Color32::WHITE,
    );
}

/// Жёлтая плашка уведомления.
fn notice_box(ui: &mut egui::Ui, text: &str) {
    let p = pal();
    Frame::new().fill(p.notice_bg).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(12, 8)).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(14.0).color(p.warn)).wrap_mode(egui::TextWrapMode::Wrap));
        },
    );
}

/// 12345 → "12 345".
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Текстуры картинок игры (обложка, размытый фон, заглушка) и акцент страницы.
#[derive(Default)]
struct ArtEntry {
    loaded: bool,
    rev: u64,
    cover_name: String,
    back_name: String,
    cover: Option<TextureHandle>,
    accent: Option<Color32>,
    back: Option<TextureHandle>,
    back_done: bool,
}

#[derive(Clone)]
struct ArtView {
    cover: Option<TextureHandle>,
    placeholder: TextureHandle,
    back: Option<TextureHandle>,
    accent: Option<Color32>,
}

/// Окно «Обложка и фон».
struct ArtEdit {
    id: String,
    name: String,
    query: String,
    /// миниатюры результатов поиска: url -> картинка (None — не загрузилась); пишут фоновые потоки
    thumbs: Arc<Mutex<HashMap<String, Option<(u32, u32, Vec<u8>)>>>>,
    tex: HashMap<String, TextureHandle>,
    requested: std::collections::HashSet<String>,
}

/// Одна строка текста с «…» на конце, если не помещается.
fn one_line_job(text: String, font: FontId, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::simple(text, font, color, width.max(10.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    job
}

/// Обложка в прямоугольнике (с обрезкой по краям до пропорций) или градиентная заглушка с первой буквой названия.
fn paint_cover(ui: &egui::Ui, view: &ArtView, rect: egui::Rect, r: CornerRadius, name: &str) {
    if let Some(t) = &view.cover {
        let uv = theme::fit_uv(t.size_vec2(), rect.width() / rect.height());
        egui::Image::new((t.id(), rect.size())).uv(uv).corner_radius(r).paint_at(ui, rect);
        return;
    }
    egui::Image::new((view.placeholder.id(), rect.size())).corner_radius(r).paint_at(ui, rect);
    let letter = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().to_string();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::new(rect.height() * 0.3, theme::bold_family()),
        Color32::from_white_alpha(225),
    );
}

pub struct MainWindow {
    store: ProfileStore,
    profiles: Vec<Profile>,
    profiles_at: Option<Instant>,
    selected: Option<String>,
    last_session_id: Option<String>,
    notice: Option<(String, Instant)>,
    confirm_delete: Option<(String, String)>,
    pending_delete: Option<String>,
    messages: Vec<(String, String)>,
    logo: Option<TextureHandle>,
    logo_big: Option<TextureHandle>,
    /// сегменты фуриганы по тексту реплики
    seg_cache: HashMap<String, Vec<Segment>>,
    /// открыта страница игры (а не библиотека)
    open_game: bool,
    /// значки exe по пути (None — значка нет)
    icons: HashMap<String, Option<TextureHandle>>,
    /// игру запустили кнопкой: (id, название, до какого времени ждём подключения)
    launching: Option<(String, String, Instant)>,
    launch_edit: Option<LaunchEdit>,
    pending_launch: Option<(String, String, String)>,
    /// поиск по играм в библиотеке
    search: String,
    /// раскрыта ли сетка достижений в библиотеке
    achievements_open: bool,
    art_cache: HashMap<String, ArtEntry>,
    /// градиентные заглушки обложек по названию
    ph_cache: HashMap<String, TextureHandle>,
    art_edit: Option<ArtEdit>,
    /// какая страница открыта (для плавного появления) и когда
    page_key: String,
    page_since: Instant,
}

impl MainWindow {
    pub fn new() -> Self {
        MainWindow {
            store: ProfileStore::new(sub_data_dir("profiles")),
            profiles: Vec::new(),
            profiles_at: None,
            selected: None,
            last_session_id: None,
            notice: None,
            confirm_delete: None,
            pending_delete: None,
            messages: Vec::new(),
            logo: None,
            logo_big: None,
            seg_cache: HashMap::new(),
            open_game: false,
            icons: HashMap::new(),
            launching: None,
            launch_edit: None,
            pending_launch: None,
            search: String::new(),
            achievements_open: false,
            art_cache: HashMap::new(),
            ph_cache: HashMap::new(),
            art_edit: None,
            page_key: String::new(),
            page_since: Instant::now(),
        }
    }

    fn icon_tex(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureId> {
        if path.is_empty() {
            return None;
        }
        let e = self.icons.entry(path.to_string()).or_insert_with(|| {
            winutil::exe_icon(path).map(|(w, h, px)| {
                ctx.load_texture(format!("exe:{path}"), ColorImage::from_rgba_unmultiplied([w, h], &px), TextureOptions::LINEAR)
            })
        });
        e.as_ref().map(|t| t.id())
    }

    /// Окошко с сообщением и кнопкой «OK» (как QMessageBox).
    pub fn add_message(&mut self, title: &str, text: &str) {
        self.messages.push((title.to_string(), text.to_string()));
    }

    /// Жёлтая плашка уведомления на 15 секунд.
    pub fn show_notice(&mut self, text: &str) {
        self.notice = Some((text.to_string(), Instant::now() + Duration::from_secs(NOTICE_SECS)));
        self.profiles_at = None;
    }

    fn snapshot(shared: &Shared) -> Snap {
        let c = lock_ctl(&shared.controller);
        Snap {
            session: c.session.as_ref().map(|s| SessSnap {
                profile: s.profile.clone(),
                pid: s.pid,
                saved: s.saved,
                connected: s.connected,
                setup: s.setup,
                has_selected: s.selected.is_some(),
                error: s.error.clone(),
                history: s.history.iter().rev().cloned().collect(),
                name_colors: s
                    .history
                    .iter()
                    .rev()
                    .map(|r| c.name_color(&r.name).map(|h| theme::parse_hex(&h, Color32::WHITE)))
                    .collect(),
            }),
            user_hidden: c.user_hidden,
            hotkey_toggle: c.settings.hotkey_toggle.clone(),
            hotkey_retranslate: c.settings.hotkey_retranslate.clone(),
            translator: c.translator_status(),
            target_title: c.settings.target_lang.title_in_text(),
            nickname: c.settings.nickname.clone(),
            update: c.update_available.clone(),
            learn: if c.settings.learn_mode { c.dict.clone() } else { None },
            demo: c.is_demo(),
            vndb: c.settings.vndb,
            art_revs: HashMap::new(),
            art_status: c.art_status.clone(),
            art_pending: c.art_pending,
            vndb_results: c.vndb_results.clone(),
            today: crate::controller::local_time_text("%Y-%m-%d"),
            anki_added: c.settings.anki_added,
        }
    }

    fn ensure_logos(&mut self, ctx: &egui::Context) {
        if self.logo.is_none() {
            let img = ColorImage::from_rgba_unmultiplied([64, 64], &theme::icon_rgba(64));
            self.logo = Some(ctx.load_texture("logo32", img, TextureOptions::LINEAR));
        }
        if self.logo_big.is_none() {
            let img = ColorImage::from_rgba_unmultiplied([144, 144], &theme::icon_rgba(144));
            self.logo_big = Some(ctx.load_texture("logo72", img, TextureOptions::LINEAR));
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        shared: &Shared,
        windows: &mut Windows,
        avatar: Option<&TextureHandle>,
    ) {
        self.ensure_logos(ctx);
        let mut snap = Self::snapshot(shared);

        if self.profiles_at.map_or(true, |t| t.elapsed() >= PROFILES_REFRESH) {
            self.profiles = self.store.all();
            self.profiles_at = Some(Instant::now());
        }
        // профили на диске + профиль текущей сессии, если он уже сохранён
        let mut profiles = self.profiles.clone();
        if let Some(s) = snap.session.as_ref().filter(|s| s.saved) {
            match profiles.iter_mut().find(|p| p.id == s.profile.id) {
                Some(p) => *p = s.profile.clone(),
                None => profiles.push(s.profile.clone()),
            }
        }
        {
            let c = lock_ctl(&shared.controller);
            snap.art_revs = profiles.iter().map(|p| (p.id.clone(), c.art_rev_of(&p.id))).collect();
        }
        // текстуры удалённых игр не держим
        self.art_cache.retain(|id, _| profiles.iter().any(|p| p.id == *id));
        let session_id = snap.session.as_ref().map(|s| s.profile.id.clone());
        if let Some(s) = snap.session.as_ref().filter(|s| s.saved) {
            if self.last_session_id.as_deref() != Some(s.profile.id.as_str()) {
                self.selected = Some(s.profile.id.clone()); // игра только что подключилась — показываем её
            }
        }
        if self.last_session_id != session_id {
            self.open_game = false; // игра запустилась или закрылась — к библиотеке/странице запущенной
            if self.launching.as_ref().is_some_and(|l| session_id.as_deref() == Some(l.0.as_str())) {
                self.launching = None;
            }
        }
        self.last_session_id = session_id.clone();
        if !self.selected.as_ref().is_some_and(|id| profiles.iter().any(|p| &p.id == id)) {
            self.selected = profiles.first().map(|p| p.id.clone());
        }

        if let Some((_, until)) = &self.notice {
            if Instant::now() >= *until {
                self.notice = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        if self.launching.as_ref().is_some_and(|l| Instant::now() >= l.2) {
            self.launching = None;
        }
        if self.launching.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        let mut actions: Vec<Action> = Vec::new();
        let p = pal();

        // ---------- левая панель ----------
        egui::SidePanel::left("sidebar")
            .exact_width(264.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.panel).inner_margin(Margin { left: 16, right: 16, top: 18, bottom: 16 }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                ui.horizontal(|ui| {
                    // аватар (круглый) или логотип
                    if let Some(a) = avatar {
                        ui.add(egui::Image::new((a.id(), Vec2::splat(40.0))).corner_radius(CornerRadius::same(20)));
                    } else if let Some(t) = &self.logo {
                        ui.image((t.id(), Vec2::splat(32.0)));
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let nick = snap.nickname.trim();
                        if nick.is_empty() {
                            ui.add(egui::Label::new(theme::h1(super::APP_NAME)).truncate());
                        } else {
                            ui.add(egui::Label::new(theme::h2(nick)).truncate());
                            ui.add(egui::Label::new(theme::small(super::APP_NAME)).truncate());
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(theme::step(t("МОИ ИГРЫ")));
                let list_h = (ui.available_height() - 96.0).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt("games")
                    .max_height(list_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for pr in &profiles {
                            let running = session_id.as_deref() == Some(pr.id.as_str());
                            let is_sel = self.selected.as_deref() == Some(pr.id.as_str());
                            let (rect, resp) =
                                ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
                            let ha = theme::anim(ctx, egui::Id::new(("side_row", &pr.id)), resp.hovered(), 0.12);
                            let fill = if is_sel { p.selected } else { p.card.gamma_multiply(ha) };
                            ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
                            let icon = self.icon_tex(ctx, &pr.exe_path);
                            let ir = egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 10.0, rect.center().y - 9.0),
                                Vec2::splat(18.0),
                            );
                            paint_game_icon(ui.painter(), ir, icon, &pr.name);
                            let text = format!("{}  {}", if running { "●" } else { "○" }, pr.name);
                            let mut job = LayoutJob::simple(
                                text,
                                FontId::proportional(14.0),
                                p.text,
                                (rect.width() - 48.0).max(10.0),
                            );
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            job.wrap.overflow_character = Some('…');
                            let galley = ui.painter().layout_job(job);
                            let pos = egui::pos2(rect.left() + 36.0, rect.center().y - galley.size().y / 2.0);
                            ui.painter().galley(pos, galley, p.text);
                            if resp.clicked() {
                                self.selected = Some(pr.id.clone());
                                self.open_game = true;
                            }
                            resp.on_hover_text(if running { t("Игра запущена") } else { t("Игра не запущена") });
                        }
                    });
                let w = ui.available_width();
                if button(ui, t("+  Добавить игру"), BtnKind::Primary, true, Some(w)).clicked() {
                    actions.push(Action::Wizard(WizardMode::New));
                }
                if button(ui, t("Настройки"), BtnKind::Normal, true, Some(w)).clicked() {
                    actions.push(Action::Settings);
                }
            });

        let current: Option<Profile> =
            self.selected.as_ref().and_then(|id| profiles.iter().find(|p| &p.id == id)).cloned();

        // страница игры — если открыта из списка или какая-то игра подключена; иначе библиотека
        let show_game = current.is_some() && (self.open_game || snap.session.is_some());

        // ---------- нижняя часть страницы игры ----------
        if let Some(profile) = current.as_ref().filter(|_| show_game) {
            let s = snap.session.as_ref().filter(|s| s.profile.id == profile.id);
            let ready = s.is_some_and(|s| s.connected && !s.setup);
            let can_pos = ready && s.is_some_and(|s| s.has_selected);
            egui::TopBottomPanel::bottom("game_bottom")
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 6, bottom: 18 }))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    let tip = t("Сначала запустите игру");
                    ui.columns(2, |cols| {
                        let w = cols[0].available_width();
                        let r = button(&mut cols[0], t("Найти текст заново"), BtnKind::Normal, ready, Some(w));
                        if r.clicked() {
                            actions.push(Action::Wizard(WizardMode::Hook));
                        }
                        if !ready {
                            r.on_hover_text(tip);
                        }
                        let w = cols[1].available_width();
                        let r = button(&mut cols[1], t("Расположение плашки"), BtnKind::Normal, can_pos, Some(w));
                        if r.clicked() {
                            actions.push(Action::Wizard(WizardMode::Position));
                        }
                        if !can_pos {
                            r.on_hover_text(tip);
                        }
                    });
                    ui.columns(2, |cols| {
                        let w = cols[0].available_width();
                        if button(&mut cols[0], t("Глоссарий имён"), BtnKind::Normal, true, Some(w)).clicked() {
                            actions.push(Action::Glossary(profile.id.clone()));
                        }
                        let w = cols[1].available_width();
                        let r = button(&mut cols[1], t("Отчёт для настройки"), BtnKind::Normal, s.is_some(), Some(w));
                        if r.clicked() {
                            actions.push(Action::Report);
                        }
                        if s.is_none() {
                            r.on_hover_text(tip);
                        }
                    });
                    ui.add_space(2.0);
                    hline(ui);
                    ui.add_space(2.0);
                    // horizontal ограничивает высоту строки: без него right_to_left(Center) занимает всю
                    // высоту панели, и после уведомления панель так и остаётся высокой
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if button(ui, t("Удалить игру"), BtnKind::Link, true, None).clicked() {
                                actions.push(Action::AskDelete(profile.id.clone(), profile.name.clone()));
                            }
                            ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                                let keys = format!(
                                    "{} — {}, {} — {}",
                                    snap.hotkey_toggle,
                                    t("скрыть плашку"),
                                    snap.hotkey_retranslate,
                                    t("перевести заново")
                                );
                                ui.add(
                                    egui::Label::new(theme::small(tf("Переводчик: {}\n{}", &[&snap.translator, &keys])))
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                            });
                        });
                    });
                    if let Some((text, _)) = &self.notice {
                        ui.add_space(4.0);
                        notice_box(ui, text);
                    }
                });
        }

        // ---------- баннер обновления ----------
        if let Some((ver, url)) = &snap.update {
            egui::TopBottomPanel::top("update_banner")
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 14, bottom: 0 }))
                .show(ctx, |ui| {
                    let r = theme::card_frame().inner_margin(Margin { left: 20, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(theme::body(tf("Вышла версия {}", &[ver])));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if button(ui, t("Скачать"), BtnKind::Primary, true, None).clicked() {
                                    actions.push(Action::OpenUrl(url.clone()));
                                }
                                if button(ui, t("Не сейчас"), BtnKind::Link, true, None).clicked() {
                                    actions.push(Action::DismissUpdate);
                                }
                            });
                        });
                    });
                    let rc = r.response.rect;
                    let strip = egui::Rect::from_min_max(rc.min, egui::pos2(rc.min.x + 6.0, rc.max.y));
                    ui.painter().rect_filled(
                        strip,
                        CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 },
                        p.accent,
                    );
                });
        }

        // ---------- центральная часть ----------
        let view = current.as_ref().filter(|_| show_game).map(|pr| self.art_view(ctx, pr, snap.art_revs.get(&pr.id).copied().unwrap_or(0), true));
        // смена страницы — плавное появление (в проверочном режиме без анимации)
        let page_key = current.as_ref().filter(|_| show_game).map(|pr| pr.id.clone()).unwrap_or_default();
        if page_key != self.page_key {
            self.page_key = page_key;
            self.page_since = Instant::now();
        }
        let fade = if winutil::test_area().is_some() { 1.0 } else { (self.page_since.elapsed().as_secs_f32() / 0.15).min(1.0) };
        if fade < 1.0 {
            ctx.request_repaint();
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 8 }))
            .show(ctx, |ui| {
                ui.set_opacity(fade);
                match &current {
                    Some(profile) if show_game => {
                        let s = snap.session.as_ref().filter(|s| s.profile.id == profile.id);
                        let launching = self.launching.as_ref().map(|l| l.0.as_str());
                        if let Some(view) = &view {
                            Self::game_page(ui, profile, s, &snap, view, launching, &mut self.seg_cache, &mut actions);
                        }
                    }
                    _ if profiles.is_empty() => self.empty_page(ui, &mut actions),
                    _ => self.library(ui, ctx, &profiles, &snap, &mut actions),
                }
            });

        if let Some(id) = windows.launch.take() {
            // только для снимков: окно «Настройки запуска» без клика
            if let Some(pr) = profiles.iter().find(|p| p.id == id) {
                let mut pr = pr.clone();
                pr.exe_path = r"C:\Games\Kirinomori Lantern\Kirinomori.exe".to_string(); // вымышленный путь: на снимках не должно быть имени пользователя
                actions.push(Action::LaunchSettings(pr));
            }
        }
        if let Some(id) = windows.art.take() {
            // только для снимков: окно «Обложка и фон» без клика (пустой id — закрыть)
            if id.is_empty() {
                self.art_edit = None;
            } else if let Some(pr) = profiles.iter().find(|p| p.id == id) {
                actions.push(Action::ArtOpen(pr.id.clone(), pr.name.clone()));
            }
        }
        self.modals(ctx, &snap, &mut actions);
        self.run_actions(actions, shared, windows, &snap, ctx);
    }

    /// Картинки игры из data/art: обложка сразу, размытый фон — только когда нужен (страница игры).
    fn art_view(&mut self, ctx: &egui::Context, pr: &Profile, rev: u64, want_back: bool) -> ArtView {
        let placeholder = self
            .ph_cache
            .entry(pr.name.clone())
            .or_insert_with(|| ctx.load_texture(format!("ph:{}", pr.name), theme::placeholder_image(&pr.name), TextureOptions::LINEAR))
            .clone();
        let e = self.art_cache.entry(pr.id.clone()).or_default();
        if !e.loaded || e.rev != rev || e.cover_name != pr.cover || e.back_name != pr.backdrop {
            *e = ArtEntry { loaded: true, rev, cover_name: pr.cover.clone(), back_name: pr.backdrop.clone(), ..ArtEntry::default() };
            if !pr.cover.is_empty() {
                if let Ok((w, h, px)) = super::avatar::decode_image(&crate::controller::art_dir(&pr.id).join(&pr.cover)) {
                    let (w, h, px) = theme::shrink_rgba(w, h, &px, 400);
                    e.accent = theme::dominant_accent(w, h, &px);
                    let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                    e.cover = Some(ctx.load_texture(format!("cover:{}", pr.id), img, TextureOptions::LINEAR));
                }
            }
        }
        if want_back && !e.back_done {
            e.back_done = true;
            let name = if pr.backdrop.is_empty() { &pr.cover } else { &pr.backdrop };
            if !name.is_empty() {
                if let Ok((w, h, px)) = super::avatar::decode_image(&crate::controller::art_dir(&pr.id).join(name)) {
                    e.back = Some(ctx.load_texture(format!("back:{}", pr.id), theme::blurred_backdrop(w, h, &px), TextureOptions::LINEAR));
                }
            }
        }
        ArtView { cover: e.cover.clone(), placeholder, back: e.back.clone(), accent: e.accent }
    }

    /// «Библиотека»: поиск, календарь чтения по всем играм и сетка обложек 2:3.
    fn library(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        profiles: &[Profile],
        snap: &Snap,
        actions: &mut Vec<Action>,
    ) {
        let p = pal();
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 12.0);
        ui.horizontal(|ui| {
            ui.label(theme::h1(t("Библиотека")));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(t("Поиск по играм")).desired_width(220.0));
            });
        });
        if snap.vndb.is_none() && !snap.demo {
            theme::card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(theme::body(t(
                        "Загрузить обложки, картинки и портреты персонажей с VNDB? Программа отправит на vndb.org названия ваших игр.",
                    )))
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.horizontal(|ui| {
                    if button(ui, t("Да"), BtnKind::Primary, true, Some(80.0)).clicked() {
                        actions.push(Action::VndbConsent(true));
                    }
                    if button(ui, t("Нет"), BtnKind::Normal, true, Some(80.0)).clicked() {
                        actions.push(Action::VndbConsent(false));
                    }
                });
            });
        }
        if let Some((text, _)) = self.notice.clone() {
            notice_box(ui, &text);
        }
        if let Some((_, name, _)) = &self.launching {
            ui.label(RichText::new(tf("Запускаю {}…", &[name])).size(14.0).color(p.warn));
        }
        const GAP: f32 = 14.0;
        let now = unix_now();
        let q = self.search.trim().to_lowercase();
        let shown: Vec<&Profile> = profiles.iter().filter(|pr| q.is_empty() || pr.name.to_lowercase().contains(&q)).collect();
        let running = snap.session.as_ref().map(|s| s.profile.id.as_str());
        let launching = self.launching.as_ref().map(|l| l.0.clone());
        egui::ScrollArea::vertical().id_salt("library").auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width() - 4.0;
            // общий календарь чтения по всем играм (компактный, 12 недель)
            let mut all = ReadStats::default();
            for pr in profiles {
                for (day, n) in &pr.stats.daily {
                    *all.daily.entry(day.clone()).or_default() += n;
                }
            }
            theme::card_frame().show(ui, |ui| {
                ui.set_width(w - 34.0); // иначе перенос значков растягивает карточку за окно
                let totals = achievements::Totals {
                    lines: profiles.iter().map(|pr| pr.stats.lines).sum(),
                    best_streak: u64::from(all.best_streak()),
                    anki: snap.anki_added,
                    games: profiles.iter().filter(|pr| pr.stats.lines > 0).count() as u64,
                };
                let badges = achievements::badges(totals);
                let mut open = self.achievements_open;
                let mut side = false;
                ui.horizontal(|ui| {
                    theme::reading_calendar(ui, &all, &snap.today, 12, p.accent, false, false);
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(theme::body(tf("Реплик сегодня: {}", &[&all.daily.get(&snap.today).copied().unwrap_or(0)])));
                        ui.label(theme::small(tf("Дней подряд: {}", &[&all.streak(&snap.today)])));
                    });
                    // медальоны (7 x 38 + зазоры) и строка «Следующее…» справа, если хватает ширины
                    if ui.available_width() > 400.0 {
                        side = true;
                        ui.add_space(12.0);
                        ui.vertical(|ui| Self::achievements_strip(ui, &badges, &mut open));
                    }
                });
                if !side {
                    ui.add_space(4.0);
                    Self::achievements_strip(ui, &badges, &mut open);
                }
                if open {
                    ui.add_space(6.0);
                    Self::achievements_grid(ui, &badges);
                }
                self.achievements_open = open;
            });
            let cols = (((w + GAP) / (170.0 + GAP)) as usize).max(1);
            let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).min(190.0);
            ui.spacing_mut().item_spacing = Vec2::new(GAP, GAP);
            let total = shown.len() + 1; // последняя карточка — «Добавить игру»
            let mut idx = 0;
            while idx < total {
                ui.horizontal_top(|ui| {
                    for _ in 0..cols {
                        if idx >= total {
                            break;
                        }
                        if let Some(pr) = shown.get(idx) {
                            let busy = launching.as_deref() == Some(pr.id.as_str());
                            self.game_card(ui, ctx, pr, snap.art_revs.get(&pr.id).copied().unwrap_or(0), cw, running == Some(pr.id.as_str()), busy, now, actions);
                        } else {
                            Self::add_card(ui, ctx, cw, actions);
                        }
                        idx += 1;
                    }
                });
            }
        });
    }

    const ACH_ICONS: [char; 7] = ['\u{E7FC}', '\u{E8A5}', '\u{E8A5}', '\u{E8A5}', '\u{E787}', '\u{E787}', '\u{E8A4}'];

    /// Медальон достижения: круг радиуса `r` с глифом (или короткой меткой без шрифта значков) в центре `c`.
    fn medal(pt: &egui::Painter, c: egui::Pos2, r: f32, icon: char, b: &achievements::Badge) {
        let p = pal();
        let (fill, ring, fg) = if b.earned() { (p.accent, p.accent, p.on_accent) } else { (Color32::TRANSPARENT, p.border, p.dim) };
        pt.circle(c, r, fill, egui::Stroke::new(if r > 20.0 { 2.0_f32 } else { 1.5 }, ring));
        let big = r > 24.0;
        if theme::has_icon_font() {
            let off = if big { 8.0 } else { 0.0 };
            pt.text(c - Vec2::new(0.0, off), egui::Align2::CENTER_CENTER, icon, egui::FontId::new(r * 0.8, theme::icon_family()), fg);
            if big {
                pt.text(c + Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, b.short, egui::FontId::new(12.0, theme::bold_family()), fg);
            }
        } else {
            pt.text(c, egui::Align2::CENTER_CENTER, b.short, egui::FontId::new(r * 0.7, theme::bold_family()), fg);
        }
    }

    /// Подсказка к медальону: название, за что, прогресс.
    fn medal_tip(b: &achievements::Badge) -> String {
        let prog = if b.streak {
            tf("рекорд: {} / {}", &[&thousands(b.have), &thousands(b.need)])
        } else {
            format!("{} / {}", thousands(b.have), thousands(b.need))
        };
        format!("{}\n{}\n{}", t(b.name), t(b.hint), prog)
    }

    /// Свёрнутый блок достижений: строка медальонов, «Следующее…» и ссылка «Все достижения»/«Свернуть».
    fn achievements_strip(ui: &mut egui::Ui, badges: &[achievements::Badge], open: &mut bool) {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        ui.horizontal(|ui| {
            for (i, b) in badges.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::hover());
                Self::medal(ui.painter(), rect.center(), 18.0, Self::ACH_ICONS[i], b);
                resp.on_hover_text(Self::medal_tip(b));
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(b) = achievements::next_badge(badges) {
                let line = if b.streak {
                    tf("Следующее: {} — рекорд: {} из {}", &[&t(b.name), &thousands(b.have), &thousands(b.need)])
                } else {
                    tf("Следующее: {} — осталось {}", &[&t(b.name), &thousands(b.need - b.have)])
                };
                ui.label(theme::small(line));
            }
            if button(ui, if *open { t("Свернуть") } else { t("Все достижения") }, BtnKind::Link, true, None).clicked() {
                *open = !*open;
            }
        });
    }

    /// Раскрытая сетка карточек-значков (по ширине, с переносом).
    fn achievements_grid(ui: &mut egui::Ui, badges: &[achievements::Badge]) {
        let p = pal();
        const GAP: f32 = 8.0;
        let avail = ui.available_width();
        let cols = (((avail + GAP) / (84.0 + GAP)) as usize).clamp(1, badges.len());
        let tw = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).min(110.0);
        ui.spacing_mut().item_spacing = Vec2::new(GAP, GAP);
        for (row, chunk) in badges.chunks(cols).enumerate() {
            let rw = chunk.len() as f32 * tw + (chunk.len() as f32 - 1.0) * GAP;
            ui.horizontal(|ui| {
                ui.add_space(((avail - rw) / 2.0).max(0.0));
                for (i, b) in chunk.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(tw, 104.0), Sense::hover());
                    let earned = b.earned();
                    let pt = ui.painter();
                    pt.rect(rect, 10.0, Color32::TRANSPARENT, egui::Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
                    Self::medal(pt, egui::pos2(rect.center().x, rect.top() + 8.0 + 22.0), 22.0, Self::ACH_ICONS[row * cols + i], b);
                    let mut job = egui::text::LayoutJob::simple(
                        t(b.name).to_string(),
                        egui::FontId::proportional(12.0),
                        if earned { p.text } else { p.dim },
                        tw - 12.0,
                    );
                    job.halign = Align::Center;
                    let g = pt.layout_job(job);
                    pt.galley(egui::pos2(rect.center().x, rect.top() + 56.0), g, p.text);
                    if !earned {
                        let bar = egui::Rect::from_min_size(egui::pos2(rect.left() + 10.0, rect.bottom() - 12.0), Vec2::new(tw - 20.0, 4.0));
                        pt.rect_filled(bar, 2.0, p.border);
                        let f = (b.have as f32 / b.need as f32).clamp(0.0, 1.0);
                        pt.rect_filled(egui::Rect::from_min_size(bar.min, Vec2::new(bar.width() * f, 4.0)), 2.0, p.accent);
                    }
                    resp.on_hover_text(Self::medal_tip(b));
                }
            });
        }
    }

    /// Карточка игры: обложка 2:3; при наведении приподнимается и показывает «Играть».
    #[allow(clippy::too_many_arguments)]
    fn game_card(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        pr: &Profile,
        rev: u64,
        cw: f32,
        running: bool,
        busy: bool,
        now: u64,
        actions: &mut Vec<Action>,
    ) {
        let p = pal();
        let view = self.art_view(ctx, pr, rev, false);
        let ch = cw * 1.5;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, ch + 62.0), Sense::click());
        let id = egui::Id::new(("libcard", &pr.id));
        let menu_id = id.with("menu");
        let hover = ui.rect_contains_pointer(rect) || ui.memory(|m| m.is_popup_open(menu_id));
        let h = theme::anim(ctx, id, hover, 0.12);
        let base = egui::Rect::from_min_size(rect.min, Vec2::new(cw, ch));
        let cover = egui::Rect::from_center_size(base.center() - Vec2::new(0.0, 4.0 * h), base.size() * (1.0 + 0.02 * h));
        let r = CornerRadius::same(8);
        ui.painter().add(
            egui::epaint::Shadow {
                offset: [0, (2.0 + 4.0 * h) as i8],
                blur: (6.0 + 10.0 * h) as u8,
                spread: 0,
                color: Color32::from_black_alpha((50.0 + 70.0 * h) as u8),
            }
            .as_shape(cover, r),
        );
        paint_cover(ui, &view, cover, r, &pr.name);
        ui.painter().rect_stroke(cover, r, egui::Stroke::new(1.0_f32, p.border), egui::StrokeKind::Inside);
        if running {
            let font = FontId::new(11.0, theme::bold_family());
            let g = ui.painter().layout_no_wrap(t("играю").to_string(), font.clone(), Color32::WHITE);
            let badge = egui::Rect::from_min_size(cover.min + Vec2::splat(8.0), Vec2::new(g.size().x + 14.0, 20.0));
            ui.painter().rect_filled(badge, CornerRadius::same(10), theme::mix(p.ok, Color32::BLACK, 0.4));
            ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, t("играю"), font, Color32::WHITE);
        }
        if h > 0.01 {
            // «Играть» внизу обложки и меню «···» справа вверху
            let pb = egui::Rect::from_min_size(
                egui::pos2(cover.left() + 10.0, cover.bottom() - 42.0),
                Vec2::new(cover.width() - 20.0, 32.0),
            );
            let play = ui.interact(pb, id.with("play"), Sense::click());
            let fill = if busy { p.primary_disabled } else if play.hovered() { p.accent_hover } else { p.accent };
            ui.painter().rect_filled(pb, CornerRadius::same(8), fill.gamma_multiply(h));
            ui.painter().text(
                pb.center(),
                egui::Align2::CENTER_CENTER,
                t("Играть"),
                FontId::new(14.0, theme::bold_family()),
                p.on_accent.gamma_multiply(h),
            );
            if play.clicked() && !busy {
                actions.push(Action::Play(pr.clone()));
            }
            let mb = egui::Rect::from_min_size(egui::pos2(cover.right() - 34.0, cover.top() + 8.0), Vec2::splat(26.0));
            let more = ui.interact(mb, id.with("more"), Sense::click());
            ui.painter().circle_filled(mb.center(), 13.0, Color32::from_black_alpha((170.0 * h) as u8));
            ui.painter().text(
                mb.center(),
                egui::Align2::CENTER_CENTER,
                "···",
                FontId::new(14.0, theme::bold_family()),
                Color32::WHITE.gamma_multiply(h),
            );
            if more.clicked() {
                ui.memory_mut(|m| m.toggle_popup(menu_id));
            }
            egui::popup_below_widget(ui, menu_id, &more, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                ui.set_min_width(170.0);
                if ui.button(t("Обложка и фон…")).clicked() {
                    actions.push(Action::ArtOpen(pr.id.clone(), pr.name.clone()));
                }
                if ui.button(t("Настройки запуска")).clicked() {
                    actions.push(Action::LaunchSettings(pr.clone()));
                }
                if ui.button(t("Глоссарий имён")).clicked() {
                    actions.push(Action::Glossary(pr.id.clone()));
                }
                if ui.button(t("Удалить игру")).clicked() {
                    actions.push(Action::AskDelete(pr.id.clone(), pr.name.clone()));
                }
            });
        }
        if resp.clicked() {
            actions.push(Action::OpenGame(pr.id.clone()));
        }
        // подписи под обложкой
        let pt = ui.painter();
        let title = pt.layout_job(one_line_job(pr.name.clone(), FontId::new(14.0, theme::bold_family()), p.text, cw));
        pt.galley(egui::pos2(rect.left(), rect.top() + ch + 8.0), title, p.text);
        let played = pt.layout_job(one_line_job(played_text(pr.last_played, now), FontId::proportional(11.5), p.dim, cw));
        pt.galley(egui::pos2(rect.left(), rect.top() + ch + 28.0), played, p.dim);
        let lines = pt.layout_job(one_line_job(tf("Реплик: {}", &[&thousands(pr.stats.lines)]), FontId::proportional(11.5), p.dim, cw));
        pt.galley(egui::pos2(rect.left(), rect.top() + ch + 44.0), lines, p.dim);
    }

    /// Последняя карточка сетки: пунктирная рамка и «+ Добавить игру».
    fn add_card(ui: &mut egui::Ui, ctx: &egui::Context, cw: f32, actions: &mut Vec<Action>) {
        let p = pal();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, cw * 1.5), Sense::click());
        let h = theme::anim(ctx, egui::Id::new("libcard_add"), resp.hovered(), 0.12);
        ui.painter().rect_filled(rect, CornerRadius::same(8), p.card.gamma_multiply(h));
        let b = rect.shrink(1.0);
        let pts = [b.left_top(), b.right_top(), b.right_bottom(), b.left_bottom(), b.left_top()];
        let color = theme::mix(p.border, p.accent, h);
        ui.painter().extend(egui::Shape::dashed_line(&pts, egui::Stroke::new(1.5_f32, color), 7.0, 5.0));
        let c = rect.center();
        ui.painter().text(c - Vec2::new(0.0, 14.0), egui::Align2::CENTER_CENTER, "+", FontId::new(40.0, theme::bold_family()), color);
        let label = t("+  Добавить игру").trim_start_matches('+').trim().to_string();
        let g = ui.painter().layout_job(one_line_job(label, FontId::proportional(14.0), p.dim, cw - 16.0));
        ui.painter().galley(egui::pos2(c.x - g.size().x / 2.0, c.y + 22.0), g, p.dim);
        if resp.clicked() {
            actions.push(Action::Wizard(WizardMode::New));
        }
    }

    fn empty_page(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = pal();
        let avail = ui.available_height();
        let content_h = 72.0 + 12.0 + 30.0 + 60.0 + 16.0 + 54.0;
        ui.add_space(((avail - content_h) / 3.0).max(0.0));
        ui.vertical_centered(|ui| {
            if let Some(t) = &self.logo_big {
                ui.image((t.id(), Vec2::splat(72.0)));
            }
            ui.add_space(4.0);
            ui.label(theme::h1(t("Добавьте первую игру")));
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width().min(520.0), 60.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                t("Запустите визуальную новеллу в оконном режиме и нажмите кнопку ниже. \
                                 Настройка займёт минуту: выбрать окно, прокликать пару реплик и поставить плашку."),
                            )
                            .size(14.0)
                            .color(p.dim),
                        )
                        .halign(Align::Center)
                        .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                },
            );
            ui.add_space(8.0);
            if button(ui, t("+  Добавить игру"), BtnKind::Big, true, None).clicked() {
                actions.push(Action::Wizard(WizardMode::New));
            }
        });
    }

    /// Шапка страницы игры: размытый фон, обложка с белой рамкой на нижнем крае, название.
    fn game_header(ui: &mut egui::Ui, profile: &Profile, snap: &Snap, view: &ArtView, actions: &mut Vec<Action>) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 108.0), Sense::hover());
        let rr = CornerRadius::same(14);
        match &view.back {
            Some(b) => {
                let uv = theme::fit_uv(b.size_vec2(), rect.width() / rect.height());
                egui::Image::new((b.id(), rect.size())).uv(uv).corner_radius(rr).paint_at(ui, rect);
            }
            None => {
                // картинок нет — градиент цвета игры, как у заглушки обложки (приглушённый)
                egui::Image::new((view.placeholder.id(), rect.size()))
                    .tint(Color32::from_gray(150))
                    .corner_radius(rr)
                    .paint_at(ui, rect);
            }
        }
        // обложка 62x93 с белой рамкой наезжает на нижний край шапки
        let cr = egui::Rect::from_min_size(rect.left_bottom() + Vec2::new(20.0, -56.0), Vec2::new(62.0, 93.0));
        ui.painter().rect_filled(cr.expand(3.0), CornerRadius::same(8), Color32::WHITE);
        paint_cover(ui, view, cr, CornerRadius::same(6), &profile.name);
        // название — белым с тенью, на затемнённом низу шапки
        let x = cr.right() + 19.0;
        let job = LayoutJob::simple(profile.name.clone(), FontId::new(22.0, theme::bold_family()), Color32::PLACEHOLDER, (rect.right() - x - 16.0).max(10.0));
        let mut job = job;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        let pos = egui::pos2(x, rect.bottom() - 18.0 - g.size().y);
        ui.painter().galley(pos + Vec2::new(1.0, 1.0), g.clone(), Color32::from_black_alpha(170));
        ui.painter().galley(pos, g, Color32::WHITE);
        // кнопки поверх шапки: тёмная полупрозрачная подложка, белый текст
        let pill = |ui: &mut egui::Ui, at: egui::Pos2, left: bool, text: &str, id: &str| -> egui::Response {
            let font = FontId::new(13.0, theme::bold_family());
            let g = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE);
            let size = Vec2::new(g.size().x + 20.0, 26.0);
            let r = if left { egui::Rect::from_min_size(at, size) } else { egui::Rect::from_min_size(at - Vec2::new(size.x, 0.0), size) };
            let resp = ui.interact(r, egui::Id::new(("hdr", id)), Sense::click());
            let a = if resp.hovered() { 170 } else { 120 };
            ui.painter().rect_filled(r, CornerRadius::same(13), Color32::from_black_alpha(a));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, font, Color32::WHITE);
            resp
        };
        if snap.session.is_none() && pill(ui, rect.left_top() + Vec2::new(12.0, 12.0), true, t("‹ Библиотека"), "lib").clicked() {
            actions.push(Action::Library);
        }
        let more = pill(ui, rect.right_top() + Vec2::new(-12.0, 12.0), false, "···", "more");
        // картинок нет (автопоиск не нашёл игру или ещё не искал) — сразу предложить выбрать игру на VNDB
        if profile.cover.is_empty()
            && !snap.demo
            && snap.vndb != Some(false)
            && pill(ui, more.rect.left_top() - Vec2::new(8.0, 0.0), false, t("Найти картинки на VNDB"), "vndb").clicked()
        {
            actions.push(Action::ArtOpen(profile.id.clone(), profile.name.clone()));
        }
        let menu_id = egui::Id::new(("game_menu", &profile.id));
        if more.clicked() {
            ui.memory_mut(|m| m.toggle_popup(menu_id));
        }
        egui::popup_below_widget(ui, menu_id, &more, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(170.0);
            if ui.button(t("Обложка и фон…")).clicked() {
                actions.push(Action::ArtOpen(profile.id.clone(), profile.name.clone()));
            }
            if ui.button(t("Настройки запуска")).clicked() {
                actions.push(Action::LaunchSettings(profile.clone()));
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn game_page(
        ui: &mut egui::Ui,
        profile: &Profile,
        s: Option<&SessSnap>,
        snap: &Snap,
        view: &ArtView,
        launching: Option<&str>,
        cache: &mut HashMap<String, Vec<Segment>>,
        actions: &mut Vec<Action>,
    ) {
        let p = pal();
        let accent = view.accent.unwrap_or(p.accent);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 12.0);
        Self::game_header(ui, profile, snap, view, actions);
        let stats = s.map_or(&profile.stats, |s| &s.profile.stats);
        // справа от обложки (она наезжает на этот ряд): языки и статистика
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), 36.0), Layout::left_to_right(Align::Min), |ui| {
            ui.set_min_height(36.0);
            ui.add_space(94.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    ui.label(theme::dim(format!(
                        "{} → {}  ·  {}",
                        profile.lang.title(),
                        snap.target_title,
                        profile.exe_name
                    )));
                    // компактный выбор языка оригинала для этой игры
                    let mut lang = profile.lang;
                    egui::ComboBox::from_id_salt("game_lang")
                        .width(110.0)
                        .selected_text(theme::small(profile.lang.title()))
                        .show_ui(ui, |ui| {
                            for l in Lang::ALL {
                                ui.selectable_value(&mut lang, l, l.title());
                            }
                        })
                        .response
                        .on_hover_text(t("Язык оригинала этой игры"));
                    if lang != profile.lang {
                        actions.push(Action::SetLang(profile.id.clone(), lang));
                    }
                });
                if stats.lines > 0 {
                    ui.label(theme::small(tf(
                        "Прочитано реплик: {} · символов: {}",
                        &[&thousands(stats.lines), &thousands(stats.chars)],
                    )));
                }
            });
        });

        // статус
        let (mut color, mut text, mut retry) =
            (p.dim, t("Игра не запущена. Запустите её — NovelTL подключится сам.").to_string(), false);
        let play = s.is_none();
        let mut show_plate = false;
        if play && launching == Some(profile.id.as_str()) {
            color = p.warn;
            text = tf("Запускаю {}…", &[&profile.name]);
        }
        if let Some(s) = s {
            if !s.error.is_empty() {
                color = p.bad;
                text = s.error.clone();
                retry = true;
            } else if !s.connected {
                color = p.warn;
                text = t("Подключаюсь к игре…").to_string();
            } else if s.setup {
                color = p.warn;
                text = t("Идёт настройка — закончите её в окне мастера.").to_string();
            } else if !s.has_selected {
                color = p.warn;
                text = t("Подключено. Жду текст — прокликайте пару реплик в игре.").to_string();
            } else {
                color = p.ok;
                text = t("Перевод работает.").to_string();
                if snap.user_hidden {
                    // в Linux глобальных клавиш нет — вместо подсказки про сочетание кнопка ниже
                    text += &if cfg!(windows) {
                        tf(" Плашка скрыта — {}, чтобы показать.", &[&snap.hotkey_toggle])
                    } else {
                        t(" Плашка скрыта.").to_string()
                    };
                    show_plate = !cfg!(windows);
                }
            }
        }
        theme::card_frame().inner_margin(Margin { left: 16, right: 12, top: 12, bottom: 12 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_sized([18.0, 20.0], egui::Label::new(RichText::new("●").size(14.0).color(color)));
                let btn_w = if show_plate { 150.0 } else if retry || play { 110.0 } else { 0.0 };
                let w = (ui.available_width() - btn_w - 8.0).max(50.0);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    ui.add(
                        egui::Label::new(RichText::new(text).size(14.0).color(p.text))
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                });
                if retry && button(ui, t("Повторить"), BtnKind::Normal, true, None).clicked() {
                    actions.push(Action::Retry);
                }
                if show_plate && button(ui, t("Показать плашку"), BtnKind::Normal, true, None).clicked() {
                    actions.push(Action::ToggleOverlay);
                }
                if play {
                    let busy = launching == Some(profile.id.as_str());
                    if theme::accent_button(ui, t("Играть"), accent, !busy, None).clicked() {
                        actions.push(Action::Play(profile.clone()));
                    }
                }
            });
        });

        // история диалога справа от календаря чтения (на узком окне — календарь под историей)
        let today = crate::controller::local_time_text("%Y-%m-%d");
        let total_w = ui.available_width();
        let weeks = if total_w >= 700.0 { 20 } else { 13 };
        let cal_outer = weeks as f32 * 14.0 - 3.0 + 34.0;
        let narrow = total_w < 520.0;
        let fill_h = (ui.available_height() - 24.0).max(60.0);
        let legend = fill_h >= 215.0; // в низком окне легенда не помещается
        let calendar = |ui: &mut egui::Ui| {
            theme::card_frame().show(ui, |ui| ui.vertical(|ui| theme::reading_calendar(ui, stats, &today, weeks, accent, true, legend)));
        };
        if narrow {
            // история ограничена по высоте, иначе она растягивается до низа и календарь уходит за край окна
            let hist_h = (fill_h - 190.0).max(100.0);
            ui.allocate_ui(Vec2::new(ui.available_width(), hist_h), |ui| {
                Self::history_card(ui, total_w - 34.0, (hist_h - 30.0).max(60.0), profile, s, snap, accent, cache, actions);
            });
            calendar(ui);
        } else {
            ui.horizontal_top(|ui| {
                Self::history_card(ui, total_w - cal_outer - 12.0 - 30.0, fill_h, profile, s, snap, accent, cache, actions);
                calendar(ui);
            });
        }
    }

    /// История диалога (самая новая реплика — сверху); имена без своего цвета — цветом акцента страницы.
    #[allow(clippy::too_many_arguments)]
    fn history_card(
        ui: &mut egui::Ui,
        inner_w: f32,
        min_h: f32,
        profile: &Profile,
        s: Option<&SessSnap>,
        snap: &Snap,
        accent: Color32,
        cache: &mut HashMap<String, Vec<Segment>>,
        actions: &mut Vec<Action>,
    ) {
        theme::card_frame().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 14 }).show(ui, |ui| ui.vertical(|ui| {
            ui.set_width(inner_w);
            ui.set_min_height(min_h);
            ui.spacing_mut().item_spacing.y = 6.0;
            let history = s.map_or(&[][..], |s| s.history.as_slice());
            let s_colors = s.map_or(&[][..], |s| s.name_colors.as_slice());
            let log = crate::controller::log_path(&profile.id);
            ui.horizontal(|ui| {
                ui.label(theme::step(match history.len() {
                    0 | 1 => t("ПОСЛЕДНЯЯ РЕПЛИКА").to_string(),
                    n => tf("ИСТОРИЯ ДИАЛОГА · {}", &[&n]),
                }));
                if log.exists() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let r = button(ui, t("Открыть журнал"), BtnKind::Link, true, None).on_hover_text(t(
                            "Все переведённые реплики этой игры — по нему можно искать (Ctrl+F)",
                        ));
                        if r.clicked() {
                            actions.push(Action::OpenLog(log.to_string_lossy().into_owned()));
                        }
                    });
                }
            });
            if history.is_empty() {
                let hint = if s.is_some() { t("Здесь появится текст из игры и его перевод.") } else { "—" };
                ui.add(egui::Label::new(theme::jp(hint)).wrap_mode(egui::TextWrapMode::Wrap));
                return;
            }
            egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (i, res) in history.iter().enumerate() {
                    if i > 0 {
                        hline(ui);
                    }
                    let src = if res.src_name.is_empty() {
                        res.src_text.clone()
                    } else {
                        format!("{}\u{3000}{}", res.src_name, res.src_text)
                    };
                    let dst = if res.name.is_empty() { res.text.clone() } else { format!("{}: {}", res.name, res.text) };
                    if let Some(dict) = &snap.learn {
                        if !res.src_name.is_empty() {
                            ui.add(egui::Label::new(theme::jp(res.src_name.as_str())).wrap_mode(egui::TextWrapMode::Wrap));
                        }
                        furigana(ui, dict, cache, res, actions);
                    } else {
                        ui.add(egui::Label::new(theme::jp(src)).wrap_mode(egui::TextWrapMode::Wrap));
                    }
                    let color = s_colors.get(i).copied().flatten().unwrap_or(accent);
                    if !res.name.is_empty() {
                        let f = FontId::proportional(16.0);
                        let mut job = LayoutJob::default();
                        job.append(&format!("{}: ", res.name), 0.0, egui::TextFormat::simple(f.clone(), color));
                        job.append(&res.text, 0.0, egui::TextFormat::simple(f, pal().text));
                        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Wrap));
                    } else {
                        ui.add(egui::Label::new(theme::tr(dst)).wrap_mode(egui::TextWrapMode::Wrap));
                    }
                    if !res.alt.is_empty() {
                        ui.add(egui::Label::new(theme::small(res.alt.as_str())).wrap_mode(egui::TextWrapMode::Wrap));
                    }
                    if i == 0 {
                        ui.label(theme::small(format!(
                            "{}{}",
                            noveltl_core::translators::engine_title(res.engine),
                            if res.cached { t(" · из кэша") } else { "" }
                        )));
                    }
                }
            });
        }));
    }

    /// Окно «Обложка и фон»: поиск на VNDB, свои файлы, кадр из игры, сброс.
    fn art_modal(&mut self, ctx: &egui::Context, snap: &Snap, actions: &mut Vec<Action>) {
        let p = pal();
        let Some(edit) = self.art_edit.as_mut() else { return };
        // готовые миниатюры -> текстуры
        for (url, img) in edit.thumbs.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            if let Some((w, h, px)) = img {
                let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                edit.tex.insert(url.clone(), ctx.load_texture(format!("thumb:{url}"), img, TextureOptions::LINEAR));
            }
        }
        if let Some(Ok(list)) = &snap.vndb_results {
            let new: Vec<String> =
                list.iter().filter_map(|h| h.cover.clone()).filter(|u| edit.requested.insert(u.clone())).collect();
            if !new.is_empty() {
                actions.push(Action::Thumbs(new));
            }
        }
        let running = snap.session.as_ref().is_some_and(|s| s.profile.id == edit.id);
        let allowed = snap.vndb != Some(false);
        let mut close = false;
        let m = egui::Modal::new(egui::Id::new("art_edit")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(theme::h2(t("Обложка и фон")));
            ui.label(theme::small(edit.name.as_str()));
            ui.add_space(6.0);
            ui.label(theme::body(t("Найти на VNDB")));
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 96.0).max(100.0);
                let r = ui.add(egui::TextEdit::singleline(&mut edit.query).desired_width(w));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let can = allowed && !edit.query.trim().is_empty();
                if (button(ui, t("Найти"), BtnKind::Primary, can, Some(80.0)).clicked() || enter) && can {
                    actions.push(Action::VndbSearch(edit.id.clone(), edit.query.trim().to_string()));
                }
            });
            if !allowed {
                ui.label(theme::small(t("Запросы к VNDB выключены в настройках.")));
            }
            match &snap.vndb_results {
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(theme::small(t("Ничего не найдено")));
                }
                Some(Ok(list)) => {
                    egui::ScrollArea::vertical().id_salt("art_results").max_height(250.0).auto_shrink([false, true]).show(ui, |ui| {
                        for hit in list {
                            ui.horizontal(|ui| {
                                ui.set_min_height(60.0);
                                let (tr, _) = ui.allocate_exact_size(Vec2::new(40.0, 60.0), Sense::hover());
                                match hit.cover.as_ref().and_then(|u| edit.tex.get(u)) {
                                    Some(tex) => {
                                        let uv = theme::fit_uv(tex.size_vec2(), 40.0 / 60.0);
                                        egui::Image::new((tex.id(), tr.size())).uv(uv).corner_radius(CornerRadius::same(4)).paint_at(ui, tr);
                                    }
                                    None => {
                                        ui.painter().rect_filled(tr, CornerRadius::same(4), p.border);
                                    }
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if button(ui, t("Выбрать"), BtnKind::Normal, true, None).clicked() {
                                        actions.push(Action::VndbPick(edit.id.clone(), hit.clone()));
                                    }
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.add(egui::Label::new(theme::body(hit.title.as_str())).truncate());
                                        if !hit.alttitle.is_empty() && hit.alttitle != hit.title {
                                            ui.add(egui::Label::new(theme::small(hit.alttitle.as_str())).truncate());
                                        }
                                    });
                                });
                            });
                        }
                    });
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e.as_str()).size(12.0).color(p.bad));
                }
                None => {}
            }
            if snap.art_pending > 0 || !snap.art_status.is_empty() {
                ui.label(theme::small(snap.art_status.as_str()));
            }
            ui.add_space(4.0);
            hline(ui);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if button(ui, t("Своя обложка…"), BtnKind::Normal, true, None).clicked() {
                    actions.push(Action::PickArt(edit.id.clone(), false));
                }
                if button(ui, t("Свой фон…"), BtnKind::Normal, true, None).clicked() {
                    actions.push(Action::PickArt(edit.id.clone(), true));
                }
                let r = button(ui, t("Кадр из игры"), BtnKind::Normal, running, None);
                if r.clicked() {
                    actions.push(Action::CaptureBackdrop(edit.id.clone()));
                }
                if !running {
                    r.on_hover_text(t("Сначала запустите игру"));
                }
                if button(ui, t("Сбросить"), BtnKind::Normal, true, None).clicked() {
                    actions.push(Action::ClearArt(edit.id.clone()));
                }
            });
            ui.add_space(6.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, t("Закрыть"), BtnKind::Primary, true, Some(110.0)).clicked() {
                    close = true;
                }
            });
        });
        if close || m.should_close() {
            self.art_edit = None;
        }
    }

    fn modals(&mut self, ctx: &egui::Context, snap: &Snap, actions: &mut Vec<Action>) {
        if let Some(edit) = self.launch_edit.as_mut() {
            let (mut save, mut cancel) = (false, false);
            let m = egui::Modal::new(egui::Id::new("launch_settings")).show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(theme::h2(t("Настройки запуска")));
                ui.label(theme::small(edit.name.as_str()));
                ui.add_space(6.0);
                ui.label(theme::body(t("Файл игры")));
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 120.0).max(100.0);
                    ui.add(egui::TextEdit::singleline(&mut edit.exe).desired_width(w));
                    if button(ui, t("Выбрать…"), BtnKind::Normal, true, None).clicked() {
                        if let Some(path) = winutil::pick_exe() {
                            edit.exe = path.to_string_lossy().into_owned();
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(theme::body(t("Своя команда запуска")));
                ui.add(egui::TextEdit::singleline(&mut edit.cmd).desired_width(f32::INFINITY));
                ui.add(
                    egui::Label::new(theme::small(t("Для DMM, Locale Emulator и других лаунчеров. Пусто — запуск как обычно.")))
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, t("Сохранить"), BtnKind::Primary, true, Some(110.0)).clicked() {
                        save = true;
                    }
                    if button(ui, t("Отмена"), BtnKind::Normal, true, Some(110.0)).clicked() {
                        cancel = true;
                    }
                });
            });
            if save {
                self.pending_launch = Some((edit.id.clone(), edit.exe.trim().to_string(), edit.cmd.clone()));
                self.launch_edit = None;
            } else if cancel || m.should_close() {
                self.launch_edit = None;
            }
            return;
        }
        if self.art_edit.is_some() {
            self.art_modal(ctx, snap, actions);
            return;
        }
        if let Some((id, name)) = self.confirm_delete.clone() {
            let mut yes = false;
            let mut no = false;
            let m = egui::Modal::new(egui::Id::new("confirm_delete")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(theme::h2(t("Удалить игру")));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(theme::body(tf(
                        "Удалить «{}» из NovelTL?\nПрофиль, глоссарий и кэш переводов будут удалены. Сама игра не затрагивается.",
                        &[&name],
                    )))
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, t("Нет"), BtnKind::Normal, true, Some(80.0)).clicked() {
                        no = true;
                    }
                    if button(ui, t("Да"), BtnKind::Primary, true, Some(80.0)).clicked() {
                        yes = true;
                    }
                });
            });
            if yes {
                self.confirm_delete = None;
                self.pending_delete = Some(id);
            } else if no || m.should_close() {
                self.confirm_delete = None;
            }
        } else if let Some((title, text)) = self.messages.first().cloned() {
            let mut ok = false;
            let m = egui::Modal::new(egui::Id::new("message")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(theme::h2(title));
                ui.add_space(4.0);
                ui.add(egui::Label::new(theme::body(text)).wrap_mode(egui::TextWrapMode::Wrap));
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "OK", BtnKind::Primary, true, Some(80.0)).clicked() {
                        ok = true;
                    }
                });
            });
            if ok || m.should_close() {
                self.messages.remove(0);
            }
        }
    }

    fn run_actions(&mut self, actions: Vec<Action>, shared: &Shared, windows: &mut Windows, snap: &Snap, ctx: &egui::Context) {
        for a in actions {
            match a {
                Action::Wizard(mode) => windows.open_wizard(mode),
                Action::Settings => windows.open_settings(),
                Action::SetLang(id, lang) => {
                    lock_ctl(&shared.controller).set_profile_lang(&id, lang);
                    self.profiles_at = None;
                }
                Action::OpenLog(path) => winutil::reveal_in_explorer(&path),
                Action::Anki(front, back) => lock_ctl(&shared.controller).add_to_anki(front, back),
                Action::Glossary(id) => windows.open_glossary(id),
                Action::Library => self.open_game = false,
                Action::OpenGame(id) => {
                    self.selected = Some(id);
                    self.open_game = true;
                }
                Action::VndbConsent(on) => lock_ctl(&shared.controller).set_vndb_consent(on),
                Action::ArtOpen(id, name) => {
                    lock_ctl(&shared.controller).art_window_opened(&id);
                    self.art_edit = Some(ArtEdit {
                        id,
                        // в поле поиска — короткий вариант названия (без подзаголовка): так VNDB находит надёжнее
                        query: vndb::title_variants(&name).pop().unwrap_or_default(),
                        name,
                        thumbs: Arc::new(Mutex::new(HashMap::new())),
                        tex: HashMap::new(),
                        requested: Default::default(),
                    });
                }
                Action::VndbSearch(id, q) => lock_ctl(&shared.controller).vndb_search(&id, &q),
                Action::VndbPick(id, hit) => lock_ctl(&shared.controller).vndb_pick(&id, hit),
                Action::PickArt(id, backdrop) => {
                    // диалог выбора файла — без замка контроллера
                    if let Some(path) = super::avatar::pick_image() {
                        let mut c = lock_ctl(&shared.controller);
                        if backdrop {
                            c.set_backdrop_file(&id, &path);
                        } else {
                            c.set_cover_file(&id, &path);
                        }
                        self.profiles_at = None;
                    }
                }
                Action::CaptureBackdrop(id) => {
                    lock_ctl(&shared.controller).capture_backdrop(&id);
                    self.profiles_at = None;
                }
                Action::ClearArt(id) => {
                    lock_ctl(&shared.controller).clear_art(&id);
                    self.profiles_at = None;
                }
                Action::Thumbs(urls) => {
                    if let Some(edit) = &self.art_edit {
                        let http = lock_ctl(&shared.controller).http();
                        for url in urls {
                            let (http, map, ctx) = (http.clone(), edit.thumbs.clone(), ctx.clone());
                            std::thread::spawn(move || {
                                let img = http.get_bytes(&url).ok().and_then(|bytes| {
                                    use std::hash::{Hash, Hasher};
                                    let mut h = std::collections::hash_map::DefaultHasher::new();
                                    url.hash(&mut h);
                                    let tmp = std::env::temp_dir().join(format!("noveltl_thumb_{}_{:x}", std::process::id(), h.finish()));
                                    std::fs::write(&tmp, bytes).ok()?;
                                    let img = super::avatar::decode_image(&tmp).ok();
                                    let _ = std::fs::remove_file(&tmp);
                                    img
                                });
                                map.lock().unwrap_or_else(|e| e.into_inner()).insert(url, img);
                                ctx.request_repaint();
                            });
                        }
                    }
                }
                Action::LaunchSettings(pr) => {
                    self.launch_edit =
                        Some(LaunchEdit { id: pr.id, name: pr.name, exe: pr.exe_path, cmd: pr.launch });
                }
                Action::Play(pr) => {
                    if winutil::test_area().is_some() {
                        // проверки разработчика не запускают игры пользователя
                        self.show_notice(&tf("Проверочный режим: игра «{}» не запускается.", &[&pr.name]));
                    } else {
                        match crate::launch::plan(&pr).and_then(|how| crate::launch::run(&how)) {
                            Ok(()) => {
                                self.launching = Some((pr.id, pr.name, Instant::now() + Duration::from_secs(30)));
                            }
                            Err(e) => self.show_notice(&e),
                        }
                    }
                }
                Action::OpenUrl(url) => winutil::open_url(&url),
                Action::DismissUpdate => lock_ctl(&shared.controller).dismiss_update(),
                Action::AskDelete(id, name) => self.confirm_delete = Some((id, name)),
                Action::Report => {
                    let result = lock_ctl(&shared.controller).save_report();
                    match result {
                        Ok(path) => {
                            let path = path.to_string_lossy().into_owned();
                            winutil::reveal_in_explorer(&path);
                            self.add_message(
                                t("Отчёт сохранён"),
                                &tf(
                                    "Отчёт сохранён в файл:\n{}\n\nПапка с ним открыта — пришлите этот файл для доработки.",
                                    &[&path],
                                ),
                            );
                        }
                        Err(e) => self.add_message(t("Отчёт"), &tf("Не удалось сохранить отчёт: {}", &[&e])),
                    }
                }
                Action::ToggleOverlay => lock_ctl(&shared.controller).toggle_overlay(),
                Action::Retry => {
                    if let Some(s) = snap.session.as_ref() {
                        let mut c = lock_ctl(&shared.controller);
                        let win = c.windows().into_iter().find(|w| w.pid == s.pid);
                        if let Some(win) = win {
                            c.start_session(s.profile.clone(), &win, s.saved, s.setup);
                        }
                    }
                }
            }
        }
        if let Some((id, exe, cmd)) = self.pending_launch.take() {
            lock_ctl(&shared.controller).set_profile_launch(&id, &exe, &cmd);
            self.profiles_at = None;
        }
        if let Some(id) = self.pending_delete.take() {
            lock_ctl(&shared.controller).delete_profile(&id);
            self.profiles_at = None;
        }
    }
}
