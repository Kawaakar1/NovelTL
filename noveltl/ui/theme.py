"""Темы оформления (тёмная, светлая, как в Windows) и значок приложения."""
from __future__ import annotations

from PySide6.QtCore import QRectF, Qt
from PySide6.QtGui import QColor, QFont, QGuiApplication, QIcon, QLinearGradient, QPainter, QPixmap

THEME_SYSTEM, THEME_DARK, THEME_LIGHT = "system", "dark", "light"
THEME_TITLES = {THEME_SYSTEM: "Как в Windows", THEME_DARK: "Тёмная", THEME_LIGHT: "Светлая"}

DARK = {
    "bg": "#14151F", "panel": "#1B1D2A", "card": "#232537", "border": "#313450",
    "text": "#E9EAF4", "dim": "#9497B3", "accent": "#8B7CFF", "accent_hover": "#A094FF",
    "on_accent": "#14122B", "ok": "#5DD39E", "warn": "#F2B45A", "bad": "#F07178",
    "selected": "#2E2B55", "menu_selected": "#3A3670", "disabled_text": "#5D6080",
    "disabled_border": "#282A40", "primary_disabled": "#3A3766", "primary_disabled_text": "#8684A8",
    "notice_bg": "#3A3120", "handle": "#E9EAF4",
}
LIGHT = {
    "bg": "#F4F5FA", "panel": "#E8EAF3", "card": "#FFFFFF", "border": "#D2D5E3",
    "text": "#1D1F2E", "dim": "#666A85", "accent": "#6C5CE7", "accent_hover": "#5747D1",
    "on_accent": "#FFFFFF", "ok": "#178A5B", "warn": "#A8660B", "bad": "#CF3B48",
    "selected": "#E3DFFF", "menu_selected": "#DAD5FF", "disabled_text": "#A9ACC0",
    "disabled_border": "#E1E3EE", "primary_disabled": "#CFCBF3", "primary_disabled_text": "#FFFFFF",
    "notice_bg": "#FFF1D0", "handle": "#6C5CE7",
}

ACCENT = DARK["accent"]  # для рамки плашки поверх игры — она от темы не зависит
_current = dict(DARK)


def c(name: str) -> str:
    """Цвет текущей темы по имени."""
    return _current[name]


def system_is_dark() -> bool:
    try:
        return QGuiApplication.styleHints().colorScheme() != Qt.ColorScheme.Light
    except AttributeError:  # старый Qt
        return True


def resolve(theme: str) -> str:
    if theme == THEME_LIGHT:
        return THEME_LIGHT
    if theme == THEME_DARK:
        return THEME_DARK
    return THEME_DARK if system_is_dark() else THEME_LIGHT


def apply_theme(app, theme: str) -> None:
    """Применяет тему ко всему приложению. theme: system / dark / light."""
    _current.clear()
    _current.update(LIGHT if resolve(theme) == THEME_LIGHT else DARK)
    app.setStyleSheet(build_qss(_current))


