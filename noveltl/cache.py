"""Кэш переводов в SQLite (отдельный файл на каждую игру)."""
from __future__ import annotations

import sqlite3
import threading
import time


class TranslationCache:
    def __init__(self, path: str = ":memory:") -> None:
        self._lock = threading.Lock()
        self._db = sqlite3.connect(path, check_same_thread=False)
        self._db.execute(
            "CREATE TABLE IF NOT EXISTS tr ("
            "src TEXT NOT NULL, engine TEXT NOT NULL, dst TEXT NOT NULL, ts REAL NOT NULL, "
            "PRIMARY KEY (src, engine))"
        )
        self._db.commit()

    def get(self, src: str, engine: str) -> str | None:
        with self._lock:
            row = self._db.execute(
                "SELECT dst FROM tr WHERE src = ? AND engine = ?", (src, engine)
            ).fetchone()
        return row[0] if row else None

    def put(self, src: str, engine: str, dst: str) -> None:
        with self._lock:
            self._db.execute(
                "INSERT OR REPLACE INTO tr (src, engine, dst, ts) VALUES (?, ?, ?, ?)",
                (src, engine, dst, time.time()),
            )
            self._db.commit()

    def count(self) -> int:
        with self._lock:
            return self._db.execute("SELECT COUNT(*) FROM tr").fetchone()[0]

    def clear(self) -> None:
        with self._lock:
            self._db.execute("DELETE FROM tr")
            self._db.commit()

    def close(self) -> None:
        with self._lock:
            self._db.close()
