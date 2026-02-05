use anyhow::Result;
use rig::completion::Message;
use rig::completion::message::{UserContent, AssistantContent, Text};
use rig::OneOrMany;
use rusqlite::{params, Connection};
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
        let _ : String = conn.query_row("PRAGMA journal_mode = WAL;", [], |row| row.get(0))?;
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
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            )",
            [],
        )?;
        
        // Index for faster history retrieval
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id)",
            [],
        )?;

        // Extend sessions for multi-agent support (safe to add columns if not exists)
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN parent_session_id TEXT",
            [],
        ).ok(); // Ignore if column exists
        
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN session_type TEXT DEFAULT 'main'",
            [],
        ).ok();
        
        conn.execute(
            "ALTER TABLE sessions ADD COLUMN cleanup_policy TEXT DEFAULT 'keep'",
            [],
        ).ok();

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

    pub fn get_history(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT role, content FROM messages WHERE session_id = ?1 ORDER BY id ASC"
        )?;
        
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
                },
                "assistant" => {
                    messages.push(Message::Assistant {
                        id: None, // We don't store ID yet, Rig generates one if needed or we ignore
                        content: OneOrMany::one(AssistantContent::Text(Text { text: content })),
                    });
                },
                _ => {
                    // Ignore unknown roles or map to system?
                    // For now, treat as user text (or skip)
                }
            }
        }
        
        Ok(messages)
    }
}
