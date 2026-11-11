"""Работа с окнами и процессами Windows через ctypes."""
from __future__ import annotations

import ctypes
import os
import subprocess
from ctypes import wintypes
from dataclasses import dataclass

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
shell32 = ctypes.WinDLL("shell32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi", use_last_error=True)

HWND = wintypes.HWND
LONG_PTR = ctypes.c_ssize_t

GWL_EXSTYLE = -20
WS_EX_TRANSPARENT = 0x00000020
WS_EX_TOOLWINDOW = 0x00000080
WS_EX_LAYERED = 0x00080000
WS_EX_NOACTIVATE = 0x08000000
DWMWA_CLOAKED = 14
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
SYNCHRONIZE = 0x00100000
CREATE_NO_WINDOW = 0x08000000
SW_HIDE = 0
SEE_MASK_NOCLOSEPROCESS = 0x00000040
SEE_MASK_NOASYNC = 0x00000100

EnumWindowsProc = ctypes.WINFUNCTYPE(wintypes.BOOL, HWND, wintypes.LPARAM)

user32.EnumWindows.argtypes = [EnumWindowsProc, wintypes.LPARAM]
user32.IsWindowVisible.argtypes = [HWND]
user32.IsWindow.argtypes = [HWND]
user32.IsIconic.argtypes = [HWND]
user32.GetWindowTextLengthW.argtypes = [HWND]
user32.GetWindowTextW.argtypes = [HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetClassNameW.argtypes = [HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetWindowThreadProcessId.argtypes = [HWND, ctypes.POINTER(wintypes.DWORD)]
user32.GetForegroundWindow.restype = HWND
user32.GetClientRect.argtypes = [HWND, ctypes.POINTER(wintypes.RECT)]
user32.ClientToScreen.argtypes = [HWND, ctypes.POINTER(wintypes.POINT)]
user32.GetWindowLongPtrW.argtypes = [HWND, ctypes.c_int]
user32.GetWindowLongPtrW.restype = LONG_PTR
user32.SetWindowLongPtrW.argtypes = [HWND, ctypes.c_int, LONG_PTR]
user32.SetWindowLongPtrW.restype = LONG_PTR
user32.RegisterHotKey.argtypes = [HWND, ctypes.c_int, wintypes.UINT, wintypes.UINT]
user32.RegisterHotKey.restype = wintypes.BOOL
user32.UnregisterHotKey.argtypes = [HWND, ctypes.c_int]
kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.QueryFullProcessImageNameW.argtypes = [
    wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
kernel32.IsWow64Process.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.BOOL)]
kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
dwmapi.DwmGetWindowAttribute.argtypes = [HWND, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD]

# Системные окна и браузеры — в списке игр их не показываем
HIDDEN_EXES = {
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
}


@dataclass
class WindowInfo:
    hwnd: int
    pid: int
    title: str
    exe_path: str

    @property
    def exe_name(self) -> str:
        return os.path.basename(self.exe_path)


def window_pid(hwnd: int) -> int:
    pid = wintypes.DWORD(0)
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def window_title(hwnd: int) -> str:
    n = user32.GetWindowTextLengthW(hwnd)
    if n <= 0:
        return ""
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


BROWSER_CLASSES = ("Chrome_WidgetWin", "MozillaWindowClass", "ApplicationFrameWindow", "CASCADIA_")


def window_class(hwnd: int) -> str:
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def process_exe_path(pid: int) -> str:
    h = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wintypes.DWORD(1024)
        if kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            return buf.value
        return ""
    finally:
        kernel32.CloseHandle(h)


def is_process_64bit(pid: int) -> bool:
    """True, если процесс 64-разрядный (на 64-разрядной Windows)."""
    h = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h:
        return True
    try:
        wow64 = wintypes.BOOL(False)
        kernel32.IsWow64Process(h, ctypes.byref(wow64))
        return not bool(wow64.value)
    finally:
        kernel32.CloseHandle(h)


def is_process_alive(pid: int) -> bool:
    h = kernel32.OpenProcess(SYNCHRONIZE, False, pid)
    if not h:
        return False
    try:
        return kernel32.WaitForSingleObject(h, 0) == 0x102  # WAIT_TIMEOUT
    finally:
        kernel32.CloseHandle(h)


def _is_cloaked(hwnd: int) -> bool:
    val = wintypes.DWORD(0)
    dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, ctypes.byref(val), ctypes.sizeof(val))
    return bool(val.value)


