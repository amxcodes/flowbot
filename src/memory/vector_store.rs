use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use super::embedding_provider::EmbeddingProvider;
use rusqlite::{params, Connection};

// Simple Vector Entry for In-Memory Cache
// We keep vectors in memory because calculating cosine similarity for all rows in Python/Rust
// is often faster than passing blobs to SQLite extension unless using vector0 extension.
// Nanobot uses standard bundled rusqlite, so we stick to in-memory vector scan.
#[derive(Serialize, Deserialize, Clone)]
pub struct VectorEntry {
    pub id: String,
    pub content: String,
    pub metadata: HashMap<String, String>,
    pub vector: Vec<f32>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub metadata: HashMap<String, String>,
}

pub struct MemoryManager {
    provider: EmbeddingProvider,
    // Database connection for persistence and FTS
    conn: Arc<Mutex<Connection>>,
    // In-memory cache for fast vector search
    vector_cache: Arc<Mutex<Vec<VectorEntry>>>,
}

impl MemoryManager {
    pub fn new(db_path: PathBuf, provider: EmbeddingProvider) -> Self {
        // Ensure parent dir
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn = Connection::open(&db_path).expect("Failed to open memory DB");
        
        let manager = Self {
            provider,
            conn: Arc::new(Mutex::new(conn)),
            vector_cache: Arc::new(Mutex::new(Vec::new())),
        };
        
        // Initialize schema
        manager.ensure_schema().expect("Failed to initialize memory schema");
        
        manager
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        
        // Main documents table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                path TEXT,
                content TEXT,
                metadata TEXT,
                vector BLOB,
                created_at INTEGER,
                updated_at INTEGER
            )",
            [],
        )?;
        
        // FTS5 Virtual Table
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(content, path)",
            [],
        )?;

        // Index on path for faster lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_path ON documents(path)",
            [],
        )?;

        Ok(())
    }

    pub fn load_index(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, path, content, metadata, vector FROM documents")?;
        
        let rows = stmt.query_map([], |row| {
             let id: String = row.get(0)?;
             let _path: String = row.get(1)?;
             let content: String = row.get(2)?;
             let metadata_json: String = row.get(3)?;
             let vector_blob: Vec<u8> = row.get(4)?;
             
             // Deserialize metadata
             let metadata: HashMap<String, String> = serde_json::from_str(&metadata_json).unwrap_or_default();
             
             // Deserialize vector (saved as JSON bytes for simplicity/compatibility)
             let vector: Vec<f32> = serde_json::from_slice(&vector_blob).unwrap_or_default();
             
             Ok(VectorEntry {
                 id,
                 content,
                 metadata,
                 vector,
             })
        })?;

        let mut cache = self.vector_cache.lock().unwrap();
        cache.clear();
        
        for row in rows {
            if let Ok(entry) = row {
                cache.push(entry);
            }
        }
        
        Ok(())
    }
    
    // Save is no-op because we save on write to SQLite
    pub fn save_index(&self) -> Result<()> {
        Ok(())
    }

    pub async fn add_document(&self, content: &str, metadata: HashMap<String, String>) -> Result<()> {
        let embeddings = self.provider.embed(vec![content]).await?;
        let vector = embeddings[0].clone();
        self.add_document_with_vector(content, metadata, &vector).await
    }
    
    /// Internal method to add document with pre-computed vector
    async fn add_document_with_vector(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        vector: &[f32],
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = metadata.get("path").map(|s| s.clone()).unwrap_or_default();
        
        let metadata_json = serde_json::to_string(&metadata)?;
        let vector_blob = serde_json::to_vec(vector)?;
        let now = chrono::Utc::now().timestamp();
        
        // 1. Update SQLite
        let conn = self.conn.clone();
        let content_clone = content.to_string();
        let path_clone = path.clone();
        let id_clone = id.clone();
        let metadata_json_clone = metadata_json.clone();
        let vector_blob_clone = vector_blob.clone();
        
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            
            // Insert Main
            tx.execute(
                "INSERT INTO documents (id, path, content, metadata, vector, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id_clone, path_clone, content_clone, metadata_json_clone, vector_blob_clone, now, now],
            )?;
            
            // Insert FTS
            tx.execute(
                "INSERT INTO documents_fts (content, path) VALUES (?1, ?2)",
                params![content_clone, path_clone],
            )?;
            
            tx.commit()?;
            Ok::<_, anyhow::Error>(())
        }).await??;
        
        // 2. Update Cache
        let entry = VectorEntry {
            id,
            content: content.to_string(),
            metadata,
            vector: vector.to_vec(),
        };
        
        let mut lock = self.vector_cache.lock().unwrap();
        lock.push(entry);
        
        Ok(())
    }
    
    /// Add multiple documents in batch (efficient for bulk indexing)
    /// Returns the number of documents successfully added
    pub async fn add_documents_batch(
        &self,
        documents: Vec<(String, HashMap<String, String>)>,  // (content, metadata)
        batch_size: Option<usize>,
    ) -> Result<usize> {
        if documents.is_empty() {
            return Ok(0);
        }
        
        let batch_size = batch_size.unwrap_or(20); // Default: 20 for API rate limits
        let mut added = 0;
        
        for chunk in documents.chunks(batch_size) {
            // Extract contents for batch embedding
            let contents: Vec<&str> = chunk.iter()
                .map(|(content, _)| content.as_str())
                .collect();
            
            // Single batched embed call
            let embeddings = self.provider.embed(contents).await?;
            
            // Insert all documents in this batch
            for (i, (content, metadata)) in chunk.iter().enumerate() {
                self.add_document_with_vector(
                    content,
                    metadata.clone(),
                    &embeddings[i]
                ).await?;
                added += 1;
            }
            
            // Rate limiting: small delay between batches
            if added < documents.len() {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
        
        Ok(added)
    }

    pub fn remove_document_by_path(&self, path: &str) -> Result<()> {
        let conn_lock = self.conn.lock().unwrap();
        
        let path_str = path.to_string();
        conn_lock.execute("DELETE FROM documents WHERE path = ?1", params![path_str])?;
        conn_lock.execute("DELETE FROM documents_fts WHERE path = ?1", params![path_str])?;
        
        // Update cache
        let mut cache = self.vector_cache.lock().unwrap();
        cache.retain(|entry| {
            entry.metadata.get("path").map(|p| p.as_str()) != Some(path_str.as_str())
        });
        
        Ok(())
    }

    pub async fn update_document(&self, path: &str, content: &str) -> Result<()> {
        // Need to call async remove logic then add
        // To ensure atomic update in DB, we use transaction in spawn_blocking
        
        let path_str = path.to_string();
        let content_str = content.to_string();
        let self_clone = self.conn.clone();
        let cache_clone = self.vector_cache.clone();
        let provider = self.provider.clone();
        
        // 1. Embed new content
        let embeddings = provider.embed(vec![&content_str]).await?;
        let vector = embeddings[0].clone();
        let vector_blob = serde_json::to_vec(&vector)?;
        
        // 2. DB Transaction (Remove + Insert)
        let _res: (String, HashMap<String, String>) = tokio::task::spawn_blocking(move || {
             let mut conn = self_clone.lock().unwrap();
             let tx = conn.transaction()?;
             
             // Remove old
             tx.execute("DELETE FROM documents WHERE path = ?1", params![path_str])?;
             tx.execute("DELETE FROM documents_fts WHERE path = ?1", params![path_str])?;
             
             // Insert new
             let id = uuid::Uuid::new_v4().to_string();
             let now = chrono::Utc::now().timestamp();
             let mut metadata = HashMap::new();
             metadata.insert("path".to_string(), path_str.clone());
             metadata.insert("updated_at".to_string(), now.to_string());
             let metadata_json = serde_json::to_string(&metadata)?;
             
             tx.execute(
                "INSERT INTO documents (id, path, content, metadata, vector, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, path_str, content_str, metadata_json, vector_blob, now, now],
            )?;
            
            tx.execute(
                "INSERT INTO documents_fts (content, path) VALUES (?1, ?2)",
                params![content_str, path_str],
            )?;
            
            tx.commit()?;
            Ok::<_, anyhow::Error>((id, metadata))
        }).await??;
        
        // 3. Update Cache
        let (new_id, new_metadata) = _res;
        {
            let mut cache = cache_clone.lock().unwrap();
            cache.retain(|entry| entry.metadata.get("path").map(|p| p.as_str()) != Some(path));
            
            // Add the updated entry with values from spawn_blocking
            let new_entry = VectorEntry {
                id: new_id,
                content: content.to_string(),
                metadata: new_metadata,
                vector,
            };
            cache.push(new_entry);
        }
        
        Ok(())
    }
    
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(f32, VectorEntry)>> {
        // 1. Vector Search (In-Memory)
        let query_embeddings = self.provider.embed(vec![query]).await?;
        let query_embedding = &query_embeddings[0];
        
        // Scope the lock to ensure Send safety across await later
        let mut vector_results = Vec::new();
        {
            let store = self.vector_cache.lock().unwrap();
            for entry in store.iter() {
                let score = cosine_similarity(query_embedding, &entry.vector);
                vector_results.push((score, entry.clone()));
            }
        }
        
        // Sort by score descending
        vector_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        vector_results.truncate(limit);
        
        // 2. Keyword Search (FTS) - Hybrid
        let conn = self.conn.clone();
        let query_str = query.to_string();
        
        let keyword_results: Vec<String> = tokio::task::spawn_blocking(move || {
           let conn = conn.lock().unwrap();
           // FTS Match
           let safe_query = format!("\"{}\"", query_str.replace("\"", ""));
           let mut stmt = conn.prepare("SELECT path FROM documents_fts WHERE documents_fts MATCH ?1 ORDER BY rank LIMIT ?2")?;
           let paths = stmt.query_map(params![safe_query, limit as i64], |row| {
               row.get::<_, String>(0)
           })?
           .filter_map(|r| r.ok())
           .collect();
           
           Ok::<_, anyhow::Error>(paths)
        }).await??;
        
        // 3. Merge (Simple Boost)
        let mut final_results = vector_results.clone();
        let vector_paths: Vec<String> = vector_results.iter().map(|(_, e)| e.metadata.get("path").cloned().unwrap_or_default()).collect();
        
        if !keyword_results.is_empty() {
             let store = self.vector_cache.lock().unwrap();
             for path in keyword_results {
                if !vector_paths.contains(&path) {
                    // Find in store
                    if let Some(entry) = store.iter().find(|e| e.metadata.get("path").map(|p| p.as_str()) == Some(&path)) {
                        let score = cosine_similarity(query_embedding, &entry.vector);
                        final_results.push((score * 1.5, entry.clone())); // 1.5x boost
                    }
                }
            }
        }
        
        // Re-sort
        final_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        final_results.truncate(limit);
        
        Ok(final_results)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot_product / (norm_a * norm_b)
}
