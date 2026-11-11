from noveltl.cleaner import split_name
from noveltl.langdetect import resolve_language
from noveltl.pipeline import TranslationPipeline, apply_glossary
from noveltl.translators import TranslatorManager
from test_translate import FakeSession


def test_split_name_not_a_name():
    assert split_name("Note: there is a door.") == ("", "Note: there is a door.")
    assert split_name("Warning: Do not enter.") == ("", "Warning: Do not enter.")
    assert split_name("Akari: hello there") == ("", "Akari: hello there")
    assert split_name("Yoshino: Good morning.") == ("Yoshino", "Good morning.")


def test_resolve_language():
    assert resolve_language("auto", "大丈夫？") is None
    assert resolve_language("zh", "大丈夫") == "zh"
    assert resolve_language("auto", "おはよう") == "ja"


def test_glossary_single_pass():
    g = [{"src": "Kevin", "dst": "Кевин"}, {"src": "Кевин", "dst": "Кэвин"}]
    assert apply_glossary("Kevin", g) == "Кевин"


def test_name_failure_keeps_text():
    class NameBroken(FakeSession):
        def get(self, url, params=None, timeout=None):
            if params["q"] == "将臣":
                raise OSError("no network")
            return super().get(url, params=params, timeout=timeout)

    p = TranslationPipeline(TranslatorManager("", session=NameBroken()), "ja")
    r = p.process("将臣", "おはよう")
    assert r.text == "G:おはよう"
    assert r.name == "将臣"
    assert r.notice == "Имя не удалось перевести — показано как есть."
