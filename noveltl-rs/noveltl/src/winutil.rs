//! Работа с окнами и процессами Windows (перенос `noveltl/winutil.py` и `net.list_interfaces`).
#![allow(dead_code)]

use noveltl_core::i18n::{t, tf};
use noveltl_core::langdetect::Lang;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows::core::{w, PCSTR, PCWSTR};
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

/// Только для проверок разработчиком: область экрана «x,y,ширина,высота» из переменной
/// NOVELTL_TEST_AREA (например, второй монитор), где открывать окна программы.
pub fn test_area() -> Option<[f32; 4]> {
    let v = std::env::var("NOVELTL_TEST_AREA").ok()?;
    let n: Vec<f32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (n.len() == 4 && n[2] > 0.0 && n[3] > 0.0).then(|| [n[0], n[1], n[2], n[3]])
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

/// Процесса нет только при ERROR_INVALID_PARAMETER; «нет доступа» (защищённый процесс, другой пользователь) — он жив.
fn open_error_means_dead(e: &windows::core::Error) -> bool {
    e.code() == windows::core::HRESULT::from_win32(87)
}

pub fn is_process_alive(pid: u32) -> bool {
    unsafe {
        let h = match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => h,
            Err(e) => return !open_error_means_dead(&e),
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

/// Путь (в нижнем регистре) лежит в системной папке Windows (%WINDIR%), а не просто в папке с именем «windows».
fn is_in_windows_dir(low_path: &str) -> bool {
    let dir = std::env::var("SystemRoot").or_else(|_| std::env::var("windir")).unwrap_or_else(|_| "C:\\Windows".into());
    low_path.starts_with(&format!("{}\\", dir.to_lowercase().trim_end_matches('\\')))
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
        if !own && is_in_windows_dir(&low) {
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
        return Err(t("Не удалось открыть раздел автозапуска в реестре").to_string());
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
                    Err(tf("Не удалось записать автозапуск в реестр (код {})", &[&rc.0]))
                }
            }
            Err(e) => Err(tf("Не удалось определить путь к программе: {}", &[&e])),
        }
    } else {
        let rc = unsafe { RegDeleteValueW(h, PCWSTR(name.as_ptr())) };
        // 2 — значения и так нет
        if rc.0 == 0 || rc.0 == 2 {
            Ok(())
        } else {
            Err(tf("Не удалось удалить автозапуск из реестра (код {})", &[&rc.0]))
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

/// «Открыть» как двойной щелчок: Windows сама спросит права администратора, если игре они нужны.
pub fn shell_open(file: &str, params: &str, dir: &str) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (f, p, d) = (to_wide(file), to_wide(params), to_wide(dir));
    let ptr = |v: &Vec<u16>, empty: bool| if empty { PCWSTR::null() } else { PCWSTR(v.as_ptr()) };
    let r = unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(f.as_ptr()), ptr(&p, params.is_empty()), ptr(&d, dir.is_empty()), SW_SHOWNORMAL)
    };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(tf("Не удалось запустить игру: {}", &[&format!("{file} (код {})", r.0 as isize)]))
    }
}

/// Значок exe 48x48 как (ширина, высота, RGBA) — для библиотеки игр.
pub fn exe_icon(path: &str) -> Option<(usize, usize, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, HBITMAP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, PrivateExtractIconsW, HICON, ICONINFO};
    const N: i32 = 48;
    let mut name = [0u16; 260];
    for (d, s) in name.iter_mut().zip(path.encode_utf16().take(259)) {
        *d = s;
    }
    unsafe {
        let mut icons = [HICON::default()];
        if PrivateExtractIconsW(&name, 0, N, N, Some(&mut icons), None, 0) == 0 || icons[0].is_invalid() {
            return None;
        }
        let mut info = ICONINFO::default();
        let got = GetIconInfo(icons[0], &mut info).is_ok();
        let mut out = None;
        if got {
            let dc = GetDC(None);
            let mem = CreateCompatibleDC(dc);
            let read = |bmp: HBITMAP| -> Vec<u8> {
                let mut bi = BITMAPINFO::default();
                bi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: N,
                    biHeight: -N, // сверху вниз
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                };
                let mut buf = vec![0u8; (N * N * 4) as usize];
                let n = GetDIBits(mem, bmp, 0, N as u32, Some(buf.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
                if n == 0 {
                    buf.clear();
                }
                buf
            };
            let mut px = read(info.hbmColor);
            if px.len() == (N * N * 4) as usize {
                if px.chunks(4).all(|c| c[3] == 0) {
                    // старый значок без альфа-канала: прозрачность — по маске (белое = прозрачно)
                    let mask = read(info.hbmMask);
                    for (i, c) in px.chunks_mut(4).enumerate() {
                        c[3] = if mask.get(i * 4).is_some_and(|m| *m != 0) { 0 } else { 255 };
                    }
                }
                for c in px.chunks_mut(4) {
                    c.swap(0, 2); // BGRA -> RGBA
                }
                out = Some((N as usize, N as usize, px));
            }
            let _ = DeleteDC(mem);
            ReleaseDC(None, dc);
            let _ = DeleteObject(info.hbmColor);
            let _ = DeleteObject(info.hbmMask);
        }
        let _ = DestroyIcon(icons[0]);
        out
    }
}

