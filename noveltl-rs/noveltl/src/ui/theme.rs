//! Темы оформления (чёрно-белые, цветные, своя) и значок приложения (перенос `noveltl/ui/theme.py`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Frame, Layout, Margin, Pos2, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, Theme, ThemePreference, Ui, UiBuilder, Vec2, Visuals,
};
use noveltl_core::profiles::AppSettings;

#[allow(dead_code)]
pub const THEME_DARK: &str = "dark";
#[allow(dead_code)]
pub const THEME_LIGHT: &str = "light";

/// Режимы темы: (id, название).
pub const MODES: [(&str, &str); 4] = [
    ("system", "Как в Windows"),
    ("dark", "Тёмная"),
    ("light", "Светлая"),
    ("custom", "Своя"),
];

/// Акценты: (hex или "" для монохрома, название).
pub const ACCENTS: [(&str, &str); 6] = [
    ("", "Монохром"),
    ("#8B7CFF", "Фиолетовый"),
    ("#4A9DFF", "Синий"),
    ("#2FBF8A", "Зелёный"),
    ("#FF6B9A", "Розовый"),
    ("#FFB547", "Янтарный"),
];

// набор цветов целиком, как в theme.py (часть пока не используется)
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub border: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub bad: Color32,
    pub selected: Color32,
    pub menu_selected: Color32,
    pub disabled_text: Color32,
    pub disabled_border: Color32,
    pub primary_disabled: Color32,
    pub primary_disabled_text: Color32,
    pub notice_bg: Color32,
    pub handle: Color32,
}

const fn rgb(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// Чёрно-белая тёмная.
const BW_DARK: Palette = Palette {
    bg: rgb(0x0B0B0C),
    panel: rgb(0x121214),
    card: rgb(0x1A1A1D),
    border: rgb(0x2C2C30),
    text: rgb(0xF2F2F2),
    dim: rgb(0x9A9A9F),
    accent: rgb(0xFFFFFF),
    accent_hover: rgb(0xD9D9D9),
    on_accent: rgb(0x0B0B0C),
    ok: rgb(0x5DD39E),
    warn: rgb(0xF2B45A),
    bad: rgb(0xF07178),
    selected: rgb(0x26262A),
    menu_selected: rgb(0x303034),
    disabled_text: rgb(0x5A5A5F),
    disabled_border: rgb(0x242428),
    primary_disabled: rgb(0x3A3A3F),
    primary_disabled_text: rgb(0x8A8A8F),
    notice_bg: rgb(0x3A3120),
    handle: rgb(0xF2F2F2),
};

/// Чёрно-белая светлая.
const BW_LIGHT: Palette = Palette {
    bg: rgb(0xF5F5F5),
    panel: rgb(0xEBEBEB),
    card: rgb(0xFFFFFF),
    border: rgb(0xD4D4D4),
    text: rgb(0x111111),
    dim: rgb(0x666666),
    accent: rgb(0x111111),
    accent_hover: rgb(0x333333),
    on_accent: rgb(0xFFFFFF),
    ok: rgb(0x178A5B),
    warn: rgb(0xA8660B),
    bad: rgb(0xCF3B48),
    selected: rgb(0xE2E2E2),
    menu_selected: rgb(0xD8D8D8),
    disabled_text: rgb(0xAAAAAA),
    disabled_border: rgb(0xE0E0E0),
    primary_disabled: rgb(0xC8C8C8),
    primary_disabled_text: rgb(0xFFFFFF),
    notice_bg: rgb(0xFFF1D0),
    handle: rgb(0x111111),
};

/// Текущая палитра и признак «visuals уже выставлены».
struct Current {
    pal: Palette,
    applied: bool,
}

static CURRENT: RwLock<Current> = RwLock::new(Current { pal: BW_DARK, applied: false });

/// Текущая палитра (копия).
pub fn pal() -> Palette {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).pal
}

// ---------- вычисление цветов ----------

