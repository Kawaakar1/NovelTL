//! Окно «Глоссарий — <игра>» (перенос `GlossaryDialog` из `noveltl/ui/dialogs.py`).
//! Таблица из двух столбцов («В игре» / «В переводе») в `Grid` внутри `ScrollArea`.

use noveltl_core::i18n::{t, tf};
use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, Frame, Id, Layout, Margin, RichText, Sense, ViewportBuilder, ViewportId,
};
use noveltl_core::profiles::GlossaryEntry;

use super::theme::{self, button, pal, BtnKind};
use super::{lock_ctl, Shared};

struct Row {
    uid: u64,
    src: String,
    dst: String,
    note: String,
}

enum Act {
    Add,
    Remove,
    Save,
    Cancel,
}

pub struct GlossaryWindow {
    id: ViewportId,
    icon: Arc<egui::IconData>,
    pos: Option<[f32; 2]>,
    profile_id: String,
    title: String,
    rows: Vec<Row>,
    filters: String,
    next_uid: u64,
    selected: Option<u64>,
    focus: Option<Id>,
    closed: bool,
    frames: u32,
}

impl GlossaryWindow {
    /// None — профиль не найден.
    pub fn new(shared: &Shared, profile_id: &str, pos: Option<[f32; 2]>) -> Option<GlossaryWindow> {
        let profile = {
            let c = lock_ctl(&shared.controller);
            c.session
                .as_ref()
                .filter(|s| s.profile.id == profile_id)
                .map(|s| s.profile.clone())
                .or_else(|| c.store.get(profile_id))
        }?;
        let mut w = GlossaryWindow {
            id: ViewportId::from_hash_of("noveltl_glossary"),
            icon: Arc::new(theme::icon_data(64)),
            pos,
            profile_id: profile_id.to_string(),
            title: tf("Глоссарий — {}", &[&profile.name]),
            rows: Vec::new(),
            filters: profile.filters.join("\n"),
            next_uid: 0,
            selected: None,
            focus: None,
            closed: false,
            frames: 0,
        };
        for e in &profile.glossary {
            w.push_row(&e.src, &e.dst, &e.note);
        }
        Some(w)
    }

