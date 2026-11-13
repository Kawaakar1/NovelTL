//! Глобальные горячие клавиши через RegisterHotKey (свой поток со скрытым окном).
#![allow(dead_code)]

use std::sync::mpsc::Sender;
#[cfg(windows)]
use std::sync::mpsc::{channel, Receiver};
#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use std::thread::JoinHandle;

#[cfg(windows)]
use windows::core::w;
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW, RegisterClassW,
    TranslateMessage, CS_HREDRAW, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_HOTKEY, WNDCLASSW,
};

use crate::source::Waker;

const MOD_ALT: u32 = 0x1;
const MOD_CONTROL: u32 = 0x2;
const MOD_SHIFT: u32 = 0x4;
const MOD_WIN: u32 = 0x8;
const MOD_NOREPEAT: u32 = 0x4000;

fn modifier(name: &str) -> Option<u32> {
    match name {
        "ctrl" => Some(MOD_CONTROL),
        "alt" => Some(MOD_ALT),
        "shift" => Some(MOD_SHIFT),
        "meta" | "win" => Some(MOD_WIN),
        _ => None,
    }
}

fn named_key(name: &str) -> Option<u32> {
    Some(match name {
        "space" => 0x20,
        "tab" => 0x09,
        "pgup" => 0x21,
        "pgdown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "ins" => 0x2D,
        "del" => 0x2E,
        "pause" => 0x13,
        "`" => 0xC0,
        "-" => 0xBD,
        "=" => 0xBB,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" => 0xDC,
        ";" => 0xBA,
        "'" => 0xDE,
        "," => 0xBC,
        "." => 0xBE,
        "/" => 0xBF,
        _ => return None,
    })
}

/// «Ctrl+Alt+H» -> (модификаторы, код клавиши). None, если разобрать нельзя.
pub fn parse_hotkey(text: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = text.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    let (key, mod_parts) = parts.split_last()?;
    let mut mods = 0u32;
    for p in mod_parts {
        mods |= modifier(&p.to_lowercase())?;
    }
    let low = key.to_lowercase();
    let mut chars = key.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c),
        _ => None,
    };
    let vk = if let Some(c) = single {
        c.to_ascii_uppercase() as u32
    } else if let Some(v) = named_key(&low) {
        v
    } else if let Some(num) = low.strip_prefix('f').filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        match num.parse::<u32>() {
            Ok(n) if (1..=24).contains(&n) => 0x70 + n - 1,
            _ => return None,
        }
    } else {
        return None;
    };
    // без модификаторов можно только F1–F24 (VK 0x70–0x87), не одиночную букву (в том числе «F»)
    if mods == 0 && !(0x70..=0x87).contains(&vk) {
        return None; // одиночная буква как глобальная клавиша мешала бы печатать
    }
    Some((mods, vk))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// Индекс сочетания в списке, переданном в `set()`.
    Fired(usize),
}

#[cfg(windows)]
enum Command {
    Set(Vec<String>, Sender<Vec<String>>),
    Stop,
}

#[cfg(windows)]
pub struct Hotkeys {
    tx: Mutex<Sender<Command>>,
    hwnd: isize,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[cfg(windows)]
impl Hotkeys {
    pub fn start(events: Sender<HotkeyEvent>, waker: Waker) -> Hotkeys {
        let (tx, rx) = channel::<Command>();
        let (ready_tx, ready_rx) = channel::<isize>();
        let handle = std::thread::spawn(move || run_thread(rx, ready_tx, events, waker));
        let hwnd = ready_rx.recv().unwrap_or(0);
        Hotkeys { tx: Mutex::new(tx), hwnd, thread: Mutex::new(Some(handle)) }
    }

    fn wake_thread(&self) {
        if self.hwnd != 0 {
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd as *mut _), WM_APP, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// Снимает старые и регистрирует новые сочетания. Возвращает те, которые занять НЕ удалось.
    pub fn set(&self, combos: &[String]) -> Vec<String> {
        let (reply_tx, reply_rx) = channel();
        let sent = match self.tx.lock() {
            Ok(tx) => tx.send(Command::Set(combos.to_vec(), reply_tx)).is_ok(),
            Err(_) => false,
        };
        if !sent {
            return combos.to_vec();
        }
        self.wake_thread();
        reply_rx.recv().unwrap_or_else(|_| combos.to_vec())
    }

    pub fn stop(&self) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(Command::Stop);
        }
        self.wake_thread();
        let handle = self.thread.lock().ok().and_then(|mut t| t.take());
        if let Some(h) = handle {
            let _ = h.join();
        }
    }
}

#[cfg(windows)]
impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