/// Смесь двух цветов: `t` = 0 даёт `a`, 1 даёт `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Яркость цвета, 0..1.
fn luma(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// Тёмная ли палитра — по яркости фона.
pub fn is_dark(p: &Palette) -> bool {
    luma(p.bg) < 0.5
}

/// Перекрашивает в `accent` всё, что от него зависит (фон, текст и карточки остаются).
pub fn with_accent(p: Palette, accent: Color32) -> Palette {
    let bg = p.bg;
    let accent_hover = if luma(accent) > 0.5 { mix(accent, Color32::BLACK, 0.15) } else { mix(accent, Color32::WHITE, 0.2) };
    let on_accent = if luma(accent) > 0.55 { rgb(0x0B0B0C) } else { Color32::WHITE };
    Palette {
        accent,
        accent_hover,
        on_accent,
        selected: mix(bg, accent, 0.2),
        menu_selected: mix(bg, accent, 0.3),
        primary_disabled: mix(bg, accent, 0.35),
        primary_disabled_text: mix(on_accent, bg, 0.3),
        handle: accent,
        ..p
    }
}

/// Из четырёх основных цветов выводит остальные.
pub fn custom_palette(bg: Color32, card: Color32, text: Color32, accent: Color32) -> Palette {
    let dark = luma(bg) < 0.5;
    let (ok, warn, bad) = if dark {
        (rgb(0x5DD39E), rgb(0xF2B45A), rgb(0xF07178))
    } else {
        (rgb(0x178A5B), rgb(0xA8660B), rgb(0xCF3B48))
    };
    let notice_bg = if dark { mix(bg, rgb(0xF2B45A), 0.2) } else { mix(bg, rgb(0xFFD25A), 0.3) };
    let base = Palette {
        bg,
        panel: mix(bg, card, 0.5),
        card,
        border: mix(card, text, 0.15),
        text,
        dim: mix(text, bg, 0.4),
        ok,
        warn,
        bad,
        disabled_text: mix(text, bg, 0.65),
        disabled_border: mix(bg, card, 0.6),
        notice_bg,
        ..BW_DARK
    };
    with_accent(base, accent)
}

/// "#RRGGBB" -> цвет; при ошибке — `fallback`.
pub fn parse_hex(s: &str, fallback: Color32) -> Color32 {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 || !t.is_ascii() {
        return fallback;
    }
    match u32::from_str_radix(t, 16) {
        Ok(v) => rgb(v),
        Err(_) => fallback,
    }
}

/// Цвет -> "#RRGGBB".
pub fn to_hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// Палитра пресета; неизвестный id — как «system».
pub fn palette_for(theme: &str, s: &AppSettings, system_dark: bool) -> Palette {
    let base = match theme {
        "dark" => BW_DARK,
        "light" => BW_LIGHT,
        "custom" => custom_palette(
            parse_hex(&s.custom_bg, rgb(0x101012)),
            parse_hex(&s.custom_card, rgb(0x1C1C20)),
            parse_hex(&s.custom_text, rgb(0xF2F2F2)),
            parse_hex(&s.accent, parse_hex(&s.custom_accent, Color32::WHITE)),
        ),
        _ => {
            if system_dark {
                BW_DARK
            } else {
                BW_LIGHT
            }
        }
    };
    if theme == "custom" {
        return base;
    }
    // TRANSPARENT — признак «акцент не задан или не разобрался»
    match parse_hex(&s.accent, Color32::TRANSPARENT) {
        Color32::TRANSPARENT => base,
        a => with_accent(base, a),
    }
}

fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.window_fill = p.bg;
    v.panel_fill = p.bg;
    v.extreme_bg_color = p.panel;
    v.faint_bg_color = p.card;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.selected;
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    let r = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.card;
    w.inactive.weak_bg_fill = p.card;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = p.card;
    w.hovered.weak_bg_fill = p.card;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.hovered.corner_radius = r;
    w.active.bg_fill = p.border;
    w.active.weak_bg_fill = p.border;
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.active.corner_radius = r;
    w.open.bg_fill = p.card;
    w.open.weak_bg_fill = p.card;
    w.open.corner_radius = r;
    v
}

/// Один раз при старте: стили обеих тем.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&BW_DARK, true));
    ctx.set_visuals_of(Theme::Light, visuals(&BW_LIGHT, false));
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(egui::TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
        s.text_styles.insert(egui::TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        s.interaction.tooltip_delay = 0.4;
    });
}

/// Применяет тему из настроек; возвращает true, если палитра изменилась.
/// Visuals пересоздаются только при изменении.
pub fn apply(ctx: &egui::Context, s: &AppSettings) -> bool {
    let system_dark = !matches!(ctx.system_theme(), Some(Theme::Light));
    let np = palette_for(&s.theme, s, system_dark);
    let dark = is_dark(&np);
    let want = if dark { Theme::Dark } else { Theme::Light };
    let mut cur = CURRENT.write().unwrap_or_else(|e| e.into_inner());
    let changed = cur.pal != np || !cur.applied;
    if changed || ctx.theme() != want {
        cur.pal = np;
        cur.applied = true;
        ctx.set_theme(if dark { ThemePreference::Dark } else { ThemePreference::Light });
        ctx.set_visuals_of(want, visuals(&np, dark));
    }
    changed
}


// ---------- стили текста ----------

pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn jp_family() -> FontFamily {
    FontFamily::Name("jp".into())
}

/// Семейство значков: системный Segoe Fluent Icons / Segoe MDL2 Assets (если нашёлся) + шрифты egui.
pub fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

static ICON_FONT: AtomicBool = AtomicBool::new(false);

pub fn set_icon_font(loaded: bool) {
    ICON_FONT.store(loaded, Ordering::Relaxed);
}

/// Загружен ли системный шрифт значков.
pub fn has_icon_font() -> bool {
    ICON_FONT.load(Ordering::Relaxed)
}

// часть значков нужна только плашке
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Palette,
    Translate,
    Overlay,
    Network,
    Keyboard,
    Book,
    Settings,
    Refresh,
    Copy,
    Lock,
    Unlock,
    Eye,
    EyeOff,
    Menu,
    Plus,
    Chevron,
    Original,
}

/// Значок: кодовая точка Segoe Fluent/MDL2, а без шрифта — символ из встроенных шрифтов egui.
pub fn icon(i: Icon) -> &'static str {
    // (Segoe, запасной): запасные символы проверены по таблицам Noto Emoji / emoji-icon-font / Hack / Ubuntu
    let (seg, fallback) = match i {
        Icon::Palette => ("\u{E790}", "🎨"),
        Icon::Translate => ("\u{E8C1}", "🔤"),
        Icon::Overlay => ("\u{E8BD}", "💬"),
        Icon::Network => ("\u{E774}", "🌐"),
        Icon::Keyboard => ("\u{E765}", "⌨"),
        Icon::Book => ("\u{E82D}", "📖"),
        Icon::Settings => ("\u{E713}", "⚙"),
        Icon::Refresh => ("\u{E72C}", "↻"),
        Icon::Copy => ("\u{E8C8}", "📋"),
        Icon::Lock => ("\u{E72E}", "🔒"),
        Icon::Unlock => ("\u{E785}", "🔓"),
        Icon::Eye => ("\u{E7B3}", "👁"),
        Icon::EyeOff => ("\u{ED1A}", "🙈"),
        Icon::Menu => ("\u{E700}", "☰"),
        Icon::Plus => ("\u{E710}", "✚"),
        Icon::Chevron => ("\u{E76C}", "›"),
        Icon::Original => ("あ", "あ"),
    };
    if has_icon_font() {
        seg
    } else {
        fallback
    }
}

pub fn h1(text: impl Into<String>) -> RichText {
    RichText::new(text).size(22.0).family(bold_family()).color(pal().text)
}

pub fn h2(text: impl Into<String>) -> RichText {
    RichText::new(text).size(17.0).family(bold_family()).color(pal().text)
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).size(14.0).color(pal().dim)
}

pub fn small(text: impl Into<String>) -> RichText {
    RichText::new(text).size(12.0).color(pal().dim)
}

pub fn step(text: impl Into<String>) -> RichText {
    RichText::new(text).size(12.0).family(bold_family()).color(pal().accent)
}

pub fn jp(text: impl Into<String>) -> RichText {
    RichText::new(text).size(15.0).family(jp_family()).color(pal().dim)
}

pub fn tr(text: impl Into<String>) -> RichText {
    RichText::new(text).size(16.0).color(pal().text)
}

pub fn body(text: impl Into<String>) -> RichText {
    RichText::new(text).size(14.0).color(pal().text)
}

// ---------- кнопки и карточки ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    Normal,
    /// акцентная заливка
    Primary,
    /// большая акцентная
    Big,
    /// «ссылка»
    Link,
}

