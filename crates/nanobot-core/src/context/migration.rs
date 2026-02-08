use anyhow::Result;
use rusqlite::{Connection, params};
use std::path::PathBuf;

/// Migrate flat PersistenceManager sessions to ContextTree format
pub struct DataMigrator {
    old_db: PathBuf,
    new_db: PathBuf,
}

impl DataMigrator {
    pub fn new(old_db: PathBuf, new_db: PathBuf) -> Self {
        Self { old_db, new_db }
    }

    /// Migrate all sessions from old flat format to new tree format
    pub fn migrate(&self) -> Result<()> {
        // Open connections
        let old_conn = Connection::open(&self.old_db)?;
        let new_conn = Connection::open(&self.new_db)?;

        // Get all unique sessions from old DB
        let mut stmt = old_conn.prepare(
            "SELECT DISTINCT session_id FROM messages ORDER BY id ASC"
        )?;
        
        let sessions: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        eprintln!("📦 Found {} sessions to migrate", sessions.len());

        for session_id in sessions {
            self.migrate_session(&old_conn, &new_conn, &session_id)?;
        }

        eprintln!("✅ Migration complete!");
        Ok(())
    }

    fn migrate_session(
        &self,
        old_conn: &Connection,
        new_conn: &Connection,
        session_id: &str,
    ) -> Result<()> {
        // Get all messages for this session in chronological order
        let mut stmt = old_conn.prepare(
            "SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id ASC"
        )?;

        let messages: Vec<(String, String)> = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        eprintln!("  📝 Migrating session '{}' ({} messages)", session_id, messages.len());

        // Insert messages into ContextTree with parent linking
        let mut parent_id: Option<i64> = None;

        for (role, content) in messages {
            let stmt_str = if let Some(pid) = parent_id {
                "INSERT INTO context_tree (session_id, role, content, parent_id) 
                 VALUES (?1, ?2, ?3, ?4)"
            } else {
                "INSERT INTO context_tree (session_id, role, content) 
                 VALUES (?1, ?2, ?3)"
            };

            if let Some(pid) = parent_id {
                new_conn.execute(stmt_str, params![session_id, role, content, pid])?;
            } else {
                new_conn.execute(stmt_str, params![session_id, role, content])?;
            }

            // Update parent to point to the message we just inserted
            parent_id = Some(new_conn.last_insert_rowid());
        }

        Ok(())
    }

    /// Check if migration is needed
    pub fn needs_migration(&self) -> Result<bool> {
        // Check if old DB exists and has data
        if !self.old_db.exists() {
            return Ok(false);
        }

        let old_conn = Connection::open(&self.old_db)?;
        let count: i64 = old_conn.query_row(
            "SELECT COUNT(*) FROM messages",
            [],
            |row| row.get(0),
        )?;

        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_migration() -> Result<()> {
        let dir = tempdir()?;
        let old_db = dir.path().join("sessions.db");
        let new_db = dir.path().join("context_tree.db");

        // Setup old DB with test data
        let conn = Connection::open(&old_db)?;
        conn.execute(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY,
                session_id TEXT,
                role TEXT,
                content TEXT
            )",
            [],
        )?;
        conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES 
             ('test1', 'user', 'Hello'),
             ('test1', 'assistant', 'Hi there'),
             ('test1', 'user', 'How are you?')",
            [],
        )?;

        // Setup new DB schema
        let new_conn = Connection::open(&new_db)?;
        new_conn.execute(
            "CREATE TABLE context_tree (
                id INTEGER PRIMARY KEY,
                session_id TEXT,
                role TEXT,
                content TEXT,
                parent_id INTEGER
            )",
            [],
        )?;

        // Run migration
        let migrator = DataMigrator::new(old_db.clone(), new_db.clone());
        migrator.migrate()?;

        // Verify
        let count: i64 = new_conn.query_row(
            "SELECT COUNT(*) FROM context_tree",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 3);

        Ok(())
    }
}
