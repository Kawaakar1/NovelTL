"""Окна настроек и глоссария."""
from __future__ import annotations

import threading
from dataclasses import asdict

from PySide6.QtCore import QObject, Qt, Signal
from PySide6.QtGui import QColor, QFont, QKeySequence, QLinearGradient, QPainter
from PySide6.QtWidgets import (
    QApplication, QColorDialog, QComboBox, QDialog, QFontComboBox, QFormLayout, QFrame, QHBoxLayout, QHeaderView,
    QKeySequenceEdit, QLabel, QLineEdit, QMessageBox, QPushButton, QScrollArea, QSlider,
    QTableWidget, QTableWidgetItem, QVBoxLayout, QWidget,
)

from .. import net
from ..hotkeys import parse_hotkey
from ..profiles import AppSettings, Profile
from .overlay import Overlay
from . import theme
from .theme import app_icon


def label(text: str, name: str = "", wrap: bool = False) -> QLabel:
    lb = QLabel(text)
    if name:
        lb.setObjectName(name)
    lb.setWordWrap(wrap)
    return lb


def button(text: str, name: str = "", on_click=None) -> QPushButton:
    b = QPushButton(text)
    if name:
        b.setObjectName(name)
    b.setCursor(Qt.PointingHandCursor)
    if on_click:
        b.clicked.connect(on_click)
    return b


def hline() -> QFrame:
    f = QFrame()
    f.setObjectName("line")
    return f


class _Plate(QWidget):
    """Плашка для предпросмотра: рисуется тем же кодом, что и настоящая."""

    _fit = Overlay._fit
    _draw_outlined = Overlay._draw_outlined
    paintEvent = Overlay.paintEvent

    def __init__(self, settings: AppSettings, parent=None) -> None:
        super().__init__(parent)
        self.settings = settings
        self.name, self.text, self.note, self.edit_mode = "Ёсино", "«Доброе утро. Вы сегодня снова рано».", "", False


class PlatePreview(QWidget):
    def __init__(self, settings: AppSettings) -> None:
        super().__init__()
        self.setFixedHeight(120)
        self.plate = _Plate(settings, self)

    def resizeEvent(self, event) -> None:
        self.plate.setGeometry(16, 14, self.width() - 32, self.height() - 28)

    def paintEvent(self, event) -> None:
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        grad = QLinearGradient(0, 0, self.width(), self.height())
        grad.setColorAt(0, QColor("#f6c7a8"))
        grad.setColorAt(0.5, QColor("#e79aa5"))
        grad.setColorAt(1, QColor("#5b4a7a"))
        p.setBrush(grad)
        p.setPen(Qt.NoPen)
        p.drawRoundedRect(self.rect(), 10, 10)


class _CheckRelay(QObject):
    """Передаёт результат проверки связи из фонового потока в GUI-поток."""

    finished = Signal(object)


