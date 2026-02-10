use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// A single node in the context tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub model: Option<String>,
    pub created_at: i64,
    pub metadata: Option<String>,
}

/// Context Tree - Git-like branching conversation history
pub struct ContextTree {
    db: Arc<Mutex<Connection>>,
}

impl ContextTree {
    /// Create a new context tree with the given database
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;

        // Run migration - embedded SQL to avoid path issues
        const MIGRATION_SQL: &str = r#"
-- Context Tree Schema: Branching Conversation History
CREATE TABLE IF NOT EXISTS context_tree (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL,
    model TEXT,
    created_at INTEGER NOT NULL,
    metadata TEXT,
    FOREIGN KEY (parent_id) REFERENCES context_tree(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_context_tree_session ON context_tree(session_id);
CREATE INDEX IF NOT EXISTS idx_context_tree_parent ON context_tree(parent_id);
CREATE INDEX IF NOT EXISTS idx_context_tree_created_at ON context_tree(created_at);

CREATE TABLE IF NOT EXISTS active_branches (
    session_id TEXT PRIMARY KEY,
    current_leaf_id TEXT NOT NULL,
    FOREIGN KEY (current_leaf_id) REFERENCES context_tree(id) ON DELETE CASCADE
);

-- Persistence tables (shared DB)
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
        "#;

        conn.execute_batch(MIGRATION_SQL)?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Add a message to the tree
    pub fn add_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        parent_id: Option<String>,
        model: Option<String>,
    ) -> Result<String> {
        self.with_transaction(|tx| {
            Self::add_message_in_tx(tx, session_id, role, content, parent_id, model)
        })
    }

    pub fn with_transaction<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction) -> Result<T>,
    ) -> Result<T> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let result = f(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn get_active_leaf_tx(
        tx: &rusqlite::Transaction,
        session_id: &str,
    ) -> Result<Option<String>> {
        tx.query_row(
            "SELECT current_leaf_id FROM active_branches WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn add_message_in_tx(
        tx: &rusqlite::Transaction,
        session_id: &str,
        role: &str,
        content: &str,
        parent_id: Option<String>,
        model: Option<String>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        if let Some(ref parent) = parent_id {
            let parent_session: Option<String> = tx
                .query_row(
                    "SELECT session_id FROM context_tree WHERE id = ?1",
                    params![parent],
                    |row| row.get(0),
                )
                .optional()?;

            if let Some(parent_session_id) = parent_session {
                if parent_session_id != session_id {
                    anyhow::bail!("Parent node belongs to a different session");
                }
            }
        }

        tx.execute(
            "INSERT INTO context_tree (id, parent_id, session_id, role, content, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, parent_id, session_id, role, content, model, created_at],
        )?;

        tx.execute(
            "INSERT OR REPLACE INTO active_branches (session_id, current_leaf_id)
             VALUES (?1, ?2)",
            params![session_id, id],
        )?;

        Ok(id)
    }

    pub fn count_session_nodes(&self, session_id: &str) -> Result<usize> {
        let db = self.db.lock().unwrap();
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM context_tree WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn prune_session(
        &self,
        session_id: &str,
        max_nodes: usize,
        keep_recent: usize,
    ) -> Result<usize> {
        if max_nodes == 0 || keep_recent == 0 {
            return Ok(0);
        }

        self.with_transaction(|tx| {
            let total: i64 = tx.query_row(
                "SELECT COUNT(*) FROM context_tree WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;

            if total as usize <= max_nodes {
                return Ok(0);
            }

            let cutoff_id: Option<String> = tx
                .query_row(
                    "WITH RECURSIVE ancestors(id, parent_id, depth) AS (
                        SELECT current_leaf_id, (SELECT parent_id FROM context_tree WHERE id = current_leaf_id), 0
                        FROM active_branches WHERE session_id = ?1
                        UNION ALL
                        SELECT ct.parent_id, (SELECT parent_id FROM context_tree WHERE id = ct.parent_id), depth + 1
                        FROM ancestors ct
                        WHERE ct.parent_id IS NOT NULL
                     )
                     SELECT id FROM ancestors WHERE depth = ?2",
                    params![session_id, (keep_recent.saturating_sub(1)) as i64],
                    |row| row.get(0),
                )
                .optional()?;

            if let Some(cutoff) = cutoff_id {
                tx.execute(
                    "UPDATE context_tree SET parent_id = NULL WHERE id = ?1",
                    params![cutoff],
                )?;
            }

            tx.execute(
                "CREATE TEMP TABLE IF NOT EXISTS keep_ids (id TEXT PRIMARY KEY)",
                [],
            )?;
            tx.execute("DELETE FROM keep_ids", [])?;

            tx.execute(
                "INSERT OR IGNORE INTO keep_ids(id)
                 WITH RECURSIVE ancestors(id) AS (
                    SELECT current_leaf_id FROM active_branches WHERE session_id = ?1
                    UNION ALL
                    SELECT context_tree.parent_id
                    FROM context_tree
                    JOIN ancestors ON context_tree.id = ancestors.id
                    WHERE context_tree.parent_id IS NOT NULL
                 )
                 SELECT id FROM ancestors",
                params![session_id],
            )?;

            tx.execute(
                "INSERT OR IGNORE INTO keep_ids(id)
                 SELECT id FROM context_tree
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
                params![session_id, keep_recent as i64],
            )?;

            let deleted = tx.execute(
                "DELETE FROM context_tree
                 WHERE session_id = ?1
                 AND id NOT IN (SELECT id FROM keep_ids)",
                params![session_id],
            )?;

            tx.execute(
                "DELETE FROM messages
                 WHERE session_id = ?1
                 AND id NOT IN (
                     SELECT id FROM messages
                     WHERE session_id = ?1
                     ORDER BY id DESC
                     LIMIT ?2
                 )",
                params![session_id, keep_recent as i64],
            )?;

            Ok(deleted)
        })
    }

    /// Get the conversation trace from root to a specific leaf
    pub fn get_trace(&self, leaf_id: &str) -> Result<Vec<ContextNode>> {
        let db = self.db.lock().unwrap();
        let mut nodes = Vec::new();
        let mut current_id = Some(leaf_id.to_string());

        while let Some(id) = current_id {
            let mut stmt = db.prepare(
                "SELECT id, parent_id, session_id, role, content, model, created_at, metadata
                 FROM context_tree WHERE id = ?1",
            )?;

            let node = stmt.query_row(params![id], |row| {
                Ok(ContextNode {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    session_id: row.get(2)?,
                    role: row.get(3)?,
                    content: row.get(4)?,
                    model: row.get(5)?,
                    created_at: row.get(6)?,
                    metadata: row.get(7)?,
                })
            })?;

            current_id = node.parent_id.clone();
            nodes.push(node);
        }

        // Reverse to get chronological order (root -> leaf)
        nodes.reverse();
        Ok(nodes)
    }

    /// Get the current active branch for a session
    pub fn get_active_leaf(&self, session_id: &str) -> Result<Option<String>> {
        let db = self.db.lock().unwrap();
        let mut stmt =
            db.prepare("SELECT current_leaf_id FROM active_branches WHERE session_id = ?1")?;

        let result = stmt
            .query_row(params![session_id], |row| row.get(0))
            .optional()?;

        Ok(result)
    }

    /// Fork a conversation at a specific node
    pub fn fork_at(&self, node_id: &str, session_id: &str) -> Result<()> {
        let db = self.db.lock().unwrap();

        let node_session: Option<String> = db
            .query_row(
                "SELECT session_id FROM context_tree WHERE id = ?1",
                params![node_id],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(node_session_id) = node_session {
            if node_session_id != session_id {
                anyhow::bail!("Node belongs to a different session");
            }
        }

        // Update active branch to this node
        db.execute(
            "INSERT OR REPLACE INTO active_branches (session_id, current_leaf_id)
             VALUES (?1, ?2)",
            params![session_id, node_id],
        )?;

        Ok(())
    }

    /// Get all children of a node (for branch visualization)
    pub fn get_children(&self, node_id: &str) -> Result<Vec<ContextNode>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, parent_id, session_id, role, content, model, created_at, metadata
             FROM context_tree WHERE parent_id = ?1
             ORDER BY created_at ASC",
        )?;

        let nodes = stmt
            .query_map(params![node_id], |row| {
                Ok(ContextNode {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    session_id: row.get(2)?,
                    role: row.get(3)?,
                    content: row.get(4)?,
                    model: row.get(5)?,
                    created_at: row.get(6)?,
                    metadata: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(nodes)
    }

    /// Get the full tree structure for a session (for visualization)
    pub fn get_session_tree(&self, session_id: &str) -> Result<Vec<ContextNode>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT id, parent_id, session_id, role, content, model, created_at, metadata
             FROM context_tree WHERE session_id = ?1
             ORDER BY created_at ASC",
        )?;

        let nodes = stmt
            .query_map(params![session_id], |row| {
                Ok(ContextNode {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    session_id: row.get(2)?,
                    role: row.get(3)?,
                    content: row.get(4)?,
                    model: row.get(5)?,
                    created_at: row.get(6)?,
                    metadata: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_tree_basic() {
        let tree = ContextTree::new(":memory:").unwrap();

        // Add root message
        let msg1 = tree
            .add_message("session1", "user", "Hello", None, None)
            .unwrap();

        // Add response
        let msg2 = tree
            .add_message(
                "session1",
                "assistant",
                "Hi!",
                Some(msg1.clone()),
                Some("gpt-4".to_string()),
            )
            .unwrap();

        // Get trace
        let trace = tree.get_trace(&msg2).unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].content, "Hello");
        assert_eq!(trace[1].content, "Hi!");
    }

    #[test]
    fn test_context_tree_branching() {
        let tree = ContextTree::new(":memory:").unwrap();

        let msg1 = tree
            .add_message("session1", "user", "Hello", None, None)
            .unwrap();
        let _msg2 = tree
            .add_message(
                "session1",
                "assistant",
                "Response A",
                Some(msg1.clone()),
                None,
            )
            .unwrap();

        // Fork back to msg1 and create alternate branch
        tree.fork_at(&msg1, "session1").unwrap();
        let msg3 = tree
            .add_message(
                "session1",
                "assistant",
                "Response B",
                Some(msg1.clone()),
                None,
            )
            .unwrap();

        // Check that msg1 has 2 children
        let children = tree.get_children(&msg1).unwrap();
        assert_eq!(children.len(), 2);

        // Active leaf should be msg3
        let active = tree.get_active_leaf("session1").unwrap();
        assert_eq!(active, Some(msg3));
    }

    #[test]
    fn test_context_tree_prune_keeps_leaf() {
        let tree = ContextTree::new(":memory:").unwrap();

        let mut last = tree
            .add_message("session1", "user", "Start", None, None)
            .unwrap();

        for i in 0..50 {
            let msg = tree
                .add_message(
                    "session1",
                    "assistant",
                    &format!("Message {}", i),
                    Some(last.clone()),
                    None,
                )
                .unwrap();
            last = msg;
        }

        let removed = tree.prune_session("session1", 20, 10).unwrap();
        assert!(removed > 0);

        let leaf = tree.get_active_leaf("session1").unwrap();
        assert_eq!(leaf, Some(last));

        let count = tree.count_session_nodes("session1").unwrap();
        assert!(count <= 20);
    }
}
