//! Главное окно (перенос `noveltl/ui/main_window.py`): список игр слева, страница игры справа.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, text::LayoutJob, Align, Color32, ColorImage, CornerRadius, FontId, Frame, Layout, Margin, RichText,
    Sense, TextureHandle, TextureOptions, Vec2,
};
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
}

enum Action {
    Retry,
    AskDelete(String, String),
    Report,
    Wizard(WizardMode),
    Glossary(String),
    Settings,
    SetLang(String, Lang),
    OpenLog(String),
}

/// 12345 → "12 345".
fn thousands(n: u64) -> String {
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
        }
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
            }),
            user_hidden: c.user_hidden,
            hotkey_toggle: c.settings.hotkey_toggle.clone(),
            hotkey_retranslate: c.settings.hotkey_retranslate.clone(),
            translator: c.translator_status(),
            target_title: c.settings.target_lang.title_in_text(),
            nickname: c.settings.nickname.clone(),
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
                            let text = format!("{}  {}", if running { "●" } else { "○" }, pr.name);
                            let mut job = LayoutJob::simple(
                                text,
                                FontId::proportional(14.0),
                                p.text,
                                (rect.width() - 24.0).max(10.0),
                            );
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            job.wrap.overflow_character = Some('…');
                            let galley = ui.painter().layout_job(job);
                            let pos = egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
                            ui.painter().galley(pos, galley, p.text);
                            if resp.clicked() {
                                self.selected = Some(pr.id.clone());
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

        // ---------- нижняя часть страницы игры ----------
        if let Some(profile) = &current {
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
                        Frame::new()
                            .fill(p.notice_bg)
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(
                                    egui::Label::new(RichText::new(text.as_str()).size(14.0).color(p.warn))
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                            });
                    }
                });
        }

        // ---------- центральная часть ----------
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 8 }))
            .show(ctx, |ui| match &current {
                None => self.empty_page(ui, &mut actions),
                Some(profile) => {
                    let s = snap.session.as_ref().filter(|s| s.profile.id == profile.id);
                    Self::game_page(ui, profile, s, &snap, &mut actions);
                }
            });

        self.modals(ctx);
        self.run_actions(actions, shared, windows, &snap);
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

    fn game_page(ui: &mut egui::Ui, profile: &Profile, s: Option<&SessSnap>, snap: &Snap, actions: &mut Vec<Action>) {
        let p = pal();
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 12.0);
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
                let btn_w = if retry { 110.0 } else { 0.0 };
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
            });
        });

        // история диалога (занимает всё свободное место), самая новая реплика — сверху
        let fill_h = (ui.available_height() - 24.0).max(60.0);
        theme::card_frame().inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 14 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(fill_h);
            ui.spacing_mut().item_spacing.y = 6.0;
            let history = s.map_or(&[][..], |s| s.history.as_slice());
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
                    ui.add(egui::Label::new(theme::jp(src)).wrap_mode(egui::TextWrapMode::Wrap));
                    ui.add(egui::Label::new(theme::tr(dst)).wrap_mode(egui::TextWrapMode::Wrap));
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
                Action::Glossary(id) => windows.open_glossary(id),
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
        if let Some(id) = self.pending_delete.take() {
            lock_ctl(&shared.controller).delete_profile(&id);
            self.profiles_at = None;
        }
    }
}
