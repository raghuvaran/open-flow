use anyhow::Result;
use rusqlite::Connection;

pub fn record_usage(conn: &Connection, app_name: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO injection_history (raw_transcript, polished_text, app_name) VALUES ('', '', ?1)",
        [app_name],
    )?;
    Ok(())
}

/// Get top N most-used apps from the last 7 days.
pub fn top_apps(conn: &Connection, limit: usize) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT app_name, COUNT(*) as cnt FROM injection_history
         WHERE app_name IS NOT NULL AND app_name != '' AND app_name != 'Unknown'
         AND created_at > datetime('now', '-7 days')
         GROUP BY app_name ORDER BY cnt DESC LIMIT ?1"
    )?;
    let rows = stmt.query_map([limit], |row| row.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Get cached hint for an app (today only).
pub fn get_hint(conn: &Connection, app_name: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT hint FROM hint_cache WHERE app_name = ?1 AND generated_date = date('now')"
    )?;
    let mut rows = stmt.query_map([app_name], |row| row.get::<_, String>(0))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

/// Store a hint for an app (today).
pub fn save_hint(conn: &Connection, app_name: &str, hint: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO hint_cache (app_name, hint, generated_date) VALUES (?1, ?2, date('now'))",
        [app_name, hint],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn test_db() -> Connection {
        schema::init_db(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn save_and_get_hint() {
        let conn = test_db();
        save_hint(&conn, "Slack", "Try voice commands").unwrap();
        let h = get_hint(&conn, "Slack").unwrap();
        assert_eq!(h, Some("Try voice commands".into()));
    }

    #[test]
    fn get_hint_missing() {
        let conn = test_db();
        assert_eq!(get_hint(&conn, "NoApp").unwrap(), None);
    }

    #[test]
    fn save_hint_overwrites_same_day() {
        let conn = test_db();
        save_hint(&conn, "App", "hint1").unwrap();
        save_hint(&conn, "App", "hint2").unwrap();
        assert_eq!(get_hint(&conn, "App").unwrap(), Some("hint2".into()));
    }

    #[test]
    fn record_usage_and_top_apps() {
        let conn = test_db();
        for _ in 0..5 { record_usage(&conn, "Slack").unwrap(); }
        for _ in 0..3 { record_usage(&conn, "Safari").unwrap(); }
        record_usage(&conn, "Notes").unwrap();
        let top = top_apps(&conn, 2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0], "Slack");
        assert_eq!(top[1], "Safari");
    }

    #[test]
    fn top_apps_excludes_unknown() {
        let conn = test_db();
        record_usage(&conn, "Unknown").unwrap();
        record_usage(&conn, "Slack").unwrap();
        let top = top_apps(&conn, 10).unwrap();
        assert!(!top.contains(&"Unknown".into()));
        assert!(top.contains(&"Slack".into()));
    }

    #[test]
    fn top_apps_empty() {
        let conn = test_db();
        assert!(top_apps(&conn, 5).unwrap().is_empty());
    }
}
