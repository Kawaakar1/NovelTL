//! Работа с окнами и процессами Windows (перенос `noveltl/winutil.py` и `net.list_interfaces`).
#![allow(dead_code)]

use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, IsWow64Process, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, TerminateProcess,
};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    InternalGetWindowText, IsIconic, IsWindow, IsWindowVisible, GWL_EXSTYLE, SW_HIDE,
    WS_EX_TOOLWINDOW,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Foundation::POINT;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Системные окна и браузеры — в списке игр их не показываем.
pub const HIDDEN_EXES: &[&str] = &[
    "explorer.exe", "applicationframehost.exe", "systemsettings.exe", "textinputhost.exe",
    "shellexperiencehost.exe", "searchhost.exe", "searchapp.exe", "startmenuexperiencehost.exe",
    "taskmgr.exe", "lockapp.exe", "dwm.exe", "sihost.exe", "ctfmon.exe", "widgets.exe",
    "cmd.exe", "powershell.exe", "pwsh.exe", "windowsterminal.exe", "conhost.exe", "openconsole.exe",
    "chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "opera_gx.exe", "brave.exe",
    "vivaldi.exe", "browser.exe", "iexplore.exe", "msedgewebview2.exe", "arc.exe",
    "steam.exe", "steamwebhelper.exe", "discord.exe", "telegram.exe", "ayugram.exe", "64gram.exe",
    "unigram.exe", "anty.exe", "dolphin anty.exe", "whatsapp.exe", "viber.exe", "zoom.exe", "code.exe", "claude.exe",
    "notepad.exe", "nvcontainer.exe", "nvidia overlay.exe", "nvidia share.exe", "gamebar.exe",
    "gamebarftserver.exe", "phoneexperiencehost.exe", "mspaint.exe", "snippingtool.exe",
    "lunatranslator.exe", "noveltl.exe", "obs64.exe", "spotify.exe", "calculatorapp.exe",
];

pub const BROWSER_CLASSES: &[&str] = &["Chrome_WidgetWin", "MozillaWindowClass", "ApplicationFrameWindow", "CASCADIA_"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    pub exe_path: String,
}

impl WindowInfo {
    pub fn exe_name(&self) -> String {
        exe_basename(&self.exe_path)
    }
}

fn exe_basename(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

fn hw(h: isize) -> HWND {
    HWND(h as _)
}

pub fn window_pid(hwnd: isize) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hw(hwnd), Some(&mut pid)) };
    pid
}