/// Стандартный диалог «Открыть» для выбора exe. None — отмена или ошибка.
pub fn pick_exe() -> Option<std::path::PathBuf> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let filter = HSTRING::from(t("Программы (*.exe)"));
        let spec = [COMDLG_FILTERSPEC { pszName: PCWSTR(filter.as_ptr()), pszSpec: w!("*.exe") }];
        dlg.SetFileTypes(&spec).ok()?;
        dlg.SetTitle(&HSTRING::from(t("Выберите файл игры"))).ok()?;
        dlg.Show(None).ok()?;
        let item = dlg.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path.map(std::path::PathBuf::from)
    }
}

/// Открывает папку или ссылку программой по умолчанию.
pub fn open_url(target: &str) {
    let _ = Command::new("explorer.exe").arg(target).spawn();
}

/// Байты в кодовой странице `cp` → текст; None, если байты для неё недопустимы.
fn decode_cp(cp: u32, bytes: &[u8]) -> Option<String> {
    use windows::Win32::Globalization::{MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    // без MB_ERR_INVALID_CHARS: мусор от LunaHook уже потерял байты («・»), строгий разбор отвергал бы почти всё
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    unsafe {
        let n = MultiByteToWideChar(cp, flags, bytes, None);
        if n <= 0 {
            return None;
        }
        let mut buf = vec![0u16; n as usize];
        let n = MultiByteToWideChar(cp, flags, bytes, Some(&mut buf));
        (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// Мусор, который LunaHook получил из Shift-JIS-декодирования чужого ANSI-текста: какой это на самом деле язык.
/// Проверяем по очереди 1251, 949, 936; Some — только если почти весь не-ASCII результат в нужном письме.
pub fn guess_codepage(mojibake: &str) -> Option<Lang> {
    use windows::Win32::Globalization::WideCharToMultiByte;
    let wide: Vec<u16> = mojibake.encode_utf16().collect();
    let bytes = unsafe {
        let n = WideCharToMultiByte(932, 0, &wide, None, PCSTR::null(), None);
        if n <= 0 {
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        let n = WideCharToMultiByte(932, 0, &wide, Some(&mut buf), PCSTR::null(), None);
        buf.truncate(n.max(0) as usize);
        buf
    };
    // доля нужного письма среди всех не-ASCII символов (без пробелов, «・» и U+FFFD): чужие символы и знаки — признак неверной кодовой страницы
    let share = |text: &str, ok: fn(char) -> bool| {
        let all: Vec<char> = text.chars().filter(|c| !c.is_ascii() && !c.is_whitespace() && *c != '\u{30FB}' && *c != '\u{FFFD}').collect();
        if all.len() >= 2 { all.iter().filter(|c| ok(**c)).count() as f32 / all.len() as f32 } else { 0.0 }
    };
    let tries: [(Lang, fn(char) -> bool); 3] = [
        (Lang::Ru, |c| ('\u{0400}'..='\u{04FF}').contains(&c)),
        (Lang::Ko, |c| ('\u{AC00}'..='\u{D7A3}').contains(&c)),
        // иероглифы и китайская пунктуация
        (Lang::Zh, |c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3000}'..='\u{303F}').contains(&c) || ('\u{FF00}'..='\u{FFEF}').contains(&c)),
    ];
    let shares = tries.map(|(lang, ok)| decode_cp(lang.codepage(), &bytes).map_or(0.0, |t| share(&t, ok)));
    // 1251 «проглатывает» любые байты (корейский в 1251 даёт ~0.8 кириллицы), поэтому русскому нужна почти полная доля
    let min = [0.95, 0.8, 0.6];
    tries.into_iter().zip(shares).zip(min).find_map(|(((lang, _), s), m)| (s > m).then_some(lang))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn windows_dir_filter_is_system_only() {
        let dir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()).to_lowercase();
        assert!(is_in_windows_dir(&format!("{dir}\\system32\\notepad.exe")));
        assert!(!is_in_windows_dir("d:\\games\\vn\\windows\\game.exe"));
    }

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
        // System Idle/System: открыть нельзя (доступ запрещён), но процесс есть
        assert!(is_process_alive(4));
        assert!(!open_error_means_dead(&windows::core::Error::from(windows::core::HRESULT::from_win32(5))));
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

    /// Строка → байты в `cp` → декодирование как Shift-JIS (то, что делает LunaHook при неверной кодовой странице).
    pub(crate) fn mojibake(text: &str, cp: u32) -> String {
        use windows::Win32::Globalization::{MultiByteToWideChar, WideCharToMultiByte, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let n = WideCharToMultiByte(cp, 0, &wide, None, PCSTR::null(), None);
            let mut b = vec![0u8; n as usize];
            WideCharToMultiByte(cp, 0, &wide, Some(&mut b), PCSTR::null(), None);
            let n = MultiByteToWideChar(932, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), &b, None);
            let mut w = vec![0u16; n as usize];
            MultiByteToWideChar(932, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), &b, Some(&mut w));
            String::from_utf16_lossy(&w)
        }
    }

    #[test]
    fn guesses_language_from_sjis_mojibake() {
        use noveltl_core::langdetect::looks_like_sjis_mojibake as looks;
        let samples: [(Lang, u32, &[&str]); 3] = [
            (
                Lang::Ru,
                1251,
                &[
                    "Привет, как дела? Сегодня отличная погода, давай погуляем после уроков.",
                    "Я не знаю, что тебе ответить. Может быть, ты просто не хочешь меня слушать?",
                    "Хорошо, я пойду с тобой, но только если ты расскажешь мне всю правду.",
                    "Она тихо закрыла дверь и пошла по тёмному коридору, не оглядываясь назад.",
                ],
            ),
            (
                Lang::Ko,
                949,
                &[
                    "안녕하세요, 오늘 날씨가 정말 좋네요. 같이 산책하러 가실래요?",
                    "저는 아직 잘 모르겠어요. 그냥 제 말을 듣고 싶지 않으신 건가요?",
                    "알겠어요, 같이 갈게요. 하지만 모든 사실을 솔직하게 말해 주셔야 해요.",
                    "그녀는 조용히 문을 닫고 어두운 복도를 따라 뒤도 돌아보지 않고 걸어갔다.",
                ],
            ),
            (
                Lang::Zh,
                936,
                &[
                    "你好，今天天气真好，我们放学后一起去散步吧。我已经等了你很久。",
                    "我不知道该怎么回答你，也许你根本不想听我说话。难道不是吗？",
                    "好吧，我跟你一起去，但是你必须把所有的真相都告诉我。",
                    "她轻轻地关上了门，沿着黑暗的走廊走了下去，没有回头。",
                ],
            ),
        ];
        for (lang, cp, lines) in samples {
            let mut all = String::new();
            for text in lines {
                let m = mojibake(text, cp);
                assert!(looks(&m), "{lang:?}: гейт не пропустил: {m}");
                assert_eq!(guess_codepage(&m), Some(lang), "{lang:?} по одной реплике: {m}");
                all.push_str(&m);
            }
            assert_eq!(guess_codepage(&all), Some(lang), "{lang:?} по склейке");
        }
    }

    /// Японские реплики не проходят гейт, поэтому язык «Авто» на них никогда не меняется.
    #[test]
    fn japanese_lines_never_reach_guess() {
        use noveltl_core::langdetect::looks_like_sjis_mojibake as looks;
        for line in [
            "「おはようございます、有地さん。今日もいい天気ですね。」",
            "そんなこと、わたしにはわからないよ……。",
            "ＡＢＣ・・・本当に行くのですか？",
            "「……うん。わかった。いっしょに行こう」",
            // строки без каны (заголовки, даты) тоже не проходят гейт
            "生徒会長選挙立候補者演説",
            "四月一日　入学式当日朝",
        ] {
            assert!(!looks(line), "{line}");
        }
    }
}

/// Язык интерфейса по языку Windows (для тех, кто его ещё не выбирал).
pub fn system_ui_lang() -> noveltl_core::i18n::UiLang {
    use noveltl_core::i18n::UiLang;
    // младшие 10 бит LANGID — основной язык
    let primary = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3FF;
    match primary {
        0x19 | 0x22 | 0x23 | 0x3F => UiLang::Ru, // русский, украинский, белорусский, казахский
        0x11 => UiLang::Ja,
        0x04 => UiLang::Zh,
        0x12 => UiLang::Ko,
        _ => UiLang::En,
    }
}
