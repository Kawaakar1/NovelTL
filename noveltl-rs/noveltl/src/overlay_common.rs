//! Общее для плашки в Windows (overlay.rs) и Linux (platform_linux/overlay.rs):
//! положение, вид, команды и события, подписи меню.

use noveltl_core::i18n::t;

/// Положение плашки в долях клиентской области окна игры.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frac {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for Frac {
    fn default() -> Self {
        Frac { x: 0.08, y: 0.70, w: 0.84, h: 0.24 }
    }
}

/// Внешний вид плашки (из настроек).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub font_family: String,
    pub font_size: f32,
    pub text_color: u32,
    pub name_color: u32,
    pub outline_color: u32,
    pub bg_color: u32,
    pub bg_opacity: u8, // проценты
    /// показывать исходную реплику мелко под переводом
    pub show_original: bool,
    /// панель кнопок приглушена (почти прозрачная, ярче под курсором)
    pub toolbar_dim: bool,
    /// скругление углов, px
    pub radius: u32,
    /// рамка 2 px цветом имени
    pub border: bool,
    pub center: bool,
    pub shadow: bool,
    /// плавное появление реплики
    pub fade: bool,
    /// эффект печати
    pub typewriter: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            font_family: "Segoe UI".into(),
            font_size: 26.0,
            text_color: 0xFFFFFF,
            name_color: 0xFFD27F,
            outline_color: 0x000000,
            bg_color: 0x0E0F1A,
            bg_opacity: 85,
            show_original: false,
            toolbar_dim: false,
            radius: 12,
            border: false,
            center: false,
            shadow: false,
            fade: true,
            typewriter: false,
        }
    }
}

/// «#RRGGBB» -> 0xRRGGBB (при ошибке — запасной цвет).
pub fn parse_color(s: &str, fallback: u32) -> u32 {
    u32::from_str_radix(s.trim().trim_start_matches('#'), 16).unwrap_or(fallback)
}

/// Кусок оригинала для плашки: чтение над иероглифами (фуригана) и подсказка из словаря.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrigSeg {
    pub text: String,
    pub reading: Option<String>,
    /// готовый текст подсказки («слово【чтение】», затем значения построчно); пусто — слова нет в словаре
    pub tip: String,
}

pub enum Cmd {
    Target(isize, Option<Frac>),
    /// реплика: имя, перевод, второй перевод (мелко под основным; пусто — нет) и исходный текст
    /// `name_color` — цвет имени персонажа из глоссария (0xRRGGBB); `segs` — оригинал по словам
    /// (пусто — без фуриганы); `back` — на сколько реплик назад от последней (0 — последняя)
    Line {
        name: String,
        text: String,
        alt: String,
        orig: String,
        name_color: Option<u32>,
        segs: Vec<OrigSeg>,
        back: usize,
    },
    Note { note: String, keep_text: bool },
    Clear,
    EditMode(bool),
    UserHidden(bool),
    Style(Style),
    /// Только для проверок и скриншотов: считать окно игры активным.
    ForceActive(bool),
    /// Режим OCR: плашка не попадает в снимки экрана, чтобы программа не читала свой же перевод.
    ExcludeFromCapture(bool),
    Quit,
}

pub enum Event {
    /// Пользователь передвинул или растянул плашку в режиме настройки.
    Moved(Frac),
    /// Не удалось создать плашку (сбой инициализации).
    Failed(String),
    /// Выбран пункт меню плашки (id пункта меню трея).
    Menu(&'static str),
    /// Колёсико над плашкой: +1 — к более старой реплике, -1 — к более новой.
    #[allow(dead_code)]
    Browse(i32),
}

/// Подпись пункта меню на текущем языке (общая с меню трея).
pub fn menu_label(id: &str) -> &'static str {
    match id {
        crate::tray::ID_OPEN => t("Открыть NovelTL"),
        crate::tray::ID_TOGGLE => t("Скрыть плашку"),
        crate::tray::ID_RETR => t("Перевести заново без кэша"),
        crate::tray::ID_COPY => t("Копировать перевод"),
        crate::tray::ID_ORIGINAL => t("Показать оригинал"),
        crate::tray::ID_SETTINGS => t("Настройки"),
        crate::tray::ID_QUIT => t("Выход"),
        _ => "",
    }
}

// ---------------- панель кнопок на плашке (общая раскладка Windows и Linux) ----------------

pub const TB_W: i32 = 30;
pub const TB_H: i32 = 28;
/// отступ кнопок от края подложки и подложки от края плашки
pub const TB_PAD: i32 = 4;
pub const TB_INSET: i32 = 8;
pub const TB_BUTTONS: usize = 7;
pub const TB_ORIGINAL: usize = 2;
pub const TB_LOCK: usize = 3;
pub const TB_MENU: usize = 6;
/// что отправляет кнопка (None — внутреннее действие: замочек, меню)
pub const TB_IDS: [Option<&str>; TB_BUTTONS] = [
    Some(crate::tray::ID_RETR),
    Some(crate::tray::ID_COPY),
    Some(crate::tray::ID_ORIGINAL),
    None,
    Some(crate::tray::ID_SETTINGS),
    Some(crate::tray::ID_TOGGLE),
    None,
];

/// Подложка панели (left, top, right, bottom) в плашке шириной `w`.
pub fn tb_pill(w: i32) -> (i32, i32, i32, i32) {
    let pw = TB_BUTTONS as i32 * TB_W + 2 * TB_PAD;
    (w - TB_INSET - pw, TB_INSET, w - TB_INSET, TB_INSET + TB_H + 2 * TB_PAD)
}

/// Кнопка `i` (left, top, right, bottom) в плашке шириной `w`.
pub fn tb_button(w: i32, i: usize) -> (i32, i32, i32, i32) {
    let (pl, pt, _, _) = tb_pill(w);
    let l = pl + TB_PAD + i as i32 * TB_W;
    (l, pt + TB_PAD, l + TB_W, pt + TB_PAD + TB_H)
}

/// Какая кнопка в точке (x, y) плашки шириной `w`.
pub fn tb_button_at(w: i32, x: i32, y: i32) -> Option<usize> {
    (0..TB_BUTTONS).find(|&i| {
        let (l, t, r, b) = tb_button(w, i);
        x >= l && x < r && y >= t && y < b
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_hit_test() {
        let w = 800;
        for i in 0..TB_BUTTONS {
            let (l, t, r, b) = tb_button(w, i);
            assert_eq!(tb_button_at(w, (l + r) / 2, (t + b) / 2), Some(i));
        }
        let (pl, pt, pr, _) = tb_pill(w);
        assert!(pr <= w && pl > 0);
        assert_eq!(tb_button_at(w, pl - 5, pt + 10), None);
        assert_eq!(tb_button_at(w, 10, 10), None);
    }

    #[test]
    fn new_menu_ids_have_labels() {
        assert!(!menu_label(crate::tray::ID_COPY).is_empty());
        assert!(!menu_label(crate::tray::ID_ORIGINAL).is_empty());
    }
}
