from noveltl.scorer import (
    ThreadTracker,
    is_confident,
    make_key,
    score_thread,
    speech_likeness,
)

DIALOG = [
    "【芳乃】「おはようございます、有地さん」",
    "【将臣】「ああ、おはよう。今日も早いな」",
    "朝の空気は澄んでいて、気持ちがいい。",
    "【芳乃】「朝のお勤めがありますから」",
]


def feed(tracker, code, name, ctx, lines, start=0.0, step=2.0, addr=0x1000):
    for i, line in enumerate(lines):
        tracker.add_text(code, name, addr, ctx, 0, line, now=start + i * step)


def test_speech_likeness():
    assert speech_likeness("「おはようございます」") > 0.9
    assert speech_likeness("Good morning, everyone.") > 0.9
    assert speech_likeness("data\\scenario\\001.ks") == 0.0
    assert speech_likeness("bg_school_01") == 0.0
    assert speech_likeness("C:/Games/save.dat") == 0.0
    assert speech_likeness("evCharaShow") == 0.0


def test_make_key_uses_low_16_bits():
    a = make_key("HQ-1C@0:x.exe", 0x7FF612345678, 0xABCD0001)
    b = make_key("HQ-1C@0:x.exe", 0x7FF699995678, 0x11110001)
    assert a == b == ("HQ-1C@0:x.exe", 0x5678, 0x0001)


def test_dialog_beats_garbage():
    t = ThreadTracker()
    feed(t, "HKIRI", "KiriKiriZ", 0x10, DIALOG)
    # спам по одному символу — много строк в секунду
    feed(t, "HGLYPH", "GetGlyphOutlineW", 0x20, list("おはようございます有地さん"), step=0.05)
    feed(t, "HPATH", "KiriKiriZ", 0x30, ["bg_001.png", "se_door.ogg", "scenario\\001.ks", "bgm_02.ogg"])
    ranked = t.ranked()
    assert ranked[0].stats.hookcode == "HKIRI"
    assert is_confident(ranked)
    assert t.auto_pick().stats.hookcode == "HKIRI"


def test_engine_hook_preferred_over_generic():
    t = ThreadTracker()
    feed(t, "HKIRI", "KiriKiriZ", 0x10, DIALOG)
    feed(t, "HTEXT", "TextOutW", 0x20, DIALOG)
    ranked = t.ranked()
    assert ranked[0].stats.hookcode == "HKIRI"
    assert ranked[0].score - ranked[1].score >= 20


def test_dialog_beats_names_only():
    t = ThreadTracker()
    feed(t, "HKIRI", "KiriKiriZ", 0x10, DIALOG)
    feed(t, "HNAME", "KiriKiriZ", 0x20, ["芳乃", "将臣", "芳乃", "芳乃"])
    assert t.ranked()[0].stats.hookcode == "HKIRI"


def test_not_confident_with_few_lines():
    t = ThreadTracker()
    feed(t, "HKIRI", "KiriKiriZ", 0x10, DIALOG[:2])
    assert t.auto_pick() is None


def test_spam_penalty():
    t = ThreadTracker()
    feed(t, "A", "x", 1, DIALOG, step=2.0)
    feed(t, "B", "x", 2, DIALOG, step=0.01)
    scores = {r.stats.hookcode: r.score for r in t.ranked()}
    assert scores["A"] > scores["B"]


def test_find_saved_hook_across_runs():
    t = ThreadTracker()
    feed(t, "HKIRI", "KiriKiriZ", 0x7FF600001234, DIALOG)
    saved = {"code": "HKIRI", "name": "KiriKiriZ", "ctx_lo": 0x1234, "ctx2_lo": 0}
    assert t.find_saved(saved) == ("HKIRI", 0x1234, 0)
    assert t.find_saved({"code": "OTHER", "ctx_lo": 0x1234, "ctx2_lo": 0}) is None
    assert t.find_saved(None) is None


def test_empty_thread_scores_zero():
    t = ThreadTracker()
    st = t.ensure("H", "n", 0, 0, 0)
    assert score_thread(st).score == 0


# --- по отчёту с настоящей Senren*Banka (англ. Steam-версия) ---
REAL_DIALOG = [
    "First of all, I don't live here.",
    "Ah! My apologies, I forgot to introduce myself. My name is Hitachi Mako.",
    "N-Nice to meet you. I'm Arichi Masaomi.",
    "I have been serving the Tomotake family for a long time now.",
    "What do you mean, serving?",
    "Huh, I see.",
    "So it's an old tradition.",
]
REAL_NAMES = ["Mako  Mako  Mako  ", "Mako  Mako  Mako  ", "Masaomi  Masaomi  Masaomi  ", "Mako  Mako  Mako  ",
              "Masaomi  Masaomi  Masaomi  ", "Mako  Mako  Mako  "]
REAL_UI = ["0001This is the load interface. Please choose a save file to load.Load 0001-01.Load 0001-01.",
           "Settings", "Replay voice", "Replay voice", "Settings", "Settings", "Settings",
           "Skip modeNext sceneNext choice"]


def real_tracker():
    t = ThreadTracker()
    feed(t, "ENHQXN-4@C3B1:textrender.dll", "TextRender", 0, REAL_DIALOG, start=10.0, step=3.0)
    # имена приходят вместе с репликами (кроме последней — это мысли героя, без имени)
    feed(t, "HW-8*14:-8*0@F4600:SenrenBanka.exe", "KiriKiriZ", 0x20, REAL_NAMES, start=9.9, step=3.0)
    feed(t, "HW-8*14:-8*0@F4600:SenrenBanka.exe", "KiriKiriZ", 0x12, REAL_UI, start=1.0, step=4.7)
    return t


def test_real_report_dialog_wins_over_menu_text():
    t = real_tracker()
    ranked = t.ranked()
    assert ranked[0].stats.hookname == "TextRender"
    assert is_confident(ranked)


def test_name_thread_is_found():
    from noveltl.scorer import find_name_thread

    t = real_tracker()
    dialog_key = ("ENHQXN-4@C3B1:textrender.dll", 0, 0)
    key = find_name_thread(t.threads, dialog_key)
    assert key == ("HW-8*14:-8*0@F4600:SenrenBanka.exe", 0x20, 0)


def test_name_thread_not_invented():
    from noveltl.scorer import find_name_thread

    t = ThreadTracker()
    feed(t, "D", "TextRender", 0, REAL_DIALOG, start=10.0, step=3.0)
    feed(t, "UI", "KiriKiriZ", 0x12, REAL_UI, start=1.0, step=4.7)
    assert find_name_thread(t.threads, ("D", 0, 0)) is None


def test_constant_line_is_not_a_name_thread():
    from noveltl.scorer import find_name_thread

    t = ThreadTracker()
    feed(t, "D", "TextRender", 0, REAL_DIALOG, start=10.0, step=3.0)
    feed(t, "TITLE", "TextOutW", 0x50, ["Senren*Banka"] * 7, start=10.0, step=3.0)
    assert find_name_thread(t.threads, ("D", 0, 0)) is None
