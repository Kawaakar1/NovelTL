"""Сборка NovelTL.exe (PyInstaller, onedir).

    .venv\\Scripts\\python.exe build.py                 (программа + установщик)
    .venv\\Scripts\\python.exe build.py --no-installer  (только программа)

Результат: dist\\NovelTL\\NovelTL.exe, рядом папка files с LunaHook.
Установщик (нужен Inno Setup 6): dist_installer\\NovelTL-Setup-<версия>.exe
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.abspath(__file__))
DIST = os.path.join(ROOT, "dist", "NovelTL")
REQUIRED = [
    "LunaHost64.dll", "LunaHook32.dll", "LunaHook64.dll", "LunaSubprocess32.exe",
    "LunaSubprocess64.exe", "msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll",
]


def make_icon(path: str) -> None:
    from PySide6.QtGui import QGuiApplication

    from noveltl.ui.theme import icon_pixmap

    app = QGuiApplication.instance() or QGuiApplication([])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if not icon_pixmap(256).save(path, "ICO"):
        raise SystemExit("не удалось сохранить значок")
    del app


# Части Qt, которые PyInstaller собирает «на всякий случай», а NovelTL не загружает
# (проверено по списку модулей работающего процесса). Пути — относительно _internal\PySide6.
QT_UNUSED = [
    "opengl32sw.dll",  # программный OpenGL, 20 МБ
    "Qt6Network.dll", "QtNetwork.pyd", "Qt6OpenGL.dll", "Qt6Pdf.dll", "Qt6Svg.dll",
    "Qt6Qml.dll", "Qt6QmlMeta.dll", "Qt6QmlModels.dll", "Qt6QmlWorkerScript.dll", "Qt6Quick.dll",
    "Qt6VirtualKeyboard.dll",
    "plugins/generic", "plugins/iconengines", "plugins/networkinformation",
    "plugins/platforminputcontexts", "plugins/tls",
    "plugins/platforms/qdirect2d.dll", "plugins/platforms/qminimal.dll", "plugins/platforms/qoffscreen.dll",
]
QT_KEEP_IMAGEFORMATS = {"qico.dll"}
QT_KEEP_TRANSLATIONS = {"qtbase_ru.qm", "qt_ru.qm"}


def prune_qt() -> None:
    """Удаляет из сборки неиспользуемые части Qt (−~55 МБ)."""
    qt = os.path.join(DIST, "_internal", "PySide6")
    before = folder_size(DIST)
    for rel in QT_UNUSED:
        path = os.path.join(qt, *rel.split("/"))
        if os.path.isdir(path):
            shutil.rmtree(path)
        elif os.path.isfile(path):
            os.remove(path)
    for sub, keep in (("plugins/imageformats", QT_KEEP_IMAGEFORMATS), ("translations", QT_KEEP_TRANSLATIONS)):
        folder = os.path.join(qt, *sub.split("/"))
        if os.path.isdir(folder):
            for name in os.listdir(folder):
                if name not in keep:
                    os.remove(os.path.join(folder, name))
    print(f"Лишние части Qt убраны: {before / 2**20:.0f} МБ -> {folder_size(DIST) / 2**20:.0f} МБ")


def folder_size(path: str) -> int:
    return sum(os.path.getsize(os.path.join(r, f)) for r, _, files in os.walk(path) for f in files)


def find_iscc() -> str | None:
    """Ищет компилятор Inno Setup (ISCC.exe)."""
    candidates = [
        os.environ.get("ISCC"),
        os.path.join(os.environ.get("LOCALAPPDATA", ""), "Programs", "Inno Setup 6", "ISCC.exe"),
        os.path.join(os.environ.get("ProgramFiles(x86)", ""), "Inno Setup 6", "ISCC.exe"),
        os.path.join(os.environ.get("ProgramFiles", ""), "Inno Setup 6", "ISCC.exe"),
    ]
    for path in candidates:
        if path and os.path.isfile(path):
            return path
    return None


def build_installer() -> None:
    from noveltl import __version__

    iscc = find_iscc()
    if not iscc:
        print("Inno Setup не найден — установщик не собран (пропускаю)")
        return
    # Тестовые данные в установщик попадать не должны
    shutil.rmtree(os.path.join(DIST, "data"), ignore_errors=True)
    # Inno Setup не понимает пути длиннее 260 символов, а папка проекта может лежать глубоко.
    # Собираем через короткую ссылку-junction на корень проекта во временной папке.
    link = os.path.join(tempfile.gettempdir(), f"ntl{os.getpid()}")
    subprocess.check_call(["cmd", "/c", "mklink", "/J", link, ROOT], stdout=subprocess.DEVNULL)
    try:
        subprocess.check_call(
            [iscc, f"/DAppVersion={__version__}", os.path.join(link, "installer", "NovelTL.iss")],
            cwd=link,
        )
    finally:
        os.rmdir(link)  # удаляет только саму ссылку, файлы проекта не трогает
    print("Установщик:", os.path.join(ROOT, "dist_installer", f"NovelTL-Setup-{__version__}.exe"))


def main() -> None:
    missing = [f for f in REQUIRED if not os.path.isfile(os.path.join(ROOT, "files", f))]
    if missing:
        raise SystemExit("В папке files не хватает: " + ", ".join(missing))
    icon = os.path.join(ROOT, "build", "icon.ico")
    make_icon(icon)
    subprocess.check_call([
        sys.executable, "-m", "PyInstaller", "--noconfirm", "--clean", "--onedir", "--windowed",
        "--name", "NovelTL", "--icon", icon,
        "--hidden-import", "socks", "--hidden-import", "urllib3.contrib.socks", "--hidden-import", "psutil",
        "--exclude-module", "tkinter", "--exclude-module", "pytest",
        "--exclude-module", "PySide6.QtNetwork", "--exclude-module", "PySide6.QtQml",
        "--exclude-module", "PySide6.QtQuick", "--exclude-module", "PySide6.QtOpenGL",
        os.path.join(ROOT, "main.py"),
    ], cwd=ROOT)
    prune_qt()
    shutil.copytree(os.path.join(ROOT, "files"), os.path.join(DIST, "files"), dirs_exist_ok=True)
    shutil.copy2(os.path.join(ROOT, "ИНСТРУКЦИЯ.txt"), DIST)
    print("Готово:", os.path.join(DIST, "NovelTL.exe"))
    if "--no-installer" not in sys.argv:
        build_installer()


if __name__ == "__main__":
    main()
