"""Профили игр (JSON в data/profiles) и общие настройки (data/settings.json)."""
from __future__ import annotations

import json
import os
import re
from dataclasses import asdict, dataclass, field, fields

from . import secret
from .langdetect import LANG_JA

DEFAULT_OVERLAY = {"x": 0.08, "y": 0.70, "w": 0.84, "h": 0.24}


def _load_json(path: str) -> dict:
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def _save_json(path: str, data: dict) -> None:
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def _from_dict(cls, data: dict):
    known = {f.name for f in fields(cls)}
    return cls(**{k: v for k, v in data.items() if k in known})


@dataclass
class Profile:
    id: str
    name: str
    exe_name: str = ""
    exe_path: str = ""
    version: str = ""  # размер exe: новая версия игры = новый профиль
    lang: str = LANG_JA
    hook: dict | None = None  # {"code", "name", "ctx_lo", "ctx2_lo"}
    name_hook: dict | None = None  # поток с именами говорящих, если имя идёт отдельно от реплики
    overlay: dict = field(default_factory=lambda: dict(DEFAULT_OVERLAY))
    glossary: list = field(default_factory=list)  # [{"src": ..., "dst": ...}]

    def set_hook(self, hookcode: str, hookname: str, ctx: int, ctx2: int) -> None:
        self.hook = {
            "code": hookcode,
            "name": hookname,
            "ctx_lo": int(ctx) & 0xFFFF,
            "ctx2_lo": int(ctx2) & 0xFFFF,
        }


def exe_version(exe_path: str) -> str:
    try:
        return format(os.path.getsize(exe_path), "x")
    except OSError:
        return "0"


def make_profile_id(exe_name: str, version: str) -> str:
    stem = os.path.splitext(exe_name)[0]
    slug = re.sub(r"[^A-Za-z0-9А-Яа-я_\-]+", "_", stem).strip("_") or "game"
    return f"{slug}_{version}"


class ProfileStore:
    def __init__(self, directory: str) -> None:
        self.directory = directory
        os.makedirs(directory, exist_ok=True)

    def _path(self, profile_id: str) -> str:
        return os.path.join(self.directory, profile_id + ".json")

    def all(self) -> list[Profile]:
        out = []
        for fn in sorted(os.listdir(self.directory)):
            if fn.endswith(".json"):
                data = _load_json(os.path.join(self.directory, fn))
                if data.get("id"):
                    out.append(_from_dict(Profile, data))
        return out

    def get(self, profile_id: str) -> Profile | None:
        data = _load_json(self._path(profile_id))
        return _from_dict(Profile, data) if data.get("id") else None

    def save(self, profile: Profile) -> None:
        _save_json(self._path(profile.id), asdict(profile))

    def delete(self, profile_id: str) -> None:
        try:
            os.remove(self._path(profile_id))
        except OSError:
            pass

    def create(self, name: str, exe_path: str, lang: str) -> Profile:
        exe_name = os.path.basename(exe_path)
        version = exe_version(exe_path)
        profile = Profile(
            id=make_profile_id(exe_name, version),
            name=name,
            exe_name=exe_name,
            exe_path=exe_path,
            version=version,
            lang=lang,
        )
        self.save(profile)
        return profile

    def find_for_exe(self, exe_path: str) -> Profile | None:
        """Профиль для запущенного exe. Для новой версии игры копирует профиль старой."""
        exe_name = os.path.basename(exe_path).lower()
        version = exe_version(exe_path)
        same_name = [p for p in self.all() if p.exe_name.lower() == exe_name]
        if not same_name:
            return None
        for p in same_name:
            if p.version == version:
                if p.exe_path != exe_path:
                    p.exe_path = exe_path
                    self.save(p)
                return p
        base = same_name[-1]
        clone = _from_dict(Profile, asdict(base))
        clone.id = make_profile_id(base.exe_name, version)
        clone.version = version
        clone.exe_path = exe_path
        self.save(clone)
        return clone


@dataclass
class AppSettings:
    deepl_key: str = ""
    theme: str = "system"  # system / dark / light
    font_family: str = "Segoe UI"
    font_size: int = 26
    text_color: str = "#FFFFFF"
    name_color: str = "#FFD27F"
    outline_color: str = "#000000"
    bg_color: str = "#0E0F1A"
    bg_opacity: int = 85  # проценты
    hotkey_toggle: str = "Ctrl+Alt+H"
    hotkey_retranslate: str = "Ctrl+Alt+R"
    net_mode: str = "system"  # system / direct / proxy
    proxy_url: str = ""
    proxy_user: str = ""
    proxy_pass: str = ""
    bind_ip: str = ""  # пусто = автоматически

    @classmethod
    def load(cls, path: str) -> "AppSettings":
        obj = _from_dict(cls, _load_json(path))
        # Секреты на диске зашифрованы через DPAPI; старый открытый текст читается как есть
        if isinstance(obj.deepl_key, str):
            obj.deepl_key = secret.unprotect(obj.deepl_key)
        if isinstance(obj.proxy_pass, str):
            obj.proxy_pass = secret.unprotect(obj.proxy_pass)
        return obj

    def save(self, path: str) -> None:
        data = asdict(self)
        data["deepl_key"] = secret.protect(data["deepl_key"])
        data["proxy_pass"] = secret.protect(data["proxy_pass"])
        _save_json(path, data)
