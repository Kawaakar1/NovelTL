"""Автоподбор хука: оценка текстовых потоков LunaHook."""
from __future__ import annotations

import re
import statistics
import time
from collections import deque
from dataclasses import dataclass, field

from .cleaner import clean
from .langdetect import is_cjk

HookKey = tuple  # (hookcode, ctx & 0xFFFF, ctx2 & 0xFFFF)

ENGINE_MARKERS = ("kirikiri", "krkr", "textrender")  # textrender.dll — плагин вывода текста KiriKiri
GENERIC_MARKERS = (
    "getglyphoutline", "textout", "drawtext", "exttextout",
    # прочие общие функции Windows, которые LunaHook перехватывает в любой программе
    "gettextextent", "getcharabcwidths", "getcharwidth", "getglyphindices", "getcharacterplacement",
    "lstr", "multibytetowidechar", "widechartomultibyte", "getstringtype", "foldstring",
    "charnext", "charprev", "wvsprintf",
)

CONFIDENT_SCORE = 55.0
CONFIDENT_MARGIN = 8.0
CONFIDENT_DISTINCT = 3

_RE_FILE = re.compile(
    r"\.(ks|tjs|png|jpe?g|bmp|tlg|ogg|wav|mp3|mp4|txt|dll|exe|xp3|scn|psb|ini|json|csv|pimg|mpg|wmv|otf|ttf)\b",
    re.I,
)
_RE_DRIVE = re.compile(r"[A-Za-z]:[\\/]")
_RE_IDENT = re.compile(r"^[A-Za-z0-9_\-.:#@$]+$")
_SPEECH_PUNCT = set("。、！？!?…「」『』（）()—―-~〜・,.'\"“”‘’:;♪　 ")
_SENTENCE_MARKS = set("。！？!?…「」『』.,、")


def make_key(hookcode: str, ctx: int, ctx2: int) -> HookKey:
    """Ключ хука, устойчивый между запусками: адреса меняются, младшие 16 бит — нет."""
    return (hookcode, int(ctx) & 0xFFFF, int(ctx2) & 0xFFFF)


def speech_likeness(text: str) -> float:
    """0..1 — насколько строка похожа на реплику, а не на путь или идентификатор."""
    s = text.strip()
    if not s:
        return 0.0
    if _RE_FILE.search(s) or _RE_DRIVE.search(s):
        return 0.0
    ascii_only = s.isascii()
    if ascii_only and ("\\" in s or "/" in s):
        return 0.0
    if ascii_only and " " not in s and _RE_IDENT.match(s):
        if "_" in s or any(c.isdigit() for c in s) or re.search(r"[a-z][A-Z]", s) or "." in s[:-1].rstrip("."):
            return 0.0
        return 0.5
    good = 0
    for ch in s:
        if is_cjk(ch) or ch.isalpha() or ch in _SPEECH_PUNCT:
            good += 1
    ratio = good / len(s)
    if any(ch in _SENTENCE_MARKS for ch in s):
        ratio += 0.15
    if "�" in s:
        ratio -= 0.5
    return max(0.0, min(1.0, ratio))


@dataclass
class ThreadStats:
    """Один текстовый поток (хук) и его последние строки."""

    hookcode: str
    hookname: str
    addr: int = 0
    ctx: int = 0
    ctx2: int = 0
    total: int = 0
    lines: deque = field(default_factory=lambda: deque(maxlen=120))  # (время, сырой текст)

    @property
    def key(self) -> HookKey:
        return make_key(self.hookcode, self.ctx, self.ctx2)

    def add(self, text: str, now: float | None = None) -> None:
        self.total += 1
        self.lines.append((time.time() if now is None else now, text))

    def last_raw(self, n: int = 5) -> list[str]:
        return [t for _, t in list(self.lines)[-n:]]

    def last_clean(self, n: int = 3) -> list[str]:
        out: list[str] = []
        for _, raw in reversed(self.lines):
            res = clean(raw)
            if not res.text:
                continue
            line = f"{res.name}: {res.text}" if res.name else res.text
            if not out or out[-1] != line:
                out.append(line)
            if len(out) >= n:
                break
        return list(reversed(out))


@dataclass
class ScoreResult:
    stats: ThreadStats
    score: float
    distinct: int
    details: dict

    @property
    def key(self) -> HookKey:
        return self.stats.key


def _peak_rate(times: list[float]) -> int:
    """Максимум строк за любую секунду."""
    peak, start = 0, 0
    for end in range(len(times)):
        while times[end] - times[start] > 1.0:
            start += 1
        peak = max(peak, end - start + 1)
    return peak


