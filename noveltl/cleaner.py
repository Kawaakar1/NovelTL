"""Очистка текста из хука: теги KiriKiri, удвоения, повторы, имя говорящего."""
from __future__ import annotations

import re
from dataclasses import dataclass

from .langdetect import has_cjk

# %f$ll; %p-1; %fSourceHanSans; и т. п.
_TAG_PERCENT = re.compile(r"%[^;%\s「」『』（）。、]{1,40};")
# [r] [l] [p] [np] [ruby text="..."] [/b]
_TAG_BRACKET = re.compile(r"\[/?[a-z_][a-z0-9_]*(?:[\s=,][^\[\]]*)?\]")
_TAG_ANGLE = re.compile(r"</?[a-zA-Z][a-zA-Z0-9]*(?:\s[^<>]*)?>")
_CONTROL = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f​-‏﻿]")

_SENTENCE_END = "。！？!?…」』）)\"”.~〜♪—―"
_OPEN_QUOTES = "「『（(“\""
_NAME_FORBIDDEN = set("。、！？!?…「」『』（）()【】,.:;：；\"“”")


@dataclass
class CleanResult:
    name: str
    text: str

    def __bool__(self) -> bool:
        return bool(self.text)


def strip_tags(text: str) -> str:
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    text = text.replace("\\n", "\n")
    text = _TAG_PERCENT.sub("", text)
    # [r] — перенос строки в KiriKiri
    text = re.sub(r"\[r\]", "\n", text)
    text = _TAG_BRACKET.sub("", text)
    text = _TAG_ANGLE.sub("", text)
    text = _CONTROL.sub("", text)
    return text


def collapse_repeated_chars(text: str) -> str:
    """«ここんんにに» -> «こんに». Строку из одного символа («ああああ») не трогаем."""
    n = len(text)
    for k in (4, 3, 2):
        if n >= 2 * k and n % k == 0:
            if all(text[i : i + k] == text[i] * k for i in range(0, n, k)):
                out = text[::k]
                if len(set(out)) > 1:
                    return out
    return text


def collapse_repeated_phrase(text: str) -> str:
    """«Привет.Привет.» -> «Привет.». Короткие слова-повторы (ドキドキ) не трогаем."""
    s = text.strip()
    lines = [ln.strip() for ln in s.split("\n") if ln.strip()]
    if len(lines) >= 2 and all(ln == lines[0] for ln in lines) and len(lines[0]) >= 2:
        return lines[0]
    # «Masaomi  Masaomi  Masaomi» — повтор через пробелы (так KiriKiri отдаёт имена)
    tokens = [t for t in re.split(r"\s{2,}", s) if t]
    if len(tokens) >= 2 and all(t == tokens[0] for t in tokens):
        return tokens[0]
    n = len(s)
    for period in range(2, n // 2 + 1):
        if n % period:
            continue
        unit = s[:period]
        if unit * (n // period) != s:
            continue
        if len(set(unit)) < 2:
            return s
        if period >= 6 or unit[-1] in _SENTENCE_END:
            return unit
        return s
    return s


def _looks_like_name(name: str, limit: int) -> bool:
    name = name.strip()
    if not name or len(name) > limit:
        return False
    return not any(ch in _NAME_FORBIDDEN for ch in name)


_RE_BRACKET_NAME = re.compile(r"^\s*【([^【】\n]{1,24})】\s*(.+)$", re.S)
_RE_QUOTE_NAME = re.compile(r"^([^\s「」『』（）()]{1,12})\s*([「『].+[」』])\s*$", re.S)
_RE_COLON_NAME = re.compile(r"^([A-Z][A-Za-z'.\-]*(?: [A-Z?][A-Za-z'.\-]*){0,2}|[^\s:：]{1,10})\s*[:：]\s*(\S.*)$", re.S)


def split_name(text: str) -> tuple[str, str]:
    """Отделяет имя говорящего. Возвращает (имя, текст); имя может быть пустым."""
    s = text.strip()
    m = _RE_BRACKET_NAME.match(s)
    if m:
        return m.group(1).strip(), m.group(2).strip()

    if "\n" in s:
        first, rest = s.split("\n", 1)
        rest = rest.strip()
        if rest and rest[0] in _OPEN_QUOTES and _looks_like_name(first, 16):
            return first.strip(), rest

    m = _RE_QUOTE_NAME.match(s)
    if m and _looks_like_name(m.group(1), 12):
        return m.group(1).strip(), m.group(2).strip()

    m = _RE_COLON_NAME.match(s)
    if m and _looks_like_name(m.group(1), 25) and not m.group(1).isdigit():
        # «http://», «12:30» и подобное именем не считаем
        if not m.group(2).startswith("//"):
            return m.group(1).strip(), m.group(2).strip()
    return "", s


def join_lines(text: str) -> str:
    parts = [p.strip() for p in text.split("\n") if p.strip()]
    if not parts:
        return ""
    sep = "" if has_cjk(text) else " "
    return re.sub(r"[ \t　]{2,}", " ", sep.join(parts)).strip()


def clean(raw: str) -> CleanResult:
    """Полная очистка одной реплики из хука."""
    if not raw:
        return CleanResult("", "")
    text = strip_tags(raw).strip()
    if not text:
        return CleanResult("", "")
    text = collapse_repeated_chars(text)
    text = "\n".join(collapse_repeated_chars(ln) for ln in text.split("\n"))
    text = collapse_repeated_phrase(text)
    name, body = split_name(text)
    body = collapse_repeated_phrase(join_lines(body))
    name = join_lines(name)
    if not body:
        return CleanResult("", name)
    return CleanResult(name, body)
