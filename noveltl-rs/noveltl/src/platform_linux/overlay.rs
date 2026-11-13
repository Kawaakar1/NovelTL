//! Плашка с переводом для Linux: прозрачные окна egui поверх всех окон.
//!
//! В Linux программа не знает, где окно игры (Wayland этого не сообщает), поэтому плашка — отдельное
//! окно, а её место задаётся долями экрана и мышью: открыли замочек — двигаете и растягиваете,
//! закрыли — клики проходят сквозь плашку. Кнопки (замочек, меню) — отдельное маленькое окно поверх
//! плашки: в окно, пропускающее клики, нажать нельзя.
//!
//! Интерфейс тот же, что у Windows-плашки (`Overlay::start`, `send(Cmd)`); рисует плашку главный
//! цикл окон: `show(ctx)` вызывается каждый кадр.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Id, Painter, Pos2, Rect,
    ResizeDirection, Sense, Shape, Stroke, StrokeKind, ViewportBuilder, ViewportCommand, ViewportId,
};
use noveltl_core::i18n::t;

use crate::ui::theme::{icon, icon_family, Icon};

pub use crate::overlay_common::*;

const MIN_W: f32 = 220.0;
const MIN_H: f32 = 70.0;
const PAD: f32 = 18.0;
const MIN_FONT: f32 = 11.0;
const EDGE: f32 = 10.0;
const MENU_IDS: [&str; 5] =
    [crate::tray::ID_OPEN, crate::tray::ID_TOGGLE, crate::tray::ID_RETR, crate::tray::ID_SETTINGS, crate::tray::ID_QUIT];
const MENU_ROW: f32 = 30.0;
const MENU_W: f32 = 250.0;

struct State {
    events: Sender<Event>,
    target: isize,
    frac: Frac,
    style: Style,
    name: String,
    text: String,
    /// второй перевод (мелко под основным)
    alt: String,
    /// исходная реплика (рисуется мелко, если включено в стиле)
    orig: String,
    note: String,
    edit: bool,
    unlocked: bool,
    user_hidden: bool,
    menu_open: bool,
    /// курсор был над плашкой или панелью кнопок в прошлом кадре
    hover: bool,
    /// последнее положение окна плашки, о котором сообщили контроллеру
    reported: Option<Rect>,
}

impl State {
    fn should_show(&self) -> bool {
        let has_content = self.edit || !self.text.is_empty() || !self.note.is_empty();
        self.target != 0 && has_content && !(self.user_hidden && !self.edit)
    }

    fn movable(&self) -> bool {
        self.edit || self.unlocked
    }

    fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Target(h, frac) => {
                self.target = h;
                if let Some(f) = frac {
                    self.frac = f;
                }
            }
            Cmd::Line { name, text, alt, orig } => {
                self.name = name;
                self.text = text;
                self.alt = alt;
                self.orig = orig;
                self.note.clear();
            }
            Cmd::Note { note, keep_text } => {
                self.note = note;
                if !keep_text {
                    self.name.clear();
                    self.text.clear();
                    self.alt.clear();
                    self.orig.clear();
                }
            }
            Cmd::Clear => {
                self.name.clear();
                self.text.clear();
                self.alt.clear();
                self.orig.clear();
                self.note.clear();
            }
            Cmd::EditMode(on) => self.edit = on,
            Cmd::UserHidden(h) => self.user_hidden = h,
            Cmd::Style(s) => self.style = s,
            // OCR есть только в Windows
            Cmd::ForceActive(_) | Cmd::ExcludeFromCapture(_) | Cmd::Quit => {}
        }
    }
}

/// Плашка, которую сейчас рисует главный цикл окон (последняя созданная).
static CURRENT: Mutex<Option<Arc<Mutex<State>>>> = Mutex::new(None);
static CTX: OnceLock<egui::Context> = OnceLock::new();