/// Заголовок окна БЕЗ отправки сообщений окну.
/// GetWindowText для окон своего процесса шлёт WM_GETTEXT потоку-владельцу и ждёт ответа —
/// если фоновый поток зовёт его, держа замок контроллера, а поток интерфейса ждёт этот замок,
/// программа зависает. InternalGetWindowText читает заголовок напрямую.
pub fn window_title(hwnd: isize) -> String {
    let mut buf = vec![0u16; 512];
    let got = unsafe { InternalGetWindowText(hw(hwnd), &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

pub fn window_class(hwnd: isize) -> String {
    let mut buf = [0u16; 256];
    let got = unsafe { GetClassNameW(hw(hwnd), &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

pub fn process_exe_path(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = vec![0u16; 1024];
        let mut size = 1024u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(h);
        match ok {
            Ok(()) => String::from_utf16_lossy(&buf[..size as usize]),
            Err(_) => String::new(),
        }
    }
}

/// true, если процесс 64-разрядный (на 64-разрядной Windows); при ошибке — true.
pub fn is_process_64bit(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return true;
        };
        let mut wow64 = BOOL(0);
        let _ = IsWow64Process(h, &mut wow64);
        let _ = CloseHandle(h);
        !wow64.as_bool()
    }
}

pub fn is_process_alive(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return false;
        };
        let r = WaitForSingleObject(h, 0);
        let _ = CloseHandle(h);
        r == WAIT_TIMEOUT
    }
}

fn is_cloaked(hwnd: isize) -> bool {
    let mut val = 0u32;
    unsafe {
        let _ = DwmGetWindowAttribute(
            hw(hwnd),
            DWMWA_CLOAKED,
            &mut val as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
    }
    val != 0
}

struct EnumCtx<'a> {
    own_pid: u32,
    own_allowed: &'a [isize],
    found: Vec<WindowInfo>,
}

unsafe extern "system" fn enum_cb(h: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    consider_window(ctx, h.0 as isize);
    BOOL(1)
}

fn consider_window(ctx: &mut EnumCtx, hwnd: isize) {
    unsafe {
        if !IsWindowVisible(hw(hwnd)).as_bool() || is_cloaked(hwnd) {
            return;
        }
        let title = window_title(hwnd);
        if title.trim().is_empty() {
            return;
        }
        if (GetWindowLongPtrW(hw(hwnd), GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW.0 != 0 {
            return;
        }
        let pid = window_pid(hwnd);
        let own = pid == ctx.own_pid;
        if own && !ctx.own_allowed.contains(&hwnd) {
            return;
        }
        let mut rect = RECT::default();
        let _ = GetClientRect(hw(hwnd), &mut rect);
        if !IsIconic(hw(hwnd)).as_bool() && (rect.right < 200 || rect.bottom < 150) {
            return;
        }
        if !own {
            let cls = window_class(hwnd);
            if BROWSER_CLASSES.iter().any(|p| cls.starts_with(p)) {
                return; // браузеры и программы на их основе
            }
        }
        let exe = process_exe_path(pid);
        if exe.is_empty() {
            return;
        }
        let low = exe.to_lowercase();
        if !own && HIDDEN_EXES.contains(&exe_basename(&exe).to_lowercase().as_str()) {
            return;
        }
        if !own && low.contains("\\windows\\") {
            return;
        }
        ctx.found.push(WindowInfo { hwnd, pid, title, exe_path: exe });
    }
}

/// Окна-кандидаты в игры. Свои окна скрыты, кроме перечисленных в own_allowed (демо).
pub fn list_windows(own_allowed: &[isize]) -> Vec<WindowInfo> {
    let mut ctx = EnumCtx { own_pid: std::process::id(), own_allowed, found: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut ctx as *mut EnumCtx as isize));
    }
    ctx.found
}

pub fn is_window(hwnd: isize) -> bool {
    hwnd != 0 && unsafe { IsWindow(hw(hwnd)).as_bool() }
}

pub fn is_minimized(hwnd: isize) -> bool {
    unsafe { IsIconic(hw(hwnd)).as_bool() }
}

pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Клиентская область окна в экранных координатах: (x, y, w, h).
pub fn client_rect_on_screen(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(hw(hwnd), &mut rect).ok()?;
        let mut pt = POINT { x: 0, y: 0 };
        if !ClientToScreen(hw(hwnd), &mut pt).as_bool() {
            return None;
        }
        if rect.right <= 0 || rect.bottom <= 0 {
            return None;
        }
        Some((pt.x, pt.y, rect.right, rect.bottom))
    }
}

// ---------- автозапуск (HKCU\...\Run, значение NovelTL) ----------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "NovelTL";

fn open_run_key(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<windows::Win32::System::Registry::HKEY> {
    use windows::Win32::System::Registry::{RegOpenKeyExW, HKEY, HKEY_CURRENT_USER};
    let key = to_wide(RUN_KEY);
    let mut h = HKEY::default();
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), 0, access, &mut h) };
    if rc.0 == 0 {
        Some(h)
    } else {
        None
    }
}

/// Есть ли значение `NovelTL` в разделе автозапуска текущего пользователя.
pub fn autostart_enabled() -> bool {
    use windows::Win32::System::Registry::{RegCloseKey, RegQueryValueExW, KEY_QUERY_VALUE};
    let Some(h) = open_run_key(KEY_QUERY_VALUE) else { return false };
    let name = to_wide(RUN_VALUE);
    let rc = unsafe { RegQueryValueExW(h, PCWSTR(name.as_ptr()), None, None, None, None) };
    unsafe {
        let _ = RegCloseKey(h);
    }
    rc.0 == 0
}

/// Включает/выключает автозапуск: пишет `"<путь к exe>"` в Run или удаляет значение.
pub fn set_autostart(on: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegSetValueExW, KEY_SET_VALUE, REG_SZ,
    };
    let Some(h) = open_run_key(KEY_SET_VALUE) else {
        return Err("Не удалось открыть раздел автозапуска в реестре".to_string());
    };
    let name = to_wide(RUN_VALUE);
    let result = if on {
        match std::env::current_exe() {
            Ok(exe) => {
                let value = to_wide(&format!("\"{}\"", exe.to_string_lossy()));
                let bytes: Vec<u8> = value.iter().flat_map(|u| u.to_le_bytes()).collect();
                let rc = unsafe { RegSetValueExW(h, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(&bytes)) };
                if rc.0 == 0 {
                    Ok(())
                } else {
                    Err(format!("Не удалось записать автозапуск в реестр (код {})", rc.0))
                }
            }
            Err(e) => Err(format!("Не удалось определить путь к программе: {e}")),
        }
    } else {
        let rc = unsafe { RegDeleteValueW(h, PCWSTR(name.as_ptr())) };
        // 2 — значения и так нет
        if rc.0 == 0 || rc.0 == 2 {
            Ok(())
        } else {
            Err(format!("Не удалось удалить автозапуск из реестра (код {})", rc.0))
        }
    };
    unsafe {
        let _ = RegCloseKey(h);
    }
    result
}

