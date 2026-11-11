"""Сетевые настройки перевода: прокси, прямое подключение, выбор интерфейса."""
from __future__ import annotations

import base64
import socket
from urllib.parse import quote, unquote, urlsplit, urlunsplit

import requests
from requests.adapters import HTTPAdapter
from requests.utils import get_auth_from_url

NET_SYSTEM = "system"
NET_DIRECT = "direct"
NET_PROXY = "proxy"
NET_TITLES = {
    NET_SYSTEM: "Как в системе",
    NET_DIRECT: "Напрямую, без прокси",
    NET_PROXY: "Через прокси",
}

_PROXY_ERROR = "Неверный адрес прокси. Пример: socks5://127.0.0.1:1080 или http://127.0.0.1:8080"
_AUTH_ERROR = "Прокси не принял логин или пароль"
_SCHEMES ={"http": "http", "https": "https", "socks5": "socks5h", "socks5h": "socks5h",
            "socks4": "socks4a", "socks4a": "socks4a"}


def normalize_proxy(text: str) -> str:
    text = (text or "").strip()
    if not text:
        raise ValueError("Укажите адрес прокси")
    if "://" not in text:
        text = "http://" + text
    try:
        parts = urlsplit(text)
        scheme = parts.scheme.lower()
        host = parts.hostname
        port = parts.port
    except ValueError:
        raise ValueError(_PROXY_ERROR) from None
    if scheme not in _SCHEMES or not host or port is None or not (1 <= port <= 65535):
        raise ValueError(_PROXY_ERROR)
    return f"{_SCHEMES[scheme]}://{parts.netloc}"


def build_proxy_url(address: str, user: str = "", password: str = "") -> str:
    base = normalize_proxy(address)
    user = (user or "").strip()
    password = password or ""
    if not user and password:
        raise ValueError("Укажите логин прокси")
    if not user:
        return base
    parts = urlsplit(base)
    if parts.scheme == "socks4a" and password:
        raise ValueError("SOCKS4 не поддерживает пароль — используйте SOCKS5")
    host = parts.hostname or ""
    if ":" in host:
        host = f"[{host}]"
    cred = quote(user, safe="") + (":" + quote(password, safe="") if password else "")
    return urlunsplit((parts.scheme, f"{cred}@{host}:{parts.port}", parts.path, parts.query, parts.fragment))


def split_proxy_credentials(url: str) -> tuple[str, str, str]:
    try:
        text = (url or "").strip()
        parts = urlsplit(text)
        if "@" not in parts.netloc:
            return url, "", ""
        userinfo, _, hostport = parts.netloc.rpartition("@")
        user, _, password = userinfo.partition(":")
        clean = urlunsplit((parts.scheme, hostport, parts.path, parts.query, parts.fragment))
        return clean, unquote(user), unquote(password)
    except Exception:
        return url, "", ""


def list_interfaces() -> list[tuple[str, str]]:
    try:
        import psutil

        stats = psutil.net_if_stats()
        result: list[tuple[str, str]] = []
        for name, addrs in psutil.net_if_addrs().items():
            st = stats.get(name)
            if not st or not st.isup:
                continue
            for a in addrs:
                if a.family != socket.AF_INET:
                    continue
                ip = a.address
                if ip.startswith("127.") or ip.startswith("169.254."):
                    continue
                result.append((name, ip))
        return result
    except Exception:
        return []


class SourceAddressAdapter(HTTPAdapter):
    """Привязка к локальному IP (если задан) и авторизация на HTTP-прокси в UTF-8.

    requests сам кодирует логин и пароль прокси в latin-1, и кириллица роняет запрос.
    """

    def __init__(self, source_ip: str = "", *args, **kwargs) -> None:
        self.source_ip = source_ip
        super().__init__(*args, **kwargs)

    def init_poolmanager(self, *args, **kwargs):
        if self.source_ip:
            kwargs["source_address"] = (self.source_ip, 0)
        super().init_poolmanager(*args, **kwargs)

    def proxy_manager_for(self, proxy, **proxy_kwargs):
        if self.source_ip:
            proxy_kwargs["source_address"] = (self.source_ip, 0)
        return super().proxy_manager_for(proxy, **proxy_kwargs)

    def proxy_headers(self, proxy):
        user, password = get_auth_from_url(proxy)
        if not user:
            return {}
        token = base64.b64encode(f"{user}:{password}".encode("utf-8")).decode("ascii")
        return {"Proxy-Authorization": "Basic " + token}


def make_session(mode: str, proxy_url: str = "", bind_ip: str = "",
                 proxy_user: str = "", proxy_pass: str = "") -> requests.Session:
    session = requests.Session()
    if mode == NET_DIRECT:
        session.trust_env = False
    elif mode == NET_PROXY:
        url = build_proxy_url(proxy_url, proxy_user, proxy_pass)
        session.trust_env = False
        session.proxies = {"http": url, "https": url}
    if bind_ip or mode == NET_PROXY:
        adapter = SourceAddressAdapter(bind_ip)
        session.mount("http://", adapter)
        session.mount("https://", adapter)
    return session


def check_google(session) -> tuple[bool, str]:
    try:
        resp = session.get(
            "https://translate.googleapis.com/translate_a/single",
            params={"client": "gtx", "sl": "en", "tl": "ru", "dt": "t", "q": "test"},
            timeout=10,
        )
    except requests.exceptions.ProxyError as e:
        if "407" in str(e) or "authentication" in str(e).lower():
            return False, _AUTH_ERROR
        return False, "Не удалось подключиться к прокси. Проверьте адрес и что прокси запущен"
    except requests.exceptions.ConnectionError as e:
        text = str(e)
        if "authentication" in text.lower():
            return False, _AUTH_ERROR
        if "10013" in text:
            return False, ("Выход через этот интерфейс запрещён системой. Обычно так делает VPN "
                           "с защитой от утечек (kill switch) — выключите её в настройках VPN или используйте прокси")
        if "10049" in text:
            return False, "Этого сетевого адреса сейчас нет на компьютере — выберите другой интерфейс"
        return False, "Нет соединения с Google. Проверьте интернет, прокси или выбранный интерфейс"
    except Exception as e:
        return False, f"Ошибка: {type(e).__name__}"
    if resp.status_code == 407:
        return False, _AUTH_ERROR
    if resp.status_code == 200:
        return True, "Google отвечает, перевод работает"
    if resp.status_code == 429:
        return False, "Google ограничил запросы с этого адреса (код 429). Попробуйте другой интерфейс или прокси"
    return False, f"Google ответил кодом {resp.status_code}"
