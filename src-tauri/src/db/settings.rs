use anyhow::Result;
use rusqlite::Connection;

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query_map([key], |row| row.get::<_, String>(0))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2",
        [key, value],
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
    fn set_and_get() {
        let conn = test_db();
        set(&conn, "key1", "value1").unwrap();
        assert_eq!(get(&conn, "key1").unwrap(), Some("value1".into()));
    }

    #[test]
    fn get_missing_key() {
        let conn = test_db();
        assert_eq!(get(&conn, "nonexistent").unwrap(), None);
    }

    #[test]
    fn set_overwrites() {
        let conn = test_db();
        set(&conn, "k", "v1").unwrap();
        set(&conn, "k", "v2").unwrap();
        assert_eq!(get(&conn, "k").unwrap(), Some("v2".into()));
    }

    #[test]
    fn multiple_keys() {
        let conn = test_db();
        set(&conn, "a", "1").unwrap();
        set(&conn, "b", "2").unwrap();
        assert_eq!(get(&conn, "a").unwrap(), Some("1".into()));
        assert_eq!(get(&conn, "b").unwrap(), Some("2".into()));
    }
}
