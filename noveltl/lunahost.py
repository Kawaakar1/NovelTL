"""Источник текста: обёртка над LunaHost64.dll (движок LunaHook, GPL-3.0)."""
from __future__ import annotations

import ctypes
import os
import threading
from ctypes import CFUNCTYPE, Structure, c_bool, c_char_p, c_int, c_uint, c_uint64, c_void_p, c_wchar_p
from ctypes.wintypes import DWORD

from PySide6.QtCore import QObject, Signal

from . import winutil
from .paths import files_dir

CONNECT_TIMEOUT = 10.0
HOSTINFO_WARNING = 1
HOSTINFO_ENGINE = 4

DEFENDER_HINT = (
    "Игра не ответила за 10 секунд. Чаще всего внедрение блокирует Защитник Windows: "
    "добавьте папку NovelTL и папку игры в исключения "
    "(Безопасность Windows → Защита от вирусов и угроз → Управление настройками → Исключения), "
    "затем перезапустите игру и NovelTL."
)


class ThreadParam(Structure):
    _fields_ = [("processId", c_uint), ("addr", c_uint64), ("ctx", c_uint64), ("ctx2", c_uint64)]


ProcessEvent = CFUNCTYPE(None, DWORD)
ThreadEventMaybeEmbed = CFUNCTYPE(None, c_wchar_p, c_char_p, ThreadParam, c_bool)
ThreadEvent = CFUNCTYPE(None, c_wchar_p, c_char_p, ThreadParam)
OutputCallback = CFUNCTYPE(None, c_wchar_p, c_char_p, ThreadParam, c_wchar_p)
HostInfoHandler = CFUNCTYPE(None, c_int, c_wchar_p)
HookInsertHandler = CFUNCTYPE(None, DWORD, c_uint64, c_wchar_p)
EmbedCallback = CFUNCTYPE(None, c_wchar_p, ThreadParam)
I18NQueryCallback = CFUNCTYPE(c_void_p, c_wchar_p)
EmuGameInfoCallback = CFUNCTYPE(None, c_wchar_p, c_wchar_p, c_wchar_p)


def thread_info(hookcode, hookname, tp) -> dict:
    name = hookname.decode("utf-8", "replace") if isinstance(hookname, bytes) else (hookname or "")
    return {
        "code": hookcode or "",
        "name": name,
        "pid": int(tp.processId),
        "addr": int(tp.addr),
        "ctx": int(tp.ctx),
        "ctx2": int(tp.ctx2),
    }


class HookSource(QObject):
    """Общий интерфейс источника текста (настоящий LunaHook или демо).

    Колбэки LunaHook приходят из чужих потоков, поэтому всё передаётся сигналами
    с типом object: 64-битные адреса не помещаются в int сигнала.
    """

    processConnected = Signal(object)  # pid
    processDisconnected = Signal(object)  # pid
    threadCreated = Signal(object)  # dict
    threadRemoved = Signal(object)  # dict
    textOutput = Signal(object, object)  # dict, text
    hostInfo = Signal(object, object)  # type, text
    attachFailed = Signal(object, object)  # pid, сообщение

    def start(self) -> None: ...
    def attach(self, pid: int, codepage: int) -> None: ...
    def detach(self, pid: int) -> None: ...
    def insert_hook(self, pid: int, hookcode: str) -> None: ...
    def shutdown(self) -> None: ...


