import pytest

from noveltl.cache import TranslationCache
from noveltl.pipeline import TranslationPipeline, apply_glossary
from noveltl.profiles import AppSettings, ProfileStore
from noveltl.translators import DeepLTranslator, TranslateError, TranslatorManager


class FakeResponse:
    def __init__(self, status, data=None):
        self.status_code = status
        self._data = data

    def json(self):
        return self._data


class FakeSession:
    """Имитация requests.Session: DeepL через post, Google через get."""

    def __init__(self, deepl_status=200):
        self.deepl_status = deepl_status
        self.posts = []
        self.gets = []

    def post(self, url, json=None, headers=None, timeout=None):
        self.posts.append({"url": url, "json": json, "headers": headers})
        if self.deepl_status != 200:
            return FakeResponse(self.deepl_status)
        return FakeResponse(200, {"translations": [{"text": "D:" + json["text"][0]}]})

    def get(self, url, params=None, timeout=None):
        self.gets.append({"url": url, "params": params})
        return FakeResponse(200, [[["G:" + params["q"], params["q"], None]], None, "ja"])


def test_deepl_free_and_pro_urls():
    assert "api-free.deepl.com" in DeepLTranslator("abc:fx", FakeSession()).url
    assert DeepLTranslator("abc", FakeSession()).url.startswith("https://api.deepl.com")


def test_deepl_request_shape():
    s = FakeSession()
    out = DeepLTranslator("key:fx", s).translate("こんにちは", "ja", context="前の文")
    assert out == "D:こんにちは"
    req = s.posts[0]
    assert req["json"] == {"text": ["こんにちは"], "target_lang": "RU", "source_lang": "JA", "context": "前の文"}
    assert req["headers"]["Authorization"] == "DeepL-Auth-Key key:fx"


def test_google_request_shape():
    s = FakeSession()
    m = TranslatorManager("", session=s)
    res = m.translate("こんにちは", "ja")
    assert (res.text, res.engine) == ("G:こんにちは", "google")
    p = s.gets[0]["params"]
    assert (p["client"], p["sl"], p["tl"]) == ("gtx", "ja", "ru")


@pytest.mark.parametrize("status", [403, 456])
def test_deepl_denied_falls_back_to_google(status):
    s = FakeSession(deepl_status=status)
    m = TranslatorManager("key", session=s)
    res = m.translate("テスト", "ja")
    assert res.engine == "google" and res.text == "G:テスト"
    assert "Переключился на Google" in res.notice and str(status) in res.notice
    assert m.active_engine == "google"
    # дальше DeepL уже не дёргаем
    res2 = m.translate("次", "ja")
    assert len(s.posts) == 1 and res2.notice == ""
    # новый ключ снова включает DeepL
    m.set_deepl_key("other")
    assert m.active_engine == "deepl"


def test_deepl_temporary_error_does_not_disable():
    s = FakeSession(deepl_status=500)
    m = TranslatorManager("key", session=s)
    res = m.translate("テスト", "ja")
    assert res.engine == "google" and res.notice
    assert m.active_engine == "deepl"


def test_cache_hit_and_bypass():
    s = FakeSession()
    cache = TranslationCache()
    m = TranslatorManager("key", cache=cache, session=s)
    assert not m.translate("テスト", "ja").cached
    assert m.translate("テスト", "ja").cached
    assert len(s.posts) == 1
    assert not m.translate("テスト", "ja", use_cache=False).cached
    assert len(s.posts) == 2
    assert cache.count() == 1


def test_cache_persists(tmp_path):
    path = str(tmp_path / "c.sqlite")
    c = TranslationCache(path)
    c.put("a", "deepl", "б")
    c.close()
    assert TranslationCache(path).get("a", "deepl") == "б"


def test_glossary():
    g = [{"src": "芳乃", "dst": "Ёсино"}, {"src": "朝武芳乃", "dst": "Томотакэ Ёсино"}]
    assert apply_glossary("朝武芳乃と芳乃", g) == "Томотакэ ЁсиноとЁсино"


