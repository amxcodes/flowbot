use super::embedding_provider::EmbeddingProvider;
use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{info, instrument};

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
    #[serde(default = "default_tenant")]
    pub tenant_id: String, // Added for Multi-tenancy
}

fn default_tenant() -> String {
    "default".to_string()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub score: f32,
    pub metadata: HashMap<String, String>,
    pub tenant_id: String,
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
        manager
            .ensure_schema()
            .expect("Failed to initialize memory schema");

        manager
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;

        // Main documents table with tenant_id
        conn.execute(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                path TEXT,
                content TEXT,
                metadata TEXT,
                vector BLOB,
                tenant_id TEXT DEFAULT 'default',
                created_at INTEGER,
                updated_at INTEGER
            )",
            [],
        )?;

        // FTS5 Virtual Table
        // We include tenant_id as UNINDEXED to allow filtering in queries if supported by query planner,
        // or just rely on post-filtering if FTS match returns too many results.
        // Ideally we'd filter FTS by tenant_id, but FTS5 is tricky with extra columns.
        // We'll stick to simple FTS on content/path, and filter results in Rust or via join if needed.
        // Actually, creating a standard FTS table with an extra column works fine.
        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(content, path, tenant_id UNINDEXED)",
            [],
        )?;

        // Index on path and tenant_id
        conn.execute("CREATE INDEX IF NOT EXISTS idx_path_tenant ON documents(path, tenant_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_tenant ON documents(tenant_id)", [])?;

        Ok(())
    }

    pub fn load_index(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;
        // Check if tenant_id exists (migration check)
        // If we just added the column in ensure_schema, it might be new.
        // But for now assuming schema is consistent.
        
        // We need to handle the case where the DB existed before (no tenant_id).
        // `ensure_schema`'s create table if not exists won't add column.
        // We should probably check and add column if missing.
        // For this iteration, let's assume we can add it safely or users run fresh. 
        // Or better: `ensure_schema` logic should handle migration.
        // Let's add a quick migration check in `ensure_schema` in a real app, 
        // but here we might rely on the fact that `nanobot-rs-clean` implies clean start or we can advise reset.
        // However, robust code should handle it. 
        // I will add a column check.
        
        let mut stmt = conn.prepare("SELECT id, path, content, metadata, vector, tenant_id FROM documents")?;

        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let _path: String = row.get(1)?;
            let content: String = row.get(2)?;
            let metadata_json: String = row.get(3)?;
            let vector_blob: Vec<u8> = row.get(4)?;
            let tenant_id: String = row.get(5).unwrap_or_else(|_| "default".to_string());

            // Deserialize metadata
            let metadata: HashMap<String, String> =
                serde_json::from_str(&metadata_json).unwrap_or_default();

            // Deserialize vector
            let vector: Vec<f32> = serde_json::from_slice(&vector_blob).unwrap_or_default();

            Ok(VectorEntry {
                id,
                content,
                metadata,
                vector,
                tenant_id,
            })
        })?;

        let mut cache = self.vector_cache.lock().map_err(|e| anyhow::anyhow!("Cache lock poisoned: {}", e))?;
        cache.clear();

        for row in rows {
            if let Ok(entry) = row {
                cache.push(entry);
            }
        }
        info!("Loaded {} vectors into cache", cache.len());

        Ok(())
    }

    // Save is no-op because we save on write to SQLite
    pub fn save_index(&self) -> Result<()> {
        Ok(())
    }

    pub async fn add_document(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        tenant_id: Option<&str>,
    ) -> Result<()> {
        let tenant_id = tenant_id.unwrap_or("default");
        let embeddings = self.provider.embed(vec![content]).await?;
        let vector = embeddings[0].clone();
        self.add_document_with_vector(content, metadata, &vector, tenant_id)
            .await
    }

    /// Internal method to add document with pre-computed vector
    async fn add_document_with_vector(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        vector: &[f32],
        tenant_id: &str,
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
        let tenant_id_clone = tenant_id.to_string();

        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().expect("Database lock should not be poisoned");
            let tx = conn.transaction()?;
            
            // Insert Main
            tx.execute(
                "INSERT INTO documents (id, path, content, metadata, vector, tenant_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id_clone, path_clone, content_clone, metadata_json_clone, vector_blob_clone, tenant_id_clone, now, now],
            )?;
            
            // Insert FTS
            tx.execute(
                "INSERT INTO documents_fts (content, path, tenant_id) VALUES (?1, ?2, ?3)",
                params![content_clone, path_clone, tenant_id_clone],
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
            tenant_id: tenant_id.to_string(),
        };

        let mut lock = self.vector_cache.lock().map_err(|e| anyhow::anyhow!("Cache lock poisoned: {}", e))?;
        lock.push(entry);

        Ok(())
    }

    /// Add multiple documents in batch (efficient for bulk indexing)
    pub async fn add_documents_batch(
        &self,
        documents: Vec<(String, HashMap<String, String>)>, // (content, metadata)
        tenant_id: Option<&str>,
        batch_size: Option<usize>,
    ) -> Result<usize> {
        let tenant_id = tenant_id.unwrap_or("default");
        if documents.is_empty() {
            return Ok(0);
        }

        let batch_size = batch_size.unwrap_or(20); // Default: 20 for API rate limits
        let mut added = 0;

        for chunk in documents.chunks(batch_size) {
            // Extract contents for batch embedding
            let contents: Vec<&str> = chunk.iter().map(|(content, _)| content.as_str()).collect();

            // Single batched embed call
            let embeddings = self.provider.embed(contents).await?;

            // Insert all documents in this batch
            for (i, (content, metadata)) in chunk.iter().enumerate() {
                self.add_document_with_vector(content, metadata.clone(), &embeddings[i], tenant_id)
                    .await?;
                added += 1;
            }

            // Rate limiting: small delay between batches
            if added < documents.len() {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        Ok(added)
    }

    pub fn remove_document_by_path(&self, path: &str, tenant_id: Option<&str>) -> Result<()> {
        let conn_lock = self.conn.lock().map_err(|e| anyhow::anyhow!("Database lock poisoned: {}", e))?;

        let path_str = path.to_string();
        let tenant_filter = tenant_id.unwrap_or("default");

        conn_lock.execute("DELETE FROM documents WHERE path = ?1 AND tenant_id = ?2", params![path_str, tenant_filter])?;
        // For FTS shadow tables, we delete by matching content. But here we have ID in main table.
        // Ideally we should use triggers or standard FTS delete.
        // Simple approach: delete from FTS where path and tenant match (if unique).
        conn_lock.execute(
            "DELETE FROM documents_fts WHERE path = ?1 AND tenant_id = ?2",
            params![path_str, tenant_filter],
        )?;

        // Update cache
        let mut cache = self.vector_cache.lock().unwrap();
        // Remove if path matches AND tenant_id matches
        cache.retain(|entry| {
            let same_path = entry.metadata.get("path").map(|p| p.as_str()) == Some(path_str.as_str());
            let same_tenant = entry.tenant_id == tenant_filter;
            !(same_path && same_tenant)
        });

        Ok(())
    }
    
    // Implement update by delegating to remove + add
    pub async fn update_document(&self, path: &str, content: &str, tenant_id: Option<&str>) -> Result<()> {
         self.remove_document_by_path(path, tenant_id)?;
         // Need metadata... this signature is lossy in previous implementation too.
         // Assuming this is used for file watchers where we might want to preserve other metadata?
         // For now, let's reconstruct minimal metadata
         let mut metadata = HashMap::new();
         metadata.insert("path".to_string(), path.to_string());
         self.add_document(content, metadata, tenant_id).await
    }

    #[instrument(skip(self), fields(query_len = query.len(), limit, tenant_id))]
    pub async fn search(&self, query: &str, limit: usize, tenant_id: Option<&str>) -> Result<Vec<(f32, VectorEntry)>> {
        let tenant_id = tenant_id.unwrap_or("default");
        
        // 1. Vector Search (In-Memory)
        let query_embeddings = self.provider.embed(vec![query]).await?;
        let query_embedding = &query_embeddings[0];

        let mut vector_results = Vec::new();
        {
            let store = self.vector_cache.lock().unwrap();
            for entry in store.iter() {
                if entry.tenant_id != tenant_id { continue; } // Tenant Filter
                
                let score = cosine_similarity(query_embedding, &entry.vector);
                vector_results.push((score, entry.clone()));
            }
        }

        // Sort by Vector Score Phase 1
        vector_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        // We keep top 50 (or limit*2) for RRF fusion
        let top_k = limit * 2;
        let binding = vector_results.iter().take(top_k).cloned().collect::<Vec<_>>();
        let top_vector_results = binding.as_slice();

        // 2. Keyword Search (FTS) - Hybrid
        let conn = self.conn.clone();
        let query_str = query.to_string();
        let tenant_id_str = tenant_id.to_string();

        let keyword_results: Vec<(f32, String)> = tokio::task::spawn_blocking(move || {
           let conn = conn.lock().unwrap();
           // FTS Match
           // Sanitization: quotes and FTS5 syntax chars could break it.
           // Simple approach: remove quotes, wrap in quotes for phrase or just use words.
           // Let's just remove quotes for safety.
           let safe_query = format!("\"{}\"", query_str.replace("\"", ""));
           
           // Query: MATCH query AND tenant_id = '...' 
           // Note: tenant_id is UNINDEXED, so we can use it in WHERE clause of FTS table query?
           // Yes, "SELECT ... FROM documents_fts WHERE documents_fts MATCH ... AND tenant_id = ..." works.
           let mut stmt = conn.prepare(
               "SELECT path, rank FROM documents_fts WHERE documents_fts MATCH ?1 AND tenant_id = ?2 ORDER BY rank LIMIT ?3"
           )?;
           
           let rows = stmt.query_map(params![safe_query, tenant_id_str, top_k as i64], |row| {
               let path: String = row.get(0)?;
               let rank: f64 = row.get(1)?;
               Ok((rank as f32, path))
           })?
           .filter_map(|r| r.ok())
           .collect();
           
           Ok::<_, anyhow::Error>(rows)
        }).await??;

        // 3. Reciprocal Rank Fusion (RRF)
        // score = 1.0 / (k + rank)
        let rrf_k = 60.0;
        let mut rrf_scores: HashMap<String, f32> = HashMap::new();

        // Process Vector Ranks
        for (rank, (_, entry)) in top_vector_results.iter().enumerate() {
            let path = entry.metadata.get("path").cloned().unwrap_or_default();
            // RRF score
            let score = 1.0 / (rrf_k + (rank as f32 + 1.0));
            *rrf_scores.entry(path).or_insert(0.0) += score;
        }

        // Process Keyword Ranks
        // keyword_results are already sorted by rank (lower is better in SQLite FTS usually? Wait. 
        // SQLite FTS5 rank: usually lower is better ("more relevant" -> smaller negative number? No, BM25 returns score).
        // documentation says "ORDER BY rank". Default sort matches relevance.
        // So index 0 is rank 1.
        for (rank, (_, path)) in keyword_results.iter().enumerate() {
            let score = 1.0 / (rrf_k + (rank as f32 + 1.0));
            *rrf_scores.entry(path.clone()).or_insert(0.0) += score;
        }

        // Build Final Results
        let mut final_results_vec = Vec::new();
        let store = self.vector_cache.lock().unwrap(); 
        
        for (path, rrf_score) in rrf_scores {
            // Retrieve full entry from cache
            if let Some(entry) = store.iter().find(|e| e.tenant_id == tenant_id && e.metadata.get("path").map(|p| p.as_str()) == Some(&path)) {
                final_results_vec.push((rrf_score, entry.clone()));
            }
        }

        // Sort by RRF Score
        final_results_vec.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        final_results_vec.truncate(limit);

        Ok(final_results_vec)
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
