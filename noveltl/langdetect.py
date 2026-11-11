"""Определение языка по кане и иероглифам."""
from __future__ import annotations

LANG_JA = "ja"
LANG_ZH = "zh"
LANG_EN = "en"
LANG_AUTO = "auto"

LANG_TITLES = {
    LANG_JA: "Японский",
    LANG_ZH: "Китайский",
    LANG_EN: "Английский",
    LANG_AUTO: "Авто",
}

# Кодовая страница для LunaHook
LANG_CODEPAGE = {LANG_JA: 932, LANG_ZH: 936, LANG_EN: 932, LANG_AUTO: 932}


def is_kana(ch: str) -> bool:
    o = ord(ch)
    return 0x3040 <= o <= 0x30FF or 0x31F0 <= o <= 0x31FF or 0xFF66 <= o <= 0xFF9D


def is_han(ch: str) -> bool:
    o = ord(ch)
    return 0x4E00 <= o <= 0x9FFF or 0x3400 <= o <= 0x4DBF or 0xF900 <= o <= 0xFAFF


def is_cjk(ch: str) -> bool:
    return is_kana(ch) or is_han(ch)


def has_cjk(text: str) -> bool:
    return any(is_cjk(c) for c in text)


def detect_language(text: str) -> str | None:
    """Возвращает ja / zh / en или None, если понять нельзя."""
    kana = han = latin = 0
    for ch in text:
        if is_kana(ch):
            kana += 1
        elif is_han(ch):
            han += 1
        elif ch.isascii() and ch.isalpha():
            latin += 1
    if kana:
        return LANG_JA
    if han:
        return LANG_ZH
    if latin:
        return LANG_EN
    return None


def resolve_language(profile_lang: str, text: str) -> str | None:
    """Язык из профиля, а для «авто» — по тексту."""
    if profile_lang in (LANG_JA, LANG_ZH, LANG_EN):
        return profile_lang
    return detect_language(text)
