-- Context Tree Schema: Branching Conversation History
-- This enables "Git-like" conversation management with undo/redo/branching

CREATE TABLE IF NOT EXISTS context_tree (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant', 'system')),
    content TEXT NOT NULL,
    model TEXT,
    created_at INTEGER NOT NULL,
    metadata TEXT, -- JSON for extensibility
    FOREIGN KEY (parent_id) REFERENCES context_tree(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_context_tree_session ON context_tree(session_id);
CREATE INDEX IF NOT EXISTS idx_context_tree_parent ON context_tree(parent_id);
CREATE INDEX IF NOT EXISTS idx_context_tree_created_at ON context_tree(created_at);

-- Active branch tracking per session
CREATE TABLE IF NOT EXISTS active_branches (
    session_id TEXT PRIMARY KEY,
    current_leaf_id TEXT NOT NULL,
    FOREIGN KEY (current_leaf_id) REFERENCES context_tree(id) ON DELETE CASCADE
);
