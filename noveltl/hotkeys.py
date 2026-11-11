"""Глобальные горячие клавиши через RegisterHotKey."""
from __future__ import annotations

import ctypes
from ctypes import wintypes

from PySide6.QtCore import QAbstractNativeEventFilter, QCoreApplication

from .winutil import user32

WM_HOTKEY = 0x0312
MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, MOD_NOREPEAT = 0x1, 0x2, 0x4, 0x8, 0x4000

_MODS = {"ctrl": MOD_CONTROL, "alt": MOD_ALT, "shift": MOD_SHIFT, "meta": MOD_WIN, "win": MOD_WIN}
_KEYS = {
    "space": 0x20, "tab": 0x09, "pgup": 0x21, "pgdown": 0x22, "end": 0x23, "home": 0x24,
    "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28, "ins": 0x2D, "del": 0x2E,
    "pause": 0x13, "`": 0xC0, "-": 0xBD, "=": 0xBB, "[": 0xDB, "]": 0xDD, "\\": 0xDC,
    ";": 0xBA, "'": 0xDE, ",": 0xBC, ".": 0xBE, "/": 0xBF,
}


def parse_hotkey(text: str) -> tuple[int, int] | None:
    """«Ctrl+Alt+H» -> (модификаторы, код клавиши). None, если разобрать нельзя."""
    parts = [p.strip() for p in (text or "").split("+") if p.strip()]
    if not parts:
        return None
    mods = 0
    for p in parts[:-1]:
        m = _MODS.get(p.lower())
        if m is None:
            return None
        mods |= m
    key = parts[-1]
    low = key.lower()
    if len(key) == 1 and (key.isascii() and key.isalnum()):
        vk = ord(key.upper())
    elif low in _KEYS:
        vk = _KEYS[low]
    elif low.startswith("f") and low[1:].isdigit() and 1 <= int(low[1:]) <= 24:
        vk = 0x70 + int(low[1:]) - 1
    else:
        return None
    if not mods and not (0x70 <= vk <= 0x87):  # без модификаторов — только F1–F24, не буква «F»
        return None  # одиночная буква как глобальная клавиша мешала бы печатать
    return mods, vk


class HotkeyManager(QAbstractNativeEventFilter):
    def __init__(self) -> None:
        super().__init__()
        self._handlers: dict[int, callable] = {}
        self._next_id = 1
        QCoreApplication.instance().installNativeEventFilter(self)

    def clear(self) -> None:
        for hid in list(self._handlers):
            user32.UnregisterHotKey(None, hid)
        self._handlers.clear()

    def register(self, text: str, handler) -> bool:
        parsed = parse_hotkey(text)
        if not parsed:
            return False
        mods, vk = parsed
        hid = self._next_id
        self._next_id += 1
        if not user32.RegisterHotKey(None, hid, mods | MOD_NOREPEAT, vk):
            return False
        self._handlers[hid] = handler
        return True

    def nativeEventFilter(self, event_type, message):
        if event_type == b"windows_generic_MSG":
            msg = wintypes.MSG.from_address(int(message))
            if msg.message == WM_HOTKEY:
                handler = self._handlers.get(int(msg.wParam))
                if handler:
                    handler()
                    return True, 0
        return False, 0
