//! Linux: замена tray.rs. Значка в трее нет; остаётся логика пунктов меню (из меню на плашке).

use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::ui::{lock_ctl, Request, Shared};

pub const ID_OPEN: &str = "open";
pub const ID_TOGGLE: &str = "toggle";
pub const ID_RETR: &str = "retr";
pub const ID_COPY: &str = "copy";
pub const ID_ORIGINAL: &str = "original";
/// глазик на панели плашки: приглушить/вернуть саму панель кнопок (перевод остаётся)
pub const ID_DIM: &str = "dim";
pub const ID_SETTINGS: &str = "settings";
pub const ID_QUIT: &str = "quit";

pub struct TrayIcon;

pub fn create(_shared: Arc<Shared>, _rgba: Vec<u8>, _size: u32) -> Option<TrayIcon> {
    None
}

pub fn find_own_window(title: &str) -> isize {
    if title == crate::demo::DEMO_TITLE {
        crate::winutil::DEMO_HWND
    } else {
        crate::winutil::MAIN_HWND
    }
}

pub fn set_state(_dot: u8, _tip: &str) {}

pub fn balloon(_title: &str, _text: &str) {}

pub fn remove_icon() {}

pub fn show_window(shared: &Shared) {
    shared.ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    shared.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    shared.visible.store(true, Ordering::SeqCst);
    shared.ctx.request_repaint();
}

fn quit_now(shared: &Shared) {
    lock_ctl(&shared.controller).shutdown();
    shared.engine_stop.store(true, Ordering::SeqCst);
    remove_icon();
    std::process::exit(0);
}

/// Пункт меню. Не вызывать, держа замок контроллера.
pub fn handle_menu(shared: &Shared, id: &str) {
    match id {
        ID_OPEN => show_window(shared),
        ID_TOGGLE => lock_ctl(&shared.controller).toggle_overlay(),
        ID_RETR => lock_ctl(&shared.controller).retranslate(),
        ID_SETTINGS => {
            shared.push_request(Request::OpenSettings);
            show_window(shared);
        }
        ID_QUIT => {
            shared.quit.store(true, Ordering::SeqCst);
            if shared.visible.load(Ordering::SeqCst) {
                shared.ctx.request_repaint();
            } else {
                quit_now(shared);
            }
        }
        _ => {}
    }
}