/// Кнопка по образцу QSS: `width` — растянуть до ширины (None — по тексту).
pub fn button(ui: &mut Ui, text: &str, kind: BtnKind, enabled: bool, width: Option<f32>) -> Response {
    let p = pal();
    let (size, family, pad) = match kind {
        BtnKind::Normal => (14.0, FontFamily::Proportional, Vec2::new(16.0, 8.0)),
        BtnKind::Primary => (14.0, bold_family(), Vec2::new(16.0, 8.0)),
        BtnKind::Big => (17.0, bold_family(), Vec2::new(24.0, 14.0)),
        BtnKind::Link => (14.0, FontFamily::Proportional, Vec2::new(2.0, 4.0)),
    };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    // сначала измеряем текст любым цветом, потом перерисовываем нужным
    let measure = ui.painter().layout_no_wrap(text.to_string(), FontId::new(size, family.clone()), Color32::WHITE);
    let w = (measure.size().x + 2.0 * pad.x).max(width.unwrap_or(0.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, measure.size().y + 2.0 * pad.y), sense);
    let hover = enabled && resp.hovered();
    let pressed = enabled && resp.is_pointer_button_down_on();
    let radius = if kind == BtnKind::Big { 10 } else { 8 };
    let (fill, stroke, color) = match kind {
        BtnKind::Normal => {
            let fill = if pressed { p.border } else { p.card };
            let stroke = if !enabled {
                p.disabled_border
            } else if hover {
                p.accent
            } else {
                p.border
            };
            (fill, Stroke::new(1.0_f32, stroke), if enabled { p.text } else { p.disabled_text })
        }
        BtnKind::Primary | BtnKind::Big => {
            if !enabled {
                (p.primary_disabled, Stroke::NONE, p.primary_disabled_text)
            } else if hover {
                (p.accent_hover, Stroke::NONE, p.on_accent)
            } else {
                (p.accent, Stroke::NONE, p.on_accent)
            }
        }
        BtnKind::Link => (Color32::TRANSPARENT, Stroke::NONE, if hover { p.accent_hover } else { p.accent }),
    };
    if kind != BtnKind::Link {
        ui.painter().rect(rect, CornerRadius::same(radius), fill, stroke, StrokeKind::Inside);
    }
    let font = FontId::new(size, family);
    if kind == BtnKind::Link {
        ui.painter().text(rect.left_center() + Vec2::new(pad.x, 0.0), Align2::LEFT_CENTER, text, font, color);
    } else {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, color);
    }
    resp
}

pub fn card_frame() -> Frame {
    let p = pal();
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(16, 12))
}

/// Горизонтальная линия на всю ширину.
pub fn hline(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, pal().border);
}

// ---------- строки настроек: подпись, карточка, строка, переключатель, сегменты ----------

/// Маленькая тусклая подпись над карточкой.
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(small(text));
}

fn first_row_id() -> egui::Id {
    egui::Id::new("theme_card_first_row")
}

/// Карточка: заливка, рамка 1 px, скругление 10. Внутри — `row`.
pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    ui.data_mut(|d| d.insert_temp(first_row_id(), true));
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

/// Строка карточки с управлением шириной до 220 px.
pub fn row(ui: &mut Ui, title: &str, desc: Option<&str>, add: impl FnOnce(&mut Ui)) {
    row_w(ui, title, desc, 220.0, add);
}

/// Строка карточки: слева название и пояснение, справа элемент шириной `need`.
/// Если места мало, элемент переносится под текст — наложения не бывает.
pub fn row_w(ui: &mut Ui, title: &str, desc: Option<&str>, need: f32, add: impl FnOnce(&mut Ui)) {
    let p = pal();
    let first = ui.data_mut(|d| {
        let f = d.get_temp::<bool>(first_row_id()).unwrap_or(false);
        d.insert_temp(first_row_id(), false);
        f
    });
    if !first {
        hline(ui);
    }
    ui.add_space(12.0);
    let w = ui.available_width();
    let wide = need > 0.0 && w >= need + 190.0;
    let text_w = if wide { w - need - 16.0 } else { w };
    let tg = ui.painter().layout(title.to_string(), FontId::new(14.0, FontFamily::Proportional), p.text, text_w);
    let dg = desc.map(|d| ui.painter().layout(d.to_string(), FontId::new(12.0, FontFamily::Proportional), p.dim, text_w));
    let th = tg.size().y + dg.as_ref().map_or(0.0, |g| g.size().y + 2.0);
    let h = if wide { th.max(32.0) } else { th };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let top = rect.top() + (h - th) / 2.0;
    let dpos = Pos2::new(rect.left(), top + tg.size().y + 2.0);
    ui.painter().galley(Pos2::new(rect.left(), top), tg, p.text);
    if let Some(g) = dg {
        ui.painter().galley(dpos, g, p.dim);
    }
    if wide {
        let ctl = Rect::from_min_size(Pos2::new(rect.right() - need, rect.top()), Vec2::new(need, h));
        let b = UiBuilder::new().id_salt(("row_ctl", title)).max_rect(ctl).layout(Layout::left_to_right(Align::Center));
        add(&mut ui.new_child(b));
    } else if need > 0.0 {
        ui.add_space(8.0);
        ui.horizontal(add);
    }
    ui.add_space(12.0);
}

/// Пояснение на всю ширину карточки (без элемента управления).
pub fn note(ui: &mut Ui, text: RichText) {
    hline(ui);
    ui.add_space(10.0);
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Wrap));
    ui.add_space(10.0);
}

