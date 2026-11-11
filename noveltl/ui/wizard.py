"""Мастер добавления игры: окно → поиск текста → расположение плашки."""
from __future__ import annotations

from PySide6.QtCore import Qt, QTimer
from PySide6.QtWidgets import (
    QComboBox, QDialog, QFrame, QHBoxLayout, QListWidget, QListWidgetItem, QProgressBar,
    QScrollArea, QStackedWidget, QVBoxLayout, QWidget,
)

from ..langdetect import LANG_AUTO, LANG_EN, LANG_JA, LANG_TITLES, LANG_ZH
from ..profiles import DEFAULT_OVERLAY
from ..scorer import CONFIDENT_DISTINCT, is_confident
from .dialogs import button, label
from . import theme
from .theme import app_icon

MODE_NEW, MODE_HOOK, MODE_POSITION = "new", "hook", "position"


def _elide(text: str, limit: int = 70) -> str:
    return text if len(text) <= limit else text[: limit - 1] + "…"


class Wizard(QDialog):
    def __init__(self, controller, mode: str = MODE_NEW, parent=None) -> None:
        super().__init__(parent)
        self.controller = controller
        self.mode = mode
        self._finished = False
        self._best_key = None
        self._rows: dict = {}
        self._windows = []
        self.setWindowTitle("Добавление игры — NovelTL" if mode == MODE_NEW else "Настройка игры — NovelTL")
        self.setWindowIcon(app_icon())
        self.setWindowFlag(Qt.WindowStaysOnTopHint, True)
        self.setWindowFlag(Qt.WindowContextHelpButtonHint, False)
        self.resize(580, 720)

        root = QVBoxLayout(self)
        root.setContentsMargins(28, 24, 28, 22)
        self.stack = QStackedWidget()
        root.addWidget(self.stack)
        self.stack.addWidget(self._build_step1())
        self.stack.addWidget(self._build_step2())
        self.stack.addWidget(self._build_step3())

        controller.threadsChanged.connect(self._refresh_step2)
        controller.stateChanged.connect(self._refresh_step2)

        if mode == MODE_NEW:
            self._reload_windows()
            self._win_timer = QTimer(self)
            self._win_timer.setInterval(2000)
            self._win_timer.timeout.connect(self._reload_windows)
            self._win_timer.start()
        elif mode == MODE_HOOK:
            controller.resume_setup()
            self._go(1)
        else:
            controller.resume_setup()
            self._go(2)

    # ---------- общий каркас шага ----------
    def _page(self, step: int, title: str, text: str) -> tuple[QWidget, QVBoxLayout]:
        page = QWidget()
        lay = QVBoxLayout(page)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(12)
        lay.addWidget(label(f"ШАГ {step} ИЗ 3", "step"))
        lay.addWidget(label(title, "h1"))
        lay.addWidget(label(text, "dim", wrap=True))
        lay.addSpacing(4)
        return page, lay

    def _go(self, index: int) -> None:
        self.stack.setCurrentIndex(index)
        self.controller.overlay.set_edit_mode(index == 2)
        if index == 1:
            self._refresh_step2()

    # ---------- шаг 1: окно игры ----------
    def _build_step1(self) -> QWidget:
        page, lay = self._page(
            1, "Выберите игру",
            "Запустите игру в оконном режиме и выберите её в списке. Браузеры и системные окна скрыты.")
        self.win_list = QListWidget()
        self.win_list.setObjectName("wins")
        self.win_list.itemSelectionChanged.connect(self._on_window_selected)
        self.win_list.itemDoubleClicked.connect(lambda _: self._start_search())
        lay.addWidget(self.win_list, 1)
        self.no_windows = label("Не вижу запущенных игр. Запустите игру — она появится здесь сама.", "dim", wrap=True)
        lay.addWidget(self.no_windows)

        row = QHBoxLayout()
        row.addWidget(label("Язык игры"))
        self.lang_box = QComboBox()
        for code in (LANG_JA, LANG_ZH, LANG_EN, LANG_AUTO):
            self.lang_box.addItem(LANG_TITLES[code] + (" (определять по тексту)" if code == LANG_AUTO else ""), code)
        self.lang_box.setMinimumWidth(240)
        row.addWidget(self.lang_box)
        row.addStretch(1)
        lay.addLayout(row)
        lay.addWidget(label("Перевод всегда на русский.", "small"))

        foot = QHBoxLayout()
        foot.addWidget(button("Отмена", on_click=self.reject))
        foot.addStretch(1)
        self.next1 = button("Далее  →", "primary", self._start_search)
        self.next1.setEnabled(False)
        foot.addWidget(self.next1)
        lay.addLayout(foot)
        return page

    def _reload_windows(self) -> None:
        if self.stack.currentIndex() != 0:
            return
        wins = self.controller.windows()
        if [w.hwnd for w in wins] == [w.hwnd for w in self._windows]:
            return
        selected = self._selected_window()
        self._windows = wins
        self.win_list.clear()
        for w in wins:
            item = QListWidgetItem(f"{w.title}\n{w.exe_name}")
            item.setData(Qt.UserRole, w.hwnd)
            self.win_list.addItem(item)
            if selected and selected.hwnd == w.hwnd:
                item.setSelected(True)
        self.no_windows.setVisible(not wins)
        if len(wins) == 1:
            self.win_list.setCurrentRow(0)
        self._on_window_selected()

    def _selected_window(self):
        items = self.win_list.selectedItems()
        if not items:
            return None
        hwnd = items[0].data(Qt.UserRole)
        return next((w for w in self._windows if w.hwnd == hwnd), None)

    def _on_window_selected(self) -> None:
        self.next1.setEnabled(self._selected_window() is not None)

    def _start_search(self) -> None:
        win = self._selected_window()
        if not win:
            return
        self.controller.begin_setup(win, self.lang_box.currentData(), name=win.title)
        self._go(1)

    # ---------- шаг 2: поиск текста ----------
    def _build_step2(self) -> QWidget:
        page, lay = self._page(
            2, "Поиск текста",
            "Перейдите в игру и прокликайте 3–4 реплики. NovelTL сам найдёт, откуда брать текст, "
            "и покажет его здесь.")
        self.conn_label = label("", "dim", wrap=True)
        lay.addWidget(self.conn_label)
        self.progress = QProgressBar()
        self.progress.setRange(0, CONFIDENT_DISTINCT)
        self.progress.setTextVisible(False)
        lay.addWidget(self.progress)

        self.best = QFrame()
        self.best.setObjectName("best")
        bl = QVBoxLayout(self.best)
        bl.setContentsMargins(18, 14, 18, 16)
        bl.setSpacing(8)
        self.best_caption = label("НАЙДЕННЫЙ ТЕКСТ", "step")
        bl.addWidget(self.best_caption)
        self.best_lines = label("", "found", wrap=True)
        self.best_lines.setMinimumHeight(96)
        self.best_lines.setAlignment(Qt.AlignTop | Qt.AlignLeft)
        self.best_lines.setTextFormat(Qt.PlainText)
        bl.addWidget(self.best_lines)
        self.best_hint = label("", "small", wrap=True)
        bl.addWidget(self.best_hint)
        self.confirm = button("Это текст диалога", "big", self._confirm_best)
        bl.addWidget(self.confirm)
        lay.addWidget(self.best)

        self.spoiler = button("", "link", self._toggle_others)
        lay.addWidget(self.spoiler)
        self.others_area = QScrollArea()
        self.others_area.setWidgetResizable(True)
        self.others_area.setVisible(False)
        holder = QWidget()
        self.others_lay = QVBoxLayout(holder)
        self.others_lay.setContentsMargins(0, 0, 6, 0)
        self.others_lay.setSpacing(6)
        self.others_lay.addStretch(1)
        self.others_area.setWidget(holder)
        lay.addWidget(self.others_area, 1)
        self._stretch2 = QWidget()
        lay.addWidget(self._stretch2, 1)

        foot = QHBoxLayout()
        self.back2 = button("←  Назад", on_click=self._back_from_search)
        foot.addWidget(self.back2)
        foot.addStretch(1)
        foot.addWidget(button("Текст не находится? Сохранить отчёт", "link", self._save_report))
        lay.addLayout(foot)
        return page

    def _back_from_search(self) -> None:
        if self.mode == MODE_NEW:
            self.controller.cancel_setup()
            self._go(0)
            self._reload_windows()
        else:
            self.reject()

    def _toggle_others(self) -> None:
        show = not self.others_area.isVisible()
        self.others_area.setVisible(show)
        self._stretch2.setVisible(not show)
        self._rebuild_others()
        self._refresh_step2()

    def _rebuild_others(self) -> None:
        for row in self._rows.values():
            row.setParent(None)
            row.deleteLater()
        self._rows.clear()

    def _other_row(self, result) -> QFrame:
        row = QFrame()
        row.setObjectName("card")
        hl = QHBoxLayout(row)
        hl.setContentsMargins(12, 8, 8, 8)
        col = QVBoxLayout()
        col.setSpacing(2)
        row.text_label = label("", "jp")
        row.text_label.setTextFormat(Qt.PlainText)
        row.name_label = label("", "small")
        col.addWidget(row.text_label)
        col.addWidget(row.name_label)
        hl.addLayout(col, 1)
        hl.addWidget(button("Выбрать", on_click=lambda _=False, k=result.key: self._confirm(k)))
        return row

    def _refresh_step2(self) -> None:
        if self.stack.currentIndex() != 1:
            return
        s = self.controller.session
        if not s:
            self.conn_label.setText("Игра закрыта. Запустите её и начните заново.")
            self.conn_label.setStyleSheet(f"color: {theme.c('bad')};")
            self.confirm.setEnabled(False)
            return
        ranked = self.controller.tracker.ranked()
        best = ranked[0] if ranked else None
        self._best_key = best.key if best else None

        if s.error:
            self.conn_label.setText(s.error)
            self.conn_label.setStyleSheet(f"color: {theme.c('bad')};")
        elif not s.connected:
            self.conn_label.setText("Подключаюсь к игре…")
            self.conn_label.setStyleSheet(f"color: {theme.c('warn')};")
        elif not best:
            self.conn_label.setText("Подключено. Жду текст — кликайте в игре.")
            self.conn_label.setStyleSheet(f"color: {theme.c('warn')};")
        elif is_confident(ranked):
            self.conn_label.setText("Текст найден. Проверьте, что ниже — реплики из игры.")
            self.conn_label.setStyleSheet(f"color: {theme.c('ok')};")
        else:
            self.conn_label.setText(
                f"Поймано реплик: {min(best.distinct, CONFIDENT_DISTINCT)} из {CONFIDENT_DISTINCT}. Кликайте дальше.")
            self.conn_label.setStyleSheet(f"color: {theme.c('warn')};")
        self.progress.setValue(min(best.distinct, CONFIDENT_DISTINCT) if best else 0)

        if best:
            self.best_lines.setText("\n".join(_elide(t, 90) for t in best.stats.last_clean(3)))
            engine = f" · движок {s.engine}" if s.engine else ""
            self.best_hint.setText(f"Источник: {best.stats.hookname or 'хук'}{engine} · оценка {best.score:.0f}")
        else:
            self.best_lines.setText("Пока пусто.\nЗдесь появятся реплики, как только вы начнёте кликать в игре.")
            self.best_hint.setText("")
        self.confirm.setEnabled(best is not None)

        others = ranked[1:]
        arrow = "▾" if self.others_area.isVisible() else "▸"
        self.spoiler.setText(f"{arrow}  Это не то? Другие варианты ({len(others)})")
        self.spoiler.setVisible(bool(others) or self.others_area.isVisible())
        if self.others_area.isVisible():
            for r in others:
                row = self._rows.get(r.key)
                if row is None:
                    row = self._other_row(r)
                    self._rows[r.key] = row
                    self.others_lay.insertWidget(self.others_lay.count() - 1, row)
                lines = r.stats.last_clean(1)
                row.text_label.setText(_elide(lines[-1] if lines else "", 46))
                row.name_label.setText(f"{r.stats.hookname or 'хук'} · оценка {r.score:.0f}")
                row.setVisible(True)
            if self._best_key in self._rows:
                self._rows[self._best_key].setVisible(False)

    def _confirm_best(self) -> None:
        if self._best_key is not None:
            self._confirm(self._best_key)

    def _confirm(self, key) -> None:
        self.controller.choose_hook(key)
        self._go(2)

    def _save_report(self) -> None:
        from .main_window import reveal_report

        reveal_report(self, self.controller)

    # ---------- шаг 3: плашка ----------
    def _build_step3(self) -> QWidget:
        page, lay = self._page(
            3, "Расположение плашки",
            "Поверх игры появилась плашка с примером перевода. Перетащите её мышью на окно диалога "
            "и растяните за края так, чтобы она закрыла оригинальный текст.")
        card = QFrame()
        card.setObjectName("card")
        cl = QVBoxLayout(card)
        cl.setContentsMargins(18, 14, 18, 14)
        cl.setSpacing(8)
        for tip in (
            "•  Тяните за середину — плашка двигается, за края и углы — меняет размер.",
            "•  Если окно игры изменит размер, плашка подстроится сама.",
            "•  Шрифт, цвета и прозрачность меняются в «Настройках».",
            "•  Не видно плашку? Разверните окно игры — плашка рисуется только поверх него.",
        ):
            cl.addWidget(label(tip, wrap=True))
        lay.addWidget(card)
        lay.addWidget(button("Вернуть положение по умолчанию", "link", self._reset_position))
        lay.addStretch(1)
        foot = QHBoxLayout()
        self.back3 = button("←  Назад", on_click=lambda: self._go(1))
        self.back3.setVisible(self.mode != MODE_POSITION)
        foot.addWidget(self.back3)
        if self.mode == MODE_POSITION:
            foot.addWidget(button("Отмена", on_click=self.reject))
        foot.addStretch(1)
        foot.addWidget(button("Готово", "big", self._finish))
        lay.addLayout(foot)
        return page

    def _reset_position(self) -> None:
        self.controller.overlay.frac = dict(DEFAULT_OVERLAY)
        self.controller.overlay.sync()

    def _finish(self) -> None:
        self._finished = True
        self.controller.finish_setup()
        self.accept()

    def done(self, result: int) -> None:
        if not self._finished:
            self.controller.cancel_setup()
        self.controller.overlay.set_edit_mode(False)
        super().done(result)
