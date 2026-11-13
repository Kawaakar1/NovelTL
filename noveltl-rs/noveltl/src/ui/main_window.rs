//! Главное окно (перенос `noveltl/ui/main_window.py`): список игр слева, страница игры справа.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, text::LayoutJob, Align, Color32, ColorImage, CornerRadius, FontId, Frame, Layout, Margin, RichText,
    Sense, TextureHandle, TextureOptions, Vec2,
};
use noveltl_core::dict::{Dictionary, Segment};
use noveltl_core::i18n::{t, tf};
use noveltl_core::langdetect::Lang;
use noveltl_core::pipeline::LineResult;
use noveltl_core::profiles::{Profile, ProfileStore};

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
}

enum Action {
    Retry,
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
}

fn engine_title(engine: &str) -> String {
    match engine {
        "deepl" => "DeepL".to_string(),
        "google" => "Google".to_string(),
        other => other.to_string(),
    }
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
        let snap = Self::snapshot(shared);

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
                            let fill = if is_sel {
                                p.selected
                            } else if resp.hovered() {
                                p.card
                            } else {
                                Color32::TRANSPARENT
                            };
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
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 8 }))
            .show(ctx, |ui| match &current {
                Some(profile) if show_game => {
                    let s = snap.session.as_ref().filter(|s| s.profile.id == profile.id);
                    let launching = self.launching.as_ref().map(|l| l.0.as_str());
                    Self::game_page(ui, profile, s, &snap, launching, &mut self.seg_cache, &mut actions);
                }
                _ if profiles.is_empty() => self.empty_page(ui, &mut actions),
                _ => self.library(ui, ctx, &profiles, &mut actions),
            });

        if let Some(id) = windows.launch.take() {
            // только для снимков: окно «Настройки запуска» без клика
            if let Some(pr) = profiles.iter().find(|p| p.id == id) {
                let mut pr = pr.clone();
                pr.exe_path = r"C:\Games\Kirinomori Lantern\Kirinomori.exe".to_string(); // вымышленный путь: на снимках не должно быть имени пользователя
                actions.push(Action::LaunchSettings(pr));
            }
        }
        self.modals(ctx);
        self.run_actions(actions, shared, windows, &snap);
    }

    /// «Библиотека»: карточки игр, когда ни одна игра не открыта.
    fn library(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, profiles: &[Profile], actions: &mut Vec<Action>) {
        let p = pal();
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 12.0);
        ui.label(theme::h1(t("Библиотека")));
        if let Some((text, _)) = self.notice.clone() {
            notice_box(ui, &text);
        }
        if let Some((_, name, _)) = &self.launching {
            ui.label(RichText::new(tf("Запускаю {}…", &[name])).size(14.0).color(p.warn));
        }
        const GAP: f32 = 12.0;
        let now = unix_now();
        egui::ScrollArea::vertical().id_salt("library").auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width() - 4.0;
            let cols = (((w + GAP) / (270.0 + GAP)) as usize).clamp(1, 3);
            let cw = (w - GAP * (cols as f32 - 1.0)) / cols as f32;
            ui.spacing_mut().item_spacing = Vec2::new(GAP, GAP);
            for chunk in profiles.chunks(cols) {
                ui.horizontal_top(|ui| {
                    for pr in chunk {
                        let busy = self.launching.as_ref().is_some_and(|l| l.0 == pr.id);
                        let icon = self.icon_tex(ctx, &pr.exe_path);
                        Self::card(ui, pr, icon, cw, busy, now, actions);
                    }
                });
            }
        });
    }

    fn card(
        ui: &mut egui::Ui,
        pr: &Profile,
        icon: Option<egui::TextureId>,
        cw: f32,
        busy: bool,
        now: u64,
        actions: &mut Vec<Action>,
    ) {
        theme::card_frame().inner_margin(Margin::same(14)).show(ui, |ui| {
            // карточка лежит в строке horizontal_top — внутри нужна своя вертикаль
            ui.vertical(|ui| {
                ui.set_width(cw - 30.0);
                ui.spacing_mut().item_spacing = Vec2::new(10.0, 8.0);
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
                    paint_game_icon(ui.painter(), rect, icon, &pr.name);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(egui::Label::new(theme::h2(pr.name.as_str())).truncate());
                        ui.add(egui::Label::new(theme::small(played_text(pr.last_played, now))).truncate());
                        ui.add(
                            egui::Label::new(theme::small(tf("Прочитано реплик: {}", &[&thousands(pr.stats.lines)])))
                                .truncate(),
                        );
                    });
                });
                ui.horizontal(|ui| {
                    let more_w = 38.0;
                    let play_w = (ui.available_width() - more_w - ui.spacing().item_spacing.x).max(60.0);
                    if button(ui, t("Играть"), BtnKind::Primary, !busy, Some(play_w)).clicked() {
                        actions.push(Action::Play(pr.clone()));
                    }
                    let r = button(ui, "···", BtnKind::Normal, true, Some(more_w));
                    let id = egui::Id::new(("library_menu", &pr.id));
                    if r.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(id));
                    }
                    egui::popup_below_widget(ui, id, &r, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        ui.set_min_width(170.0);
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
                });
            });
        });
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

    fn game_page(
        ui: &mut egui::Ui,
        profile: &Profile,
        s: Option<&SessSnap>,
        snap: &Snap,
        launching: Option<&str>,
        cache: &mut HashMap<String, Vec<Segment>>,
        actions: &mut Vec<Action>,
    ) {
        let p = pal();
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 12.0);
        if snap.session.is_none() && button(ui, t("‹ Библиотека"), BtnKind::Link, true, None).clicked() {
            actions.push(Action::Library);
        }
        ui.label(theme::h1(profile.name.as_str()));
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
        let stats = s.map_or(&profile.stats, |s| &s.profile.stats);
        if stats.lines > 0 {
            ui.label(theme::small(tf(
                "Прочитано реплик: {} · символов: {}",
                &[&thousands(stats.lines), &thousands(stats.chars)],
            )));
        }

        // статус
        let (mut color, mut text, mut retry) =
            (p.dim, t("Игра не запущена. Запустите её — NovelTL подключится сам.").to_string(), false);
        let play = s.is_none();
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
                    text += &tf(" Плашка скрыта — {}, чтобы показать.", &[&snap.hotkey_toggle]);
                }
            }
        }
        theme::card_frame().inner_margin(Margin { left: 16, right: 12, top: 12, bottom: 12 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add_sized([18.0, 20.0], egui::Label::new(RichText::new("●").size(14.0).color(color)));
                let btn_w = if retry || play { 110.0 } else { 0.0 };
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
                if play {
                    let busy = launching == Some(profile.id.as_str());
                    if button(ui, t("Играть"), BtnKind::Primary, !busy, None).clicked() {
                        actions.push(Action::Play(profile.clone()));
                    }
                }
            });
        });

        // история диалога (занимает всё свободное место), самая новая реплика — сверху
        let fill_h = (ui.available_height() - 24.0).max(60.0);
        theme::card_frame().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 14 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(fill_h);
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
                    let color = s_colors.get(i).copied().flatten();
                    if let (Some(c), false) = (color, res.name.is_empty()) {
                        let f = FontId::proportional(16.0);
                        let mut job = LayoutJob::default();
                        job.append(&format!("{}: ", res.name), 0.0, egui::TextFormat::simple(f.clone(), c));
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
                            engine_title(res.engine),
                            if res.cached { t(" · из кэша") } else { "" }
                        )));
                    }
                }
            });
        });
    }

    fn modals(&mut self, ctx: &egui::Context) {
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

    fn run_actions(&mut self, actions: Vec<Action>, shared: &Shared, windows: &mut Windows, snap: &Snap) {
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
                Action::Retry => {
                    if let Some(s) = snap.session.as_ref() {
                        let mut c = lock_ctl(&shared.controller);
                        let win = c.windows().into_iter().find(|w| w.pid == s.pid);
                        if let Some(win) = win {
                            c.start_session(s.profile.clone(), &win, s.saved, false);
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