/// Переключатель-«пилюля» 36x20.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = pal();
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    // ручка — цвет текста на акценте: у монохромной тёмной темы акцент белый, белая ручка на нём не видна
    let (fill, knob, x) = if *on { (p.accent, p.on_accent, rect.right() - 10.0) } else { (p.border, Color32::WHITE, rect.left() + 10.0) };
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), 7.0, knob);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Ширина сегментного переключателя с такими подписями.
pub fn segmented_width(ui: &Ui, labels: &[&str]) -> f32 {
    let f = FontId::new(13.0, FontFamily::Proportional);
    let w: f32 = labels.iter().map(|l| ui.painter().layout_no_wrap(l.to_string(), f.clone(), Color32::WHITE).size().x + 24.0).sum();
    w + 6.0
}

/// Сегментный переключатель: тёмная подложка, выбранный сегмент подсвечен. Возвращает номер нажатого.
pub fn segmented(ui: &mut Ui, labels: &[&str], cur: usize) -> Option<usize> {
    let p = pal();
    let f = FontId::new(13.0, FontFamily::Proportional);
    let widths: Vec<f32> =
        labels.iter().map(|l| ui.painter().layout_no_wrap(l.to_string(), f.clone(), Color32::WHITE).size().x + 24.0).collect();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(widths.iter().sum::<f32>() + 6.0, 30.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(8), p.bg, Stroke::new(1.0_f32, p.border), StrokeKind::Inside);
    let mut x = rect.left() + 3.0;
    let mut clicked = None;
    for (i, l) in labels.iter().enumerate() {
        let seg = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(widths[i], 24.0));
        let resp = ui.interact(seg, ui.id().with(("seg", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if i == cur {
            ui.painter().rect_filled(seg, CornerRadius::same(6), p.selected);
        }
        let color = if i == cur { p.text } else { p.dim };
        ui.painter().text(seg.center(), Align2::CENTER_CENTER, *l, f.clone(), color);
        if resp.clicked() {
            clicked = Some(i);
        }
        x += widths[i];
    }
    clicked
}

// ---------- значок программы ----------

const GLYPH: &[(f32, f32, f32, f32)] = &[
    // 言: точка, длинная черта, две короткие, «рот»
    (24.0, 4.0, 30.0, 14.0),
    (6.0, 24.0, 46.0, 24.0),
    (10.0, 36.0, 42.0, 36.0),
    (10.0, 48.0, 42.0, 48.0),
    (12.0, 60.0, 40.0, 60.0),
    (12.0, 60.0, 12.0, 90.0),
    (40.0, 60.0, 40.0, 90.0),
    (12.0, 90.0, 40.0, 90.0),
    // 尺: верхняя черта, правый загиб, левая вертикаль, две диагонали
    (58.0, 14.0, 92.0, 14.0),
    (92.0, 14.0, 92.0, 36.0),
    (60.0, 14.0, 58.0, 52.0),
    (78.0, 40.0, 60.0, 90.0),
    (72.0, 48.0, 96.0, 90.0),
];

fn seg_dist(px: f32, py: f32, s: (f32, f32, f32, f32)) -> f32 {
    let (x1, y1, x2, y2) = s;
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0 { 0.0 } else { (((px - x1) * dx + (py - y1) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (x1 + t * dx, y1 + t * dy);
    ((px - cx) * (px - cx) + (py - cy) * (py - cy)).sqrt()
}

/// Значок: градиентный скруглённый квадрат с иероглифом 訳 (RGBA, не премультиплицированный).
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let ss = 4u32;
    let radius = s * 0.22;
    let gsize = s * 0.66;
    let gx0 = (s - gsize) / 2.0;
    let gy0 = (s - gsize) / 2.0 - s * 0.03;
    let half = (gsize * 0.036).max(0.55);
    let (c0, c1) = ((0xA0 as f32, 0x94 as f32, 0xFF as f32), (0xE8 as f32, 0x6A as f32, 0xA6 as f32));
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (mut n_in, mut r, mut g, mut b) = (0u32, 0.0f32, 0.0f32, 0.0f32);
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f32 + (sx as f32 + 0.5) / ss as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / ss as f32;
                    let dx = (radius - px).max(px - (s - radius)).max(0.0);
                    let dy = (radius - py).max(py - (s - radius)).max(0.0);
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    n_in += 1;
                    // координаты в «сотенной» коробке иероглифа
                    let (u, v) = ((px - gx0) / gsize * 100.0, (py - gy0) / gsize * 100.0);
                    let in_glyph = GLYPH.iter().any(|&sg| seg_dist(u, v, sg) * gsize / 100.0 <= half);
                    if in_glyph {
                        r += 255.0;
                        g += 255.0;
                        b += 255.0;
                    } else {
                        let t = ((px + py) / (2.0 * s)).clamp(0.0, 1.0);
                        r += c0.0 + (c1.0 - c0.0) * t;
                        g += c0.1 + (c1.1 - c0.1) * t;
                        b += c0.2 + (c1.2 - c0.2) * t;
                    }
                }
            }
            let i = ((y * size + x) * 4) as usize;
            if n_in > 0 {
                let n = n_in as f32;
                out[i] = (r / n).round() as u8;
                out[i + 1] = (g / n).round() as u8;
                out[i + 2] = (b / n).round() as u8;
                out[i + 3] = (255.0 * n / (ss * ss) as f32).round() as u8;
            }
        }
    }
    out
}

