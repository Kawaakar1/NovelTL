"""Контроллер: связывает источник текста, автоподбор хука, перевод и плашку."""
from __future__ import annotations

import json
import os
import queue
import time
from dataclasses import asdict, dataclass, field

from PySide6.QtCore import QCoreApplication, QObject, QThread, QTimer, Signal

from . import APP_NAME, __version__, net, winutil
from .cache import TranslationCache
from .cleaner import clean
from .hotkeys import HotkeyManager
from .langdetect import LANG_CODEPAGE
from .lunahost import HOSTINFO_ENGINE, HOSTINFO_WARNING, HookSource
from .paths import data_dir, sub_data_dir
from .pipeline import TranslationPipeline
from .profiles import AppSettings, Profile, ProfileStore, exe_version, make_profile_id
from .scorer import NAME_PAIR_WINDOW, ThreadTracker, find_name_thread
from .translators import TranslatorManager
from .ui.overlay import Overlay


class TranslateWorker(QThread):
    """Переводит в фоне. Если реплики идут быстрее перевода — берётся только последняя."""

    done = Signal(object, object)  # seq, LineResult
    failed = Signal(object, object)  # seq, сообщение

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self._queue: queue.Queue = queue.Queue()
        self.pipeline: TranslationPipeline | None = None

    def submit(self, seq: int, name: str, text: str, use_cache: bool) -> None:
        self._queue.put((seq, name, text, use_cache))

    def stop(self) -> None:
        self._queue.put(None)
        self.wait(3000)

    def run(self) -> None:
        while True:
            job = self._queue.get()
            while job is not None and not self._queue.empty():
                job = self._queue.get_nowait()
            if job is None:
                return
            seq, name, text, use_cache = job
            pipeline = self.pipeline
            if pipeline is None:
                continue
            try:
                self.done.emit(seq, pipeline.process(name, text, use_cache))
            except Exception as e:  # noqa: BLE001
                self.failed.emit(seq, str(e) or type(e).__name__)


@dataclass
class Session:
    profile: Profile
    pid: int
    hwnd: int
    title: str
    saved: bool  # профиль уже есть на диске
    setup: bool = False  # идёт мастер настройки
    connected: bool = False
    selected: tuple | None = None
    name_key: tuple | None = None  # поток с именами говорящих
    name_used: float = 0.0  # время последнего использованного имени
    engine: str = ""
    warnings: list = field(default_factory=list)
    error: str = ""
    last_src: tuple = ("", "")
    last_result: object = None
    cache: TranslationCache | None = None
    started: float = field(default_factory=time.time)


