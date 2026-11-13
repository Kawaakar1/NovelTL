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
        }
    }
}

/// «#RRGGBB» -> 0xRRGGBB (при ошибке — запасной цвет).
pub fn parse_color(s: &str, fallback: u32) -> u32 {
    u32::from_str_radix(s.trim().trim_start_matches('#'), 16).unwrap_or(fallback)
}

pub enum Cmd {
    Target(isize, Option<Frac>),
    Line { name: String, text: String },
    Note { note: String, keep_text: bool },
    Clear,
    EditMode(bool),
    UserHidden(bool),
    Style(Style),
    /// Только для проверок и скриншотов: считать окно игры активным.
    ForceActive(bool),
    Quit,
}

pub enum Event {
    /// Пользователь передвинул или растянул плашку в режиме настройки.
    Moved(Frac),
    /// Не удалось создать плашку (сбой инициализации).
    Failed(String),
    /// Выбран пункт меню плашки (id пункта меню трея).
    Menu(&'static str),
}

/// Подпись пункта меню на текущем языке (общая с меню трея).
pub fn menu_label(id: &str) -> &'static str {
    match id {
        crate::tray::ID_OPEN => t("Открыть NovelTL"),
        crate::tray::ID_TOGGLE => t("Скрыть плашку"),
        crate::tray::ID_RETR => t("Перевести заново без кэша"),
        crate::tray::ID_SETTINGS => t("Настройки"),
        crate::tray::ID_QUIT => t("Выход"),
        _ => "",
    }
}