pub fn icon_data(size: u32) -> egui::IconData {
    egui::IconData { rgba: icon_rgba(size), width: size, height: size }
}

// ---------- плавность, акцентная кнопка, картинки игр, календарь чтения ----------

/// Плавное значение 0..1 для подсветки; в проверочном режиме (снимки) анимаций нет.
pub fn anim(ctx: &egui::Context, id: egui::Id, on: bool, secs: f32) -> f32 {
    static TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TEST.get_or_init(|| crate::winutil::test_area().is_some()) {
        return if on { 1.0 } else { 0.0 };
    }
    ctx.animate_bool_with_time(id, on, secs)
}

/// Кнопка с заливкой цветом акцента страницы игры (белый текст).
pub fn accent_button(ui: &mut Ui, text: &str, accent: Color32, enabled: bool, width: Option<f32>) -> Response {
    let p = pal();
    let font = FontId::new(14.0, bold_family());
    let m = ui.painter().layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE);
    let w = (m.size().x + 32.0).max(width.unwrap_or(0.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, m.size().y + 16.0), sense);
    let (fill, col) = if !enabled {
        (p.primary_disabled, p.primary_disabled_text)
    } else if resp.hovered() {
        (mix(accent, Color32::WHITE, 0.15), Color32::WHITE)
    } else {
        (accent, Color32::WHITE)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, col);
    resp
}

/// Часть картинки (uv), которая заполняет прямоугольник с пропорциями `aspect` (ширина/высота) без искажений.
pub fn fit_uv(img: Vec2, aspect: f32) -> Rect {
    let img_aspect = img.x / img.y.max(1.0);
    let (w, h) = if img_aspect > aspect { (aspect / img_aspect, 1.0) } else { (1.0, img_aspect / aspect) };
    Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(w, h))
}

/// Размытый и затемнённый снизу фон для шапки страницы игры: уменьшение, box-blur, затемнение.
pub fn blurred_backdrop(w: u32, h: u32, rgba: &[u8]) -> egui::ColorImage {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let (tw, th) = (160usize, (h * 160 / w).max(1));
    let mut cur = vec![[0f32; 3]; tw * th];
    for y in 0..th {
        for x in 0..tw {
            let (x0, x1) = (x * w / tw, ((x + 1) * w / tw).max(x * w / tw + 1).min(w));
            let (y0, y1) = (y * h / th, ((y + 1) * h / th).max(y * h / th + 1).min(h));
            let (mut s, mut n) = ([0f32; 3], 0f32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * w + sx) * 4;
                    if let Some(px) = rgba.get(i..i + 3) {
                        (0..3).for_each(|c| s[c] += px[c] as f32);
                        n += 1.0;
                    }
                }
            }
            cur[y * tw + x] = if n > 0.0 { [s[0] / n, s[1] / n, s[2] / n] } else { [0.0; 3] };
        }
    }
    let r = 3isize;
    for _ in 0..2 {
        for horizontal in [true, false] {
            let src = cur.clone();
            for y in 0..th as isize {
                for x in 0..tw as isize {
                    let mut s = [0f32; 3];
                    for k in -r..=r {
                        let (sx, sy) = if horizontal {
                            ((x + k).clamp(0, tw as isize - 1), y)
                        } else {
                            (x, (y + k).clamp(0, th as isize - 1))
                        };
                        let v = src[sy as usize * tw + sx as usize];
                        (0..3).for_each(|c| s[c] += v[c]);
                    }
                    cur[y as usize * tw + x as usize] = s.map(|v| v / (2 * r + 1) as f32);
                }
            }
        }
    }
    let mut out = egui::ColorImage::new([tw, th], Color32::BLACK);
    for y in 0..th {
        let k = 0.95 - 0.45 * (y as f32 / th as f32); // к низу темнее — на нём лежит название
        for x in 0..tw {
            let v = cur[y * tw + x];
            out.pixels[y * tw + x] = Color32::from_rgb((v[0] * k) as u8, (v[1] * k) as u8, (v[2] * k) as u8);
        }
    }
    out
}

