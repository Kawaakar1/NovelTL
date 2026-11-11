//! Кэш переводов в SQLite (отдельный файл на каждую игру).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

/// Ключ записи: текст + переводчик + язык перевода.
const NEW_SCHEMA: &str = "CREATE TABLE tr (\
     src TEXT NOT NULL, engine TEXT NOT NULL, target TEXT NOT NULL DEFAULT 'ru', \
     dst TEXT NOT NULL, ts REAL NOT NULL, PRIMARY KEY (src, engine, target))";

pub struct TranslationCache {
    db: Mutex<Connection>,
}

impl TranslationCache {
    fn init(db: Connection) -> rusqlite::Result<Self> {
        // есть ли уже колонка target (старые базы её не имеют)
        let has_target = {
            let mut st = db.prepare("PRAGMA table_info(tr)")?;
            let names: Vec<String> = st.query_map([], |r| r.get::<_, String>(1))?.filter_map(Result::ok).collect();
            (!names.is_empty()).then(|| names.iter().any(|n| n == "target"))
        };
        match has_target {
            Some(true) => {}
            None => db.execute_batch(NEW_SCHEMA)?,
            // миграция: старые записи — перевод на русский; PRIMARY KEY меняем пересозданием таблицы
            Some(false) => db.execute_batch(&format!(
                "BEGIN; ALTER TABLE tr RENAME TO tr_old; {NEW_SCHEMA}; \
                 INSERT INTO tr (src, engine, target, dst, ts) \
                 SELECT src, engine, 'ru', dst, ts FROM tr_old; \
                 DROP TABLE tr_old; COMMIT;"
            ))?,
        }
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

    pub fn get(&self, src: &str, engine: &str, target: &str) -> Option<String> {
        self.lock()
            .query_row(
                "SELECT dst FROM tr WHERE src = ?1 AND engine = ?2 AND target = ?3",
                rusqlite::params![src, engine, target],
                |r| r.get::<_, String>(0),
            )
            .ok()
    }

    /// Ошибки записи молча игнорируются.
    pub fn put(&self, src: &str, engine: &str, target: &str, dst: &str) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = self.lock().execute(
            "INSERT OR REPLACE INTO tr (src, engine, target, dst, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![src, engine, target, dst, ts],
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
        assert_eq!(c.get("テスト", "deepl", "ru"), None);
        c.put("テスト", "deepl", "ru", "тест");
        assert_eq!(c.get("テスト", "deepl", "ru").as_deref(), Some("тест"));
        assert_eq!(c.get("テスト", "other", "ru"), None);
        // тот же ключ заменяется, а не дублируется
        c.put("テスト", "deepl", "ru", "тест2");
        assert_eq!(c.count(), 1);
        assert_eq!(c.get("テスト", "deepl", "ru").as_deref(), Some("тест2"));
        c.clear();
        assert_eq!(c.count(), 0);
    }

    #[test]
    fn different_targets_are_separate() {
        let c = TranslationCache::in_memory().unwrap();
        c.put("テスト", "google", "ru", "тест");
        c.put("テスト", "google", "en", "test");
        assert_eq!(c.count(), 2);
        assert_eq!(c.get("テスト", "google", "ru").as_deref(), Some("тест"));
        assert_eq!(c.get("テスト", "google", "en").as_deref(), Some("test"));
        assert_eq!(c.get("テスト", "google", "ko"), None);
    }

    #[test]
    fn old_schema_is_migrated() {
        let dir = tmp_dir("mig");
        let path = dir.join("old.sqlite");
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE tr (src TEXT NOT NULL, engine TEXT NOT NULL, dst TEXT NOT NULL, \
                 ts REAL NOT NULL, PRIMARY KEY (src, engine)); \
                 INSERT INTO tr VALUES ('こんにちは', 'deepl', 'привет', 1.0);",
            )
            .unwrap();
        }
        {
            let c = TranslationCache::open(&path).unwrap();
            assert_eq!(c.count(), 1);
            assert_eq!(c.get("こんにちは", "deepl", "ru").as_deref(), Some("привет"));
            assert_eq!(c.get("こんにちは", "deepl", "en"), None);
            c.put("こんにちは", "deepl", "en", "hello");
            assert_eq!(c.count(), 2);
        }
        // повторное открытие уже новой базы ничего не ломает
        let c = TranslationCache::open(&path).unwrap();
        assert_eq!(c.count(), 2);
        assert_eq!(c.get("こんにちは", "deepl", "en").as_deref(), Some("hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_persists() {
        let dir = tmp_dir("cache");
        let path = dir.join("c.sqlite");
        {
            let c = TranslationCache::open(&path).unwrap();
            c.put("a", "deepl", "ru", "б");
        }
        assert_eq!(TranslationCache::open(&path).unwrap().get("a", "deepl", "ru").as_deref(), Some("б"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
