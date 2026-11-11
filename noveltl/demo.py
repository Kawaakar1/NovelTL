"""Демо-режим (--demo): имитация игры и потоков LunaHook, чтобы смотреть окна без игры."""
from __future__ import annotations

import os

from PySide6.QtCore import QRectF, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QFont, QLinearGradient, QPainter, QPen
from PySide6.QtWidgets import QWidget

from .lunahost import HookSource
from .translators import ENGINE_GOOGLE, TranslateResult, TranslatorManager

# (имя, реплика, перевод имени, перевод реплики) — собственный текст для демонстрации
SCRIPT = [
    ("", "朝の境内は静かで、空気がひんやりと澄んでいた。", "", "Утром на территории храма было тихо, а воздух был прохладным и прозрачным."),
    ("芳乃", "「おはようございます。今日も早いですね」", "Ёсино", "«Доброе утро. Вы сегодня снова рано»."),
    ("将臣", "「ああ、おはよう。朝の掃除を手伝おうと思って」", "Масаоми", "«А, доброе утро. Я подумал, что помогу с утренней уборкой»."),
    ("芳乃", "「ふふ、ありがとうございます。それでは、箒をお願いしますね」", "Ёсино", "«Хе-хе, спасибо большое. Тогда возьмите, пожалуйста, метлу»."),
    ("", "彼女は小さく微笑んで、箒を差し出した。", "", "Она слегка улыбнулась и протянула метлу."),
    ("茉子", "「お二人とも、朝ごはんの用意ができましたよ」", "Мако", "«Вы оба, завтрак уже готов»."),
    ("将臣", "「もうそんな時間か。すぐ行くよ」", "Масаоми", "«Уже так поздно? Сейчас приду»."),
    ("芳乃", "「今日の朝ごはんは何でしょう。楽しみです」", "Ёсино", "«Интересно, что сегодня на завтрак. Жду с нетерпением»."),
]

DEMO_PID = os.getpid()
_TEXT_HOOK = ("HQ-1C*0@0:kirikiri.exe", "KiriKiriZ", 0x7FF6A0012340, 0x7FF6A0AB0010, 0x1)
_NAME_HOOK = ("HQ-1C*0@0:kirikiri.exe", "KiriKiriZ", 0x7FF6A0012340, 0x7FF6A0AB0020, 0x2)
_FILE_HOOK = ("HS-8@0:kernel32.dll:CreateFileW", "KiriKiriZ_File", 0x7FF6A0055000, 0x7FF6A0CC0030, 0x0)
_GLYPH_HOOK = ("HQ8@0:gdi32.dll:GetGlyphOutlineW", "GetGlyphOutlineW", 0x7FFB10020000, 0x7FF6A0DD0040, 0x0)
_TITLE_HOOK = ("HQ-C@0:gdi32.dll:TextOutW", "TextOutW", 0x7FFB10030000, 0x7FF6A0EE0050, 0x0)
_FILES = ["bg_keidai_morning.png", "voice\\yos_0012.ogg", "scenario\\001_prologue.ks", "se_broom_01.ogg", "bgm_03.ogg"]


def _info(hook) -> dict:
    code, name, addr, ctx, ctx2 = hook
    return {"code": code, "name": name, "pid": DEMO_PID, "addr": addr, "ctx": ctx, "ctx2": ctx2}


class DemoHookSource(HookSource):
    """Выдаёт те же сигналы, что и LunaHost: реплики, имена и разный мусор."""

    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self._attached = False

    def attach(self, pid: int, codepage: int) -> None:
        QTimer.singleShot(500, self._connect)

    def _connect(self) -> None:
        self._attached = True
        self.processConnected.emit(DEMO_PID)
        self.hostInfo.emit(4, "KiriKiriZ")
        self.textOutput.emit(_info(_TITLE_HOOK), "Senren＊Banka")

    def detach(self, pid: int) -> None:
        if self._attached:
            self._attached = False
            self.processDisconnected.emit(DEMO_PID)

    def emit_line(self, index: int) -> None:
        """Игра показала реплику: шлём её так, как это сделали бы хуки."""
        if not self._attached:
            return
        name, text, _, _ = SCRIPT[index % len(SCRIPT)]
        # движок: имя в 【】, теги KiriKiri и перенос посередине
        half = len(text) // 2
        raw = (f"【{name}】" if name else "") + "%f$ll;" + text[:half] + "[r]" + text[half:] + "[p]"
        self.textOutput.emit(_info(_TEXT_HOOK), raw)
        if name:
            self.textOutput.emit(_info(_NAME_HOOK), name)
        self.textOutput.emit(_info(_FILE_HOOK), _FILES[index % len(_FILES)])
        self.textOutput.emit(_info(_TITLE_HOOK), "Senren＊Banka")
        # хук рисования: по одному символу, много строк в секунду
        for i, ch in enumerate(text[:14]):
            QTimer.singleShot(20 * i, lambda c=ch: self._attached and self.textOutput.emit(_info(_GLYPH_HOOK), c + c))