/// Акцент страницы игры: средний насыщенный цвет обложки, яркость прижата, чтобы белый текст читался.
pub fn dominant_accent(w: u32, h: u32, rgba: &[u8]) -> Option<Color32> {
    let (w, h) = (w as usize, h as usize);
    let (mut sum, mut wsum) = ([0f32; 3], 0f32);
    for y in (0..h).step_by((h / 48).max(1)) {
        for x in (0..w).step_by((w / 48).max(1)) {
            let Some(px) = rgba.get((y * w + x) * 4..(y * w + x) * 4 + 3) else { continue };
            let hsv = egui::ecolor::Hsva::from_srgb([px[0], px[1], px[2]]);
            let wt = hsv.s * hsv.s * hsv.v;
            if hsv.s > 0.25 && hsv.v > 0.2 {
                (0..3).for_each(|c| sum[c] += px[c] as f32 * wt);
                wsum += wt;
            }
        }
    }
    if wsum < 1e-3 {
        return None;
    }
    let mut hsv = egui::ecolor::Hsva::from_srgb([(sum[0] / wsum) as u8, (sum[1] / wsum) as u8, (sum[2] / wsum) as u8]);
    hsv.s = hsv.s.clamp(0.45, 0.85);
    hsv.v = hsv.v.clamp(0.18, 0.38); // Hsva хранит линейные значения: 0.38 ≈ 65% яркости в sRGB
    let [r, g, b] = hsv.to_srgb(); // to_srgb — в той же (гамма) шкале, что и from_srgb
    Some(Color32::from_rgb(r, g, b))
}

