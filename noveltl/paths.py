"""Пути приложения: всё хранится рядом с exe (или рядом с main.py при запуске из исходников)."""
from __future__ import annotations

import os
import sys


def app_dir() -> str:
    if getattr(sys, "frozen", False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def data_dir() -> str:
    path = os.path.join(app_dir(), "data")
    os.makedirs(path, exist_ok=True)
    return path


def files_dir() -> str:
    return os.path.join(app_dir(), "files")


def sub_data_dir(name: str) -> str:
    path = os.path.join(data_dir(), name)
    os.makedirs(path, exist_ok=True)
    return path
