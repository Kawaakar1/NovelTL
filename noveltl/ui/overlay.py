"""Плашка с переводом поверх окна игры."""
from __future__ import annotations

import os

from PySide6.QtCore import QPoint, QPointF, QRect, QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QFont, QFontMetricsF, QPainter, QPainterPath, QPen, QTextLayout, QTextOption
from PySide6.QtWidgets import QWidget

from .. import winutil
from ..profiles import DEFAULT_OVERLAY, AppSettings
from .theme import ACCENT

EDGE = 10  # ширина зоны, за которую тянут край
MIN_W, MIN_H = 220, 70
PAD = 18
MIN_FONT = 11

SAMPLE_NAME = "Ёсино"
SAMPLE_TEXT = "«Так будет выглядеть перевод. Перетащите плашку на окно диалога игры и растяните по размеру»."


def _wrap(text: str, font: QFont, width: float) -> tuple[list[tuple[str, float]], float]:
    """Разбивает текст на строки. Возвращает [(строка, базовая линия)], высоту."""
    layout = QTextLayout(text, font)
    opt = QTextOption()
    opt.setWrapMode(QTextOption.WrapAtWordBoundaryOrAnywhere)
    layout.setTextOption(opt)
    layout.beginLayout()
    lines, y = [], 0.0
    while True:
        line = layout.createLine()
        if not line.isValid():
            break
        line.setLineWidth(max(10.0, width))
        lines.append((text[line.textStart() : line.textStart() + line.textLength()], y + line.ascent()))
        y += line.height()
    layout.endLayout()
    return lines, y


