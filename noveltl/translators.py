"""Переводчики: DeepL (основной, с ключом) и Google без ключа (запасной)."""
from __future__ import annotations

from dataclasses import dataclass

from .cache import TranslationCache
from .langdetect import LANG_EN, LANG_JA, LANG_ZH

ENGINE_DEEPL = "deepl"
ENGINE_GOOGLE = "google"
ENGINE_TITLES = {ENGINE_DEEPL: "DeepL", ENGINE_GOOGLE: "Google"}

TIMEOUT = 12


class TranslateError(Exception):
    pass


class DeepLDenied(TranslateError):
    """403 (неверный ключ) или 456 (лимит исчерпан) — DeepL отключается до смены ключа."""

    def __init__(self, status: int) -> None:
        self.status = status
        reason = {403: "ключ не принят", 456: "лимит символов исчерпан"}.get(status, "доступ запрещён")
        super().__init__(f"ошибка {status}: {reason}")


def _default_session():
    import requests

    return requests.Session()


class DeepLTranslator:
    engine = ENGINE_DEEPL
    _LANGS = {LANG_JA: "JA", LANG_ZH: "ZH", LANG_EN: "EN"}

    def __init__(self, api_key: str, session=None) -> None:
        self.api_key = api_key.strip()
        self.session = session or _default_session()

    @property
    def url(self) -> str:
        host = "api-free.deepl.com" if self.api_key.endswith(":fx") else "api.deepl.com"
        return f"https://{host}/v2/translate"

    def translate(self, text: str, src_lang: str | None, context: str = "") -> str:
        payload: dict = {"text": [text], "target_lang": "RU"}
        if src_lang in self._LANGS:
            payload["source_lang"] = self._LANGS[src_lang]
        if context:
            payload["context"] = context
        try:
            resp = self.session.post(
                self.url,
                json=payload,
                headers={"Authorization": f"DeepL-Auth-Key {self.api_key}"},
                timeout=TIMEOUT,
            )
        except Exception as e:  # сеть
            raise TranslateError(f"нет связи с DeepL ({type(e).__name__})") from e
        if resp.status_code in (403, 456):
            raise DeepLDenied(resp.status_code)
        if resp.status_code != 200:
            raise TranslateError(f"DeepL ответил кодом {resp.status_code}")
        try:
            return resp.json()["translations"][0]["text"]
        except Exception as e:
            raise TranslateError("непонятный ответ DeepL") from e


class GoogleTranslator:
    engine = ENGINE_GOOGLE
    URL = "https://translate.googleapis.com/translate_a/single"
    _LANGS = {LANG_JA: "ja", LANG_ZH: "zh-CN", LANG_EN: "en"}

    def __init__(self, session=None) -> None:
        self.session = session or _default_session()

    def translate(self, text: str, src_lang: str | None, context: str = "") -> str:
        params = {
            "client": "gtx",
            "sl": self._LANGS.get(src_lang, "auto"),
            "tl": "ru",
            "dt": "t",
            "q": text,
        }
        try:
            resp = self.session.get(self.URL, params=params, timeout=TIMEOUT)
        except Exception as e:
            raise TranslateError(f"нет связи с Google ({type(e).__name__})") from e
        if resp.status_code == 429:
            raise TranslateError(
                "Google временно ограничил запросы с вашего адреса (код 429). "
                "Укажите ключ DeepL или смените сеть/прокси в настройках")
        if resp.status_code != 200:
            raise TranslateError(f"Google ответил кодом {resp.status_code}")
        try:
            return "".join(seg[0] for seg in resp.json()[0] if seg and seg[0])
        except Exception as e:
            raise TranslateError("непонятный ответ Google") from e


@dataclass
class TranslateResult:
    text: str
    engine: str
    cached: bool = False
    notice: str = ""  # сообщение для пользователя (например, о переключении на Google)


class TranslatorManager:
    """DeepL -> при 403/456 автоматически Google. Кэш — по тексту и переводчику."""

    def __init__(self, deepl_key: str = "", cache: TranslationCache | None = None, session=None) -> None:
        self._session = session
        self.cache = cache
        self.google = GoogleTranslator(session)
        self.deepl: DeepLTranslator | None = None
        self.deepl_disabled_reason = ""
        self.set_deepl_key(deepl_key)

    def set_session(self, session) -> None:
        self._session = session
        self.google.session = session
        if self.deepl:
            self.deepl.session = session

    def set_deepl_key(self, key: str) -> None:
        key = (key or "").strip()
        self.deepl = DeepLTranslator(key, self._session) if key else None
        self.deepl_disabled_reason = ""

    @property
    def active_engine(self) -> str:
        if self.deepl and not self.deepl_disabled_reason:
            return ENGINE_DEEPL
        return ENGINE_GOOGLE

    def status_text(self) -> str:
        if self.active_engine == ENGINE_DEEPL:
            return "DeepL"
        if self.deepl_disabled_reason:
            return f"Google (DeepL недоступен: {self.deepl_disabled_reason})"
        return "Google (ключ DeepL не задан)"

    def _cached(self, text: str, engine: str, use_cache: bool) -> str | None:
        if use_cache and self.cache:
            return self.cache.get(text, engine)
        return None

    def _store(self, text: str, engine: str, result: str) -> None:
        if self.cache and result:
            self.cache.put(text, engine, result)

    def translate(self, text: str, src_lang: str | None, context: str = "", use_cache: bool = True) -> TranslateResult:
        notice = ""
        if self.active_engine == ENGINE_DEEPL:
            hit = self._cached(text, ENGINE_DEEPL, use_cache)
            if hit is not None:
                return TranslateResult(hit, ENGINE_DEEPL, cached=True)
            try:
                out = self.deepl.translate(text, src_lang, context)
                self._store(text, ENGINE_DEEPL, out)
                return TranslateResult(out, ENGINE_DEEPL)
            except DeepLDenied as e:
                self.deepl_disabled_reason = str(e)
                notice = f"DeepL недоступен ({e}). Переключился на Google."
            except TranslateError as e:
                notice = f"DeepL: {e}. Эта реплика переведена через Google."

        hit = self._cached(text, ENGINE_GOOGLE, use_cache)
        if hit is not None:
            return TranslateResult(hit, ENGINE_GOOGLE, cached=True, notice=notice)
        out = self.google.translate(text, src_lang)
        self._store(text, ENGINE_GOOGLE, out)
        return TranslateResult(out, ENGINE_GOOGLE, notice=notice)