/// Контекст окон: чтобы перерисовать плашку, когда приходит новая реплика.
pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Управление плашкой из любого потока.
#[derive(Clone)]
pub struct Overlay {
    state: Arc<Mutex<State>>,
}

impl Overlay {
    pub fn start() -> (Overlay, Receiver<Event>) {
        let (tx, rx) = channel::<Event>();
        let state = Arc::new(Mutex::new(State {
            events: tx,
            target: 0,
            frac: Frac::default(),
            style: Style::default(),
            name: String::new(),
            text: String::new(),
            alt: String::new(),
            orig: String::new(),
            note: String::new(),
            edit: false,
            unlocked: false,
            user_hidden: false,
            menu_open: false,
            hover: false,
            reported: None,
        }));
        *lock(&CURRENT) = Some(state.clone());
        (Overlay { state }, rx)
    }

    pub fn send(&self, cmd: Cmd) {
        lock(&self.state).apply(cmd);
        if let Some(ctx) = CTX.get() {
            ctx.request_repaint();
        }
    }

    /// Своего HWND в Linux нет.
    pub fn hwnd(&self) -> isize {
        0
    }
}

fn rgba(rgb: u32, a: f32) -> Color32 {
    let (r, g, b) = (((rgb >> 16) & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, (rgb & 0xFF) as u8);
    Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Место плашки на экране (в точках egui) по долям экрана.
fn plate_rect(f: Frac, screen: egui::Vec2) -> Rect {
    let w = (f.w * screen.x).max(MIN_W);
    let h = (f.h * screen.y).max(MIN_H);
    Rect::from_min_size(pos2(f.x * screen.x, f.y * screen.y), vec2(w, h))
}

fn frac_of(r: Rect, screen: egui::Vec2) -> Frac {
    let round = |v: f32| (v * 10000.0).round() / 10000.0;
    Frac {
        x: round((r.min.x / screen.x).clamp(-0.5, 1.0)),
        y: round((r.min.y / screen.y).clamp(-0.5, 1.0)),
        w: round((r.width() / screen.x).clamp(0.05, 1.5)),
        h: round((r.height() / screen.y).clamp(0.05, 1.5)),
    }
}

/// Рисует плашку; вызывается каждый кадр из главного цикла окон.
pub fn show(ctx: &egui::Context) {
    let Some(state) = lock(&CURRENT).clone() else { return };
    let mut s = lock(&state);
    if !s.should_show() {
        s.reported = None;
        s.menu_open = false;
        s.hover = false;
        return;
    }
    let screen = ctx.input(|i| i.viewport().monitor_size).filter(|v| v.x > 0.0 && v.y > 0.0).unwrap_or(vec2(1920.0, 1080.0));
    let plate = plate_rect(s.frac, screen);
    let movable = s.movable();

    let builder = ViewportBuilder::default()
        .with_title("NovelTL overlay")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_active(false)
        .with_mouse_passthrough(!movable)
        .with_position(plate.min)
        .with_inner_size(plate.size());
    ctx.show_viewport_immediate(ViewportId::from_hash_of("noveltl_overlay_plate"), builder, |ctx, _| {
        egui::CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
            let rect = ui.max_rect();
            draw_plate(ui.painter(), rect, &s);
            if movable {
                let resp = ui.interact(rect, Id::new("overlay_plate"), Sense::click_and_drag());
                let dir = ctx.pointer_hover_pos().and_then(|p| edge_dir(rect, p));
                if let Some(d) = dir {
                    ctx.set_cursor_icon(edge_cursor(d));
                } else if resp.hovered() {
                    ctx.set_cursor_icon(CursorIcon::Grab);
                }
                if resp.drag_started() {
                    // перемещение и растягивание делает сам оконный менеджер
                    ctx.send_viewport_cmd(match dir {
                        Some(d) => ViewportCommand::BeginResize(d),
                        None => ViewportCommand::StartDrag,
                    });
                }
            }
        });
        // окно передвинули или растянули — запоминаем новое место (в Wayland положение неизвестно)
        if movable && !ctx.input(|i| i.pointer.any_down()) {
            if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
                let moved = (r.min - plate.min).length() > 1.0 || (r.size() - plate.size()).length() > 1.0;
                if moved && s.reported != Some(r) {
                    s.reported = Some(r);
                    s.frac = frac_of(r, screen);
                    let _ = s.events.send(Event::Moved(s.frac));
                }
            }
        }
    });

    show_buttons(ctx, &mut s, plate);
}