class DemoTranslatorManager(TranslatorManager):
    """Для демо: готовые переводы реплик сценария, остальное — обычным путём."""

    def __init__(self, *args, **kwargs) -> None:
        super().__init__(*args, **kwargs)
        self._table: dict[str, str] = {}
        for name, text, name_ru, text_ru in SCRIPT:
            self._table[text] = text_ru
            if name:
                self._table[name] = name_ru

    def translate(self, text, src_lang, context="", use_cache=True) -> TranslateResult:
        if text in self._table:
            return TranslateResult(self._table[text], ENGINE_GOOGLE, cached=True)
        return super().translate(text, src_lang, context, use_cache)


class DemoGameWindow(QWidget):
    """Окно «игры»: по щелчку показывает следующую реплику."""

    lineShown = Signal(int)

    def __init__(self) -> None:
        super().__init__()
        self.setWindowTitle("Senren＊Banka (демо NovelTL)")
        self.setFixedSize(1280, 720)
        self.index = -1
        self.setCursor(Qt.PointingHandCursor)

    def advance(self) -> None:
        self.index += 1
        self.update()
        self.lineShown.emit(self.index)

    def mousePressEvent(self, event) -> None:
        self.advance()

    def keyPressEvent(self, event) -> None:
        if event.key() in (Qt.Key_Space, Qt.Key_Return, Qt.Key_Enter):
            self.advance()

    def paintEvent(self, event) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        w, h = self.width(), self.height()
        sky = QLinearGradient(0, 0, 0, h)
        sky.setColorAt(0.0, QColor("#f6c7a8"))
        sky.setColorAt(0.45, QColor("#e79aa5"))
        sky.setColorAt(1.0, QColor("#5b4a7a"))
        p.fillRect(self.rect(), sky)
        # условные «тории» и холмы
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(60, 45, 85, 150))
        p.drawEllipse(QRectF(-200, h * 0.55, w * 0.9, h * 0.8))
        p.drawEllipse(QRectF(w * 0.45, h * 0.6, w * 0.9, h * 0.8))
        p.setBrush(QColor("#b5373f"))
        gx, gy = w * 0.5 - 150, h * 0.22
        p.drawRoundedRect(QRectF(gx - 30, gy, 360, 22), 6, 6)
        p.drawRect(QRectF(gx, gy + 50, 300, 14))
        p.drawRect(QRectF(gx + 30, gy + 20, 22, 300))
        p.drawRect(QRectF(gx + 248, gy + 20, 22, 300))

        # диалоговое окно игры
        box = QRectF(60, h - 210, w - 120, 180)
        p.setBrush(QColor(20, 14, 34, 215))
        p.setPen(QPen(QColor(255, 255, 255, 60), 2))
        p.drawRoundedRect(box, 14, 14)
        if self.index < 0:
            p.setPen(QColor("#ffffff"))
            p.setFont(QFont("Segoe UI", 16))
            p.drawText(box, Qt.AlignCenter, "Это имитация игры. Щёлкайте здесь, чтобы листать реплики.")
            return
        name, text, _, _ = SCRIPT[self.index % len(SCRIPT)]
        if name:
            p.setPen(QColor("#ffd27f"))
            p.setFont(QFont("Yu Gothic UI", 17, QFont.Bold))
            p.drawText(box.adjusted(28, 14, -28, 0), Qt.AlignLeft | Qt.AlignTop, name)
        p.setPen(QColor("#ffffff"))
        p.setFont(QFont("Yu Gothic UI", 20))
        p.drawText(box.adjusted(28, 58, -28, -16), Qt.AlignLeft | Qt.AlignTop | Qt.TextWordWrap, text)
