"""Главное окно и значок в трее."""
from __future__ import annotations

import os
import subprocess

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QAction
from PySide6.QtWidgets import (
    QApplication, QFrame, QGridLayout, QHBoxLayout, QListWidget, QListWidgetItem, QMenu, QMessageBox,
    QStackedWidget, QSystemTrayIcon, QVBoxLayout, QWidget,
)

from .. import APP_NAME
from ..langdetect import LANG_TITLES
from ..translators import ENGINE_TITLES
from .dialogs import GlossaryDialog, SettingsDialog, button, hline, label
from . import theme
from .theme import app_icon, icon_pixmap
from .wizard import MODE_HOOK, MODE_NEW, MODE_POSITION, Wizard


def reveal_report(parent, controller) -> None:
    path = controller.save_report()
    try:
        subprocess.Popen(["explorer", "/select,", path])
    except OSError:
        pass
    QMessageBox.information(
        parent, "Отчёт сохранён",
        "Отчёт сохранён в файл:\n" + path + "\n\nПапка с ним открыта — пришлите этот файл для доработки.")


class MainWindow(QWidget):
    def __init__(self, controller, demo: bool = False) -> None:
        super().__init__()
        self.controller = controller
        self.demo = demo
        self.wizard: Wizard | None = None
        self._quitting = False
        self._tray_hint_shown = False
        self.setObjectName("root")
        self.setWindowTitle(APP_NAME + (" — демо-режим" if demo else ""))
        self.setWindowIcon(app_icon())
        self.resize(900, 600)
        self.setMinimumSize(780, 520)

        root = QHBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)
        root.addWidget(self._build_sidebar())
        self.pages = QStackedWidget()
        self.pages.addWidget(self._build_empty())
        self.pages.addWidget(self._build_game())
        root.addWidget(self.pages, 1)

        self._build_tray()
        controller.stateChanged.connect(self.refresh)
        controller.notice.connect(self.show_notice)
        self._notice_timer = QTimer(self)
        self._notice_timer.setSingleShot(True)
        self._notice_timer.timeout.connect(lambda: self.notice_label.setVisible(False))
        self.refresh()

    # ---------- построение ----------
    def _build_sidebar(self) -> QWidget:
        panel = QFrame()
        panel.setObjectName("panel")
        panel.setFixedWidth(264)
        lay = QVBoxLayout(panel)
        lay.setContentsMargins(16, 18, 16, 16)
        lay.setSpacing(10)
        head = QHBoxLayout()
        logo = label("")
        logo.setPixmap(icon_pixmap(32))
        head.addWidget(logo)
        head.addWidget(label(APP_NAME, "h1"))
        head.addStretch(1)
        lay.addLayout(head)
        lay.addSpacing(8)
        lay.addWidget(label("МОИ ИГРЫ", "step"))
        self.games = QListWidget()
        self.games.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self.games.setTextElideMode(Qt.ElideRight)
        self.games.itemSelectionChanged.connect(self._refresh_page)
        lay.addWidget(self.games, 1)
        lay.addWidget(button("＋  Добавить игру", "primary", self.add_game))
        lay.addWidget(button("Настройки", on_click=self.open_settings))
        return panel

    def _build_empty(self) -> QWidget:
        page = QWidget()
        lay = QVBoxLayout(page)
        lay.setContentsMargins(60, 40, 60, 40)
        lay.addStretch(1)
        icon = label("")
        icon.setPixmap(icon_pixmap(72))
        icon.setAlignment(Qt.AlignCenter)
        lay.addWidget(icon)
        lay.addSpacing(12)
        title = label("Добавьте первую игру", "h1")
        title.setAlignment(Qt.AlignCenter)
        lay.addWidget(title)
        text = label(
            "Запустите визуальную новеллу в оконном режиме и нажмите кнопку ниже. "
            "Настройка займёт минуту: выбрать окно, прокликать пару реплик и поставить плашку.",
            "dim", wrap=True)
        text.setAlignment(Qt.AlignCenter)
        lay.addWidget(text)
        lay.addSpacing(16)
        row = QHBoxLayout()
        row.addStretch(1)
        row.addWidget(button("＋  Добавить игру", "big", self.add_game))
        row.addStretch(1)
        lay.addLayout(row)
        lay.addStretch(2)
        return page

    def _build_game(self) -> QWidget:
        page = QWidget()
        lay = QVBoxLayout(page)
        lay.setContentsMargins(28, 24, 28, 18)
        lay.setSpacing(12)
        self.title_label = label("", "h1")
        self.sub_label = label("", "dim")
        lay.addWidget(self.title_label)
        lay.addWidget(self.sub_label)

        status = QFrame()
        status.setObjectName("card")
        sl = QHBoxLayout(status)
        sl.setContentsMargins(16, 12, 12, 12)
        self.dot = label("●")
        self.dot.setFixedWidth(18)
        self.dot.setAlignment(Qt.AlignTop)
        self.status_label = label("", wrap=True)
        self.retry_btn = button("Повторить", on_click=self._retry)
        sl.addWidget(self.dot)
        sl.addWidget(self.status_label, 1)
        sl.addWidget(self.retry_btn)
        lay.addWidget(status)

        last = QFrame()
        last.setObjectName("card")
        ll = QVBoxLayout(last)
        ll.setContentsMargins(16, 12, 16, 14)
        ll.setSpacing(6)
        ll.addWidget(label("ПОСЛЕДНЯЯ РЕПЛИКА", "step"))
        self.src_label = label("", "jp", wrap=True)
        self.src_label.setTextFormat(Qt.PlainText)
        self.tr_label = label("", "tr", wrap=True)
        self.tr_label.setTextFormat(Qt.PlainText)
        self.engine_label = label("", "small")
        ll.addWidget(self.src_label)
        ll.addWidget(self.tr_label)
        ll.addWidget(self.engine_label)
        ll.addStretch(1)
        lay.addWidget(last, 1)

        row = QGridLayout()
        row.setSpacing(8)
        self.hook_btn = button("Найти текст заново", on_click=lambda: self.open_wizard(MODE_HOOK))
        self.pos_btn = button("Расположение плашки", on_click=lambda: self.open_wizard(MODE_POSITION))
        self.gloss_btn = button("Глоссарий имён", on_click=self.open_glossary)
        self.report_btn = button("Отчёт для настройки", on_click=lambda: reveal_report(self, self.controller))
        for i, b in enumerate((self.hook_btn, self.pos_btn, self.gloss_btn, self.report_btn)):
            row.addWidget(b, i // 2, i % 2)
        lay.addLayout(row)

        lay.addWidget(hline())
        foot = QHBoxLayout()
        self.translator_label = label("", "small", wrap=True)
        foot.addWidget(self.translator_label, 1)
        foot.addWidget(button("Удалить игру", "link", self.delete_game))
        lay.addLayout(foot)
        self.notice_label = label("", "notice", wrap=True)
        self.notice_label.setVisible(False)
        lay.addWidget(self.notice_label)
        return page

    def _build_tray(self) -> None:
        self.tray = QSystemTrayIcon(app_icon(), self)
        self.tray.setToolTip(APP_NAME)
        menu = QMenu()
        s = self.controller.settings
        self.act_open = QAction("Открыть NovelTL", menu, triggered=self.show_window)
        self.act_toggle = QAction("", menu, triggered=self.controller.toggle_overlay)
        self.act_retr = QAction("", menu, triggered=self.controller.retranslate)
        self.act_settings = QAction("Настройки", menu, triggered=self.open_settings)
        self.act_quit = QAction("Выход", menu, triggered=self.quit)
        menu.addAction(self.act_open)
        menu.addSeparator()
        menu.addAction(self.act_toggle)
        menu.addAction(self.act_retr)
        menu.addSeparator()
        menu.addAction(self.act_settings)
        menu.addAction(self.act_quit)
        self._tray_menu = menu
        self.tray.setContextMenu(menu)
        self.tray.activated.connect(
            lambda reason: self.show_window() if reason in (QSystemTrayIcon.Trigger, QSystemTrayIcon.DoubleClick) else None)
        self.tray.show()
        del s

    # ---------- обновление ----------
    def _selected_profile_id(self) -> str | None:
        items = self.games.selectedItems()
        return items[0].data(Qt.UserRole) if items else None

    def refresh(self) -> None:
        c = self.controller
        s = c.session
        profiles = {p.id: p for p in c.store.all()}
        if s and s.saved:
            profiles[s.profile.id] = s.profile
        selected = self._selected_profile_id()
        if s and s.saved and getattr(self, "_last_session_id", None) != s.profile.id:
            selected = s.profile.id  # игра только что подключилась — показываем её
        self._last_session_id = s.profile.id if s else None
        if selected not in profiles:
            selected = next(iter(profiles), None)

        self.games.blockSignals(True)
        self.games.clear()
        for pid, p in profiles.items():
            running = bool(s and s.profile.id == pid)
            item = QListWidgetItem(("●  " if running else "○  ") + p.name)
            item.setData(Qt.UserRole, pid)
            item.setToolTip("Игра запущена" if running else "Игра не запущена")
            self.games.addItem(item)
            if pid == selected:
                item.setSelected(True)
                self.games.setCurrentItem(item)
        self.games.blockSignals(False)
        self._profiles = profiles
        self._refresh_page()

        st = c.settings
        hidden = c.overlay.user_hidden
        self.act_toggle.setText(("Показать плашку" if hidden else "Скрыть плашку") + f"\t{st.hotkey_toggle}")
        self.act_retr.setText(f"Перевести заново без кэша\t{st.hotkey_retranslate}")
        self.act_retr.setEnabled(bool(s))

    def _refresh_page(self) -> None:
        c = self.controller
        profile = getattr(self, "_profiles", {}).get(self._selected_profile_id())
        if profile is None:
            self.pages.setCurrentIndex(0)
            return
        self.pages.setCurrentIndex(1)
        s = c.session if c.session and c.session.profile.id == profile.id else None
        self.title_label.setText(profile.name)
        self.sub_label.setText(f"{LANG_TITLES.get(profile.lang, profile.lang)} → русский  ·  {profile.exe_name}")

        color, text, retry = theme.c("dim"), "Игра не запущена. Запустите её — NovelTL подключится сам.", False
        if s:
            if s.error:
                color, text, retry = theme.c("bad"), s.error, True
            elif not s.connected:
                color, text = theme.c("warn"), "Подключаюсь к игре…"
            elif s.setup:
                color, text = theme.c("warn"), "Идёт настройка — закончите её в окне мастера."
            elif s.selected is None:
                color, text = theme.c("warn"), "Подключено. Жду текст — прокликайте пару реплик в игре."
            else:
                color, text = theme.c("ok"), "Перевод работает."
                if c.overlay.user_hidden:
                    text += f" Плашка скрыта — {c.settings.hotkey_toggle}, чтобы показать."
        self.dot.setStyleSheet(f"color: {color};")
        self.status_label.setText(text)
        self.retry_btn.setVisible(retry)

        res = s.last_result if s else None
        if res:
            src = f"{res.src_name}　{res.src_text}" if res.src_name else res.src_text
            dst = f"{res.name}: {res.text}" if res.name else res.text
            self.src_label.setText(src)
            self.tr_label.setText(dst)
            self.engine_label.setText(ENGINE_TITLES.get(res.engine, res.engine) + (" · из кэша" if res.cached else ""))
        else:
            self.src_label.setText("Здесь появится текст из игры и его перевод." if s else "—")
            self.tr_label.setText("")
            self.engine_label.setText("")

        ready = bool(s and s.connected and not s.setup)
        self.hook_btn.setEnabled(ready)
        self.pos_btn.setEnabled(ready and s.selected is not None)
        self.report_btn.setEnabled(bool(s))
        for b in (self.hook_btn, self.pos_btn, self.report_btn):
            b.setToolTip("" if b.isEnabled() else "Сначала запустите игру")
        keys = f"{c.settings.hotkey_toggle} — скрыть плашку, {c.settings.hotkey_retranslate} — перевести заново"
        self.translator_label.setText(f"Переводчик: {c.manager.status_text()}\n{keys}")

    def show_notice(self, text: str) -> None:
        self.notice_label.setText(text)
        self.notice_label.setVisible(True)
        self._notice_timer.start(15000)
        if not self.isVisible() or self.isMinimized():
            self.tray.showMessage(APP_NAME, text, app_icon(), 6000)
        self.refresh()

    # ---------- действия ----------
    def show_window(self) -> None:
        self.showNormal()
        self.raise_()
        self.activateWindow()

    def add_game(self) -> None:
        self.open_wizard(MODE_NEW)

    def open_wizard(self, mode: str) -> Wizard | None:
        if self.wizard is not None:
            self.wizard.raise_()
            self.wizard.activateWindow()
            return self.wizard
        self.wizard = Wizard(self.controller, mode)
        self.wizard.finished.connect(self._wizard_closed)
        self.wizard.show()
        return self.wizard

    def _wizard_closed(self) -> None:
        if self.wizard is not None:
            self.wizard.deleteLater()
            self.wizard = None
        self.refresh()

    def open_settings(self) -> None:
        self.show_window()
        SettingsDialog(self.controller, self).exec()

    def open_glossary(self) -> None:
        profile = self._profiles.get(self._selected_profile_id())
        if profile:
            s = self.controller.session
            if s and s.profile.id == profile.id:
                profile = s.profile
            GlossaryDialog(self.controller, profile, self).exec()

    def delete_game(self) -> None:
        profile = self._profiles.get(self._selected_profile_id())
        if not profile:
            return
        answer = QMessageBox.question(
            self, "Удалить игру",
            f"Удалить «{profile.name}» из NovelTL?\nПрофиль, глоссарий и кэш переводов будут удалены. Сама игра не затрагивается.")
        if answer == QMessageBox.Yes:
            self.controller.delete_profile(profile.id)

    def _retry(self) -> None:
        s = self.controller.session
        if s:
            win = next((w for w in self.controller.windows() if w.pid == s.pid), None)
            if win:
                self.controller.start_session(s.profile, win, saved=s.saved)

    def quit(self) -> None:
        self._quitting = True
        self.tray.hide()
        if self.wizard is not None:
            self.wizard.reject()
        self.controller.shutdown()
        QApplication.instance().quit()

    def closeEvent(self, event) -> None:
        if self._quitting:
            event.accept()
            return
        event.ignore()
        self.hide()
        if not self._tray_hint_shown:
            self._tray_hint_shown = True
            self.tray.showMessage(
                APP_NAME, "NovelTL продолжает работать в трее. Выход — через меню значка.", app_icon(), 5000)
