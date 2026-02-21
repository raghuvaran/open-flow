use anyhow::Result;
use rusqlite::Connection;

pub fn get_all(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT spoken_form, written_form FROM personal_dict")?;
    let entries = stmt
        .query_map([], |row| {
            let spoken: String = row.get(0)?;
            let written: String = row.get(1)?;
            Ok(format!("{} → {}", spoken, written))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(entries)
}

pub fn add(conn: &Connection, spoken: &str, written: &str, category: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO personal_dict (spoken_form, written_form, category) VALUES (?1, ?2, ?3)",
        [spoken, written, category],
    )?;
    Ok(())
}
