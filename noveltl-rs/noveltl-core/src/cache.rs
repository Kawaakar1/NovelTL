//! Кэш переводов в SQLite (отдельный файл на каждую игру).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub struct TranslationCache {
    db: Mutex<Connection>,
}

impl TranslationCache {
    fn init(db: Connection) -> rusqlite::Result<Self> {
        // схема в точности как в Python-версии
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS tr (\
             src TEXT NOT NULL, engine TEXT NOT NULL, dst TEXT NOT NULL, ts REAL NOT NULL, \
             PRIMARY KEY (src, engine))",
        )?;
        Ok(TranslationCache { db: Mutex::new(db) })
    }

    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, src: &str, engine: &str) -> Option<String> {
        self.lock()
            .query_row(
                "SELECT dst FROM tr WHERE src = ?1 AND engine = ?2",
                rusqlite::params![src, engine],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    /// Ошибки записи молча игнорируются.
    pub fn put(&self, src: &str, engine: &str, dst: &str) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = self.lock().execute(
            "INSERT OR REPLACE INTO tr (src, engine, dst, ts) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![src, engine, dst, ts],
        );
    }

    pub fn count(&self) -> usize {
        self.lock()
            .query_row("SELECT COUNT(*) FROM tr", [], |r| r.get::<_, i64>(0))
            .map(|n| n.max(0) as usize)
            .unwrap_or(0)
    }

    pub fn clear(&self) {
        let _ = self.lock().execute("DELETE FROM tr", []);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let d = std::env::temp_dir().join(format!(
            "noveltl_{}_{}_{}_{}",
            tag,
            std::process::id(),
            nanos,
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn send_sync() {
        fn is<T: Send + Sync>() {}
        is::<TranslationCache>();
    }

    #[test]
    fn put_get_count_clear() {
        let c = TranslationCache::in_memory().unwrap();
        assert_eq!(c.get("テスト", "deepl"), None);
        c.put("テスト", "deepl", "тест");
        assert_eq!(c.get("テスト", "deepl").as_deref(), Some("тест"));
        assert_eq!(c.get("テスト", "other"), None);
        // тот же ключ заменяется, а не дублируется
        c.put("テスト", "deepl", "тест2");
        assert_eq!(c.count(), 1);
        assert_eq!(c.get("テスト", "deepl").as_deref(), Some("тест2"));
        c.clear();
        assert_eq!(c.count(), 0);
    }

    #[test]
    fn cache_persists() {
        let dir = tmp_dir("cache");
        let path = dir.join("c.sqlite");
        {
            let c = TranslationCache::open(&path).unwrap();
            c.put("a", "deepl", "б");
        }
        assert_eq!(TranslationCache::open(&path).unwrap().get("a", "deepl").as_deref(), Some("б"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