/// Градиентная заглушка вместо обложки: цвет по названию. Текстура 2x64.
pub fn placeholder_image(name: &str) -> egui::ColorImage {
    let hue = (name.chars().map(|c| c as u32).sum::<u32>() % 360) as f32 / 360.0;
    let top: Color32 = egui::ecolor::Hsva::new(hue, 0.45, 0.62, 1.0).into();
    let bottom: Color32 = egui::ecolor::Hsva::new((hue + 0.05).fract(), 0.55, 0.34, 1.0).into();
    let mut img = egui::ColorImage::new([2, 64], top);
    for y in 0..64 {
        let c = mix(top, bottom, y as f32 / 63.0);
        img.pixels[y * 2] = c;
        img.pixels[y * 2 + 1] = c;
    }
    img
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// День недели "YYYY-MM-DD": 0 — понедельник … 6 — воскресенье.
fn weekday_mon0(day: &str) -> Option<usize> {
    let mut it = day.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    Some((days_from_civil(y, m, d) + 3).rem_euclid(7) as usize)
}

fn month_day(day: &str) -> (usize, u32) {
    let mut it = day.split('-').skip(1).map(|p| p.parse::<u32>().unwrap_or(1));
    (it.next().unwrap_or(1).clamp(1, 12) as usize, it.next().unwrap_or(1))
}

/// Уровень 0..=4 для числа реплик относительно максимума.
fn cal_level(n: u32, max: u32) -> usize {
    if n == 0 {
        0
    } else {
        (n * 4).div_ceil(max.max(1)).clamp(1, 4) as usize
    }
}

fn cal_color(level: usize, accent: Color32) -> Color32 {
    let p = pal();
    mix(mix(p.card, p.text, 0.10), accent, [0.0, 0.35, 0.6, 0.8, 1.0][level])
}

/// Календарь чтения как на GitHub: колонки — недели (последняя заканчивается сегодняшним днём), строки — дни недели
/// с понедельника. `summary` — строки «Сегодня/Серия» сверху, `legend` — «меньше … больше» снизу.
pub fn reading_calendar(
    ui: &mut Ui,
    stats: &noveltl_core::profiles::ReadStats,
    today: &str,
    weeks: usize,
    accent: Color32,
    summary: bool,
    legend: bool,
) {
    use noveltl_core::i18n::{t, tf};
    const CELL: f32 = 11.0;
    const GAP: f32 = 3.0;
    let p = pal();
    ui.spacing_mut().item_spacing.y = 3.0;
    let wd = weekday_mon0(today).unwrap_or(0);
    let days = stats.last_days(today, (weeks - 1) * 7 + wd + 1);
    let max = days.iter().map(|d| d.1).max().unwrap_or(0).max(1);
    if summary {
        let n = days.last().map_or(0, |d| d.1);
        ui.label(body(tf("Сегодня: {} реплик", &[&n])));
        ui.label(small(tf("Серия: {} дн. подряд", &[&stats.streak(today)])));
        ui.add_space(4.0);
    }
    let width = weeks as f32 * (CELL + GAP) - GAP;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 16.0 + 7.0 * (CELL + GAP) - GAP), Sense::hover());
    let cell_rect = |s: usize| {
        let (col, row) = ((s / 7) as f32, (s % 7) as f32);
        Rect::from_min_size(rect.min + Vec2::new(col * (CELL + GAP), 16.0 + row * (CELL + GAP)), Vec2::splat(CELL))
    };
    let months_text = t("янв фев мар апр май июн июл авг сен окт ноя дек");
    let months: Vec<&str> = months_text.split(' ').collect();
    let month_at = |col: usize| days.get(col * 7).map(|(d, _)| month_day(d).0);
    let mut last_label: Option<usize> = None;
    for col in 0..weeks {
        let Some(m) = month_at(col) else { continue };
        // подпись месяца — там, где он сменился; первую колонку — если дальше месяц не меняется сразу
        let changed = col > 0 && month_at(col - 1) != Some(m);
        let first_ok = col == 0 && month_at(1) == Some(m) && month_at(2) == Some(m);
        if (changed || first_ok) && last_label.is_none_or(|l| col >= l + 3) {
            let x = rect.left() + col as f32 * (CELL + GAP);
            let name = months.get(m - 1).copied().unwrap_or("");
            ui.painter().text(egui::pos2(x, rect.top()), Align2::LEFT_TOP, name, FontId::proportional(10.0), p.dim);
            last_label = Some(col);
        }
    }
    for (s, (_, n)) in days.iter().enumerate() {
        ui.painter().rect_filled(cell_rect(s), CornerRadius::same(2), cal_color(cal_level(*n, max), accent));
    }
    // подсказка у квадрата под курсором: «12 ноября: 340 реплик»
    let hover = ui.rect_contains_pointer(rect).then(|| ui.input(|i| i.pointer.hover_pos())).flatten();
    if let Some(pos) = hover {
        if let Some(i) = (0..days.len()).find(|&i| cell_rect(i).expand(GAP / 2.0).contains(pos)) {
            let (day, n) = &days[i];
            let (m, d) = month_day(day);
            let full_months = t("января февраля марта апреля мая июня июля августа сентября октября ноября декабря");
            let name = full_months.split(' ').nth(m - 1).unwrap_or("");
            let date = t("{d} {m}").replace("{d}", &d.to_string()).replace("{m}", name);
            ui.interact(cell_rect(i), ui.id().with(("cal_cell", i)), Sense::hover())
                .on_hover_text(tf("{}: {} реплик", &[&date, n]));
        }
    }
    if legend {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label(small(t("меньше")));
            for l in 0..5 {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(CELL), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(2), cal_color(l, accent));
            }
            ui.label(small(t("больше")));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_applies() {
        let mut s = AppSettings::default();
        assert_eq!(palette_for("dark", &s, true), BW_DARK);
        s.accent = "#4A9DFF".into();
        assert_eq!(palette_for("dark", &s, true).accent, rgb(0x4A9DFF));
        assert_eq!(palette_for("dark", &s, true).bg, BW_DARK.bg);
        assert_eq!(palette_for("system", &s, false).accent, rgb(0x4A9DFF));
        s.accent = "мусор".into();
        assert_eq!(palette_for("light", &s, true), BW_LIGHT);
    }

    #[test]
    fn icons_fall_back_to_plain_glyphs() {
        assert!(!has_icon_font());
        assert_eq!(icon(Icon::Settings), "⚙");
        assert_eq!(icon(Icon::Original), "あ");
    }
    #[test]
    fn calendar_helpers() {
        assert_eq!(weekday_mon0("2024-01-01"), Some(0)); // понедельник
        assert_eq!(weekday_mon0("1970-01-01"), Some(3)); // четверг
        assert_eq!(weekday_mon0("2024-03-03"), Some(6)); // воскресенье
        assert_eq!((cal_level(0, 10), cal_level(1, 10), cal_level(5, 10), cal_level(10, 10)), (0, 1, 2, 4));
    }

    #[test]
    fn art_pixels() {
        // красная картинка: акцент красноватый, яркость прижата; размытие даёт 160 px по ширине
        let px: Vec<u8> = (0..64 * 36).flat_map(|_| [220u8, 30, 30, 255]).collect();
        let a = dominant_accent(64, 36, &px).unwrap();
        assert!(a.r() > a.g() && a.r() > a.b() && a.r() <= 190, "{a:?}");
        let img = blurred_backdrop(64, 36, &px);
        assert_eq!(img.size, [160, 90]);
        assert!(img.pixels.iter().all(|c| c.r() > c.g()));
        assert!(dominant_accent(4, 4, &[128u8; 64]).is_none()); // серое — акцента нет
    }
}