/// Окно с панелью кнопок у правого верхнего угла плашки: те же кнопки и порядок, что в Windows.
fn show_buttons(ctx: &egui::Context, s: &mut State, plate: Rect) {
    let (pl, pt, pr, pb) = tb_pill(plate.width() as i32);
    let bar = vec2((pr - pl + TB_INSET) as f32, pb as f32 + 4.0);
    let size = if s.menu_open { vec2(MENU_W.max(bar.x), bar.y + 6.0 + MENU_IDS.len() as f32 * MENU_ROW + 8.0) } else { bar };
    let pos = pos2(plate.max.x - size.x, plate.min.y);
    let builder = ViewportBuilder::default()
        .with_title("NovelTL overlay buttons")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_active(false)
        .with_position(pos)
        .with_inner_size(size);
    let mut over = false;
    ctx.show_viewport_immediate(ViewportId::from_hash_of("noveltl_overlay_buttons"), builder, |ctx, _| {
        egui::CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
            let full = ui.max_rect();
            over = ui.rect_contains_pointer(full);
            // без наведения, замочка и настройки панель бледная: Linux не знает, где мышь над плашкой
            if !(s.hover || s.unlocked || s.edit || s.menu_open) {
                ui.set_opacity(0.35);
            }
            // приглушённая панель: почти прозрачна, пока курсор не над ней (в настройке — всегда яркая)
            if s.style.toolbar_dim && !s.edit && !over {
                ui.set_opacity(0.22);
            }
            // подложка-«пилюля»; координаты панели считаются от правого края окна кнопок
            let dx = full.max.x - plate.width();
            let pill = Rect::from_min_max(pos2(pl as f32 + dx, pt as f32), pos2(pr as f32 + dx, pb as f32));
            ui.painter().rect_filled(pill, CornerRadius::same(9), rgba(0x1B1C25, 1.0));
            ui.painter().rect_stroke(pill, CornerRadius::same(9), Stroke::new(1.0_f32, rgba(0xFFFFFF, 0.18)), StrokeKind::Inside);
            let icons = icon_family();
            for i in 0..TB_BUTTONS {
                let (l, t, r, bt) = tb_button(plate.width() as i32, i);
                let rect = Rect::from_min_max(pos2(l as f32 + dx, t as f32), pos2(r as f32 + dx, bt as f32));
                let resp = ui.interact(rect, Id::new(("overlay_btn", i)), Sense::click());
                let active = i == TB_ORIGINAL && s.style.show_original;
                if resp.hovered() || active {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), rgba(0xFFFFFF, if resp.hovered() { 0.18 } else { 0.10 }));
                }
                let alpha = if resp.hovered() || active { 1.0 } else { 0.8 };
                let glyph = match i {
                    0 => icon(Icon::Refresh),
                    1 => icon(Icon::Copy),
                    TB_ORIGINAL => icon(Icon::Original),
                    TB_LOCK if s.unlocked => icon(Icon::Unlock),
                    TB_LOCK => icon(Icon::Lock),
                    4 => icon(Icon::Settings),
                    5 => icon(Icon::EyeOff),
                    _ => icon(Icon::Menu),
                };
                let family = if i == TB_ORIGINAL { FontFamily::Proportional } else { icons.clone() };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::new(15.0, family), rgba(0xFFFFFF, alpha));
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    match TB_IDS[i] {
                        Some(id) => {
                            s.menu_open = false;
                            let _ = s.events.send(Event::Menu(id));
                        }
                        None if i == TB_LOCK => {
                            s.unlocked = !s.unlocked;
                            s.menu_open = false;
                        }
                        None => s.menu_open = !s.menu_open,
                    }
                }
            }
            if s.menu_open {
                let top = pb as f32 + 6.0;
                let box_rect = Rect::from_min_max(pos2(full.min.x, top), pos2(full.max.x, full.max.y));
                ui.painter().rect_filled(box_rect, CornerRadius::same(8), rgba(0x1C1C20, 0.97));
                for (k, id) in MENU_IDS.into_iter().enumerate() {
                    let row = Rect::from_min_size(pos2(box_rect.min.x + 4.0, top + 4.0 + k as f32 * MENU_ROW), vec2(box_rect.width() - 8.0, MENU_ROW));
                    let resp = ui.interact(row, Id::new(("overlay_menu", k)), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(5), rgba(0xFFFFFF, 0.12));
                    }
                    ui.painter().text(
                        pos2(row.min.x + 10.0, row.center().y),
                        egui::Align2::LEFT_CENTER,
                        menu_label(id),
                        FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                    if resp.clicked() {
                        s.menu_open = false;
                        let _ = s.events.send(Event::Menu(id));
                    }
                }
            }
        });
    });
    s.hover = over;
}