#[cfg(windows)]
fn run_thread(rx: Receiver<Command>, ready: Sender<isize>, events: Sender<HotkeyEvent>, waker: Waker) {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("NovelTLHotkeysWnd");
        let wc = WNDCLASSW {
            style: CS_HREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        // повторная регистрация класса вернёт 0 — это нормально
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("NovelTL hotkeys"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinst,
            None,
        )
        .unwrap_or_default();
        let _ = ready.send(hwnd.0 as isize);
        if hwnd.0.is_null() {
            return;
        }

        let mut registered: Vec<i32> = Vec::new();
        let mut msg = MSG::default();
        'outer: while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY {
                let id = msg.wParam.0 as i32;
                if id >= 1 {
                    let _ = events.send(HotkeyEvent::Fired((id - 1) as usize));
                    waker();
                }
                continue;
            }
            if msg.message == WM_APP {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Command::Set(combos, reply) => {
                            for id in registered.drain(..) {
                                let _ = UnregisterHotKey(hwnd, id);
                            }
                            let mut failed = Vec::new();
                            for (i, text) in combos.iter().enumerate() {
                                let id = (i + 1) as i32;
                                let ok = match parse_hotkey(text) {
                                    Some((mods, vk)) => RegisterHotKey(
                                        hwnd,
                                        id,
                                        HOT_KEY_MODIFIERS(mods | MOD_NOREPEAT),
                                        vk,
                                    )
                                    .is_ok(),
                                    None => false,
                                };
                                if ok {
                                    registered.push(id);
                                } else {
                                    failed.push(text.clone());
                                }
                            }
                            let _ = reply.send(failed);
                        }
                        Command::Stop => break 'outer,
                    }
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for id in registered.drain(..) {
            let _ = UnregisterHotKey(hwnd, id);
        }
        let _ = DestroyWindow(hwnd);
    }
}

/// В Linux глобальных горячих клавиш нет: заглушка с тем же интерфейсом.
#[cfg(not(windows))]
pub struct Hotkeys;

#[cfg(not(windows))]
impl Hotkeys {
    pub fn start(_events: Sender<HotkeyEvent>, _waker: Waker) -> Hotkeys {
        Hotkeys
    }
    pub fn set(&self, _combos: &[String]) -> Vec<String> {
        Vec::new()
    }
    pub fn stop(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_letter_f_is_rejected() {
        assert_eq!(parse_hotkey("F"), None);
        assert_eq!(parse_hotkey("f"), None);
        assert!(parse_hotkey("F12").is_some());
    }

    #[test]
    fn parse_ctrl_alt_h() {
        assert_eq!(parse_hotkey("Ctrl+Alt+H"), Some((MOD_CONTROL | MOD_ALT, 0x48)));
        assert_eq!(parse_hotkey("ctrl+alt+h"), Some((MOD_CONTROL | MOD_ALT, 0x48)));
        assert_eq!(parse_hotkey("CTRL + ALT + H"), Some((MOD_CONTROL | MOD_ALT, 0x48)));
    }

    #[test]
    fn parse_function_keys() {
        assert_eq!(parse_hotkey("F9"), Some((0, 0x78)));
        assert_eq!(parse_hotkey("f1"), Some((0, 0x70)));
        assert_eq!(parse_hotkey("F24"), Some((0, 0x87)));
        assert_eq!(parse_hotkey("F25"), None);
        assert_eq!(parse_hotkey("F0"), None);
        assert_eq!(parse_hotkey("Ctrl+F5"), Some((MOD_CONTROL, 0x74)));
    }

    #[test]
    fn parse_rejects() {
        assert_eq!(parse_hotkey("H"), None);
        assert_eq!(parse_hotkey("5"), None);
        assert_eq!(parse_hotkey("Ctrl+Foo"), None);
        assert_eq!(parse_hotkey("Foo+H"), None);
        assert_eq!(parse_hotkey(""), None);
        assert_eq!(parse_hotkey("  "), None);
        assert_eq!(parse_hotkey("Ctrl+"), None);
        assert_eq!(parse_hotkey("Ctrl+Alt"), None);
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(parse_hotkey("Shift+`"), Some((MOD_SHIFT, 0xC0)));
        assert_eq!(parse_hotkey("Win+Space"), Some((MOD_WIN, 0x20)));
        assert_eq!(parse_hotkey("Meta+Home"), Some((MOD_WIN, 0x24)));
        assert_eq!(parse_hotkey("Ctrl+PgDown"), Some((MOD_CONTROL, 0x22)));
        assert_eq!(parse_hotkey("Alt+\\"), Some((MOD_ALT, 0xDC)));
        assert_eq!(parse_hotkey("Ctrl+1"), Some((MOD_CONTROL, 0x31)));
    }

    #[cfg(windows)]
    #[test]
    fn live_register() {
        use std::sync::Arc;
        let (tx, _rx) = channel();
        let waker: Waker = Arc::new(|| {});
        let hk = Hotkeys::start(tx, waker);
        let combo = "Ctrl+Alt+Shift+F12".to_string();
        assert!(hk.set(&[combo.clone()]).is_empty());
        let failed = hk.set(&[combo, "мусор".to_string()]);
        assert_eq!(failed, vec!["мусор".to_string()]);
        hk.stop();
    }
}
