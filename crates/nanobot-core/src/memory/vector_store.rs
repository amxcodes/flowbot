use super::embedding_provider::EmbeddingProvider;
use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::cmp::Ordering;
use tracing::{info, instrument};

// Scoring helpers were removed; streaming sort is used directly.

// Complete Entry for compatibility
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VectorEntry {
    pub id: String,
    pub content: String,
    pub metadata: HashMap<String, String>,
    pub vector: Vec<f32>,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
}

fn default_tenant() -> String {
    "default".to_string()
}

pub struct MemoryManager {
    provider: EmbeddingProvider,
    conn: Arc<Mutex<Connection>>,
    // No vector_cache!
}

impl MemoryManager {
    pub fn new(db_path: PathBuf, provider: EmbeddingProvider) -> Self {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let conn = Connection::open(&db_path).expect("Failed to open memory DB");

        let manager = Self {
            provider,
            conn: Arc::new(Mutex::new(conn)),
        };

        manager.ensure_schema().expect("Failed to initialize memory schema");
        manager
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow!("Database lock poisoned: {}", e))?;

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

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(content, path, tenant_id UNINDEXED)",
            [],
        )?;

        conn.execute("CREATE INDEX IF NOT EXISTS idx_path_tenant ON documents(path, tenant_id)", [])?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_tenant ON documents(tenant_id)", [])?;

        Ok(())
    }

    pub fn load_index(&self) -> Result<()> {
        info!("MemoryManager: Streaming mode enabled. No index loading needed.");
        Ok(())
    }

    pub fn save_index(&self) -> Result<()> {
        Ok(())
    }

    pub async fn add_document(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        tenant_id: Option<&str>,
    ) -> Result<()> {
        let tenant_id = tenant_id.ok_or_else(|| anyhow::anyhow!("tenant_id is required"))?;
        let embeddings = self.provider.embed(vec![content]).await?;
        let vector = embeddings[0].clone();
        self.add_document_with_vector(content, metadata, &vector, tenant_id).await
    }

    async fn add_document_with_vector(
        &self,
        content: &str,
        metadata: HashMap<String, String>,
        vector: &[f32],
        tenant_id: &str,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = metadata.get("path").cloned().unwrap_or_default();
        let metadata_json = serde_json::to_string(&metadata)?;
        let vector_blob = serde_json::to_vec(vector)?;
        let now = chrono::Utc::now().timestamp();

        let conn = self.conn.clone();
        let content = content.to_string();
        let tid = tenant_id.to_string();

        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            
            tx.execute(
                "INSERT INTO documents (id, path, content, metadata, vector, tenant_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, path, content, metadata_json, vector_blob, tid, now, now],
            )?;
            
            tx.execute(
                "INSERT INTO documents_fts (content, path, tenant_id) VALUES (?1, ?2, ?3)",
                params![content, path, tid],
            )?;
            
            tx.commit()?;
            Ok::<_, anyhow::Error>(())
        }).await??;

        Ok(())
    }

    pub async fn add_documents_batch(
        &self,
        documents: Vec<(String, HashMap<String, String>)>,
        tenant_id: Option<&str>,
        batch_size: Option<usize>,
    ) -> Result<usize> {
        let tenant_id = tenant_id.ok_or_else(|| anyhow::anyhow!("tenant_id is required"))?;
        if documents.is_empty() { return Ok(0); }
        let batch_size = batch_size.unwrap_or(20);
        let mut added = 0;

        for chunk in documents.chunks(batch_size) {
            let contents: Vec<&str> = chunk.iter().map(|(c, _)| c.as_str()).collect();
            let embeddings = self.provider.embed(contents).await?;
            for (i, (content, metadata)) in chunk.iter().enumerate() {
                self.add_document_with_vector(content, metadata.clone(), &embeddings[i], tenant_id).await?;
                added += 1;
            }
        }
        Ok(added)
    }

    pub fn remove_document_by_path(&self, path: &str, tenant_id: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tid = tenant_id.ok_or_else(|| anyhow::anyhow!("tenant_id is required"))?;
        conn.execute("DELETE FROM documents WHERE path = ?1 AND tenant_id = ?2", params![path, tid])?;
        conn.execute("DELETE FROM documents_fts WHERE path = ?1 AND tenant_id = ?2", params![path, tid])?;
        Ok(())
    }
    
    pub async fn update_document(&self, path: &str, content: &str, tenant_id: Option<&str>) -> Result<()> {
         let tenant_id = tenant_id.ok_or_else(|| anyhow::anyhow!("tenant_id is required"))?;
         let embeddings = self.provider.embed(vec![content]).await?;
         let vector = embeddings[0].clone();
         let mut metadata = HashMap::new();
         metadata.insert("path".to_string(), path.to_string());
         self.update_document_with_vector(path, content, metadata, &vector, tenant_id).await
    }

    async fn update_document_with_vector(
        &self,
        path: &str,
        content: &str,
        metadata: HashMap<String, String>,
        vector: &[f32],
        tenant_id: &str,
    ) -> Result<()> {
        let metadata_json = serde_json::to_string(&metadata)?;
        let vector_blob = serde_json::to_vec(vector)?;
        let now = chrono::Utc::now().timestamp();

        let conn = self.conn.clone();
        let path = path.to_string();
        let content = content.to_string();
        let tid = tenant_id.to_string();

        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;

            tx.execute(
                "DELETE FROM documents WHERE path = ?1 AND tenant_id = ?2",
                params![path, tid],
            )?;
            tx.execute(
                "DELETE FROM documents_fts WHERE path = ?1 AND tenant_id = ?2",
                params![path, tid],
            )?;

            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO documents (id, path, content, metadata, vector, tenant_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, path, content, metadata_json, vector_blob, tid, now, now],
            )?;
            tx.execute(
                "INSERT INTO documents_fts (content, path, tenant_id) VALUES (?1, ?2, ?3)",
                params![content, path, tid],
            )?;

            tx.commit()?;
            Ok::<_, anyhow::Error>(())
        }).await??;

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn search(&self, query: &str, limit: usize, tenant_id: Option<&str>) -> Result<Vec<(f32, VectorEntry)>> {
        let tenant_id = tenant_id.ok_or_else(|| anyhow::anyhow!("tenant_id is required"))?;
        
        // 1. Embed Query
        let query_embeddings = self.provider.embed(vec![query]).await?;
        let query_vec = query_embeddings[0].clone();

        let conn_arc = self.conn.clone();
        let tid = tenant_id.to_string();

        // 2. Vector Search (Streaming Scan)
        let vector_scores: HashMap<String, f32> = tokio::task::spawn_blocking(move || {
            let conn = conn_arc.lock().unwrap();
            // Read ID and Vector only
            let mut stmt = conn.prepare("SELECT id, vector FROM documents WHERE tenant_id = ?1")?;
            
            let mut scores = HashMap::new();
            let rows = stmt.query_map(params![tid], |row| {
                let id: String = row.get(0)?;
                let vec_blob: Vec<u8> = row.get(1)?;
                Ok((id, vec_blob))
            })?;

            for res in rows {
                if let Ok((id, blob)) = res {
                    if let Ok(vec) = serde_json::from_slice::<Vec<f32>>(&blob) {
                         let score = cosine_similarity(&query_vec, &vec);
                         scores.insert(id, score);
                    }
                }
            }
            Ok::<_, anyhow::Error>(scores)
        }).await??;

        // Get Top K Vector Results for RRF
        let mut top_vector_results: Vec<(String, f32)> = vector_scores.iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        top_vector_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        top_vector_results.truncate(limit * 2);

        // 3. FTS Search
        let conn_arc = self.conn.clone();
        let q_str = query.to_string();
        let tid_str = tenant_id.to_string();
        let limit_fts = limit * 2;

        let fts_ids: Vec<String> = tokio::task::spawn_blocking(move || {
            let conn = conn_arc.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT path FROM documents_fts WHERE documents_fts MATCH ?1 AND tenant_id = ?2 ORDER BY rank LIMIT ?3"
            )?;
            let paths: Vec<String> = stmt.query_map(params![format!("\"{}\"", q_str.replace("\"", "")), tid_str, limit_fts as i64], |row| {
                Ok(row.get::<_, String>(0)?)
            })?.filter_map(|r| r.ok()).collect();

            let mut ids = Vec::new();
            for path in paths {
                let mut id_stmt = conn.prepare("SELECT id FROM documents WHERE path = ?1 AND tenant_id = ?2")?;
                if let Ok(id) = id_stmt.query_row(params![path, tid_str], |r| r.get(0)) {
                    ids.push(id);
                }
            }
            Ok::<_, anyhow::Error>(ids)
        }).await??;

        // 4. RRF
        let rrf_k = 60.0;
        let mut rrf_scores: HashMap<String, f32> = HashMap::new();

        for (rank, (id, _)) in top_vector_results.iter().enumerate() {
            let score = 1.0 / (rrf_k + (rank as f32 + 1.0));
            *rrf_scores.entry(id.clone()).or_insert(0.0) += score;
        }

        for (rank, id) in fts_ids.iter().enumerate() {
            let score = 1.0 / (rrf_k + (rank as f32 + 1.0));
            *rrf_scores.entry(id.clone()).or_insert(0.0) += score;
        }

        // 5. Hydrate
        let ids: Vec<String> = rrf_scores.keys().cloned().collect();
        if ids.is_empty() { return Ok(Vec::new()); }

        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id, content, metadata, vector, tenant_id FROM documents WHERE id IN ({})", placeholders);
        let params_vec = ids.clone();
        let conn_arc = self.conn.clone();

        let documents: Vec<VectorEntry> = tokio::task::spawn_blocking(move || {
            let conn = conn_arc.lock().unwrap();
            let mut stmt = conn.prepare(&sql)?;
            let docs = stmt.query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                 let id: String = row.get(0)?;
                 let content: String = row.get(1)?;
                 let meta_str: String = row.get(2)?;
                 let vec_blob: Vec<u8> = row.get(3)?;
                 let tid: String = row.get(4)?;
                 let vec: Vec<f32> = serde_json::from_slice(&vec_blob).unwrap_or_default();
                 Ok(VectorEntry {
                     id, content, metadata: serde_json::from_str(&meta_str).unwrap_or_default(),
                     vector: vec, tenant_id: tid
                 })
            })?.filter_map(|r| r.ok()).collect();
            Ok::<Vec<VectorEntry>, anyhow::Error>(docs)
        }).await??;

        let mut final_results = Vec::new();
        for doc in documents {
            if let Some(&score) = rrf_scores.get(&doc.id) {
                final_results.push((score, doc));
            }
        }
        final_results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        final_results.truncate(limit);
        
        Ok(final_results)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot_product / (norm_a * norm_b) }
}
