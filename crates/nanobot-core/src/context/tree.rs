use anyhow::Result;
use rusqlite::{Connection, params, OptionalExtension};
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
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;

        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO context_tree (id, parent_id, session_id, role, content, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, parent_id, session_id, role, content, model, created_at],
        )?;

        // Update active branch
        db.execute(
            "INSERT OR REPLACE INTO active_branches (session_id, current_leaf_id)
             VALUES (?1, ?2)",
            params![session_id, id],
        )?;

        Ok(id)
    }

    /// Get the conversation trace from root to a specific leaf
    pub fn get_trace(&self, leaf_id: &str) -> Result<Vec<ContextNode>> {
        let db = self.db.lock().unwrap();
        let mut nodes = Vec::new();
        let mut current_id = Some(leaf_id.to_string());

        while let Some(id) = current_id {
            let mut stmt = db.prepare(
                "SELECT id, parent_id, session_id, role, content, model, created_at, metadata
                 FROM context_tree WHERE id = ?1"
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
        let mut stmt = db.prepare("SELECT current_leaf_id FROM active_branches WHERE session_id = ?1")?;
        
        let result = stmt.query_row(params![session_id], |row| row.get(0))
            .optional()?;
        
        Ok(result)
    }

    /// Fork a conversation at a specific node
    pub fn fork_at(&self, node_id: &str, session_id: &str) -> Result<()> {
        let db = self.db.lock().unwrap();
        
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
             ORDER BY created_at ASC"
        )?;

        let nodes = stmt.query_map(params![node_id], |row| {
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
             ORDER BY created_at ASC"
        )?;

        let nodes = stmt.query_map(params![session_id], |row| {
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
        let msg1 = tree.add_message("session1", "user", "Hello", None, None).unwrap();
        
        // Add response
        let msg2 = tree.add_message("session1", "assistant", "Hi!", Some(msg1.clone()), Some("gpt-4".to_string())).unwrap();
        
        // Get trace
        let trace = tree.get_trace(&msg2).unwrap();
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].content, "Hello");
        assert_eq!(trace[1].content, "Hi!");
    }

    #[test]
    fn test_context_tree_branching() {
        let tree = ContextTree::new(":memory:").unwrap();
        
        let msg1 = tree.add_message("session1", "user", "Hello", None, None).unwrap();
        let msg2 = tree.add_message("session1", "assistant", "Response A", Some(msg1.clone()), None).unwrap();
        
        // Fork back to msg1 and create alternate branch
        tree.fork_at(&msg1, "session1").unwrap();
        let msg3 = tree.add_message("session1", "assistant", "Response B", Some(msg1.clone()), None).unwrap();
        
        // Check that msg1 has 2 children
        let children = tree.get_children(&msg1).unwrap();
        assert_eq!(children.len(), 2);
        
        // Active leaf should be msg3
        let active = tree.get_active_leaf("session1").unwrap();
        assert_eq!(active, Some(msg3));
    }
}
