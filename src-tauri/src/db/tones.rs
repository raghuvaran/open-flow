use anyhow::Result;
use rusqlite::Connection;

pub fn get_tone(conn: &Connection, bundle_id: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT tone_directive FROM app_tones WHERE bundle_id = ?1")?;
    let tone = stmt
        .query_row([bundle_id], |row| row.get(0))
        .ok();
    Ok(tone)
}

pub fn set_tone(conn: &Connection, bundle_id: &str, app_name: &str, category: &str, tone: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO app_tones (bundle_id, app_name, category, tone_directive) VALUES (?1, ?2, ?3, ?4)",
        [bundle_id, app_name, category, tone],
    )?;
    Ok(())
}