def test_pipeline_name_context_glossary():
    s = FakeSession()
    m = TranslatorManager("key", cache=TranslationCache(), session=s)
    p = TranslationPipeline(m, "ja", [{"src": "芳乃", "dst": "Ёсино"}])
    r1 = p.process("芳乃", "「おはよう」")
    assert r1.name == "Ёсино"  # имя из глоссария, без запроса
    assert r1.text == "D:「おはよう」"
    r2 = p.process("将臣", "「芳乃、おはよう」")
    assert r2.name == "D:将臣"
    assert r2.text == "D:「Ёсино、おはよう」"
    # в DeepL ушёл контекст с предыдущей репликой
    line_reqs = [q["json"] for q in s.posts if "おはよう" in q["json"]["text"][0]]
    assert "context" not in line_reqs[0]
    assert line_reqs[1]["context"] == "「おはよう」"


def test_pipeline_context_is_limited():
    s = FakeSession()
    p = TranslationPipeline(TranslatorManager("key", session=s), "ja")
    for i in range(6):
        p.process("", f"реплика{i}")
    assert s.posts[-1]["json"]["context"] == "реплика2\nреплика3\nреплика4"


def test_pipeline_auto_language():
    s = FakeSession()
    p = TranslationPipeline(TranslatorManager("", session=s), "auto")
    p.process("", "你好")
    p.process("", "おはよう")
    p.process("", "Hello")
    assert [g["params"]["sl"] for g in s.gets] == ["zh-CN", "ja", "en"]


def test_google_failure_raises():
    class Broken(FakeSession):
        def get(self, *a, **k):
            raise OSError("no network")

    with pytest.raises(TranslateError):
        TranslatorManager("", session=Broken()).translate("x", "ja")


def test_profiles_roundtrip(tmp_path):
    exe = tmp_path / "SenrenBanka.exe"
    exe.write_bytes(b"x" * 100)
    store = ProfileStore(str(tmp_path / "profiles"))
    p = store.create("Senren*Banka", str(exe), "ja")
    p.set_hook("HKIRI", "KiriKiriZ", 0x7FF600001234, 0x10005)
    p.glossary.append({"src": "芳乃", "dst": "Ёсино"})
    store.save(p)

    found = store.find_for_exe(str(exe))
    assert found.id == p.id
    assert found.hook == {"code": "HKIRI", "name": "KiriKiriZ", "ctx_lo": 0x1234, "ctx2_lo": 0x0005}
    assert found.glossary == [{"src": "芳乃", "dst": "Ёсино"}]
    assert 0 < found.overlay["w"] <= 1


def test_profile_new_version_is_cloned(tmp_path):
    exe = tmp_path / "game.exe"
    exe.write_bytes(b"x" * 100)
    store = ProfileStore(str(tmp_path / "profiles"))
    p = store.create("Game", str(exe), "ja")
    p.set_hook("H", "n", 1, 2)
    store.save(p)
    exe.write_bytes(b"x" * 200)  # игра обновилась
    p2 = store.find_for_exe(str(exe))
    assert p2.id != p.id and p2.hook == p.hook
    assert len(store.all()) == 2
    assert store.find_for_exe(str(tmp_path / "other.exe")) is None


def test_settings_roundtrip(tmp_path):
    path = str(tmp_path / "settings.json")
    s = AppSettings.load(path)
    assert s.hotkey_toggle == "Ctrl+Alt+H" and s.hotkey_retranslate == "Ctrl+Alt+R"
    s.deepl_key = "k:fx"
    s.font_size = 30
    s.save(path)
    s2 = AppSettings.load(path)
    assert (s2.deepl_key, s2.font_size) == ("k:fx", 30)


def test_settings_have_theme_and_survive_old_file(tmp_path):
    path = tmp_path / "settings.json"
    path.write_text('{"deepl_key": "k:fx", "font_size": 30}', encoding="utf-8")  # файл от старой версии
    s = AppSettings.load(str(path))
    assert s.theme == "system" and s.deepl_key == "k:fx"
