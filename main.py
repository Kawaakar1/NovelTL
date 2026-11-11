"""NovelTL — точка входа.

    NovelTL.exe            обычный запуск
    NovelTL.exe --demo     демо-режим: имитация игры, настоящая игра не нужна
"""
from __future__ import annotations

import argparse
import os
import sys


def main() -> int:
    parser = argparse.ArgumentParser(prog="NovelTL")
    parser.add_argument("--demo", action="store_true", help="демо-режим с имитацией игры")
    parser.add_argument("--shots", metavar="ПАПКА", help="(для разработки) снять скриншоты окон и выйти")
    parser.add_argument("--theme", choices=("system", "dark", "light"), help="тема на этот запуск")
    parser.add_argument("--selfcheck", action="store_true", help="проверить загрузку LunaHook и выйти")
    args = parser.parse_args()

    from noveltl import paths

    if args.demo or args.shots:
        # демо не трогает настоящие профили
        base = paths.app_dir()
        demo_dir = os.path.join(base, "data", "demo")
        if args.shots:
            import shutil

            demo_dir = os.path.join(base, "data", "demo_shots")
            shutil.rmtree(demo_dir, ignore_errors=True)
        os.makedirs(demo_dir, exist_ok=True)
        paths.data_dir = lambda: demo_dir  # type: ignore[assignment]

    if sys.stderr is None or sys.stdout is None:
        # у оконного exe нет консоли: ошибки пишем в лог рядом с данными
        log = open(os.path.join(paths.data_dir(), "noveltl.log"), "a", encoding="utf-8", buffering=1)
        sys.stdout = sys.stderr = log

    from PySide6.QtCore import QLockFile, Qt
    from PySide6.QtGui import QGuiApplication
    from PySide6.QtWidgets import QApplication, QMessageBox

    from noveltl import APP_NAME
    from noveltl.ui.theme import app_icon, apply_theme

    QGuiApplication.setHighDpiScaleFactorRoundingPolicy(Qt.HighDpiScaleFactorRoundingPolicy.PassThrough)
    app = QApplication(sys.argv[:1])
    app.setApplicationName(APP_NAME)
    app.setStyle("Fusion")
    # русские подписи стандартных кнопок и окон Qt («Да/Нет», выбор цвета)
    import PySide6
    from PySide6.QtCore import QLibraryInfo, QTranslator

    qt_ru = QTranslator(app)
    for folder in (QLibraryInfo.path(QLibraryInfo.LibraryPath.TranslationsPath),
                   os.path.join(os.path.dirname(PySide6.__file__), "translations")):
        if qt_ru.load("qtbase_ru", folder):
            app.installTranslator(qt_ru)
            break
    app.setWindowIcon(app_icon())
    app.setQuitOnLastWindowClosed(False)

    lock = QLockFile(os.path.join(paths.data_dir(), "noveltl.lock"))
    lock.setStaleLockTime(0)
    if not lock.tryLock(100):
        QMessageBox.information(None, APP_NAME, "NovelTL уже запущен — посмотрите значок в трее.")
        return 0

    # метка «программа запущена»: по ней установщик просит закрыть NovelTL перед обновлением
    import ctypes

    running_mutex = ctypes.WinDLL("kernel32").CreateMutexW(None, False, "NovelTL_Running")  # noqa: F841

    from noveltl.controller import Controller
    from noveltl.ui.main_window import MainWindow

    demo = bool(args.demo or args.shots)
    game = None
    if demo:
        from noveltl.demo import DemoGameWindow, DemoHookSource, DemoTranslatorManager

        game = DemoGameWindow()
        game.show()
        source = DemoHookSource()
        game.lineShown.connect(source.emit_line)
        manager = DemoTranslatorManager()
        controller = Controller(source, manager, demo_hwnd=int(game.winId()))
    else:
        from noveltl.lunahost import LunaHost
        from noveltl.translators import TranslatorManager

        source = LunaHost()
        controller = Controller(source, TranslatorManager())
        if args.selfcheck:
            import importlib.util
            import json

            result = {
                "frozen": bool(getattr(sys, "frozen", False)),
                "files_dir": os.path.isdir(paths.files_dir()),
                "lunahost_loaded": source._dll is not None,
                "error": source.error,
                "hotkey_errors": controller.hotkey_errors,
                "socks_ok": importlib.util.find_spec("socks") is not None
                and importlib.util.find_spec("urllib3.contrib.socks") is not None,
                "psutil_ok": importlib.util.find_spec("psutil") is not None,
                "qt_russian": QApplication.translate("QPlatformTheme", "&Yes") != "&Yes",
            }
            with open(os.path.join(paths.data_dir(), "selfcheck.json"), "w", encoding="utf-8") as f:
                json.dump(result, f, ensure_ascii=False, indent=2)
            controller.shutdown()
            lock.unlock()
            return 0 if result["lunahost_loaded"] else 2

    if args.theme:
        controller.settings.theme = args.theme
    apply_theme(app, controller.settings.theme)

    def on_system_theme_changed(*_):
        # Windows переключили между светлой и тёмной темой — подстраиваемся на лету
        apply_theme(app, controller.settings.theme)
        controller.stateChanged.emit()

    app.styleHints().colorSchemeChanged.connect(on_system_theme_changed)
    window = MainWindow(controller, demo=demo)
    window.show()
    if not demo and getattr(source, "error", ""):
        QMessageBox.critical(window, APP_NAME, source.error + "\n\nПроверьте, что папка files лежит рядом с программой.")

    if args.shots:
        from noveltl.shots import run_shots

        run_shots(app, window, controller, game, args.shots)

    code = app.exec()
    lock.unlock()
    return code


if __name__ == "__main__":
    sys.exit(main())
