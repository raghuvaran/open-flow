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