def list_windows(own_allowed: set | None = None) -> list[WindowInfo]:
    """Окна-кандидаты в игры. Свои окна скрыты, кроме перечисленных в own_allowed (демо)."""
    own_allowed = own_allowed or set()
    own_pid = os.getpid()
    found: list[WindowInfo] = []

    def cb(hwnd, _lparam):
        hwnd = int(hwnd or 0)
        if not user32.IsWindowVisible(hwnd) or _is_cloaked(hwnd):
            return True
        title = window_title(hwnd)
        if not title.strip():
            return True
        if user32.GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW:
            return True
        pid = window_pid(hwnd)
        if pid == own_pid and hwnd not in own_allowed:
            return True
        rect = wintypes.RECT()
        user32.GetClientRect(hwnd, ctypes.byref(rect))
        if not user32.IsIconic(hwnd) and (rect.right < 200 or rect.bottom < 150):
            return True
        if pid != own_pid and window_class(hwnd).startswith(BROWSER_CLASSES):
            return True  # браузеры и программы на их основе
        exe = process_exe_path(pid)
        if not exe or (pid != own_pid and os.path.basename(exe).lower() in HIDDEN_EXES):
            return True
        if "\\windows\\" in exe.lower() and pid != own_pid:
            return True
        found.append(WindowInfo(hwnd, pid, title, exe))
        return True

    user32.EnumWindows(EnumWindowsProc(cb), 0)
    return found


def is_window(hwnd: int) -> bool:
    return bool(hwnd) and bool(user32.IsWindow(hwnd))


def is_minimized(hwnd: int) -> bool:
    return bool(user32.IsIconic(hwnd))


def foreground_window() -> int:
    return int(user32.GetForegroundWindow() or 0)


def client_rect_on_screen(hwnd: int) -> tuple[int, int, int, int] | None:
    """Клиентская область окна в экранных координатах: (x, y, w, h)."""
    rect = wintypes.RECT()
    if not user32.GetClientRect(hwnd, ctypes.byref(rect)):
        return None
    pt = wintypes.POINT(0, 0)
    if not user32.ClientToScreen(hwnd, ctypes.byref(pt)):
        return None
    if rect.right <= 0 or rect.bottom <= 0:
        return None
    return pt.x, pt.y, rect.right, rect.bottom


def set_overlay_styles(hwnd: int, click_through: bool) -> None:
    """Не забирать фокус; при click_through клики проходят сквозь окно."""
    style = user32.GetWindowLongPtrW(hwnd, GWL_EXSTYLE)
    new = style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED
    if click_through:
        new |= WS_EX_TRANSPARENT
    else:
        new &= ~WS_EX_TRANSPARENT
    if new != style:
        user32.SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new)


def run_hidden(args: list[str], timeout: float = 30) -> int:
    """Запускает программу без окна консоли, возвращает код возврата (-1 при сбое)."""
    try:
        return subprocess.run(args, creationflags=CREATE_NO_WINDOW, timeout=timeout).returncode
    except (OSError, subprocess.SubprocessError):
        return -1


class SHELLEXECUTEINFOW(ctypes.Structure):
    _fields_ = [
        ("cbSize", wintypes.DWORD), ("fMask", wintypes.ULONG), ("hwnd", HWND),
        ("lpVerb", wintypes.LPCWSTR), ("lpFile", wintypes.LPCWSTR),
        ("lpParameters", wintypes.LPCWSTR), ("lpDirectory", wintypes.LPCWSTR),
        ("nShow", ctypes.c_int), ("hInstApp", wintypes.HINSTANCE), ("lpIDList", ctypes.c_void_p),
        ("lpClass", wintypes.LPCWSTR), ("hkeyClass", wintypes.HKEY), ("dwHotKey", wintypes.DWORD),
        ("hIcon", wintypes.HANDLE), ("hProcess", wintypes.HANDLE),
    ]


shell32.ShellExecuteExW.argtypes = [ctypes.POINTER(SHELLEXECUTEINFOW)]


def run_elevated(exe: str, params: str, timeout_ms: int = 30000) -> int:
    """Запуск с правами администратора (runas). Возвращает код возврата, -1 при отказе."""
    info = SHELLEXECUTEINFOW()
    info.cbSize = ctypes.sizeof(info)
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC
    info.lpVerb = "runas"
    info.lpFile = exe
    info.lpParameters = params
    info.nShow = SW_HIDE
    if not shell32.ShellExecuteExW(ctypes.byref(info)) or not info.hProcess:
        return -1
    try:
        kernel32.WaitForSingleObject(info.hProcess, timeout_ms)
        code = wintypes.DWORD(0)
        kernel32.GetExitCodeProcess(info.hProcess, ctypes.byref(code))
        return int(code.value)
    finally:
        kernel32.CloseHandle(info.hProcess)