def build_qss(p: dict) -> str:
    return f"""
* {{ font-family: "Segoe UI"; font-size: 14px; color: {p['text']}; }}
QWidget#root, QDialog {{ background: {p['bg']}; }}
QToolTip {{ background: {p['card']}; color: {p['text']}; border: 1px solid {p['border']}; padding: 4px 8px; }}
QLabel {{ background: transparent; }}
QLabel#h1 {{ font-size: 22px; font-weight: 600; }}
QLabel#h2 {{ font-size: 17px; font-weight: 600; }}
QLabel#dim {{ color: {p['dim']}; }}
QLabel#small {{ color: {p['dim']}; font-size: 12px; }}
QLabel#jp {{ font-family: "Yu Gothic UI", "Meiryo", "Segoe UI"; font-size: 15px; color: {p['dim']}; }}
QLabel#found {{ font-family: "Yu Gothic UI", "Meiryo", "Segoe UI"; font-size: 17px; color: {p['text']}; }}
QLabel#tr {{ font-size: 16px; }}
QLabel#step {{ color: {p['accent']}; font-size: 12px; font-weight: 600; letter-spacing: 1px; }}
QLabel#notice {{ background: {p['notice_bg']}; color: {p['warn']}; border-radius: 8px; padding: 8px 12px; }}
QFrame#panel {{ background: {p['panel']}; border: none; }}
QFrame#card {{ background: {p['card']}; border: 1px solid {p['border']}; border-radius: 12px; }}
QFrame#best {{ background: {p['card']}; border: 2px solid {p['accent']}; border-radius: 14px; }}
QFrame#line {{ background: {p['border']}; max-height: 1px; min-height: 1px; border: none; }}
QPushButton {{
    background: {p['card']}; border: 1px solid {p['border']}; border-radius: 8px; padding: 8px 16px;
}}
QPushButton:hover {{ border-color: {p['accent']}; }}
QPushButton:pressed {{ background: {p['border']}; }}
QPushButton:checked {{ background: {p['selected']}; border-color: {p['accent']}; }}
QPushButton:disabled {{ color: {p['disabled_text']}; border-color: {p['disabled_border']}; }}
QPushButton#primary {{ background: {p['accent']}; border: none; color: {p['on_accent']}; font-weight: 600; }}
QPushButton#primary:hover {{ background: {p['accent_hover']}; }}
QPushButton#primary:disabled {{ background: {p['primary_disabled']}; color: {p['primary_disabled_text']}; }}
QPushButton#big {{
    background: {p['accent']}; border: none; color: {p['on_accent']}; font-weight: 700; font-size: 17px;
    padding: 14px 24px; border-radius: 10px;
}}
QPushButton#big:hover {{ background: {p['accent_hover']}; }}
QPushButton#big:disabled {{ background: {p['primary_disabled']}; color: {p['primary_disabled_text']}; }}
QPushButton#link {{ background: transparent; border: none; color: {p['accent']}; padding: 4px 2px; text-align: left; }}
QPushButton#link:hover {{ color: {p['accent_hover']}; }}
QPushButton#danger:hover {{ border-color: {p['bad']}; color: {p['bad']}; }}
QPushButton#swatch {{ border-radius: 6px; min-width: 56px; max-width: 56px; min-height: 26px; padding: 0; }}
QListWidget {{ background: transparent; border: none; outline: none; }}
QListWidget::item {{ border-radius: 8px; padding: 10px 12px; margin: 2px 0; color: {p['text']}; }}
QListWidget::item:hover {{ background: {p['card']}; }}
QListWidget::item:selected {{ background: {p['selected']}; color: {p['text']}; }}
QListWidget#wins::item {{ background: {p['card']}; border: 1px solid {p['border']}; margin: 3px 0; padding: 12px; }}
QListWidget#wins::item:selected {{ background: {p['selected']}; border: 1px solid {p['accent']}; }}
QLineEdit, QComboBox, QFontComboBox, QKeySequenceEdit {{
    background: {p['panel']}; border: 1px solid {p['border']}; border-radius: 8px; padding: 6px 10px;
    selection-background-color: {p['accent']}; selection-color: {p['on_accent']}; min-height: 22px;
}}
QLineEdit:focus, QComboBox:focus {{ border-color: {p['accent']}; }}
QComboBox::drop-down {{ border: none; width: 24px; }}
QComboBox QAbstractItemView {{
    background: {p['card']}; border: 1px solid {p['border']}; outline: none;
    selection-background-color: {p['menu_selected']}; selection-color: {p['text']};
}}
QSlider::groove:horizontal {{ height: 4px; background: {p['border']}; border-radius: 2px; }}
QSlider::sub-page:horizontal {{ background: {p['accent']}; border-radius: 2px; }}
QSlider::handle:horizontal {{ background: {p['handle']}; width: 14px; margin: -6px 0; border-radius: 7px; }}
QTableWidget {{
    background: {p['panel']}; border: 1px solid {p['border']}; border-radius: 8px; gridline-color: {p['border']};
}}
QHeaderView::section {{ background: {p['card']}; border: none; border-bottom: 1px solid {p['border']}; padding: 6px; }}
QTableWidget::item:selected {{ background: {p['selected']}; color: {p['text']}; }}
QScrollBar:vertical {{ background: transparent; width: 10px; margin: 0; }}
QScrollBar::handle:vertical {{ background: {p['border']}; border-radius: 5px; min-height: 30px; }}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {{ height: 0; }}
QScrollArea, QScrollArea > QWidget > QWidget {{ background: transparent; border: none; }}
QMenu {{ background: {p['card']}; border: 1px solid {p['border']}; padding: 6px; }}
QMenu::item {{ padding: 6px 22px; border-radius: 6px; }}
QMenu::item:selected {{ background: {p['menu_selected']}; }}
QMenu::item:disabled {{ color: {p['disabled_text']}; }}
QMenu::separator {{ height: 1px; background: {p['border']}; margin: 4px 8px; }}
QProgressBar {{ background: {p['border']}; border: none; border-radius: 3px; max-height: 6px; }}
QProgressBar::chunk {{ background: {p['accent']}; border-radius: 3px; }}
QMessageBox, QColorDialog {{ background: {p['bg']}; }}
"""


def app_icon() -> QIcon:
    icon = QIcon()
    for size in (16, 24, 32, 48, 64, 128, 256):
        icon.addPixmap(icon_pixmap(size))
    return icon


def icon_pixmap(size: int) -> QPixmap:
    pm = QPixmap(size, size)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    grad = QLinearGradient(0, 0, size, size)
    grad.setColorAt(0, QColor("#A094FF"))
    grad.setColorAt(1, QColor("#E86AA6"))
    p.setBrush(grad)
    p.setPen(Qt.NoPen)
    r = size * 0.22
    p.drawRoundedRect(QRectF(0, 0, size, size), r, r)
    font = QFont("Yu Gothic UI", 10, QFont.Bold)
    font.setPixelSize(int(size * 0.66))
    p.setFont(font)
    p.setPen(QColor("#FFFFFF"))
    p.drawText(QRectF(0, -size * 0.03, size, size), Qt.AlignCenter, "訳")
    p.end()
    return pm
