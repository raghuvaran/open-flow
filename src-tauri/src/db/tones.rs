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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn test_db() -> Connection {
        schema::init_db(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn set_and_get_tone() {
        let conn = test_db();
        set_tone(&conn, "com.apple.mail", "Mail", "email", "Professional tone").unwrap();
        assert_eq!(get_tone(&conn, "com.apple.mail").unwrap(), Some("Professional tone".into()));
    }

    #[test]
    fn get_tone_missing() {
        let conn = test_db();
        assert_eq!(get_tone(&conn, "com.unknown").unwrap(), None);
    }

    #[test]
    fn set_tone_overwrites() {
        let conn = test_db();
        set_tone(&conn, "com.slack", "Slack", "slack", "Casual").unwrap();
        set_tone(&conn, "com.slack", "Slack", "slack", "Very casual").unwrap();
        assert_eq!(get_tone(&conn, "com.slack").unwrap(), Some("Very casual".into()));
    }
}