/// Запускает программу без окна консоли, возвращает код возврата (-1 при сбое/таймауте).
pub fn run_hidden(exe: &str, args: &[&str], timeout_secs: u64) -> i32 {
    let Ok(mut child) = Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return -1;
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.code().unwrap_or(-1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return -1;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return -1,
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Запуск с правами администратора (runas). Возвращает код возврата, -1 при отказе.
pub fn run_elevated(exe: &str, params: &str, timeout_ms: u32) -> i32 {
    let file = to_wide(exe);
    let par = to_wide(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(par.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if ShellExecuteExW(&mut info).is_err() || info.hProcess.is_invalid() {
            return -1;
        }
        if WaitForSingleObject(info.hProcess, timeout_ms) != WAIT_OBJECT_0 {
            // таймаут или сбой ожидания — убиваем процесс, как run_hidden
            let _ = TerminateProcess(info.hProcess, 1);
            let _ = CloseHandle(info.hProcess);
            return -1;
        }
        let mut code = 0u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        code as i32
    }
}

/// Включённые сетевые адаптеры: (понятное имя, IPv4), без 127.* и 169.254.*.
pub fn list_interfaces() -> Vec<(String, String)> {
    let mut result = Vec::new();
    unsafe {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size = 15000u32;
        let mut buf: Vec<u64> = Vec::new(); // u64 — для выравнивания
        let mut rc = 111u32; // ERROR_BUFFER_OVERFLOW
        for _ in 0..4 {
            buf.resize((size as usize).div_ceil(8) + 1, 0);
            rc = GetAdaptersAddresses(
                AF_INET.0 as u32,
                flags,
                None,
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut size,
            );
            if rc != 111 {
                break;
            }
        }
        if rc != 0 {
            return result;
        }
        let mut cur = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !cur.is_null() {
            let a = &*cur;
            if a.OperStatus == IfOperStatusUp && !a.FriendlyName.is_null() {
                let name = a.FriendlyName.to_string().unwrap_or_default();
                let mut u = a.FirstUnicastAddress;
                while !u.is_null() {
                    let ua = &*u;
                    let sa = ua.Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = &*(sa as *const SOCKADDR_IN);
                        let b = sin.sin_addr.S_un.S_addr.to_ne_bytes();
                        let ip = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
                        if !ip.starts_with("127.") && !ip.starts_with("169.254.") {
                            result.push((name.clone(), ip));
                        }
                    }
                    u = ua.Next;
                }
            }
            cur = a.Next;
        }
    }
    result
}

/// Показать файл в проводнике.
pub fn reveal_in_explorer(path: &str) {
    use std::os::windows::process::CommandExt as _;
    // аргумент собираем вручную, чтобы кавычки остались как в `explorer /select,"path"`
    let _ = Command::new("explorer")
        .raw_arg(format!("/select,\"{}\"", path))
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_windows_filters() {
        let wins = list_windows(&[]);
        for w in &wins {
            assert!(!HIDDEN_EXES.contains(&w.exe_name().to_lowercase().as_str()), "{:?}", w);
        }
    }

    #[test]
    fn process_info() {
        let pid = std::process::id();
        assert!(process_exe_path(pid).to_lowercase().ends_with(".exe"));
        assert!(is_process_alive(pid));
        assert!(!is_process_alive(0xFFFF_FFF0));
        assert!(is_process_64bit(pid));
    }

    #[test]
    fn interfaces() {
        for (_name, ip) in list_interfaces() {
            let parts: Vec<&str> = ip.split('.').collect();
            assert_eq!(parts.len(), 4, "{ip}");
            assert!(parts.iter().all(|p| p.parse::<u8>().is_ok()), "{ip}");
            assert!(!ip.starts_with("127."));
        }
    }

    #[test]
    fn autostart_read_does_not_panic() {
        // только чтение реестра; запись в тестах не трогаем
        let _ = autostart_enabled();
    }

    #[test]
    fn run_hidden_exit_code() {
        assert_eq!(run_hidden("cmd", &["/c", "exit", "3"], 10), 3);
    }
}
