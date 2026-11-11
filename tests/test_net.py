import pytest

from noveltl import net
from noveltl.profiles import AppSettings
from noveltl.translators import TranslatorManager

from tests.test_translate import FakeSession


@pytest.mark.parametrize("src,expected", [
    ("127.0.0.1:8080", "http://127.0.0.1:8080"),
    ("socks5://127.0.0.1:1080", "socks5h://127.0.0.1:1080"),
    ("  http://user:pass@host:3128 ", "http://user:pass@host:3128"),
])
def test_normalize_proxy_ok(src, expected):
    assert net.normalize_proxy(src) == expected


@pytest.mark.parametrize("src", ["", "ftp://h:1", "socks5://h", "http://:80", "http://h:99999"])
def test_normalize_proxy_errors(src):
    with pytest.raises(ValueError):
        net.normalize_proxy(src)


def test_make_session_modes():
    s = net.make_session("system")
    assert s.trust_env is True and not s.proxies
    assert net.make_session("direct").trust_env is False
    p = net.make_session("proxy", "socks5://127.0.0.1:1080")
    assert p.proxies == {"http": "socks5h://127.0.0.1:1080", "https": "socks5h://127.0.0.1:1080"}
    with pytest.raises(ValueError):
        net.make_session("proxy", "bad")


def test_make_session_bind_ip():
    s = net.make_session("direct", bind_ip="127.0.0.1")
    ad = s.get_adapter("https://example.com")
    assert isinstance(ad, net.SourceAddressAdapter) and ad.source_ip == "127.0.0.1"


def test_proxy_is_really_used():
    ok, text = net.check_google(net.make_session("proxy", "http://127.0.0.1:9"))
    assert ok is False and "прокси" in text


def test_bind_to_missing_address_does_not_crash():
    ok, text = net.check_google(net.make_session("direct", bind_ip="10.255.255.254"))
    assert ok is False and text


def test_manager_set_session():
    m = TranslatorManager("key", session=FakeSession())
    new = FakeSession()
    m.set_session(new)
    assert m.google.session is new and m.deepl.session is new


def test_settings_old_file_has_net_defaults(tmp_path):
    path = tmp_path / "settings.json"
    path.write_text('{"deepl_key": "k"}', encoding="utf-8")
    s = AppSettings.load(str(path))
    assert s.net_mode == "system" and s.bind_ip == "" and s.proxy_url == ""


# ---------- логин и пароль прокси ----------
import base64
import socket
import threading


def test_build_proxy_url_basic():
    assert net.build_proxy_url("127.0.0.1:1080", "", "") == "http://127.0.0.1:1080"
    assert net.build_proxy_url("socks5://h:1080", "user", "p@ss:w/rd") == "socks5h://user:p%40ss%3Aw%2Frd@h:1080"
    assert net.build_proxy_url("socks5://h:1080", "user") == "socks5h://user@h:1080"


def test_build_proxy_url_fields_replace_address_data():
    assert net.build_proxy_url("http://old:x@h:3128", "new", "y") == "http://new:y@h:3128"
    assert net.build_proxy_url("http://old:x@h:3128") == "http://old:x@h:3128"


def test_build_proxy_url_errors():
    with pytest.raises(ValueError):
        net.build_proxy_url("http://h:1", "", "secret")
    with pytest.raises(ValueError):
        net.build_proxy_url("socks4://h:1080", "user", "secret")


def test_build_proxy_url_cyrillic_roundtrip():
    url = net.build_proxy_url("socks5://h:1080", "юзер", "пароль:1 @")
    assert "юзер" not in url
    addr, user, pw = net.split_proxy_credentials(url)
    assert (addr, user, pw) == ("socks5h://h:1080", "юзер", "пароль:1 @")


def test_split_proxy_credentials():
    assert net.split_proxy_credentials("http://h:1") == ("http://h:1", "", "")
    assert net.split_proxy_credentials("http://u:p@h:1") == ("http://h:1", "u", "p")
    assert net.split_proxy_credentials("http://u%40x:p%3A%2F@h:1") == ("http://h:1", "u@x", "p:/")
    assert net.split_proxy_credentials("") == ("", "", "")


def _serve(handler):
    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(5)
    srv.settimeout(5)

    def loop():
        while True:
            try:
                conn, _ = srv.accept()
            except OSError:
                return
            conn.settimeout(3)
            try:
                handler(conn)
            except Exception:
                pass
            finally:
                conn.close()

    threading.Thread(target=loop, daemon=True).start()
    return srv, srv.getsockname()[1]


def _recvn(conn, n):
    data = b""
    while len(data) < n:
        chunk = conn.recv(n - len(data))
        if not chunk:
            raise OSError("closed")
        data += chunk
    return data


def test_socks5_credentials_reach_server_decoded():
    got = []
    expected = ("юзер".encode("utf-8"), "p@ss:w/rd".encode("utf-8"))

    def handler(conn):
        _, n = _recvn(conn, 2)
        _recvn(conn, n)
        conn.sendall(b"\x05\x02")
        _recvn(conn, 1)
        user = _recvn(conn, _recvn(conn, 1)[0])
        pw = _recvn(conn, _recvn(conn, 1)[0])
        got.append((user, pw))
        if (user, pw) == expected:
            conn.sendall(b"\x01\x00")
            conn.recv(256)
        else:
            conn.sendall(b"\x01\x01")

    srv, port = _serve(handler)
    try:
        session = net.make_session("proxy", f"socks5://127.0.0.1:{port}", "", "юзер", "p@ss:w/rd")
        ok, text = net.check_google(session)
        assert got[-1] == expected
        assert "p@ss" not in text and "%40" not in text
        session = net.make_session("proxy", f"socks5://127.0.0.1:{port}", "", "юзер", "wrong")
        assert net.check_google(session) == (False, "Прокси не принял логин или пароль")
    finally:
        srv.close()


def test_http_proxy_basic_auth():
    got = []

    def handler(conn):
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = conn.recv(4096)
            if not chunk:
                return
            data += chunk
        creds = ""
        for line in data.split(b"\r\n"):
            if line.lower().startswith(b"proxy-authorization:"):
                token = line.split(b" ", 2)[2]
                creds = base64.b64decode(token).decode("utf-8")
        got.append(creds)
        if creds == "user:p@ss:w/rd":
            conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
        else:
            conn.sendall(b"HTTP/1.1 407 Proxy Authentication Required\r\n"
                         b"Proxy-Authenticate: Basic\r\nContent-Length: 0\r\n\r\n")

    srv, port = _serve(handler)
    try:
        session = net.make_session("proxy", f"http://127.0.0.1:{port}", "", "user", "p@ss:w/rd")
        ok, text = net.check_google(session)
        assert got[-1] == "user:p@ss:w/rd"
        assert "p@ss" not in text
        session = net.make_session("proxy", f"http://127.0.0.1:{port}", "", "user", "wrong")
        assert net.check_google(session) == (False, "Прокси не принял логин или пароль")
        # кириллица в логине и пароле уходит в UTF-8, а не роняет запрос
        session = net.make_session("proxy", f"http://127.0.0.1:{port}", "", "юзер", "пароль@1")
        assert net.check_google(session) == (False, "Прокси не принял логин или пароль")
        assert got[-1] == "юзер:пароль@1"
    finally:
        srv.close()


def test_settings_old_file_has_no_proxy_credentials(tmp_path):
    path = tmp_path / "settings.json"
    path.write_text('{"deepl_key": "k", "proxy_url": "http://h:1"}', encoding="utf-8")
    s = AppSettings.load(str(path))
    assert s.proxy_user == "" and s.proxy_pass == ""