class Controller(QObject):
    stateChanged = Signal()
    threadsChanged = Signal()
    notice = Signal(str)

    def __init__(self, source: HookSource, manager: TranslatorManager, demo_hwnd: int = 0) -> None:
        super().__init__()
        self.settings_path = os.path.join(data_dir(), "settings.json")
        self.settings = AppSettings.load(self.settings_path)
        self.store = ProfileStore(sub_data_dir("profiles"))
        self.source = source
        self.manager = manager
        self.manager.set_deepl_key(self.settings.deepl_key)
        net_msg = self.apply_network()
        if net_msg:
            QTimer.singleShot(0, lambda: self.notice.emit(net_msg))
        self.demo_hwnd = demo_hwnd
        self.tracker = ThreadTracker()
        self.session: Session | None = None
        self.overlay = Overlay(self.settings)
        self.overlay.geometryEdited.connect(self._on_overlay_moved)
        self.hotkeys = HotkeyManager()
        self.hotkey_errors: list[str] = []
        self._seq = 0
        self._threads_dirty = False
        self._closed = False
        QCoreApplication.instance().aboutToQuit.connect(self.shutdown)

        self.worker = TranslateWorker(self)
        self.worker.done.connect(self._on_translated)
        self.worker.failed.connect(self._on_translate_failed)
        self.worker.start()

        source.processConnected.connect(self._on_connected)
        source.processDisconnected.connect(self._on_disconnected)
        source.threadCreated.connect(self._on_thread)
        source.textOutput.connect(self._on_text)
        source.hostInfo.connect(self._on_hostinfo)
        source.attachFailed.connect(self._on_attach_failed)
        source.start()

        self._scan_timer = QTimer(self)
        self._scan_timer.setInterval(1500)
        self._scan_timer.timeout.connect(self._scan)
        self._scan_timer.start()
        self._ui_timer = QTimer(self)
        self._ui_timer.setInterval(300)
        self._ui_timer.timeout.connect(self._flush_threads)
        self._ui_timer.start()
        self.register_hotkeys()

    # ---------- окна и сессии ----------
    def windows(self) -> list[winutil.WindowInfo]:
        allowed = {self.demo_hwnd} if self.demo_hwnd else set()
        return winutil.list_windows(allowed)

    def _scan(self) -> None:
        s = self.session
        if s:
            if not winutil.is_process_alive(s.pid) or (self.demo_hwnd and not winutil.is_window(s.hwnd)):
                self.end_session()
            elif not winutil.is_window(s.hwnd):
                # окно пересоздано (например, смена режима экрана) — ищем новое
                wins = [w for w in self.windows() if w.pid == s.pid]
                if wins:
                    s.hwnd = wins[0].hwnd
                    self.overlay.set_target(s.hwnd)
            return
        for w in self.windows():
            profile = self.store.find_for_exe(w.exe_path)
            if profile and profile.hook:
                self.start_session(profile, w, saved=True)
                self.notice.emit(f"Игра «{profile.name}» найдена — перевод включён.")
                break

    def start_session(self, profile: Profile, win: winutil.WindowInfo, saved: bool, setup: bool = False) -> None:
        self.end_session()
        self.tracker.clear()
        s = Session(profile, win.pid, win.hwnd, win.title, saved=saved, setup=setup)
        s.cache = TranslationCache(os.path.join(sub_data_dir("cache"), profile.id + ".sqlite"))
        self.manager.cache = s.cache
        self.worker.pipeline = TranslationPipeline(self.manager, profile.lang, profile.glossary)
        self.session = s
        self.overlay.clear()
        self.overlay.set_target(win.hwnd, profile.overlay)
        self.source.attach(win.pid, LANG_CODEPAGE.get(profile.lang, 932))
        self.stateChanged.emit()

    def end_session(self) -> None:
        s = self.session
        if not s:
            return
        self.session = None
        self.worker.pipeline = None
        self.manager.cache = None
        self.overlay.set_edit_mode(False)
        self.overlay.clear()
        self.overlay.set_target(0)
        self.source.detach(s.pid)
        if s.cache:
            s.cache.close()
        self.tracker.clear()
        self.stateChanged.emit()

    # ---------- события источника ----------
    def _mine(self, pid) -> Session | None:
        s = self.session
        return s if s and int(pid) == s.pid else None

    def _on_connected(self, pid) -> None:
        s = self._mine(pid)
        if s:
            s.connected, s.error = True, ""
            self.stateChanged.emit()

    def _on_disconnected(self, pid) -> None:
        if self._mine(pid):
            self.end_session()

    def _on_attach_failed(self, pid, message) -> None:
        s = self._mine(pid)
        if s:
            s.error = str(message)
            self.stateChanged.emit()
            self.notice.emit(s.error)

    def _on_hostinfo(self, kind, text) -> None:
        s = self.session
        if not s:
            return
        if kind == HOSTINFO_ENGINE:
            s.engine = str(text)
            self.stateChanged.emit()
        elif kind == HOSTINFO_WARNING and len(s.warnings) < 50:
            s.warnings.append(str(text))

    def _on_thread(self, info) -> None:
        if self._mine(info["pid"]):
            self.tracker.ensure(info["code"], info["name"], info["addr"], info["ctx"], info["ctx2"])
            self._threads_dirty = True

    def _on_text(self, info, text) -> None:
        s = self._mine(info["pid"])
        if not s:
            return
        st = self.tracker.add_text(info["code"], info["name"], info["addr"], info["ctx"], info["ctx2"], text)
        self._threads_dirty = True
        if s.setup:
            return
        if s.selected is None:
            self._select_hook(s)
        if s.selected is not None and st.key == s.selected:
            # небольшая пауза: имя говорящего может прийти из соседнего потока чуть позже
            t_line = st.lines[-1][0]
            QTimer.singleShot(120, lambda: self._handle_line(text, t_line=t_line))

    def _select_hook(self, s: Session) -> None:
        key = self.tracker.find_saved(s.profile.hook)
        if key is not None:
            s.selected = key
            self.stateChanged.emit()
            return
        pick = self.tracker.auto_pick()
        if pick is not None:
            st = pick.stats
            s.selected = st.key
            s.profile.set_hook(st.hookcode, st.hookname, st.ctx, st.ctx2)
            if s.saved:
                self.store.save(s.profile)
            self.stateChanged.emit()
            self.notice.emit("Сохранённый источник текста не нашёлся — NovelTL сам выбрал другой.")

    def _flush_threads(self) -> None:
        if self._threads_dirty:
            self._threads_dirty = False
            self.threadsChanged.emit()

    # ---------- перевод ----------
    def _companion_name(self, s: Session, t_line: float) -> str:
        """Имя из отдельного потока, пришедшее вместе с этой репликой."""
        if s.name_key is None:
            s.name_key = self.tracker.find_saved(s.profile.name_hook)
        if s.name_key is None:
            key = find_name_thread(self.tracker.threads, s.selected)
            if key is None:
                return ""
            st = self.tracker.threads[key]
            s.name_key = key
            s.profile.name_hook = {
                "code": st.hookcode, "name": st.hookname, "ctx_lo": key[1], "ctx2_lo": key[2]}
            if s.saved:
                self.store.save(s.profile)
        st = self.tracker.threads.get(s.name_key)
        if st is None or not st.lines:
            return ""
        t_name, raw = st.lines[-1]
        if t_name <= s.name_used or abs(t_name - t_line) > NAME_PAIR_WINDOW:
            return ""
        s.name_used = t_name
        name = clean(raw).text
        return name if len(name) <= 30 else ""

    def _handle_line(self, raw: str, use_cache: bool = True, force: bool = False, t_line: float | None = None) -> None:
        s = self.session
        res = clean(raw)
        if not s or s.setup or not res.text:
            return
        if not res.name and s.selected is not None:
            res.name = self._companion_name(s, time.time() if t_line is None else t_line)
        src = (res.name, res.text)
        if src == s.last_src and not force:
            return
        s.last_src = src
        self._submit(res.name, res.text, use_cache)

    def _submit(self, name: str, text: str, use_cache: bool) -> None:
        self._seq += 1
        self.overlay.show_note("…", keep_text=True)
        self.worker.submit(self._seq, name, text, use_cache)

    def _on_translated(self, seq, result) -> None:
        s = self.session
        if not s or seq != self._seq:
            return
        s.last_result = result
        self.overlay.show_line(result.name, result.text)
        if result.notice:
            self.notice.emit(result.notice)
        self.stateChanged.emit()

    def _on_translate_failed(self, seq, message) -> None:
        if self.session and seq == self._seq:
            self.overlay.show_note(f"⚠ Не удалось перевести: {message}")
            self.stateChanged.emit()

    def retranslate(self) -> None:
        """Перевести последнюю реплику заново, минуя кэш."""
        s = self.session
        if s and s.last_src[1]:
            self._submit(s.last_src[0], s.last_src[1], use_cache=False)

    def toggle_overlay(self) -> None:
        self.overlay.user_hidden = not self.overlay.user_hidden
        self.overlay.sync()
        self.stateChanged.emit()

    # ---------- мастер настройки ----------
    def begin_setup(self, win: winutil.WindowInfo, lang: str, name: str = "") -> None:
        profile = self.store.find_for_exe(win.exe_path)
        saved = profile is not None
        if profile is None:
            version = exe_version(win.exe_path)
            exe_name = os.path.basename(win.exe_path)
            profile = Profile(
                id=make_profile_id(exe_name, version), name=name or win.title,
                exe_name=exe_name, exe_path=win.exe_path, version=version,
            )
        profile.lang = lang
        self.start_session(profile, win, saved=saved, setup=True)

    def resume_setup(self) -> bool:
        """Повторная настройка для уже подключённой игры."""
        if not self.session:
            return False
        self.session.setup = True
        self.overlay.clear()
        return True

    def choose_hook(self, key: tuple) -> None:
        s = self.session
        st = self.tracker.threads.get(key)
        if s and st:
            s.selected = key
            s.profile.set_hook(st.hookcode, st.hookname, st.ctx, st.ctx2)
            s.profile.name_hook, s.name_key = None, None  # поток имён подберётся заново

    def finish_setup(self) -> None:
        s = self.session
        if not s:
            return
        s.profile.overlay = dict(self.overlay.frac)
        self.store.save(s.profile)
        s.saved, s.setup = True, False
        self.overlay.set_edit_mode(False)
        st = self.tracker.threads.get(s.selected) if s.selected else None
        if st and st.lines:
            self._handle_line(st.lines[-1][1], force=True, t_line=st.lines[-1][0])
        self.stateChanged.emit()

    def cancel_setup(self) -> None:
        s = self.session
        if not s:
            return
        self.overlay.set_edit_mode(False)
        if not s.saved:
            self.end_session()
            return
        stored = self.store.get(s.profile.id)
        if stored:
            s.profile = stored
            self.worker.pipeline = TranslationPipeline(self.manager, stored.lang, stored.glossary)
            self.overlay.set_target(s.hwnd, stored.overlay)
        s.setup = False
        s.selected = self.tracker.find_saved(s.profile.hook)
        self.stateChanged.emit()

    def _on_overlay_moved(self, frac) -> None:
        s = self.session
        if s and not s.setup and s.saved:
            s.profile.overlay = dict(frac)
            self.store.save(s.profile)

    # ---------- профили и настройки ----------
    def delete_profile(self, profile_id: str) -> None:
        if self.session and self.session.profile.id == profile_id:
            self.end_session()
        self.store.delete(profile_id)
        try:
            os.remove(os.path.join(sub_data_dir("cache"), profile_id + ".sqlite"))
        except OSError:
            pass
        self.stateChanged.emit()

    def save_glossary(self, profile: Profile, entries: list) -> None:
        profile.glossary = entries
        self.store.save(profile)
        s = self.session
        if s and s.profile.id == profile.id:
            s.profile.glossary = entries
            if self.worker.pipeline:
                self.worker.pipeline.glossary = entries
            self.retranslate()

    def apply_network(self) -> str:
        s = self.settings
        msg = ""
        bind_ip = s.bind_ip
        if bind_ip and bind_ip not in [ip for _, ip in net.list_interfaces()]:
            bind_ip = ""
            msg = "Сохранённый сетевой интерфейс сейчас недоступен — используется автоматический выбор"
        try:
            session = net.make_session(s.net_mode, s.proxy_url, bind_ip, s.proxy_user, s.proxy_pass)
        except ValueError:
            session = net.make_session(net.NET_SYSTEM)
            msg = "Настройки прокси неверные — используется подключение как в системе"
        self.manager.set_session(session)
        return msg

    def save_settings(self) -> None:
        self.settings.save(self.settings_path)
        self.manager.set_deepl_key(self.settings.deepl_key)
        net_msg = self.apply_network()
        if net_msg:
            self.notice.emit(net_msg)
        self.register_hotkeys()
        self.overlay.update()
        self.stateChanged.emit()

    def register_hotkeys(self) -> None:
        self.hotkeys.clear()
        self.hotkey_errors = []
        for combo, handler in (
            (self.settings.hotkey_toggle, self.toggle_overlay),
            (self.settings.hotkey_retranslate, self.retranslate),
        ):
            if combo and not self.hotkeys.register(combo, handler):
                self.hotkey_errors.append(combo)

    # ---------- отчёт ----------
    def build_report(self) -> dict:
        s = self.session
        report: dict = {
            "программа": f"{APP_NAME} {__version__}",
            "время": time.strftime("%Y-%m-%d %H:%M:%S"),
            "переводчик": self.manager.status_text(),
        }
        if s:
            profile = asdict(s.profile)
            profile.pop("exe_path", None)  # в пути может быть имя пользователя
            profile["glossary"] = len(s.profile.glossary)
            report["игра"] = {
                "окно": s.title,
                "exe": s.profile.exe_name,
                "разрядность": 64 if winutil.is_process_64bit(s.pid) else 32,
                "движок": s.engine,
                "подключено": s.connected,
                "ошибка": s.error,
                "предупреждения": s.warnings,
                "секунд_с_подключения": round(time.time() - s.started),
            }
            report["профиль"] = profile
            report["выбранный_хук"] = list(s.selected) if s.selected else None
            report["хук_имён"] = list(s.name_key) if s.name_key else None
        ranked = {r.key: r for r in self.tracker.ranked()}
        threads = []
        for key, st in self.tracker.threads.items():
            r = ranked.get(key)
            threads.append({
                "код": st.hookcode,
                "имя": st.hookname,
                "addr": hex(st.addr),
                "ctx": hex(st.ctx),
                "ctx2": hex(st.ctx2),
                "ctx_lo": key[1],
                "ctx2_lo": key[2],
                "строк_всего": st.total,
                "оценка": r.score if r else 0,
                "детали_оценки": r.details if r else {},
                "последние_строки": st.last_raw(12),
            })
        threads.sort(key=lambda t: t["оценка"], reverse=True)
        report["потоки"] = threads
        return report

    def save_report(self) -> str:
        path = os.path.join(sub_data_dir("reports"), time.strftime("report_%Y%m%d_%H%M%S.json"))
        with open(path, "w", encoding="utf-8") as f:
            json.dump(self.build_report(), f, ensure_ascii=False, indent=2)
        return path

    # ---------- завершение ----------
    def shutdown(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._scan_timer.stop()
        self.hotkeys.clear()
        self.end_session()
        self.source.shutdown()
        self.worker.stop()
        self.overlay.close()