class Overlay(QWidget):
    geometryEdited = Signal(object)  # доли от клиентской области окна игры

    def __init__(self, settings: AppSettings) -> None:
        super().__init__(None)
        self.settings = settings
        self.setWindowFlags(
            Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
        )
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setAttribute(Qt.WA_ShowWithoutActivating)
        self.setMouseTracking(True)
        self.setWindowTitle("NovelTL — плашка")

        self.target = 0
        self.frac = dict(DEFAULT_OVERLAY)
        self.edit_mode = False
        self.user_hidden = False
        self.force_active = False  # только для съёмки скриншотов
        self.name = ""
        self.text = ""
        self.note = ""  # служебная строка: «…», ошибка перевода
        self._drag = None  # (режим, стартовая точка, стартовая геометрия)
        self._game_rect: QRect | None = None

        self._timer = QTimer(self)
        self._timer.setInterval(50)
        self._timer.timeout.connect(self.sync)
        self._timer.start()

    # ---- содержимое ----
    def set_target(self, hwnd: int, frac: dict | None = None) -> None:
        self.target = int(hwnd or 0)
        if frac:
            self.frac = dict(frac)
        self.sync()

    def show_line(self, name: str, text: str) -> None:
        self.name, self.text, self.note = name, text, ""
        self.update()

    def show_note(self, note: str, keep_text: bool = False) -> None:
        self.note = note
        if not keep_text:
            self.name, self.text = "", ""
        self.update()

    def clear(self) -> None:
        self.name = self.text = self.note = ""
        self.update()

    def set_edit_mode(self, on: bool) -> None:
        self.edit_mode = on
        self._drag = None
        if not on:
            self.unsetCursor()
        self.sync()
        self.update()

    # ---- слежение за окном игры ----
    def _dpr(self) -> float:
        screen = self.screen()
        return screen.devicePixelRatio() if screen else 1.0

    def _client_rect(self) -> QRect | None:
        rect = winutil.client_rect_on_screen(self.target)
        if not rect:
            return None
        d = self._dpr()
        x, y, w, h = (int(round(v / d)) for v in rect)
        return QRect(x, y, w, h)

    def sync(self) -> None:
        if not self.target or not winutil.is_window(self.target) or winutil.is_minimized(self.target):
            self.hide()
            return
        fg = winutil.foreground_window()
        active = fg == self.target
        if self.edit_mode and fg and winutil.window_pid(fg) == os.getpid():
            active = True
        if self.force_active:
            active = True
        has_content = self.edit_mode or bool(self.text or self.note)
        if not active or not has_content or (self.user_hidden and not self.edit_mode):
            self.hide()
            return
        game = self._client_rect()
        if game is None:
            self.hide()
            return
        self._game_rect = game
        if self._drag is None:
            f = self.frac
            geo = QRect(
                game.x() + int(round(f["x"] * game.width())),
                game.y() + int(round(f["y"] * game.height())),
                max(MIN_W, int(round(f["w"] * game.width()))),
                max(MIN_H, int(round(f["h"] * game.height()))),
            )
            if geo != self.geometry():
                self.setGeometry(geo)
        if not self.isVisible():
            self.show()
        winutil.set_overlay_styles(int(self.winId()), click_through=not self.edit_mode)

    # ---- перетаскивание в режиме настройки ----
    def _hit(self, pos: QPoint) -> str:
        mode = ""
        if pos.y() <= EDGE:
            mode += "t"
        elif pos.y() >= self.height() - EDGE:
            mode += "b"
        if pos.x() <= EDGE:
            mode += "l"
        elif pos.x() >= self.width() - EDGE:
            mode += "r"
        return mode or "move"

    def _cursor_for(self, mode: str):
        return {
            "move": Qt.SizeAllCursor, "t": Qt.SizeVerCursor, "b": Qt.SizeVerCursor,
            "l": Qt.SizeHorCursor, "r": Qt.SizeHorCursor, "tl": Qt.SizeFDiagCursor,
            "br": Qt.SizeFDiagCursor, "tr": Qt.SizeBDiagCursor, "bl": Qt.SizeBDiagCursor,
        }[mode]

    def mousePressEvent(self, event) -> None:
        if self.edit_mode and event.button() == Qt.LeftButton:
            self._drag = (self._hit(event.position().toPoint()), event.globalPosition().toPoint(), self.geometry())

    def mouseMoveEvent(self, event) -> None:
        if not self.edit_mode:
            return
        if self._drag is None:
            self.setCursor(self._cursor_for(self._hit(event.position().toPoint())))
            return
        mode, start, geo = self._drag
        delta = event.globalPosition().toPoint() - start
        bounds = self._game_rect or geo
        left, top, right, bottom = geo.left(), geo.top(), geo.left() + geo.width(), geo.top() + geo.height()
        if mode == "move":
            dx = max(bounds.left() - left, min(delta.x(), bounds.left() + bounds.width() - right))
            dy = max(bounds.top() - top, min(delta.y(), bounds.top() + bounds.height() - bottom))
            left, right, top, bottom = left + dx, right + dx, top + dy, bottom + dy
        else:
            if "l" in mode:
                left = max(bounds.left(), min(left + delta.x(), right - MIN_W))
            if "r" in mode:
                right = min(bounds.left() + bounds.width(), max(right + delta.x(), left + MIN_W))
            if "t" in mode:
                top = max(bounds.top(), min(top + delta.y(), bottom - MIN_H))
            if "b" in mode:
                bottom = min(bounds.top() + bounds.height(), max(bottom + delta.y(), top + MIN_H))
        self.setGeometry(QRect(left, top, right - left, bottom - top))

    def mouseReleaseEvent(self, event) -> None:
        if self._drag is None:
            return
        self._drag = None
        game = self._game_rect
        if game and game.width() > 0 and game.height() > 0:
            g = self.geometry()
            self.frac = {
                "x": round((g.x() - game.x()) / game.width(), 4),
                "y": round((g.y() - game.y()) / game.height(), 4),
                "w": round(g.width() / game.width(), 4),
                "h": round(g.height() / game.height(), 4),
            }
            self.geometryEdited.emit(dict(self.frac))

    # ---- отрисовка ----
    def _fit(self, name: str, text: str, width: float, height: float):
        """Подбирает размер шрифта, при котором текст помещается."""
        s = self.settings
        size = max(MIN_FONT, int(s.font_size))
        while True:
            text_font = QFont(s.font_family)
            text_font.setPixelSize(size)
            name_font = QFont(s.font_family)
            name_font.setPixelSize(max(MIN_FONT, int(size * 0.8)))
            name_font.setBold(True)
            name_h = QFontMetricsF(name_font).height() + 2 if name else 0.0
            lines, text_h = _wrap(text, text_font, width)
            if name_h + text_h <= height or size <= MIN_FONT:
                return size, text_font, name_font, name_h, lines
            size -= 1

    def _draw_outlined(self, p: QPainter, x: float, baseline: float, font: QFont, text: str, color: str, size: int):
        path = QPainterPath()
        path.addText(QPointF(x, baseline), font, text)
        pen = QPen(QColor(self.settings.outline_color), max(2.0, size / 6.5))
        pen.setJoinStyle(Qt.RoundJoin)
        p.strokePath(path, pen)
        p.fillPath(path, QColor(color))

    def paintEvent(self, event) -> None:
        s = self.settings
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.setRenderHint(QPainter.TextAntialiasing)
        rect = QRectF(self.rect()).adjusted(1, 1, -1, -1)

        bg = QColor(s.bg_color)
        bg.setAlphaF(max(0.0, min(1.0, s.bg_opacity / 100)))
        p.setPen(Qt.NoPen)
        p.setBrush(bg)
        p.drawRoundedRect(rect, 12, 12)

        name, text = self.name, self.text
        if self.edit_mode and not text:
            name, text = SAMPLE_NAME, SAMPLE_TEXT

        inner = rect.adjusted(PAD, PAD * 0.6, -PAD, -PAD * 0.6)
        if text:
            size, text_font, name_font, name_h, lines = self._fit(name, text, inner.width(), inner.height())
            y = inner.top()
            if name:
                fm = QFontMetricsF(name_font)
                self._draw_outlined(p, inner.left(), y + fm.ascent(), name_font, name, s.name_color, size)
                y += name_h
            for line, baseline in lines:
                self._draw_outlined(p, inner.left(), y + baseline, text_font, line, s.text_color, size)
        if self.note:
            font = QFont(s.font_family)
            font.setPixelSize(15 if text else 18)
            fm = QFontMetricsF(font)
            if text:
                note = fm.elidedText(self.note, Qt.ElideRight, inner.width())
                x, base = inner.right() - fm.horizontalAdvance(note), rect.bottom() - 8
                self._draw_outlined(p, x, base, font, note, "#C9CBE0", 15)
            else:
                # сообщение без перевода (например, ошибка) — показываем целиком, с переносом
                for line, baseline in _wrap(self.note, font, inner.width())[0]:
                    self._draw_outlined(p, inner.left(), inner.top() + 4 + baseline, font, line, "#C9CBE0", 15)

        if self.edit_mode:
            pen = QPen(QColor(ACCENT), 2, Qt.DashLine)
            p.setPen(pen)
            p.setBrush(Qt.NoBrush)
            p.drawRoundedRect(rect, 12, 12)
            p.setPen(Qt.NoPen)
            p.setBrush(QColor(ACCENT))
            r = 5.0
            for cx in (rect.left() + 3, rect.center().x(), rect.right() - 3):
                for cy in (rect.top() + 3, rect.center().y(), rect.bottom() - 3):
                    if (cx, cy) != (rect.center().x(), rect.center().y()):
                        p.drawEllipse(QPointF(cx, cy), r, r)