def score_thread(stats: ThreadStats) -> ScoreResult:
    entries = list(stats.lines)
    cleaned = [clean(raw).text for _, raw in entries]
    cleaned = [c for c in cleaned if c]
    name = (stats.hookname or "").lower()
    details: dict = {}
    if not cleaned:
        return ScoreResult(stats, 0.0, 0, {"пусто": True})

    distinct = len(set(cleaned))
    details["разных_реплик"] = round(min(distinct, 4) / 4 * 30, 1)

    speech = statistics.fmean(speech_likeness(c) for c in cleaned)
    details["похоже_на_речь"] = round(speech * 35, 1)

    peak = _peak_rate([t for t, _ in entries])
    spam = 0.0
    if peak >= 3:
        spam -= min(40.0, (peak - 2) * 10.0)
    if len(cleaned) >= 4:
        # одни и те же строки снова и снова — меню, подписи кнопок, имена
        ratio = distinct / len(cleaned)
        if ratio < 0.75:
            spam -= min(30.0, round((0.75 - ratio) * 60, 1))
    details["спам"] = spam

    median_len = statistics.median(len(c) for c in cleaned)
    if median_len < 2:
        length = -15.0
    elif median_len < 6:
        length = 5.0
    elif median_len <= 150:
        length = 15.0
    elif median_len <= 300:
        length = 0.0
    else:
        length = -10.0
    details["длина"] = length

    bonus = 0.0
    if any(m in name for m in ENGINE_MARKERS):
        bonus += 12.0
    if any(m in name for m in GENERIC_MARKERS):
        bonus -= 12.0
    details["тип_хука"] = bonus

    score = sum(v for v in details.values() if isinstance(v, (int, float)))
    return ScoreResult(stats, round(max(0.0, score), 1), distinct, details)


def rank_threads(threads) -> list[ScoreResult]:
    results = [score_thread(t) for t in threads]
    results = [r for r in results if r.score > 0]
    results.sort(key=lambda r: (r.score, r.stats.total), reverse=True)
    return results


def is_confident(ranked: list[ScoreResult]) -> bool:
    if not ranked:
        return False
    best = ranked[0]
    if best.score < CONFIDENT_SCORE or best.distinct < CONFIDENT_DISTINCT:
        return False
    return len(ranked) == 1 or best.score - ranked[1].score >= CONFIDENT_MARGIN


NAME_WINDOW = 1.0  # секунд между именем и репликой (поиск потока имён)
# Имя относится к ЭТОЙ реплике, только если пришло почти одновременно с ней
# (иначе при быстром пролистывании реплика рассказчика получает имя предыдущей).
NAME_PAIR_WINDOW = 0.35
_NAME_END = set("。.!?！？…")


def find_name_thread(threads: dict, dialog_key: HookKey) -> HookKey | None:
    """Ищет поток с именами говорящих: короткие строки, приходящие вместе с репликами."""
    dialog = threads.get(dialog_key)
    if dialog is None or len(dialog.lines) < 3:
        return None
    dialog_times = [t for t, _ in dialog.lines]
    best, best_rank = None, None
    for key, st in threads.items():
        if key == dialog_key or len(st.lines) < 3:
            continue
        if _peak_rate([t for t, _ in st.lines]) >= 4:
            continue
        entries = [(t, clean(raw).text) for t, raw in st.lines]
        entries = [(t, c) for t, c in entries if c]
        if len(entries) < 3:
            continue
        texts = [c for _, c in entries]
        if len(set(texts)) < 2:
            continue  # одна и та же строка (заголовок окна и т. п.) — это не имена
        if statistics.median(len(c) for c in texts) > 24:
            continue
        if sum(c[-1] in _NAME_END for c in texts) / len(texts) > 0.2:
            continue
        if statistics.fmean(speech_likeness(c) for c in texts) < 0.5:
            continue
        near = sum(1 for t, _ in entries if any(abs(t - d) <= NAME_WINDOW for d in dialog_times))
        share = near / len(entries)
        if share < 0.7:
            continue
        rank = (round(share, 2), len(entries))
        if best_rank is None or rank > best_rank:
            best, best_rank = key, rank
    return best


class ThreadTracker:
    """Все потоки текущей игры. Потоки с одинаковым ключом объединяются."""

    def __init__(self) -> None:
        self.threads: dict[HookKey, ThreadStats] = {}

    def clear(self) -> None:
        self.threads.clear()

    def ensure(self, hookcode: str, hookname: str, addr: int, ctx: int, ctx2: int) -> ThreadStats:
        key = make_key(hookcode, ctx, ctx2)
        st = self.threads.get(key)
        if st is None:
            st = ThreadStats(hookcode, hookname, addr, ctx, ctx2)
            self.threads[key] = st
        return st

    def add_text(self, hookcode, hookname, addr, ctx, ctx2, text, now=None) -> ThreadStats:
        st = self.ensure(hookcode, hookname, addr, ctx, ctx2)
        st.add(text, now)
        return st

    def ranked(self) -> list[ScoreResult]:
        return rank_threads(self.threads.values())

    def find_saved(self, saved: dict | None) -> HookKey | None:
        """Ищет сохранённый хук среди живых потоков."""
        if not saved:
            return None
        key = (saved.get("code"), saved.get("ctx_lo"), saved.get("ctx2_lo"))
        return key if key in self.threads else None

    def auto_pick(self) -> ScoreResult | None:
        ranked = self.ranked()
        return ranked[0] if is_confident(ranked) else None
