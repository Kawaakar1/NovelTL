import pytest

from noveltl.cleaner import (
    clean,
    collapse_repeated_chars,
    collapse_repeated_phrase,
    split_name,
    strip_tags,
)
from noveltl.langdetect import detect_language, resolve_language


def test_strip_kirikiri_tags():
    assert strip_tags("こんにちは[l][r]世界[p]") == "こんにちは\n世界"
    assert strip_tags("%f$ll;おはよう%p-1;") == "おはよう"
    assert strip_tags('[ruby text="よしの"]芳乃[np]') == "芳乃"
    assert strip_tags("100% sure; really") == "100% sure; really"


@pytest.mark.parametrize(
    "src,expected",
    [
        ("ここんんににちちはは", "こんにちは"),
        ("こここんんんにににちちちははは", "こんにちは"),
        ("ああああ", "ああああ"),  # одиночный повтор не трогаем
        ("こんにちは", "こんにちは"),
        ("HHeelllloo", "Hello"),
    ],
)
def test_collapse_repeated_chars(src, expected):
    assert collapse_repeated_chars(src) == expected


@pytest.mark.parametrize(
    "src,expected",
    [
        ("おはよう。おはよう。", "おはよう。"),
        ("今日はいい天気今日はいい天気", "今日はいい天気"),
        ("おはよう\nおはよう", "おはよう"),
        ("ドキドキ", "ドキドキ"),  # настоящее слово
        ("ははは", "ははは"),
        ("「うん」「うん」「うん」", "「うん」"),
    ],
)
def test_collapse_repeated_phrase(src, expected):
    assert collapse_repeated_phrase(src) == expected


@pytest.mark.parametrize(
    "src,name,text",
    [
        ("【芳乃】「おはようございます」", "芳乃", "「おはようございます」"),
        ("芳乃「おはようございます」", "芳乃", "「おはようございます」"),
        ("芳乃\n「おはようございます」", "芳乃", "「おはようございます」"),
        ("Yoshino: Good morning.", "Yoshino", "Good morning."),
        ("彼は「そう」と言った。", "", "彼は「そう」と言った。"),
        ("ただの地の文です。", "", "ただの地の文です。"),
        ("It was 12:30 already.", "", "It was 12:30 already."),
    ],
)
def test_split_name(src, name, text):
    assert split_name(src) == (name, text)


def test_clean_full():
    res = clean("【芳乃】%f$ll;「おはよう[r]ございます」[p]")
    assert (res.name, res.text) == ("芳乃", "「おはようございます」")


def test_clean_joins_lines():
    assert clean("今日は\nいい天気").text == "今日はいい天気"
    assert clean("It is\na good day.").text == "It is a good day."


def test_clean_doubled_with_name():
    res = clean("【【芳芳乃乃】】「「ははいい」」")
    assert (res.name, res.text) == ("芳乃", "「はい」")


def test_clean_empty():
    assert not clean("")
    assert not clean("[p][l]")


def test_detect_language():
    assert detect_language("おはよう、世界") == "ja"
    assert detect_language("你好，世界") == "zh"
    assert detect_language("Hello") == "en"
    assert detect_language("123 ...") is None
    assert resolve_language("zh", "おはよう") == "zh"
    assert resolve_language("auto", "おはよう") == "ja"


def test_whitespace_repeated_name():
    assert clean("Masaomi  Masaomi  Masaomi  ").text == "Masaomi"
    assert clean("Titty Ninja  Titty Ninja  ").text == "Titty Ninja"
    assert clean("Mako  Mako  ").text == "Mako"
