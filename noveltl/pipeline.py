"""Реплика из игры -> глоссарий -> кэш/переводчик -> готовый перевод с именем."""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass

from .langdetect import resolve_language
from .translators import TranslatorManager

CONTEXT_LINES = 3


def apply_glossary(text: str, glossary: list) -> str:
    """Подставляет готовые переводы имён и терминов за один проход, без каскада."""
    entries = [(e.get("src", ""), e.get("dst", "")) for e in glossary or []]
    entries = [(s, d) for s, d in entries if s and d]
    if not entries:
        return text
    out: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        best = None
        for src, dst in entries:
            # самое длинное совпадение; при равной длине — первое в глоссарии
            if text.startswith(src, i) and (best is None or len(src) > len(best[0])):
                best = (src, dst)
        if best:
            out.append(best[1])
            i += len(best[0])
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def glossary_exact(text: str, glossary: list) -> str | None:
    for e in glossary or []:
        if e.get("src") == text and e.get("dst"):
            return e["dst"]
    return None


@dataclass
class LineResult:
    name: str  # переведённое имя
    text: str  # переведённый текст
    src_name: str
    src_text: str
    engine: str
    cached: bool
    notice: str = ""


class TranslationPipeline:
    def __init__(self, manager: TranslatorManager, lang: str, glossary: list | None = None) -> None:
        self.manager = manager
        self.lang = lang
        self.glossary = glossary if glossary is not None else []
        self._context: deque[str] = deque(maxlen=CONTEXT_LINES)

    def reset_context(self) -> None:
        self._context.clear()

    def _translate_name(self, name: str, lang: str | None, use_cache: bool) -> tuple[str, str]:
        exact = glossary_exact(name, self.glossary)
        if exact:
            return exact, ""
        res = self.manager.translate(apply_glossary(name, self.glossary), lang, use_cache=use_cache)
        return res.text.strip(" .。"), res.notice

    def process(self, name: str, text: str, use_cache: bool = True) -> LineResult:
        lang = resolve_language(self.lang, text)
        context = "\n".join(c for c in self._context if c != text)
        prepared = apply_glossary(text, self.glossary)
        res = self.manager.translate(prepared, lang, context=context, use_cache=use_cache)
        notice = res.notice
        name_tr = ""
        if name:
            # имя переводим всегда через кэш: оно не меняется от реплики к реплике
            try:
                name_tr, name_notice = self._translate_name(name, lang, True)
                notice = notice or name_notice
            except Exception:  # noqa: BLE001
                # перевод текста готов — не теряем его из-за имени
                name_tr = name
                notice = notice or "Имя не удалось перевести — показано как есть."
        if not self._context or self._context[-1] != text:
            self._context.append(text)
        return LineResult(name_tr, res.text, name, text, res.engine, res.cached, notice)
