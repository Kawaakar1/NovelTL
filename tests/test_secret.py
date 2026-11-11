import base64
import json

from noveltl import secret
from noveltl.profiles import AppSettings


def test_roundtrip():
    for plain in ("ключ:fx", "секрет-ключ:fx"):
        stored = secret.protect(plain)
        assert stored.startswith(secret.PREFIX)
        assert plain not in stored
        assert secret.unprotect(stored) == plain


def test_empty_and_plain():
    assert secret.protect("") == ""
    assert secret.unprotect("plain") == "plain"


def test_bad_input():
    assert secret.unprotect("dpapi:!!!") == ""
    bad = "dpapi:" + base64.b64encode(b"\x01\x02\x03").decode("ascii")
    assert secret.unprotect(bad) == ""


def test_settings_save_load(tmp_path):
    path = str(tmp_path / "settings.json")
    s = AppSettings(deepl_key="секрет-ключ:fx", proxy_pass="пароль123")
    s.save(path)
    raw = open(path, "r", encoding="utf-8").read()
    assert "секрет-ключ:fx" not in raw
    assert "пароль123" not in raw
    data = json.loads(raw)
    assert data["deepl_key"].startswith("dpapi:")
    assert data["proxy_pass"].startswith("dpapi:")
    loaded = AppSettings.load(path)
    assert loaded.deepl_key == "секрет-ключ:fx"
    assert loaded.proxy_pass == "пароль123"
    assert s.deepl_key == "секрет-ключ:fx"


def test_legacy_plain_file(tmp_path):
    path = tmp_path / "settings.json"
    path.write_text(json.dumps({"deepl_key": "k:fx"}), encoding="utf-8")
    assert AppSettings.load(str(path)).deepl_key == "k:fx"