class LunaHost(HookSource):
    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self._dll = None
        self._callbacks: list = []  # ссылки на колбэки должны жить, пока жива DLL
        self._connected: dict[int, threading.Event] = {}
        self._attached: set[int] = set()
        self.error = ""

    # ---- загрузка ----
    def start(self) -> None:
        directory = files_dir()
        path = os.path.join(directory, "LunaHost64.dll")
        if not os.path.isfile(path):
            self.error = f"Не найден файл {path}"
            return
        try:
            os.add_dll_directory(directory)
            dll = ctypes.CDLL(path)
        except OSError as e:
            self.error = f"Не удалось загрузить LunaHost64.dll: {e}"
            return

        dll.Luna_Settings.argtypes = (c_int, c_int, c_int, c_int, c_bool)
        dll.Luna_Start.argtypes = (
            ProcessEvent, ProcessEvent, ThreadEventMaybeEmbed, ThreadEvent, OutputCallback,
            HostInfoHandler, HookInsertHandler, EmbedCallback, I18NQueryCallback, EmuGameInfoCallback,
        )
        dll.Luna_ConnectProcess.argtypes = (DWORD,)
        dll.Luna_CheckIfNeedInject.argtypes = (DWORD,)
        dll.Luna_CheckIfNeedInject.restype = c_bool
        dll.Luna_InsertHookCode.argtypes = (DWORD, c_wchar_p)
        dll.Luna_DetachProcess.argtypes = (DWORD,)

        self._callbacks = [
            ProcessEvent(self._on_connect),
            ProcessEvent(self._on_disconnect),
            ThreadEventMaybeEmbed(self._on_create),
            ThreadEvent(self._on_destroy),
            OutputCallback(self._on_output),
            HostInfoHandler(self._on_hostinfo),
            HookInsertHandler(lambda pid, addr, code: None),
            EmbedCallback(lambda text, tp: None),
            I18NQueryCallback(lambda text: None),  # NULL
            EmuGameInfoCallback(lambda a, b, c: None),
        ]
        dll.Luna_Start(*self._callbacks)
        self._dll = dll

    # ---- колбэки (чужие потоки!) ----
    def _on_connect(self, pid):
        ev = self._connected.get(int(pid))
        if ev:
            ev.set()
        self.processConnected.emit(int(pid))

    def _on_disconnect(self, pid):
        self._attached.discard(int(pid))
        self.processDisconnected.emit(int(pid))

    def _on_create(self, hookcode, hookname, tp, _embedable):
        self.threadCreated.emit(thread_info(hookcode, hookname, tp))

    def _on_destroy(self, hookcode, hookname, tp):
        self.threadRemoved.emit(thread_info(hookcode, hookname, tp))

    def _on_output(self, hookcode, hookname, tp, text):
        self.textOutput.emit(thread_info(hookcode, hookname, tp), text or "")

    def _on_hostinfo(self, kind, text):
        self.hostInfo.emit(int(kind), text or "")

    # ---- управление ----
    def attach(self, pid: int, codepage: int) -> None:
        if self._dll is None:
            self.attachFailed.emit(pid, self.error or "Движок LunaHook не загружен.")
            return
        if pid in self._attached:
            self.processConnected.emit(pid)
            return
        self._attached.add(pid)
        self._connected[pid] = threading.Event()
        threading.Thread(target=self._attach_worker, args=(pid, codepage), daemon=True).start()

    def _attach_worker(self, pid: int, codepage: int) -> None:
        dll = self._dll
        try:
            dll.Luna_Settings(500, codepage, 3000, 1000000, False)
            dll.Luna_ConnectProcess(pid)
            if dll.Luna_CheckIfNeedInject(pid):
                bits = "64" if winutil.is_process_64bit(pid) else "32"
                exe = os.path.join(files_dir(), f"LunaSubprocess{bits}.exe")
                hook_dll = os.path.join(files_dir(), f"LunaHook{bits}.dll")
                code = winutil.run_hidden([exe, "dllinject", str(pid), hook_dll])
                if code != 1:
                    # обычного доступа не хватило — пробуем с правами администратора
                    code = winutil.run_elevated(exe, f'dllinject {pid} "{hook_dll}"')
                if code != 1:
                    self._attached.discard(pid)
                    self.attachFailed.emit(
                        pid,
                        "Не удалось подключиться к игре (код %d). Если игра запущена от имени "
                        "администратора — запустите так же NovelTL. " % code + DEFENDER_HINT,
                    )
                    return
            if not self._connected[pid].wait(CONNECT_TIMEOUT):
                self._attached.discard(pid)
                self.attachFailed.emit(pid, DEFENDER_HINT)
        except Exception as e:  # noqa: BLE001 — чужая DLL, ловим всё
            self._attached.discard(pid)
            self.attachFailed.emit(pid, f"Ошибка подключения: {e}")

    def insert_hook(self, pid: int, hookcode: str) -> None:
        if self._dll is not None and hookcode:
            self._dll.Luna_InsertHookCode(pid, hookcode)

    def detach(self, pid: int) -> None:
        if self._dll is not None and pid in self._attached:
            self._attached.discard(pid)
            try:
                self._dll.Luna_DetachProcess(pid)
            except OSError:
                pass

    def shutdown(self) -> None:
        for pid in list(self._attached):
            self.detach(pid)
