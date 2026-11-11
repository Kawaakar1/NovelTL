//! Значок в трее с меню (часть `main_window.py`). Крейт `tray-icon`; события приходят в обработчики,
//! которые работают и тогда, когда главное окно спрятано (интерфейс в это время не рисуется).

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Arc;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_INFO, NIIF_INFO, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
};

use crate::ui::{lock_ctl, Request, Shared};
use crate::winutil;

const ID_OPEN: &str = "open";
const ID_TOGGLE: &str = "toggle";
const ID_RETR: &str = "retr";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";

const TRAY_CLASS: &str = "tray_icon_app";

/// HWND окна-приёмника значка (нужен для всплывающих сообщений) и номер значка.
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static TRAY_UID: AtomicU32 = AtomicU32::new(0);

struct Items {
    toggle: MenuItem,
    retr: MenuItem,
}

thread_local! {
    // пункты меню нельзя передавать между потоками: обработчики событий работают в главном потоке
    static ITEMS: RefCell<Option<Items>> = const { RefCell::new(None) };
}

/// Окно нашего процесса с таким заголовком (главное окно или окно демо-игры).
pub fn find_own_window(title: &str) -> isize {
    struct Ctx {
        pid: u32,
        title: String,
        found: isize,
    }
    unsafe extern "system" fn cb(h: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let hwnd = h.0 as isize;
        if winutil::window_pid(hwnd) == ctx.pid && winutil::window_title(hwnd) == ctx.title {
            ctx.found = hwnd;
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut ctx = Ctx { pid: std::process::id(), title: title.to_string(), found: 0 };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found
}

fn find_tray_window() -> isize {
    struct Ctx {
        pid: u32,
        found: isize,
    }
    unsafe extern "system" fn cb(h: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam.0 as *mut Ctx);
        let hwnd = h.0 as isize;
        if winutil::window_pid(hwnd) == ctx.pid && winutil::window_class(hwnd) == TRAY_CLASS {
            ctx.found = hwnd;
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut ctx = Ctx { pid: std::process::id(), found: 0 };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found
}

/// Показать окно напрямую средствами Windows (у спрятанного окна egui команды не обрабатывает).
pub fn show_window(shared: &Shared) {
    let hwnd = shared.main_hwnd.load(Ordering::SeqCst);
    if hwnd == 0 {
        return;
    }
    unsafe {
        let h = HWND(hwnd as _);
        let _ = ShowWindow(h, if IsIconic(h).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(h);
    }
    shared.visible.store(true, Ordering::SeqCst);
    shared.ctx.request_repaint();
}

fn fill_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        dst[i] = c;
    }
}

fn notify_info(uid: u32, hwnd: isize, title: &str, text: &str) -> bool {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(hwnd as _),
        uID: uid,
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    fill_wide(&mut nid.szInfoTitle, title);
    fill_wide(&mut nid.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() }
}

/// Всплывающее сообщение у значка в трее.
pub fn balloon(title: &str, text: &str) {
    let mut hwnd = TRAY_HWND.load(Ordering::SeqCst);
    if hwnd == 0 {
        hwnd = find_tray_window();
        TRAY_HWND.store(hwnd, Ordering::SeqCst);
    }
    if hwnd == 0 {
        return;
    }
    let known = TRAY_UID.load(Ordering::SeqCst);
    if known != 0 && notify_info(known, hwnd, title, text) {
        return;
    }
    // номер значка внутри крейта нам не виден — перебираем небольшие числа
    for uid in 1..=8u32 {
        if notify_info(uid, hwnd, title, text) {
            TRAY_UID.store(uid, Ordering::SeqCst);
            return;
        }
    }
}

/// Убрать значок из трея (при аварийном выходе мимо обычного закрытия окна).
pub fn remove_icon() {
    let hwnd = TRAY_HWND.load(Ordering::SeqCst);
    let uid = TRAY_UID.load(Ordering::SeqCst);
    if hwnd == 0 || uid == 0 {
        return;
    }
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(hwnd as _),
        uID: uid,
        ..Default::default()
    };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Подписи пунктов меню по текущему состоянию.
fn refresh_menu(shared: &Shared) {
    let (hidden, toggle_key, retr_key, has_session) = {
        let c = lock_ctl(&shared.controller);
        (c.user_hidden, c.settings.hotkey_toggle.clone(), c.settings.hotkey_retranslate.clone(), c.session.is_some())
    };
    ITEMS.with(|cell| {
        if let Some(items) = cell.borrow().as_ref() {
            items
                .toggle
                .set_text(format!("{}\t{}", if hidden { "Показать плашку" } else { "Скрыть плашку" }, toggle_key));
            items.retr.set_text(format!("Перевести заново без кэша\t{retr_key}"));
            items.retr.set_enabled(has_session);
        }
    });
}

/// Полное завершение, когда окно спрятано и интерфейс не работает.
fn quit_now(shared: &Shared) {
    lock_ctl(&shared.controller).shutdown();
    shared.engine_stop.store(true, Ordering::SeqCst);
    remove_icon();
    std::process::exit(0);
}

fn handle_menu(shared: &Shared, id: &str) {
    match id {
        ID_OPEN => show_window(shared),
        ID_TOGGLE => {
            lock_ctl(&shared.controller).toggle_overlay();
            refresh_menu(shared);
        }
        ID_RETR => lock_ctl(&shared.controller).retranslate(),
        ID_SETTINGS => {
            shared.push_request(Request::OpenSettings);
            show_window(shared);
        }
        ID_QUIT => {
            if shared.visible.load(Ordering::SeqCst) {
                // окно живое: пусть интерфейс закроется сам (ViewportCommand::Close)
                shared.quit.store(true, Ordering::SeqCst);
                shared.ctx.request_repaint();
            } else {
                shared.quit.store(true, Ordering::SeqCst);
                quit_now(shared);
            }
        }
        _ => {}
    }
}

/// Создаёт значок. Вызывать из главного потока после запуска цикла окон.
pub fn create(shared: Arc<Shared>, rgba: Vec<u8>, size: u32) -> Option<TrayIcon> {
    let icon = Icon::from_rgba(rgba, size, size).ok()?;
    let menu = Menu::new();
    let open = MenuItem::with_id(ID_OPEN, "Открыть NovelTL", true, None);
    let toggle = MenuItem::with_id(ID_TOGGLE, "Скрыть плашку", true, None);
    let retr = MenuItem::with_id(ID_RETR, "Перевести заново без кэша", true, None);
    let settings = MenuItem::with_id(ID_SETTINGS, "Настройки", true, None);
    let quit = MenuItem::with_id(ID_QUIT, "Выход", true, None);
    let _ = menu.append_items(&[
        &open,
        &PredefinedMenuItem::separator(),
        &toggle,
        &retr,
        &PredefinedMenuItem::separator(),
        &settings,
        &quit,
    ]);
    ITEMS.with(|cell| *cell.borrow_mut() = Some(Items { toggle, retr }));

    let tray = TrayIconBuilder::new()
        .with_id("noveltl")
        .with_tooltip("NovelTL")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .ok()?;

    let sh = shared.clone();
    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| match ev {
        TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
        | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => show_window(&sh),
        // подписи меню обновляем, когда мышь подходит к значку (до открытия меню)
        TrayIconEvent::Enter { .. } | TrayIconEvent::Move { .. } => refresh_menu(&sh),
        _ => {}
    }));
    let sh = shared;
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| handle_menu(&sh, ev.id.0.as_str())));
    Some(tray)
}
