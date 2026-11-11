"""Служебное: проходит по окнам в демо-режиме и сохраняет скриншоты (--shots ПАПКА)."""
from __future__ import annotations

import os

from PySide6.QtCore import QTimer
from PySide6.QtGui import QGuiApplication, QPainter

from .ui.dialogs import GlossaryDialog, SettingsDialog
from .ui.wizard import MODE_NEW


def run_shots(app, window, controller, game, out_dir: str) -> None:
    os.makedirs(out_dir, exist_ok=True)
    state: dict = {}
    steps: list = []

    def save(widget, name: str) -> None:
        widget.grab().save(os.path.join(out_dir, name))

    def composite(name: str) -> None:
        """Окно игры + плашка поверх него (как это выглядит на экране)."""
        pm = game.grab()
        ov = controller.overlay
        if ov.isVisible():
            p = QPainter(pm)
            p.drawPixmap(ov.pos() - game.mapToGlobal(game.rect().topLeft()), ov.grab())
            p.end()
        pm.save(os.path.join(out_dir, name))

    def screen_grab(name: str) -> None:
        g = game.frameGeometry()
        QGuiApplication.primaryScreen().grabWindow(0, g.x(), g.y(), g.width(), g.height()).save(
            os.path.join(out_dir, name))

    def step(delay: int):
        def deco(fn):
            steps.append((delay, fn))
            return fn
        return deco

    game.move(60, 40)
    window.move(120, 90)
    controller.overlay.force_active = True

    @step(700)
    def _():
        save(window, "01_main_empty.png")
        state["wiz"] = window.open_wizard(MODE_NEW)
        state["wiz"].move(700, 60)

    @step(700)
    def _():
        save(state["wiz"], "02_wizard_step1.png")
        state["wiz"].win_list.setCurrentRow(0)
        state["wiz"]._start_search()

    @step(900)
    def _():
        save(state["wiz"], "03_wizard_step2_waiting.png")
        game.advance()

    for _i in range(3):
        @step(900)
        def _():
            game.advance()

    @step(900)
    def _():
        save(state["wiz"], "04_wizard_step2_found.png")
        state["wiz"]._toggle_others()

    @step(500)
    def _():
        save(state["wiz"], "05_wizard_step2_others.png")
        state["wiz"]._toggle_others()
        state["wiz"]._confirm_best()
        game.raise_()

    @step(700)
    def _():
        save(state["wiz"], "06_wizard_step3.png")
        composite("07_overlay_edit_mode.png")
        screen_grab("verify_screen_edit.png")
        state["wiz"]._finish()

    @step(600)
    def _():
        game.advance()

    @step(900)
    def _():
        composite("08_overlay_in_game.png")
        screen_grab("verify_screen_game.png")
        save(window, "09_main_running.png")
        state["dlg"] = SettingsDialog(controller, window)
        state["dlg"].show()

    @step(600)
    def _():
        save(state["dlg"], "10_settings.png")
        state["dlg"].reject()
        profile = controller.session.profile
        profile.glossary = [{"src": "芳乃", "dst": "Ёсино"}, {"src": "将臣", "dst": "Масаоми"}, {"src": "茉子", "dst": "Мако"}]
        state["dlg"] = GlossaryDialog(controller, profile, window)
        state["dlg"].show()

    @step(600)
    def _():
        save(state["dlg"], "11_glossary.png")
        state["dlg"].reject()
        path = controller.save_report()
        print("REPORT", path)
        window.quit()

    def run_next() -> None:
        if not steps:
            return
        delay, fn = steps.pop(0)

        def call():
            try:
                fn()
            except Exception:  # noqa: BLE001
                import traceback

                traceback.print_exc()
                app.exit(1)
                return
            run_next()

        QTimer.singleShot(delay, call)

    run_next()
