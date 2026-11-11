//! Мастер добавления игры (перенос `noveltl/ui/wizard.py`): окно → поиск текста → расположение плашки.
//! Отдельное окно (immediate viewport); состояние контроллера читается копией, замок держится недолго.

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, text::LayoutJob, Align, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, ViewportBuilder, ViewportId,
};
use noveltl_core::langdetect::Lang;
use noveltl_core::scorer::{is_confident, HookKey, CONFIDENT_DISTINCT};

use super::theme::{self, button, pal, BtnKind};
use super::{lock_ctl, Shared, WizardMode};
use crate::winutil::{self, WindowInfo};

const WINDOWS_REFRESH: Duration = Duration::from_secs(2);

/// Одна строка рейтинга потоков (копия из контроллера).
struct Row {
    key: HookKey,
    name: String,
    score: f64,
    distinct: usize,
    /// последние очищенные реплики (у лучшего — 3, у остальных — 1)
    last: Vec<String>,
}

#[derive(Default)]
struct Snap {
    has_session: bool,
    error: String,
    connected: bool,
    engine: String,
    confident: bool,
    rows: Vec<Row>,
}

enum Act {
    Start,
    Confirm(HookKey),
    BackFromSearch,
    ToggleOthers,
    Report,
    BackToSearch,
    ResetPosition,
    Finish,
    Cancel,
}

pub struct Wizard {
    mode: WizardMode,
    id: ViewportId,
    icon: Arc<egui::IconData>,
    pos: Option<[f32; 2]>,
    step: usize,
    finished: bool,
    closed: bool,
    frames: u32,
    windows: Vec<WindowInfo>,
    windows_at: Option<Instant>,
    selected: Option<isize>,
    lang: Lang,
    /// название языка перевода (строчными) для подписи
    target_title: String,
    show_others: bool,
    best_key: Option<HookKey>,
    messages: Vec<(String, String)>,
}

fn elide(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(limit.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

fn lang_title(l: Lang) -> String {
    if l == Lang::Auto {
        format!("{} (определять по тексту)", l.title())
    } else {
        l.title().to_string()
    }
}

fn wrap(ui: &mut Ui, text: RichText) {
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Wrap));
}

fn header(ui: &mut Ui, step: usize, title: &str, text: &str) {
    ui.spacing_mut().item_spacing.y = 12.0;
    ui.label(theme::step(format!("ШАГ {step} ИЗ 3")));
    ui.label(theme::h1(title));
    wrap(ui, theme::dim(text));
    ui.add_space(4.0);
}

/// Одна строка текста с «…» на конце, если не влезает.
fn paint_line(ui: &Ui, pos: Pos2, text: &str, size: f32, color: Color32, width: f32) {
    let mut job = LayoutJob::simple(text.to_string(), FontId::proportional(size), color, width.max(10.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos, galley, color);
}

fn window_card(ui: &mut Ui, w: &WindowInfo, selected: bool) -> Response {
    let p = pal();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
    let fill = if selected {
        p.selected
    } else if resp.hovered() {
        p.card
    } else {
        p.panel
    };
    let stroke = Stroke::new(1.0_f32, if selected { p.accent } else { p.border });
    ui.painter().rect(rect, CornerRadius::same(8), fill, stroke, StrokeKind::Inside);
    let width = rect.width() - 28.0;
    paint_line(ui, Pos2::new(rect.left() + 14.0, rect.top() + 10.0), &w.title, 14.0, p.text, width);
    paint_line(ui, Pos2::new(rect.left() + 14.0, rect.top() + 32.0), &w.exe_name(), 13.0, p.dim, width);
    resp
}

fn progress(ui: &mut Ui, value: usize, max: usize) {
    let p = pal();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(3), p.card);
    if value > 0 && max > 0 {
        let w = rect.width() * value.min(max) as f32 / max as f32;
        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(w, 6.0)), CornerRadius::same(3), p.accent);
    }
}

fn footer_frame() -> Frame {
    Frame::new().fill(pal().bg).inner_margin(Margin { left: 28, right: 28, top: 6, bottom: 22 })
}

fn central_frame() -> Frame {
    Frame::new().fill(pal().bg).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 8 })
}

