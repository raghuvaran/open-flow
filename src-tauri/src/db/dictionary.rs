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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn test_db() -> Connection {
        schema::init_db(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn add_and_get_all() {
        let conn = test_db();
        add(&conn, "kubernetes", "Kubernetes", "general").unwrap();
        add(&conn, "grpc", "gRPC", "general").unwrap();
        let all = get_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].contains("kubernetes"));
        assert!(all[0].contains("Kubernetes"));
        assert!(all[1].contains("gRPC"));
    }

    #[test]
    fn get_all_empty() {
        let conn = test_db();
        assert!(get_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn format_is_arrow() {
        let conn = test_db();
        add(&conn, "k8s", "Kubernetes", "tech").unwrap();
        let all = get_all(&conn).unwrap();
        assert_eq!(all[0], "k8s → Kubernetes");
    }
}