fn edge_dir(r: Rect, p: Pos2) -> Option<ResizeDirection> {
    let (l, t, rr, b) = (p.x - r.min.x <= EDGE, p.y - r.min.y <= EDGE, r.max.x - p.x <= EDGE, r.max.y - p.y <= EDGE);
    Some(match (l, t, rr, b) {
        (true, true, _, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, _, true, true) => ResizeDirection::SouthEast,
        (true, _, _, _) => ResizeDirection::West,
        (_, _, true, _) => ResizeDirection::East,
        (_, true, _, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

fn edge_cursor(d: ResizeDirection) -> CursorIcon {
    match d {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    }
}

/// Фон, текст с контуром, заметка и рамка режима перемещения.
fn draw_plate(p: &Painter, rect: Rect, s: &State) {
    let st = &s.style;
    let full = rect.shrink(1.0);
    p.rect_filled(full, CornerRadius::same(12), rgba(st.bg_color, st.bg_opacity.min(100) as f32 / 100.0));

    let (name, text) = if s.edit && s.text.is_empty() {
        (t("Акари"), t("«Так будет выглядеть перевод. Перетащите плашку на окно диалога игры и растяните по размеру»."))
    } else {
        (s.name.as_str(), s.text.as_str())
    };
    let inner_w = (rect.width() - 2.0 * PAD).max(10.0);
    let inner_h = (rect.height() - 1.2 * PAD).max(10.0);
    let (left, top) = (rect.min.x + PAD, rect.min.y + 0.6 * PAD);

    if !text.is_empty() {
        // автоуменьшение шрифта, пока текст не влезет
        let mut size = st.font_size.max(MIN_FONT);
        loop {
            let name_size = (size * 0.8).max(MIN_FONT);
            let font = |sz: f32| FontId::new(sz, FontFamily::Proportional);
            let name_g = (!name.is_empty()).then(|| p.layout(name.to_string(), font(name_size), rgba(st.name_color, 1.0), inner_w));
            let body_g = p.layout(text.to_string(), font(size), rgba(st.text_color, 1.0), inner_w);
            // второй перевод — мельче и бледнее, под основным (в режиме настройки — нет)
            let alt_size = (size * 0.7).max(MIN_FONT);
            let alt_g = (!s.alt.is_empty() && !s.edit).then(|| p.layout(s.alt.clone(), font(alt_size), rgba(0xC9CBE0, 1.0), inner_w));
            // оригинал — ещё мельче и бледнее, под переводом (по настройке; в режиме настройки — нет)
            let orig_size = (size * 0.62).max(MIN_FONT);
            let orig_g = (!s.orig.is_empty() && !s.edit && st.show_original)
                .then(|| p.layout(s.orig.clone(), font(orig_size), rgba(0xA2A4B8, 1.0), inner_w));
            let name_h = name_g.as_ref().map_or(0.0, |g| g.size().y + 2.0);
            let alt_h = alt_g.as_ref().map_or(0.0, |g| g.size().y + 4.0);
            let orig_h = orig_g.as_ref().map_or(0.0, |g| g.size().y + 4.0);
            if name_h + body_g.size().y + alt_h + orig_h <= inner_h || size <= MIN_FONT {
                let mut y = top;
                if let Some(g) = name_g {
                    outlined(p, pos2(left, y), g, st.outline_color, name_size);
                    y += name_h;
                }
                let body_h = body_g.size().y;
                outlined(p, pos2(left, y), body_g, st.outline_color, size);
                if let Some(g) = alt_g {
                    outlined(p, pos2(left, y + body_h + 4.0), g, st.outline_color, alt_size);
                }
                if let Some(g) = orig_g {
                    outlined(p, pos2(left, y + body_h + alt_h + 4.0), g, st.outline_color, orig_size);
                }
                break;
            }
            size -= 1.0;
        }
    }
    if !s.note.is_empty() {
        let small = if text.is_empty() { 18.0 } else { 15.0 };
        let g = p.layout(s.note.clone(), FontId::proportional(small), rgba(0xC9CBE0, 1.0), inner_w);
        let pos = if text.is_empty() {
            pos2(left, top + 4.0)
        } else {
            pos2(left + inner_w - g.size().x, rect.max.y - g.size().y - 6.0)
        };
        outlined(p, pos, g, st.outline_color, small);
    }
    if s.movable() {
        // рамка режима перемещения — белая пунктирная, с ручками по углам и сторонам
        let white = rgba(0xFFFFFF, 0.9);
        let c = [full.left_top(), full.right_top(), full.right_bottom(), full.left_bottom(), full.left_top()];
        p.extend(Shape::dashed_line(&c, Stroke::new(2.0_f32, white), 6.0, 4.0));
        let i = full.shrink(3.0);
        let (cx, cy) = (i.center().x, i.center().y);
        for pt in [
            i.left_top(),
            pos2(cx, i.min.y),
            i.right_top(),
            pos2(i.min.x, cy),
            pos2(i.max.x, cy),
            i.left_bottom(),
            pos2(cx, i.max.y),
            i.right_bottom(),
        ] {
            p.circle_filled(pt, 5.0, white);
        }
    }
    let _ = StrokeKind::Inside;
}

/// Текст с контуром: контур — тот же текст, нарисованный со сдвигами по кругу.
fn outlined(p: &Painter, pos: Pos2, galley: Arc<egui::Galley>, outline: u32, size: f32) {
    let r = (size / 13.0).max(1.2);
    let oc = rgba(outline, 1.0);
    for ring in [r, r * 0.55] {
        for i in 0..12 {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            p.galley_with_override_text_color(pos + vec2(a.cos() * ring, a.sin() * ring), galley.clone(), oc);
        }
    }
    p.galley(pos, galley, Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frac_round_trip() {
        let screen = vec2(1920.0, 1080.0);
        let f = Frac { x: 0.1, y: 0.7, w: 0.8, h: 0.2 };
        let back = frac_of(plate_rect(f, screen), screen);
        assert!((back.x - f.x).abs() < 1e-3 && (back.w - f.w).abs() < 1e-3 && (back.h - f.h).abs() < 1e-3);
        // слишком маленькая плашка растягивается до минимума
        assert!(plate_rect(Frac { x: 0.0, y: 0.0, w: 0.01, h: 0.01 }, screen).width() >= MIN_W);
    }
}