impl Wizard {
    pub fn new(mode: WizardMode, shared: &Shared, pos: Option<[f32; 2]>) -> Wizard {
        let (default_lang, target_lang) = {
            let c = lock_ctl(&shared.controller);
            (c.settings.default_source_lang, c.settings.target_lang)
        };
        let mut w = Wizard {
            mode,
            id: ViewportId::from_hash_of("noveltl_wizard"),
            icon: Arc::new(theme::icon_data(64)),
            pos,
            step: 0,
            finished: false,
            closed: false,
            frames: 0,
            windows: Vec::new(),
            windows_at: None,
            selected: None,
            lang: default_lang,
            target_title: target_lang.title().to_lowercase(),
            show_others: false,
            best_key: None,
            messages: Vec::new(),
        };
        match mode {
            WizardMode::New => {}
            WizardMode::Hook => {
                lock_ctl(&shared.controller).resume_setup();
                w.go(shared, 1);
            }
            WizardMode::Position => {
                lock_ctl(&shared.controller).resume_setup();
                w.go(shared, 2);
            }
        }
        w
    }

    pub fn viewport_id(&self) -> ViewportId {
        self.id
    }

    /// Заголовок окна (по нему окно находят для снимка с экрана).
    pub fn window_title(&self) -> &'static str {
        if self.mode == WizardMode::New {
            "Добавление игры — NovelTL"
        } else {
            "Настройка игры — NovelTL"
        }
    }

    /// Сколько раз окно уже отрисовано.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    fn go(&mut self, shared: &Shared, step: usize) {
        self.step = step;
        lock_ctl(&shared.controller).set_overlay_edit(step == 2);
    }

    fn reload_windows(&mut self, shared: &Shared, force: bool) {
        if self.step != 0 {
            return;
        }
        if !force && self.windows_at.is_some_and(|t| t.elapsed() < WINDOWS_REFRESH) {
            return;
        }
        let wins = lock_ctl(&shared.controller).windows();
        self.windows_at = Some(Instant::now());
        if wins.iter().map(|w| w.hwnd).eq(self.windows.iter().map(|w| w.hwnd)) {
            return;
        }
        self.windows = wins;
        if !self.selected.is_some_and(|h| self.windows.iter().any(|w| w.hwnd == h)) {
            self.selected = None;
        }
        if self.windows.len() == 1 {
            self.selected = Some(self.windows[0].hwnd);
        }
    }

    fn start_search(&mut self, shared: &Shared) {
        let Some(win) = self.selected.and_then(|h| self.windows.iter().find(|w| w.hwnd == h)).cloned() else {
            return;
        };
        lock_ctl(&shared.controller).begin_setup(&win, self.lang, &win.title);
        self.go(shared, 1);
    }

    /// Для `--shots`: выбрать окно и нажать «Далее».
    pub fn select_and_start(&mut self, shared: &Shared, hwnd: isize) -> bool {
        if self.step != 0 {
            return false;
        }
        self.reload_windows(shared, true);
        if !self.windows.iter().any(|w| w.hwnd == hwnd) {
            return false;
        }
        self.selected = Some(hwnd);
        self.start_search(shared);
        true
    }

    /// Для `--shots`: раскрыть/свернуть «Другие варианты».
    pub fn set_others(&mut self, on: bool) {
        self.show_others = on;
    }

    /// «Это текст диалога» (для `--shots`).
    pub fn confirm_best(&mut self, shared: &Shared) -> bool {
        match self.best_key.clone() {
            Some(key) => {
                lock_ctl(&shared.controller).choose_hook(&key);
                self.go(shared, 2);
                true
            }
            None => false,
        }
    }

    /// «Готово» (для `--shots` тоже).
    pub fn finish(&mut self, shared: &Shared) {
        self.finished = true;
        lock_ctl(&shared.controller).finish_setup();
        self.closed = true;
    }

    /// Окно закрыто: без «Готово» настройка отменяется, режим правки плашки выключается.
    pub fn close(&mut self, shared: &Shared) {
        let mut c = lock_ctl(&shared.controller);
        if !self.finished {
            c.cancel_setup();
        }
        c.set_overlay_edit(false);
    }

    fn snapshot(&mut self, shared: &Shared) -> Snap {
        if self.step != 1 {
            return Snap::default();
        }
        let c = lock_ctl(&shared.controller);
        let Some(s) = c.session.as_ref() else { return Snap::default() };
        let ranked = c.tracker.ranked();
        let rows = ranked
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let want = if i == 0 { 3 } else if self.show_others { 1 } else { 0 };
                let last = if want > 0 { c.tracker.get(&r.key).map(|st| st.last_clean(want)).unwrap_or_default() } else { Vec::new() };
                Row { key: r.key.clone(), name: r.hookname.clone(), score: r.score, distinct: r.distinct, last }
            })
            .collect();
        Snap {
            has_session: true,
            error: s.error.clone(),
            connected: s.connected,
            engine: s.engine.clone(),
            confident: is_confident(&ranked),
            rows,
        }
    }

    /// Рисует окно; false — окно нужно закрыть (после этого вызвать `close`).
    pub fn show(&mut self, ctx: &egui::Context, shared: &Shared) -> bool {
        self.frames += 1;
        self.reload_windows(shared, false);
        let snap = self.snapshot(shared);
        self.best_key = snap.rows.first().map(|r| r.key.clone());
        self.target_title = lock_ctl(&shared.controller).settings.target_lang.title().to_lowercase();

        let mut builder = ViewportBuilder::default()
            .with_title(self.window_title())
            .with_inner_size([580.0, 720.0])
            .with_always_on_top()
            .with_icon(self.icon.clone());
        if let Some(pos) = self.pos {
            builder = builder.with_position(pos);
        }
        let mut acts: Vec<Act> = Vec::new();
        let id = self.id;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            self.draw(ctx, &snap, &mut acts);
        });
        for a in acts {
            self.apply(shared, a);
        }
        if self.step == 0 {
            ctx.request_repaint_after(WINDOWS_REFRESH);
        }
        !self.closed
    }

    fn apply(&mut self, shared: &Shared, act: Act) {
        match act {
            Act::Start => self.start_search(shared),
            Act::Confirm(key) => {
                lock_ctl(&shared.controller).choose_hook(&key);
                self.go(shared, 2);
            }
            Act::BackFromSearch => {
                if self.mode == WizardMode::New {
                    lock_ctl(&shared.controller).cancel_setup();
                    self.go(shared, 0);
                    self.windows_at = None;
                } else {
                    self.closed = true;
                }
            }
            Act::ToggleOthers => self.show_others = !self.show_others,
            Act::Report => {
                let result = lock_ctl(&shared.controller).save_report();
                match result {
                    Ok(path) => {
                        let path = path.to_string_lossy().into_owned();
                        winutil::reveal_in_explorer(&path);
                        self.messages.push((
                            "Отчёт сохранён".to_string(),
                            format!(
                                "Отчёт сохранён в файл:\n{path}\n\nПапка с ним открыта — пришлите этот файл для доработки."
                            ),
                        ));
                    }
                    Err(e) => self.messages.push(("Отчёт".to_string(), format!("Не удалось сохранить отчёт: {e}"))),
                }
            }
            Act::BackToSearch => self.go(shared, 1),
            Act::ResetPosition => lock_ctl(&shared.controller).reset_overlay_position(),
            Act::Finish => self.finish(shared),
            Act::Cancel => self.closed = true,
        }
    }

    fn draw(&mut self, ctx: &egui::Context, snap: &Snap, acts: &mut Vec<Act>) {
        if ctx.input(|i| i.viewport().close_requested()) {
            acts.push(Act::Cancel);
        }
        match self.step {
            0 => self.draw_step1(ctx, acts),
            1 => self.draw_step2(ctx, snap, acts),
            _ => self.draw_step3(ctx, acts),
        }
        self.modal(ctx);
    }

    fn modal(&mut self, ctx: &egui::Context) {
        let Some((title, text)) = self.messages.first().cloned() else { return };
        let mut ok = false;
        let m = egui::Modal::new(egui::Id::new("wizard_message")).show(ctx, |ui| {
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
        }
    }

    // ---------- шаг 1: окно игры ----------
    fn draw_step1(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let can_next = self.selected.is_some();
        egui::TopBottomPanel::bottom("wiz_foot1").show_separator_line(false).frame(footer_frame()).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            ui.horizontal(|ui| {
                ui.label(theme::body("Язык игры"));
                egui::ComboBox::from_id_salt("wiz_lang")
                    .width(240.0)
                    .selected_text(theme::body(lang_title(self.lang)))
                    .show_ui(ui, |ui| {
                        for l in Lang::ALL {
                            ui.selectable_value(&mut self.lang, l, lang_title(l));
                        }
                    });
            });
            ui.label(theme::small(format!(
                "Перевод на: {} — меняется в настройках, раздел «Перевод».",
                self.target_title
            )));
            ui.horizontal(|ui| {
                if button(ui, "Отмена", BtnKind::Normal, true, None).clicked() {
                    acts.push(Act::Cancel);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Далее  →", BtnKind::Primary, can_next, None).clicked() {
                        acts.push(Act::Start);
                    }
                });
            });
        });
        egui::CentralPanel::default().frame(central_frame()).show(ctx, |ui| {
            header(
                ui,
                1,
                "Выберите игру",
                "Запустите игру в оконном режиме и выберите её в списке. Браузеры и системные окна скрыты.",
            );
            if self.windows.is_empty() {
                wrap(ui, theme::dim("Не вижу запущенных игр. Запустите игру — она появится здесь сама."));
                return;
            }
            let mut picked: Option<isize> = None;
            let mut start = false;
            egui::ScrollArea::vertical().id_salt("wiz_windows").auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for w in &self.windows {
                    let resp = window_card(ui, w, self.selected == Some(w.hwnd));
                    if resp.clicked() || resp.double_clicked() {
                        picked = Some(w.hwnd);
                    }
                    if resp.double_clicked() {
                        start = true;
                    }
                }
            });
            if picked.is_some() {
                self.selected = picked;
            }
            if start {
                acts.push(Act::Start);
            }
        });
    }

    // ---------- шаг 2: поиск текста ----------
    fn draw_step2(&mut self, ctx: &egui::Context, snap: &Snap, acts: &mut Vec<Act>) {
        egui::TopBottomPanel::bottom("wiz_foot2").show_separator_line(false).frame(footer_frame()).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if button(ui, "←  Назад", BtnKind::Normal, true, None).clicked() {
                    acts.push(Act::BackFromSearch);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Текст не находится? Сохранить отчёт", BtnKind::Link, true, None).clicked() {
                        acts.push(Act::Report);
                    }
                });
            });
        });
        egui::CentralPanel::default().frame(central_frame()).show(ctx, |ui| {
            let p = pal();
            header(
                ui,
                2,
                "Поиск текста",
                "Перейдите в игру и прокликайте 3–4 реплики. NovelTL сам найдёт, откуда брать текст, и покажет его здесь.",
            );
            let best = snap.rows.first();
            let (color, text) = if !snap.has_session {
                (p.bad, "Игра закрыта. Запустите её и начните заново.".to_string())
            } else if !snap.error.is_empty() {
                (p.bad, snap.error.clone())
            } else if !snap.connected {
                (p.warn, "Подключаюсь к игре…".to_string())
            } else if let Some(b) = best {
                if snap.confident {
                    (p.ok, "Текст найден. Проверьте, что ниже — реплики из игры.".to_string())
                } else {
                    (
                        p.warn,
                        format!(
                            "Поймано реплик: {} из {}. Кликайте дальше.",
                            b.distinct.min(CONFIDENT_DISTINCT),
                            CONFIDENT_DISTINCT
                        ),
                    )
                }
            } else {
                (p.warn, "Подключено. Жду текст — кликайте в игре.".to_string())
            };
            wrap(ui, RichText::new(text).size(14.0).color(color));
            let shown = if snap.has_session { best } else { None };
            progress(ui, shown.map_or(0, |b| b.distinct.min(CONFIDENT_DISTINCT)), CONFIDENT_DISTINCT);

            // рамка с найденным текстом
            Frame::new()
                .fill(p.card)
                .stroke(Stroke::new(2.0_f32, p.accent))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin { left: 18, right: 18, top: 14, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label(theme::step("НАЙДЕННЫЙ ТЕКСТ"));
                    ui.vertical(|ui| {
                        ui.set_min_height(96.0);
                        ui.set_width(ui.available_width());
                        match shown {
                            Some(b) => {
                                let lines: Vec<String> = b.last.iter().map(|t| elide(t, 90)).collect();
                                wrap(ui, RichText::new(lines.join("\n")).size(17.0).family(theme::jp_family()).color(p.text));
                            }
                            None => wrap(
                                ui,
                                RichText::new(
                                    "Пока пусто.\nЗдесь появятся реплики, как только вы начнёте кликать в игре.",
                                )
                                .size(17.0)
                                .family(theme::jp_family())
                                .color(p.text),
                            ),
                        }
                    });
                    let hint = match shown {
                        Some(b) => {
                            let engine =
                                if snap.engine.is_empty() { String::new() } else { format!(" · движок {}", snap.engine) };
                            let name = if b.name.is_empty() { "хук" } else { b.name.as_str() };
                            format!("Источник: {name}{engine} · оценка {:.0}", b.score)
                        }
                        None => String::new(),
                    };
                    wrap(ui, theme::small(hint));
                    let w = ui.available_width();
                    if button(ui, "Это текст диалога", BtnKind::Big, shown.is_some(), Some(w)).clicked() {
                        if let Some(b) = shown {
                            acts.push(Act::Confirm(b.key.clone()));
                        }
                    }
                });

            let others: &[Row] = if snap.rows.len() > 1 { &snap.rows[1..] } else { &[] };
            if !others.is_empty() || self.show_others {
                let arrow = if self.show_others { "↓" } else { "→" };
                let label = format!("{arrow}  Это не то? Другие варианты ({})", others.len());
                if button(ui, &label, BtnKind::Link, true, None).clicked() {
                    acts.push(Act::ToggleOthers);
                }
            }
            if self.show_others {
                egui::ScrollArea::vertical().id_salt("wiz_others").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for r in others {
                        let line = r.last.last().map(|t| elide(t, 46)).unwrap_or_default();
                        let name = if r.name.is_empty() { "хук" } else { r.name.as_str() };
                        theme::card_frame().inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 8 }).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.label(theme::jp(line));
                                    ui.label(theme::small(format!("{name} · оценка {:.0}", r.score)));
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if button(ui, "Выбрать", BtnKind::Normal, true, None).clicked() {
                                        acts.push(Act::Confirm(r.key.clone()));
                                    }
                                });
                            });
                        });
                    }
                });
            }
        });
    }

    // ---------- шаг 3: плашка ----------
    fn draw_step3(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let position = self.mode == WizardMode::Position;
        egui::TopBottomPanel::bottom("wiz_foot3").show_separator_line(false).frame(footer_frame()).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !position && button(ui, "←  Назад", BtnKind::Normal, true, None).clicked() {
                    acts.push(Act::BackToSearch);
                }
                if position && button(ui, "Отмена", BtnKind::Normal, true, None).clicked() {
                    acts.push(Act::Cancel);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "Готово", BtnKind::Big, true, None).clicked() {
                        acts.push(Act::Finish);
                    }
                });
            });
        });
        egui::CentralPanel::default().frame(central_frame()).show(ctx, |ui| {
            header(
                ui,
                3,
                "Расположение плашки",
                "Поверх игры появилась плашка с примером перевода. Перетащите её мышью на окно диалога \
                 и растяните за края так, чтобы она закрыла оригинальный текст.",
            );
            theme::card_frame().inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                for tip in [
                    "•  Тяните за середину — плашка двигается, за края и углы — меняет размер.",
                    "•  Если окно игры изменит размер, плашка подстроится сама.",
                    "•  Шрифт, цвета и прозрачность меняются в «Настройках».",
                    "•  Не видно плашку? Разверните окно игры — плашка рисуется только поверх него.",
                ] {
                    wrap(ui, theme::body(tip));
                }
            });
            if button(ui, "Вернуть положение по умолчанию", BtnKind::Link, true, None).clicked() {
                acts.push(Act::ResetPosition);
            }
        });
    }
}
