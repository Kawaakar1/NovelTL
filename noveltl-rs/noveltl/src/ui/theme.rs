//! Темы оформления (тёмная, светлая, как в Windows) и значок приложения (перенос `noveltl/ui/theme.py`).

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Response, RichText, Sense, Stroke,
    StrokeKind, Theme, ThemePreference, Ui, Vec2, Visuals,
};

pub const THEME_DARK: &str = "dark";
pub const THEME_LIGHT: &str = "light";

// набор цветов целиком, как в theme.py (часть пока не используется)
#[allow(dead_code)]
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

pub static DARK: Palette = Palette {
    bg: rgb(0x14151F),
    panel: rgb(0x1B1D2A),
    card: rgb(0x232537),
    border: rgb(0x313450),
    text: rgb(0xE9EAF4),
    dim: rgb(0x9497B3),
    accent: rgb(0x8B7CFF),
    accent_hover: rgb(0xA094FF),
    on_accent: rgb(0x14122B),
    ok: rgb(0x5DD39E),
    warn: rgb(0xF2B45A),
    bad: rgb(0xF07178),
    selected: rgb(0x2E2B55),
    menu_selected: rgb(0x3A3670),
    disabled_text: rgb(0x5D6080),
    disabled_border: rgb(0x282A40),
    primary_disabled: rgb(0x3A3766),
    primary_disabled_text: rgb(0x8684A8),
    notice_bg: rgb(0x3A3120),
    handle: rgb(0xE9EAF4),
};

pub static LIGHT: Palette = Palette {
    bg: rgb(0xF4F5FA),
    panel: rgb(0xE8EAF3),
    card: rgb(0xFFFFFF),
    border: rgb(0xD2D5E3),
    text: rgb(0x1D1F2E),
    dim: rgb(0x666A85),
    accent: rgb(0x6C5CE7),
    accent_hover: rgb(0x5747D1),
    on_accent: rgb(0xFFFFFF),
    ok: rgb(0x178A5B),
    warn: rgb(0xA8660B),
    bad: rgb(0xCF3B48),
    selected: rgb(0xE3DFFF),
    menu_selected: rgb(0xDAD5FF),
    disabled_text: rgb(0xA9ACC0),
    disabled_border: rgb(0xE1E3EE),
    primary_disabled: rgb(0xCFCBF3),
    primary_disabled_text: rgb(0xFFFFFF),
    notice_bg: rgb(0xFFF1D0),
    handle: rgb(0x6C5CE7),
};

static CURRENT_LIGHT: AtomicBool = AtomicBool::new(false);

/// Палитра текущей темы.
pub fn pal() -> &'static Palette {
    if CURRENT_LIGHT.load(Ordering::Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

/// Тема из настроек (`system`/`dark`/`light`) -> реальная светлая или тёмная.
pub fn resolve(setting: &str, ctx: &egui::Context) -> Theme {
    match setting {
        THEME_LIGHT => Theme::Light,
        THEME_DARK => Theme::Dark,
        _ => match ctx.system_theme() {
            Some(Theme::Light) => Theme::Light,
            _ => Theme::Dark,
        },
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
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, true));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, false));
    ctx.all_styles_mut(|s| {
        s.text_styles.insert(egui::TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
        s.text_styles.insert(egui::TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        s.interaction.tooltip_delay = 0.4;
    });
}

/// Применяет тему из настроек; возвращает true, если реальная тема сменилась.
pub fn apply(ctx: &egui::Context, setting: &str) -> bool {
    let want = resolve(setting, ctx);
    let light = want == Theme::Light;
    let changed = CURRENT_LIGHT.swap(light, Ordering::Relaxed) != light || ctx.theme() != want;
    if changed {
        ctx.set_theme(if light { ThemePreference::Light } else { ThemePreference::Dark });
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