class SettingsDialog(QDialog):
    COLORS = (
        ("text_color", "Цвет текста"),
        ("name_color", "Цвет имени"),
        ("outline_color", "Цвет контура"),
        ("bg_color", "Цвет фона"),
    )

    def __init__(self, controller, parent=None) -> None:
        super().__init__(parent)
        self.controller = controller
        self.settings: AppSettings = controller.settings
        self._backup = asdict(self.settings)
        self._saved = False
        self.setWindowTitle("Настройки — NovelTL")
        self.setWindowIcon(app_icon())
        self.setMinimumWidth(520)

        # содержимое прокручивается, кнопки «Отмена/Сохранить» всегда видны внизу
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)
        self._scroll = QScrollArea()
        self._scroll.setWidgetResizable(True)
        self._scroll.setFrameShape(QFrame.NoFrame)
        self._scroll.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        content = QWidget()
        self._scroll.setWidget(content)
        outer.addWidget(self._scroll, 1)
        root = QVBoxLayout(content)
        root.setContentsMargins(24, 20, 24, 8)
        root.setSpacing(12)

        # --- тема ---
        theme_row = QHBoxLayout()
        theme_row.addWidget(label("Тема оформления", "h2"))
        theme_row.addStretch(1)
        self.theme_box = QComboBox()
        for code, title in theme.THEME_TITLES.items():
            self.theme_box.addItem(title, code)
        self.theme_box.setCurrentIndex(max(0, self.theme_box.findData(self.settings.theme)))
        self.theme_box.setMinimumWidth(180)
        self.theme_box.currentIndexChanged.connect(self._on_theme)
        theme_row.addWidget(self.theme_box)
        root.addLayout(theme_row)
        root.addWidget(hline())

        # --- переводчик ---
        root.addWidget(label("Переводчик", "h2"))
        row = QHBoxLayout()
        self.key_edit = QLineEdit(self.settings.deepl_key)
        self.key_edit.setEchoMode(QLineEdit.Password)
        self.key_edit.setPlaceholderText("Ключ DeepL API (необязательно)")
        show = button("Показать")
        show.setCheckable(True)
        show.toggled.connect(lambda on: self.key_edit.setEchoMode(QLineEdit.Normal if on else QLineEdit.Password))
        row.addWidget(self.key_edit, 1)
        row.addWidget(show)
        root.addLayout(row)
        root.addWidget(label(
            "С ключом переводит DeepL (ключ на «:fx» — бесплатный тариф). Без ключа или при ошибке "
            "DeepL перевод идёт через Google. Сейчас: " + controller.manager.status_text() + ".",
            "small", wrap=True))
        root.addWidget(hline())

        # --- сеть ---
        root.addWidget(label("Сеть", "h2"))
        net_form = QFormLayout()
        net_form.setHorizontalSpacing(18)
        net_form.setVerticalSpacing(10)
        self.net_box = QComboBox()
        for code, title in net.NET_TITLES.items():
            self.net_box.addItem(title, code)
        self.net_box.setCurrentIndex(max(0, self.net_box.findData(self.settings.net_mode)))
        self.net_box.currentIndexChanged.connect(self._on_net_mode)
        net_form.addRow("Подключение", self.net_box)
        init_addr, init_user, init_pass = self.settings.proxy_url, self.settings.proxy_user, self.settings.proxy_pass
        if not init_user:
            init_addr, u, p = net.split_proxy_credentials(init_addr)
            if u:
                init_user, init_pass = u, p
        self.proxy_edit = QLineEdit(init_addr)
        self.proxy_edit.setPlaceholderText("socks5://127.0.0.1:1080 или http://127.0.0.1:8080")
        net_form.addRow("Адрес прокси", self.proxy_edit)
        self.proxy_user_edit = QLineEdit(init_user)
        self.proxy_user_edit.setPlaceholderText("если прокси требует")
        net_form.addRow("Логин", self.proxy_user_edit)
        self.proxy_pass_edit = QLineEdit(init_pass)
        self.proxy_pass_edit.setEchoMode(QLineEdit.Password)
        self.proxy_pass_edit.setPlaceholderText("если прокси требует")
        self.proxy_pass_show = button("Показать")
        self.proxy_pass_show.setCheckable(True)
        self.proxy_pass_show.toggled.connect(
            lambda on: self.proxy_pass_edit.setEchoMode(QLineEdit.Normal if on else QLineEdit.Password))
        pass_row = QHBoxLayout()
        pass_row.addWidget(self.proxy_pass_edit, 1)
        pass_row.addWidget(self.proxy_pass_show)
        net_form.addRow("Пароль", pass_row)
        self.iface_box = QComboBox()
        self.iface_box.addItem("Автоматически", "")
        ifaces = net.list_interfaces()
        for name, ip in ifaces:
            self.iface_box.addItem(f"{name} — {ip}", ip)
        saved_ip = self.settings.bind_ip
        if saved_ip and saved_ip not in [ip for _, ip in ifaces]:
            self.iface_box.addItem(f"{saved_ip} (сейчас недоступен)", saved_ip)
        self.iface_box.setCurrentIndex(max(0, self.iface_box.findData(saved_ip)))
        net_form.addRow("Сетевой интерфейс", self.iface_box)
        root.addLayout(net_form)
        root.addWidget(label(
            "Если Google пишет «код 429», выберите другой интерфейс (например, обычное подключение вместо VPN) "
            "или прокси. Настройка влияет только на перевод. Логин и пароль нужны, только если их требует прокси.",
            "small", wrap=True))
        check_row = QHBoxLayout()
        self.check_btn = button("Проверить связь с Google", on_click=self._check_net)
        self.check_result = label("", wrap=True)
        check_row.addWidget(self.check_btn)
        check_row.addWidget(self.check_result, 1)
        root.addLayout(check_row)
        self._check_relay = _CheckRelay(self)
        self._check_relay.finished.connect(self._on_check_done)
        self._on_net_mode()
        root.addWidget(hline())

        # --- внешний вид ---
        root.addWidget(label("Плашка с переводом", "h2"))
        self.preview = PlatePreview(self.settings)
        root.addWidget(self.preview)
        form = QFormLayout()
        form.setHorizontalSpacing(18)
        form.setVerticalSpacing(10)
        self.font_box = QFontComboBox()
        self.font_box.setCurrentFont(QFont(self.settings.font_family))
        self.font_box.currentFontChanged.connect(lambda f: self._set("font_family", f.family()))
        form.addRow("Шрифт", self.font_box)
        size_row = QHBoxLayout()
        self.size_box = QSlider(Qt.Horizontal)
        self.size_box.setRange(14, 60)
        self.size_box.setValue(self.settings.font_size)
        self.size_label = label(f"{self.settings.font_size} px", "dim")
        self.size_label.setFixedWidth(44)
        self.size_box.valueChanged.connect(self._on_size)
        size_row.addWidget(self.size_box, 1)
        size_row.addWidget(self.size_label)
        form.addRow("Размер текста", size_row)

        colors = QHBoxLayout()
        colors.setSpacing(14)
        self._swatches = {}
        for attr, title in self.COLORS:
            col = QVBoxLayout()
            col.setSpacing(4)
            sw = button("", "swatch", lambda _=False, a=attr: self._pick_color(a))
            sw.setToolTip(title)
            self._swatches[attr] = sw
            col.addWidget(sw)
            col.addWidget(label(title.replace("Цвет ", "").capitalize(), "small"))
            colors.addLayout(col)
        colors.addStretch(1)
        form.addRow("Цвета", colors)
        self._paint_swatches()

        op_row = QHBoxLayout()
        self.opacity = QSlider(Qt.Horizontal)
        self.opacity.setRange(0, 100)
        self.opacity.setValue(self.settings.bg_opacity)
        self.op_label = label(f"{self.settings.bg_opacity}%", "dim")
        self.op_label.setFixedWidth(44)
        self.opacity.valueChanged.connect(self._on_opacity)
        op_row.addWidget(self.opacity, 1)
        op_row.addWidget(self.op_label)
        form.addRow("Непрозрачность фона", op_row)
        root.addLayout(form)
        root.addWidget(hline())

        # --- горячие клавиши ---
        root.addWidget(label("Горячие клавиши", "h2"))
        keys = QFormLayout()
        keys.setHorizontalSpacing(18)
        self.hk_toggle = self._key_edit(self.settings.hotkey_toggle)
        self.hk_retr = self._key_edit(self.settings.hotkey_retranslate)
        keys.addRow("Скрыть / показать плашку", self.hk_toggle)
        keys.addRow("Перевести заново без кэша", self.hk_retr)
        root.addLayout(keys)
        root.addWidget(label("Щёлкните по полю и нажмите новое сочетание. Работают в любой программе.", "small"))

        root.addStretch(1)
        buttons = QHBoxLayout()
        buttons.setContentsMargins(24, 10, 24, 18)
        buttons.addStretch(1)
        buttons.addWidget(button("Отмена", on_click=self.reject))
        buttons.addWidget(button("Сохранить", "primary", self._save))
        outer.addWidget(hline())
        outer.addLayout(buttons)

        # высота — по содержимому, но не больше экрана
        want = content.sizeHint().height() + 80
        screen = (parent.screen() if parent is not None else None) or QApplication.primaryScreen()
        limit = screen.availableGeometry().height() - 60 if screen else want
        self.resize(560, max(420, min(want, limit)))

    def _on_net_mode(self) -> None:
        on = self.net_box.currentData() == net.NET_PROXY
        self.proxy_edit.setEnabled(on)
        self.proxy_user_edit.setEnabled(on)
        self.proxy_pass_edit.setEnabled(on)
        self.proxy_pass_show.setEnabled(on)

    def _check_net(self) -> None:
        try:
            session = net.make_session(
                self.net_box.currentData(), self.proxy_edit.text(), self.iface_box.currentData() or "",
                self.proxy_user_edit.text(), self.proxy_pass_edit.text())
        except ValueError as e:
            self.check_result.setStyleSheet(f"color: {theme.c('bad')};")
            self.check_result.setText(str(e))
            return
        self.check_btn.setEnabled(False)
        self.check_result.setStyleSheet("")
        self.check_result.setText("Проверяю…")
        relay = self._check_relay

        def work() -> None:
            result = net.check_google(session)
            try:
                relay.finished.emit(result)
            except RuntimeError:  # диалог уже закрыт
                pass

        threading.Thread(target=work, daemon=True).start()

    def _on_check_done(self, result) -> None:
        try:
            ok, text = result
            color = theme.c("ok") if ok else theme.c("bad")
            self.check_result.setStyleSheet(f"color: {color};")
            self.check_result.setText(text)
            self.check_btn.setEnabled(True)
        except RuntimeError:
            pass

    def _key_edit(self, value: str) -> QKeySequenceEdit:
        edit = QKeySequenceEdit(QKeySequence(value))
        edit.setMaximumSequenceLength(1)
        return edit

    def _set(self, attr: str, value) -> None:
        setattr(self.settings, attr, value)
        self.preview.plate.update()
        self.controller.overlay.update()

    def _apply_theme(self) -> None:
        theme.apply_theme(QApplication.instance(), self.settings.theme)
        self._paint_swatches()
        self.controller.stateChanged.emit()

    def _on_theme(self) -> None:
        self.settings.theme = self.theme_box.currentData()
        self._apply_theme()

    def _on_size(self, value: int) -> None:
        self.size_label.setText(f"{value} px")
        self._set("font_size", value)

    def _on_opacity(self, value: int) -> None:
        self.op_label.setText(f"{value}%")
        self._set("bg_opacity", value)

    def _paint_swatches(self) -> None:
        for attr, sw in self._swatches.items():
            sw.setStyleSheet(f"background: {getattr(self.settings, attr)}; border: 1px solid {theme.c('dim')};")

    def _pick_color(self, attr: str) -> None:
        color = QColorDialog.getColor(QColor(getattr(self.settings, attr)), self, "Выберите цвет")
        if color.isValid():
            self._set(attr, color.name().upper())
            self._paint_swatches()

    def _save(self) -> None:
        toggle = self.hk_toggle.keySequence().toString(QKeySequence.PortableText)
        retr = self.hk_retr.keySequence().toString(QKeySequence.PortableText)
        for combo in (toggle, retr):
            if combo and not parse_hotkey(combo):
                QMessageBox.warning(self, "NovelTL", f"Сочетание «{combo}» не подходит. Нужна клавиша с Ctrl, Alt или Shift.")
                return
        if toggle and toggle == retr:
            QMessageBox.warning(self, "NovelTL", "Для двух действий выбрано одно и то же сочетание.")
            return
        mode = self.net_box.currentData()
        proxy_text = self.proxy_edit.text().strip()
        proxy_user = self.proxy_user_edit.text()
        proxy_pass = self.proxy_pass_edit.text()
        if mode == net.NET_PROXY:
            try:
                net.build_proxy_url(proxy_text, proxy_user, proxy_pass)
                proxy_text = net.normalize_proxy(proxy_text)
            except ValueError as e:
                QMessageBox.warning(self, "NovelTL", str(e))
                return
            proxy_text, u, p = net.split_proxy_credentials(proxy_text)
            if u and not proxy_user.strip():
                proxy_user, proxy_pass = u, p
        self.settings.net_mode = mode
        self.settings.proxy_url = proxy_text
        self.settings.proxy_user = proxy_user
        self.settings.proxy_pass = proxy_pass
        self.settings.bind_ip = self.iface_box.currentData() or ""
        self.settings.deepl_key = self.key_edit.text().strip()
        self.settings.hotkey_toggle = toggle
        self.settings.hotkey_retranslate = retr
        self._saved = True
        self.controller.save_settings()
        if self.controller.hotkey_errors:
            QMessageBox.warning(
                self, "NovelTL",
                "Не удалось занять сочетание: " + ", ".join(self.controller.hotkey_errors)
                + ".\nСкорее всего, его уже использует другая программа — выберите другое.")
        self.accept()

    def done(self, result: int) -> None:
        if not self._saved:
            theme_changed = self.settings.theme != self._backup.get("theme")
            for k, v in self._backup.items():
                setattr(self.settings, k, v)
            self.controller.overlay.update()
            if theme_changed:
                self._apply_theme()
        super().done(result)


