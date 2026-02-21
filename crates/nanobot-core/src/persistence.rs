use anyhow::Result;
use rig::completion::message::{AssistantContent, Text, UserContent};
use rig::completion::Message;
use rig::OneOrMany;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::path::PathBuf;

// Global connection (simplified for single-process use)
// In a production server, we might want r2d2 connection pooling,
// but for this agent gateway, a single connection protected by Mutex is acceptable
// or we can just open a connection per request (sqlite is fast).
// Let's use opening per operation for simplicity and thread safety without global state complexity.

pub struct PersistenceManager {
    db_path: PathBuf,
}

impl PersistenceManager {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub fn init(&self) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;

        // Optimize for performance
        // PRAGMA journal_mode returns a row ("wal"), so we can't use execute.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL;", [], |row| row.get(0))?;
        conn.execute("PRAGMA synchronous = NORMAL;", [])?;

        // Sessions table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Messages table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                request_id TEXT,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            )",
            [],
        )?;

        conn.execute("ALTER TABLE messages ADD COLUMN request_id TEXT", [])
            .ok();

        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_session_role_request
             ON messages(session_id, role, request_id)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS message_request_commits (
                session_id TEXT NOT NULL,
                request_id TEXT NOT NULL,
                role TEXT NOT NULL,
                committed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY(session_id, request_id, role)
            )",
            [],
        )?;

        // Index for faster history retrieval
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id)",
            [],
        )?;

        // Extend sessions for multi-agent support (safe to add columns if not exists)
        conn.execute("ALTER TABLE sessions ADD COLUMN parent_session_id TEXT", [])
            .ok(); // Ignore if column exists

        conn.execute(
            "ALTER TABLE sessions ADD COLUMN session_type TEXT DEFAULT 'main'",
            [],
        )
        .ok();

        conn.execute(
            "ALTER TABLE sessions ADD COLUMN cleanup_policy TEXT DEFAULT 'keep'",
            [],
        )
        .ok();

        // Cron jobs table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS cron_jobs (
                id TEXT PRIMARY KEY,
                name TEXT,
                schedule_kind TEXT NOT NULL,
                schedule_data TEXT NOT NULL,
                payload_kind TEXT NOT NULL,
                payload_data TEXT NOT NULL,
                session_target TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        // Session tasks (for multi-agent delegation)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS session_tasks (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                task TEXT NOT NULL,
                status TEXT NOT NULL,
                result TEXT,
                created_at INTEGER NOT NULL,
                completed_at INTEGER,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            )",
            [],
        )?;

        Self::validate_schema_conn(&conn)?;

        Ok(())
    }

    pub fn validate_schema(&self) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        Self::validate_schema_conn(&conn)
    }

    fn table_exists(conn: &Connection, table_name: &str) -> Result<bool> {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                params![table_name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    fn index_exists(conn: &Connection, index_name: &str) -> Result<bool> {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1 LIMIT 1",
                params![index_name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
        let pragma = format!("PRAGMA table_info({})", table);
        let mut stmt = conn.prepare(&pragma)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn validate_schema_conn(conn: &Connection) -> Result<()> {
        for table in [
            "sessions",
            "messages",
            "message_request_commits",
            "cron_jobs",
            "session_tasks",
        ] {
            if !Self::table_exists(conn, table)? {
                anyhow::bail!(
                    "persistence schema invalid: missing required table '{}'",
                    table
                );
            }
        }

        for column in [
            "id",
            "session_id",
            "role",
            "request_id",
            "content",
            "created_at",
        ] {
            if !Self::column_exists(conn, "messages", column)? {
                anyhow::bail!(
                    "persistence schema invalid: missing required column 'messages.{}'",
                    column
                );
            }
        }

        if !Self::index_exists(conn, "idx_messages_session")? {
            anyhow::bail!(
                "persistence schema invalid: missing required index 'idx_messages_session'"
            );
        }

        if !Self::index_exists(conn, "idx_messages_session_role_request")? {
            anyhow::bail!(
                "persistence schema invalid: missing required index 'idx_messages_session_role_request'"
            );
        }

        Ok(())
    }

    pub fn ensure_session(&self, session_id: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO sessions (id) VALUES (?1) ON CONFLICT(id) DO NOTHING",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn ensure_session_tx(tx: &Transaction, session_id: &str) -> Result<()> {
        tx.execute(
            "INSERT INTO sessions (id) VALUES (?1) ON CONFLICT(id) DO NOTHING",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn save_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        // Ensure session exists first
        self.ensure_session(session_id)?;

        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
            params![session_id, role, content],
        )?;
        Ok(())
    }

    pub fn save_message_for_request(
        &self,
        session_id: &str,
        role: &str,
        request_id: &str,
        content: &str,
    ) -> Result<bool> {
        let mut conn = Connection::open(&self.db_path)?;
        let tx = conn.transaction()?;
        let committed =
            Self::save_message_tx_for_request(&tx, session_id, role, request_id, content)?;
        tx.commit()?;
        Ok(committed)
    }

    pub fn save_message_tx(
        tx: &Transaction,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<()> {
        Self::ensure_session_tx(tx, session_id)?;
        tx.execute(
            "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
            params![session_id, role, content],
        )?;
        Ok(())
    }

    pub fn save_message_tx_for_request(
        tx: &Transaction,
        session_id: &str,
        role: &str,
        request_id: &str,
        content: &str,
    ) -> Result<bool> {
        Self::ensure_session_tx(tx, session_id)?;

        if role == "user" {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT content FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3 LIMIT 1",
                    params![session_id, role, request_id],
                    |row| row.get(0),
                )
                .optional()?;

            if let Some(existing_content) = existing {
                if existing_content != content {
                    anyhow::bail!(
                        "request_id_content_mismatch for session={} request_id={} role=user",
                        session_id,
                        request_id
                    );
                }

                tx.execute(
                    "INSERT INTO message_request_commits (session_id, request_id, role)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(session_id, request_id, role) DO NOTHING",
                    params![session_id, request_id, role],
                )?;
                return Ok(false);
            }

            tx.execute(
                "INSERT INTO messages (session_id, role, request_id, content)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session_id, role, request_id, content],
            )?;

            let inserted = tx.execute(
                "INSERT INTO message_request_commits (session_id, request_id, role)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(session_id, request_id, role) DO NOTHING",
                params![session_id, request_id, role],
            )?;
            return Ok(inserted > 0);
        }

        tx.execute(
            "INSERT INTO messages (session_id, role, request_id, content)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, role, request_id)
             DO UPDATE SET content = excluded.content",
            params![session_id, role, request_id, content],
        )?;

        let inserted = tx.execute(
            "INSERT INTO message_request_commits (session_id, request_id, role)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id, request_id, role) DO NOTHING",
            params![session_id, request_id, role],
        )?;
        Ok(inserted > 0)
    }

    pub fn start_message(&self, session_id: &str, role: &str) -> Result<i64> {
        self.ensure_session(session_id)?;

        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO messages (session_id, role, content) VALUES (?1, ?2, ?3)",
            params![session_id, role, ""],
        )?;

        Ok(conn.last_insert_rowid())
    }

    pub fn start_message_for_request(
        &self,
        session_id: &str,
        role: &str,
        request_id: &str,
    ) -> Result<i64> {
        self.ensure_session(session_id)?;

        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO messages (session_id, role, request_id, content)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, role, request_id) DO NOTHING",
            params![session_id, role, request_id, ""],
        )?;

        let id: i64 = conn.query_row(
            "SELECT id FROM messages
             WHERE session_id = ?1 AND role = ?2 AND request_id = ?3
             ORDER BY id DESC LIMIT 1",
            params![session_id, role, request_id],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn append_message_content(&self, message_id: i64, chunk: &str) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        conn.execute(
            "UPDATE messages SET content = content || ?1 WHERE id = ?2",
            params![chunk, message_id],
        )?;
        Ok(())
    }

    pub fn get_history(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = Connection::open(&self.db_path)?;
        let mut stmt = conn
            .prepare("SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id ASC")?;

        let message_iter = stmt.query_map(params![session_id], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok((role, content))
        })?;

        let mut messages = Vec::new();
        for msg in message_iter {
            let (role, content) = msg?;
            match role.as_str() {
                "user" => {
                    messages.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text { text: content })),
                    });
                }
                "assistant" => {
                    messages.push(Message::Assistant {
                        id: None, // We don't store ID yet, Rig generates one if needed or we ignore
                        content: OneOrMany::one(AssistantContent::Text(Text { text: content })),
                    });
                }
                _ => {
                    // Ignore unknown roles or map to system?
                    // For now, treat as user text (or skip)
                }
            }
        }

        Ok(messages)
    }

    pub fn get_session_stats(&self, session_id: &str) -> Result<(i64, Option<String>)> {
        let conn = Connection::open(&self.db_path)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;

        let last_created: Option<String> = conn
            .query_row(
                "SELECT created_at FROM messages WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;

        Ok((count, last_created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        path.push(format!("nanobot-persistence-{name}-{nonce}.db"));
        path
    }

    #[test]
    fn save_message_tx_for_request_is_idempotent() {
        let db_path = temp_db_path("idempotent");
        let pm = PersistenceManager::new(db_path.clone());
        pm.init().expect("init should succeed");

        let mut conn = Connection::open(&db_path).expect("open db");
        let tx = conn.transaction().expect("start tx");
        let first = PersistenceManager::save_message_tx_for_request(
            &tx,
            "s1",
            "assistant",
            "req-1",
            "first",
        )
        .expect("first save should work");
        tx.commit().expect("commit first");
        assert!(first, "first commit should be marked new");

        let mut conn = Connection::open(&db_path).expect("open db second");
        let tx = conn.transaction().expect("start tx second");
        let second = PersistenceManager::save_message_tx_for_request(
            &tx,
            "s1",
            "assistant",
            "req-1",
            "final",
        )
        .expect("second save should work");
        tx.commit().expect("commit second");
        assert!(!second, "second commit should be treated as duplicate");

        let conn = Connection::open(&db_path).expect("open db read");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s1", "assistant", "req-1"],
                |row| row.get(0),
            )
            .expect("count query should work");
        assert_eq!(count, 1, "idempotent write should keep single row");

        let content: String = conn
            .query_row(
                "SELECT content FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s1", "assistant", "req-1"],
                |row| row.get(0),
            )
            .expect("content query should work");
        assert_eq!(content, "final", "second write should finalize content");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn start_message_for_request_reuses_existing_row() {
        let db_path = temp_db_path("start-reuse");
        let pm = PersistenceManager::new(db_path.clone());
        pm.init().expect("init should succeed");

        let id1 = pm
            .start_message_for_request("s2", "assistant", "req-stream")
            .expect("start row should succeed");
        let id2 = pm
            .start_message_for_request("s2", "assistant", "req-stream")
            .expect("start row repeat should succeed");
        assert_eq!(id1, id2, "stream start should reuse same request row");

        let conn = Connection::open(&db_path).expect("open db read");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s2", "assistant", "req-stream"],
                |row| row.get(0),
            )
            .expect("count query should work");
        assert_eq!(count, 1, "stream row should not duplicate");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn stream_partial_then_finalize_keeps_single_final_row() {
        let db_path = temp_db_path("stream-finalize");
        let pm = PersistenceManager::new(db_path.clone());
        pm.init().expect("init should succeed");

        let msg_id = pm
            .start_message_for_request("s3", "assistant", "req-final")
            .expect("stream start should succeed");
        pm.append_message_content(msg_id, "partial ")
            .expect("append should succeed");

        let mut conn = Connection::open(&db_path).expect("open db");
        let tx = conn.transaction().expect("start tx");
        let first = PersistenceManager::save_message_tx_for_request(
            &tx,
            "s3",
            "assistant",
            "req-final",
            "final response",
        )
        .expect("finalize should succeed");
        tx.commit().expect("commit tx");
        assert!(first, "first finalize should commit context marker");

        let conn = Connection::open(&db_path).expect("open db read");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s3", "assistant", "req-final"],
                |row| row.get(0),
            )
            .expect("count query should work");
        assert_eq!(count, 1, "finalize should keep single assistant row");

        let content: String = conn
            .query_row(
                "SELECT content FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s3", "assistant", "req-final"],
                |row| row.get(0),
            )
            .expect("content query should work");
        assert_eq!(content, "final response");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn user_request_id_mismatch_is_rejected_and_not_overwritten() {
        let db_path = temp_db_path("user-mismatch");
        let pm = PersistenceManager::new(db_path.clone());
        pm.init().expect("init should succeed");

        let mut conn = Connection::open(&db_path).expect("open db");
        let tx = conn.transaction().expect("start tx");
        let first =
            PersistenceManager::save_message_tx_for_request(&tx, "s4", "user", "req-user-1", "A")
                .expect("first user write should succeed");
        tx.commit().expect("commit first");
        assert!(first);

        let mut conn = Connection::open(&db_path).expect("open db second");
        let tx = conn.transaction().expect("start tx second");
        let err =
            PersistenceManager::save_message_tx_for_request(&tx, "s4", "user", "req-user-1", "B")
                .expect_err("mismatch replay should fail");
        assert!(
            err.to_string().contains("request_id_content_mismatch"),
            "should signal replay mismatch"
        );
        drop(tx);

        let conn = Connection::open(&db_path).expect("open db read");
        let content: String = conn
            .query_row(
                "SELECT content FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s4", "user", "req-user-1"],
                |row| row.get(0),
            )
            .expect("content query should work");
        assert_eq!(
            content, "A",
            "mismatch replay must not overwrite user content"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn stream_retry_finalize_keeps_single_marker_and_row() {
        let db_path = temp_db_path("stream-retry-marker");
        let pm = PersistenceManager::new(db_path.clone());
        pm.init().expect("init should succeed");

        let msg_id = pm
            .start_message_for_request("s5", "assistant", "req-stream-2")
            .expect("stream start should succeed");
        pm.append_message_content(msg_id, "partial ")
            .expect("append should succeed");

        let mut conn = Connection::open(&db_path).expect("open db first");
        let tx = conn.transaction().expect("tx first");
        let first = PersistenceManager::save_message_tx_for_request(
            &tx,
            "s5",
            "assistant",
            "req-stream-2",
            "final value",
        )
        .expect("first finalize should succeed");
        tx.commit().expect("commit first");
        assert!(first, "first finalize should claim commit marker");

        let mut conn = Connection::open(&db_path).expect("open db second");
        let tx = conn.transaction().expect("tx second");
        let second = PersistenceManager::save_message_tx_for_request(
            &tx,
            "s5",
            "assistant",
            "req-stream-2",
            "final value",
        )
        .expect("retry finalize should succeed idempotently");
        tx.commit().expect("commit second");
        assert!(!second, "retry finalize should not claim commit marker");

        let conn = Connection::open(&db_path).expect("open db read");
        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s5", "assistant", "req-stream-2"],
                |row| row.get(0),
            )
            .expect("row count query should work");
        assert_eq!(
            row_count, 1,
            "finalize retry must keep single assistant row"
        );

        let marker_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_request_commits WHERE session_id = ?1 AND role = ?2 AND request_id = ?3",
                params!["s5", "assistant", "req-stream-2"],
                |row| row.get(0),
            )
            .expect("marker count query should work");
        assert_eq!(
            marker_count, 1,
            "finalize retry must keep single commit marker"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_migrates_legacy_messages_without_request_id_column() {
        let db_path = temp_db_path("legacy-messages-no-request-id");

        let conn = Connection::open(&db_path).expect("open legacy db");
        conn.execute(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .expect("create legacy sessions");
        conn.execute(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            )",
            [],
        )
        .expect("create legacy messages");
        drop(conn);

        let pm = PersistenceManager::new(db_path.clone());
        pm.init()
            .expect("init should migrate legacy messages schema");
        pm.validate_schema()
            .expect("schema should validate after migration");

        let conn = Connection::open(&db_path).expect("open migrated db");
        let has_request_id = PersistenceManager::column_exists(&conn, "messages", "request_id")
            .expect("pragma should work");
        assert!(has_request_id, "init should add messages.request_id column");

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn init_fails_on_incompatible_messages_schema() {
        let db_path = temp_db_path("incompatible-messages-schema");

        let conn = Connection::open(&db_path).expect("open incompatible db");
        conn.execute(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .expect("create sessions");
        conn.execute(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            )",
            [],
        )
        .expect("create incompatible messages");
        drop(conn);

        let pm = PersistenceManager::new(db_path.clone());
        let err = pm
            .init()
            .expect_err("init should fail for incompatible schema missing content column");
        assert!(
            err.to_string()
                .contains("missing required column 'messages.content'"),
            "error should identify incompatible schema cause"
        );

        let _ = std::fs::remove_file(db_path);
    }
}
