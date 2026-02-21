use anyhow::Result;
use rusqlite::Connection;

pub fn get_all(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT trigger_phrase, expansion FROM snippets")?;
    let entries = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

pub fn add(conn: &Connection, trigger: &str, expansion: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO snippets (trigger_phrase, expansion) VALUES (?1, ?2)",
        [trigger, expansion],
    )?;
    Ok(())
}