class GlossaryDialog(QDialog):
    def __init__(self, controller, profile: Profile, parent=None) -> None:
        super().__init__(parent)
        self.controller = controller
        self.profile = profile
        self.setWindowTitle(f"Глоссарий — {profile.name}")
        self.setWindowIcon(app_icon())
        self.resize(520, 480)

        root = QVBoxLayout(self)
        root.setContentsMargins(24, 20, 24, 20)
        root.setSpacing(12)
        root.addWidget(label("Глоссарий имён и терминов", "h2"))
        root.addWidget(label(
            "Слева — как написано в игре, справа — как должно быть в переводе. "
            "Так имена героев всегда переводятся одинаково.", "dim", wrap=True))

        self.table = QTableWidget(0, 2)
        self.table.setHorizontalHeaderLabels(["В игре", "В переводе"])
        self.table.horizontalHeader().setSectionResizeMode(QHeaderView.Stretch)
        self.table.verticalHeader().setVisible(False)
        root.addWidget(self.table, 1)
        for entry in profile.glossary:
            self._add_row(entry.get("src", ""), entry.get("dst", ""))

        row = QHBoxLayout()
        row.addWidget(button("＋ Добавить", on_click=lambda: self._add_row("", "", edit=True)))
        row.addWidget(button("Удалить выбранное", "danger", self._remove))
        row.addStretch(1)
        row.addWidget(button("Отмена", on_click=self.reject))
        row.addWidget(button("Сохранить", "primary", self._save))
        root.addLayout(row)

    def _add_row(self, src: str, dst: str, edit: bool = False) -> None:
        r = self.table.rowCount()
        self.table.insertRow(r)
        self.table.setItem(r, 0, QTableWidgetItem(src))
        self.table.setItem(r, 1, QTableWidgetItem(dst))
        if edit:
            self.table.setCurrentCell(r, 0)
            self.table.editItem(self.table.item(r, 0))

    def _remove(self) -> None:
        for r in sorted({i.row() for i in self.table.selectedIndexes()}, reverse=True):
            self.table.removeRow(r)

    def entries(self) -> list:
        out = []
        for r in range(self.table.rowCount()):
            src = (self.table.item(r, 0).text() if self.table.item(r, 0) else "").strip()
            dst = (self.table.item(r, 1).text() if self.table.item(r, 1) else "").strip()
            if src and dst:
                out.append({"src": src, "dst": dst})
        return out

    def _save(self) -> None:
        self.table.setCurrentCell(-1, -1)  # завершить редактирование ячейки
        self.controller.save_glossary(self.profile, self.entries())
        self.accept()
