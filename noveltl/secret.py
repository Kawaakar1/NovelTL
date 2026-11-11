"""Шифрование секретов в настройках через Windows DPAPI (формат общий с Rust-версией)."""
from __future__ import annotations

import base64
import ctypes
from ctypes import wintypes

PREFIX = "dpapi:"
_ENTROPY = b"NovelTL"
_CRYPTPROTECT_UI_FORBIDDEN = 0x1


class DATA_BLOB(ctypes.Structure):
    _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_char))]


def _make_blob(data: bytes) -> tuple[DATA_BLOB, ctypes.Array]:
    buf = ctypes.create_string_buffer(data, len(data))
    blob = DATA_BLOB(len(data), ctypes.cast(buf, ctypes.POINTER(ctypes.c_char)))
    return blob, buf  # buf нужно держать живым, пока используется blob


def _crypt(data: bytes, protect: bool) -> bytes:
    """Вызов CryptProtectData / CryptUnprotectData. При ошибке бросает OSError."""
    crypt32 = ctypes.windll.crypt32
    kernel32 = ctypes.windll.kernel32
    ptr = ctypes.POINTER(DATA_BLOB)
    if protect:
        func = crypt32.CryptProtectData
        func.argtypes = [ptr, wintypes.LPCWSTR, ptr, ctypes.c_void_p, ctypes.c_void_p,
                         wintypes.DWORD, ptr]
    else:
        func = crypt32.CryptUnprotectData
        func.argtypes = [ptr, ctypes.c_void_p, ptr, ctypes.c_void_p, ctypes.c_void_p,
                         wintypes.DWORD, ptr]
    func.restype = wintypes.BOOL
    kernel32.LocalFree.argtypes = [ctypes.c_void_p]
    kernel32.LocalFree.restype = ctypes.c_void_p

    blob_in, buf_in = _make_blob(data)
    blob_ent, buf_ent = _make_blob(_ENTROPY)
    blob_out = DATA_BLOB()
    ok = func(ctypes.byref(blob_in), None, ctypes.byref(blob_ent), None, None,
              _CRYPTPROTECT_UI_FORBIDDEN, ctypes.byref(blob_out))
    if not ok:
        raise OSError("DPAPI: операция не удалась")
    try:
        return ctypes.string_at(blob_out.pbData, blob_out.cbData)
    finally:
        kernel32.LocalFree(blob_out.pbData)


def protect(plain: str) -> str:
    """Зашифровать строку. Пустая остаётся пустой; при любой ошибке — как есть."""
    if not plain:
        return ""
    try:
        blob = _crypt(plain.encode("utf-8"), True)
        return PREFIX + base64.b64encode(blob).decode("ascii")
    except Exception:
        return plain


def unprotect(stored: str) -> str:
    """Расшифровать строку. Без префикса — как есть; не удалось расшифровать — пусто."""
    if not stored.startswith(PREFIX):
        return stored
    try:
        blob = base64.b64decode(stored[len(PREFIX):], validate=True)
        return _crypt(blob, False).decode("utf-8")
    except Exception:
        return ""
