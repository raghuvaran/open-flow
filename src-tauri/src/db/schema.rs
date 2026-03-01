use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub fn init_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS personal_dict (
            id INTEGER PRIMARY KEY,
            spoken_form TEXT NOT NULL,
            written_form TEXT NOT NULL,
            category TEXT DEFAULT 'general',
            usage_count INTEGER DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS app_tones (
            id INTEGER PRIMARY KEY,
            bundle_id TEXT NOT NULL UNIQUE,
            app_name TEXT NOT NULL,
            category TEXT NOT NULL,
            tone_directive TEXT NOT NULL,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS injection_history (
            id INTEGER PRIMARY KEY,
            raw_transcript TEXT NOT NULL,
            polished_text TEXT NOT NULL,
            app_bundle_id TEXT,
            app_name TEXT,
            language TEXT,
            latency_ms INTEGER,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS snippets (
            id INTEGER PRIMARY KEY,
            trigger_phrase TEXT NOT NULL,
            expansion TEXT NOT NULL,
            match_type TEXT DEFAULT 'fuzzy',
            usage_count INTEGER DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS model_config (
            id INTEGER PRIMARY KEY,
            model_type TEXT NOT NULL,
            model_name TEXT NOT NULL,
            model_path TEXT NOT NULL,
            quantization TEXT,
            is_active INTEGER DEFAULT 0,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hint_cache (
            app_name TEXT NOT NULL,
            hint TEXT NOT NULL,
            generated_date TEXT NOT NULL,
            PRIMARY KEY (app_name, generated_date)
        );
        ",
    )?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        init_db(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn init_creates_all_tables() {
        let conn = test_db();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(tables.contains(&"personal_dict".into()));
        assert!(tables.contains(&"app_tones".into()));
        assert!(tables.contains(&"injection_history".into()));
        assert!(tables.contains(&"snippets".into()));
        assert!(tables.contains(&"model_config".into()));
        assert!(tables.contains(&"settings".into()));
        assert!(tables.contains(&"hint_cache".into()));
    }

    #[test]
    fn init_is_idempotent() {
        let conn = test_db();
        // Second init on same connection should not fail
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
        ).unwrap();
    }

    #[test]
    fn init_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sub/dir/test.db");
        let conn = init_db(&db_path).unwrap();
        assert!(db_path.exists());
        drop(conn);
    }
}