    pub fn viewport_id(&self) -> ViewportId {
        self.id
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// Заголовок окна (по нему окно находят для снимка с экрана).
    pub fn window_title(&self) -> String {
        self.title.clone()
    }

    /// Сколько раз окно уже отрисовано.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Закрыть как «Отмена» (для `--shots`).
    pub fn cancel(&mut self) {
        self.closed = true;
    }

    /// Для `--shots`: заменить строки (только в окне, без сохранения).
    pub fn fill_for_shots(&mut self, entries: &[(&str, &str)]) {
        self.rows.clear();
        self.selected = None;
        for (s, d) in entries {
            self.push_row(s, d, "");
        }
    }

    fn push_row(&mut self, src: &str, dst: &str, note: &str) -> u64 {
        let uid = self.next_uid;
        self.next_uid += 1;
        self.rows.push(Row { uid, src: src.to_string(), dst: dst.to_string(), note: note.to_string() });
        uid
    }

    /// Рисует окно; false — окно нужно закрыть.
    pub fn show(&mut self, ctx: &egui::Context, shared: &Shared) -> bool {
        self.frames += 1;
        let mut builder = ViewportBuilder::default()
            .with_title(self.title.clone())
            .with_inner_size([760.0, 620.0])
            .with_min_inner_size([560.0, 420.0])
            .with_always_on_top()
            .with_icon(self.icon.clone());
        if let Some(pos) = self.pos {
            builder = builder.with_position(pos);
        }
        let mut acts: Vec<Act> = Vec::new();
        let id = self.id;
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            self.draw(ctx, &mut acts);
        });
        for a in acts {
            match a {
                Act::Add => {
                    let uid = self.push_row("", "", "");
                    self.selected = Some(uid);
                    self.focus = Some(Id::new(("gl_src", uid)));
                }
                Act::Remove => {
                    if let Some(sel) = self.selected.take() {
                        self.rows.retain(|r| r.uid != sel);
                    }
                }
                Act::Save => {
                    // пустые строки отбрасываются
                    let entries: Vec<GlossaryEntry> = self
                        .rows
                        .iter()
                        .filter_map(|r| {
                            let (s, d) = (r.src.trim(), r.dst.trim());
                            (!s.is_empty() && !d.is_empty())
                                .then(|| GlossaryEntry {
                                    src: s.to_string(),
                                    dst: d.to_string(),
                                    note: r.note.trim().to_string(),
                                })
                        })
                        .collect();
                    let filters: Vec<String> =
                        self.filters.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                    lock_ctl(&shared.controller).save_glossary(&self.profile_id, entries, filters);
                    self.closed = true;
                }
                Act::Cancel => self.closed = true,
            }
        }
        !self.closed
    }

    fn draw(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        if ctx.input(|i| i.viewport().close_requested()) {
            acts.push(Act::Cancel);
        }
        let p = pal();
        egui::TopBottomPanel::bottom("gl_foot")
            .show_separator_line(false)
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 24, right: 24, top: 6, bottom: 20 }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if button(ui, t("＋ Добавить"), BtnKind::Normal, true, None).clicked() {
                        acts.push(Act::Add);
                    }
                    if button(ui, t("Удалить выбранное"), BtnKind::Normal, self.selected.is_some(), None).clicked() {
                        acts.push(Act::Remove);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, t("Сохранить"), BtnKind::Primary, true, None).clicked() {
                            acts.push(Act::Save);
                        }
                        if button(ui, t("Отмена"), BtnKind::Normal, true, None).clicked() {
                            acts.push(Act::Cancel);
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("gl_filters")
            .show_separator_line(false)
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 24, right: 24, top: 6, bottom: 6 }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(RichText::new(t("Убирать из текста этой игры")).size(13.0).family(theme::bold_family()).color(p.dim));
                ui.add(
                    egui::Label::new(theme::small(t(
                        "По одному фрагменту на строку — например, служебные значки, которые попадают в перевод.",
                    )))
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut self.filters)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(8, 6)),
                );
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 24, right: 24, top: 20, bottom: 8 }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 12.0;
                ui.label(theme::h2(t("Глоссарий имён и терминов")));
                ui.add(
                    egui::Label::new(theme::dim(
                        t("Слева — как написано в игре, справа — как должно быть в переводе. \
                         Так имена героев всегда переводятся одинаково."),
                    ))
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add(
                    egui::Label::new(theme::dim(t(
                        "Заметку получают DeepL и локальный ИИ, когда имя есть в реплике, — так легче угадать род и тон.",
                    )))
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                self.draw_table(ui);
            });
    }

    fn draw_table(&mut self, ui: &mut egui::Ui) {
        let p = pal();
        let free = (ui.available_width() - 40.0 - 3.0 * 8.0 - 16.0).max(240.0);
        let col_w = free * 0.28;
        let note_w = free - 2.0 * col_w;
        let sel_index = self.selected.and_then(|s| self.rows.iter().position(|r| r.uid == s));
        let selected_fill: Color32 = p.selected;
        let focus = self.focus.take();
        let mut clicked: Option<u64> = None;
        egui::ScrollArea::vertical().id_salt("gl_scroll").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("gl_grid")
                .num_columns(4)
                .spacing([8.0, 6.0])
                .with_row_color(move |row, _| (sel_index.is_some_and(|i| i + 1 == row)).then_some(selected_fill))
                .show(ui, |ui| {
                    let head = |t: &str| RichText::new(t).size(13.0).family(theme::bold_family()).color(pal().dim);
                    ui.label("");
                    ui.label(head(t("В игре")));
                    ui.label(head(t("В переводе")));
                    ui.label(head(t("Заметка для ИИ")));
                    ui.end_row();
                    for (i, r) in self.rows.iter_mut().enumerate() {
                        let num = ui.add_sized(
                            [40.0, 30.0],
                            egui::Label::new(theme::small(format!("{}", i + 1))).sense(Sense::click()),
                        );
                        if num.clicked() {
                            clicked = Some(r.uid);
                        }
                        let src_id = Id::new(("gl_src", r.uid));
                        let dst_id = Id::new(("gl_dst", r.uid));
                        if focus == Some(src_id) {
                            ui.memory_mut(|m| m.request_focus(src_id));
                        }
                        for (id, text) in [(src_id, &mut r.src), (dst_id, &mut r.dst)] {
                            let resp = ui.add_sized(
                                [col_w, 30.0],
                                egui::TextEdit::singleline(text).id(id).margin(Margin::symmetric(8, 6)),
                            );
                            if resp.clicked() || resp.gained_focus() {
                                clicked = Some(r.uid);
                            }
                            if focus == Some(id) {
                                resp.scroll_to_me(None);
                            }
                        }
                        let note_id = Id::new(("gl_note", r.uid));
                        let resp = ui.add_sized(
                            [note_w, 30.0],
                            egui::TextEdit::singleline(&mut r.note)
                                .id(note_id)
                                .hint_text(t("например: девушка, говорит вежливо"))
                                .margin(Margin::symmetric(8, 6)),
                        );
                        if resp.clicked() || resp.gained_focus() {
                            clicked = Some(r.uid);
                        }
                        ui.end_row();
                    }
                });
            if self.rows.is_empty() {
                ui.add_space(8.0);
                ui.label(theme::small(t("Пока пусто. Нажмите «＋ Добавить».")));
            }
        });
        if clicked.is_some() {
            self.selected = clicked;
        }
    }
}
