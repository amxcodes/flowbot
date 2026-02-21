//! Tool batching and parallel execution optimizations
//!
//! Provides high-performance batching for:
//! - Parallel file reads (safe operations)
//! - Batch tool execution
//! - Concurrent safe operations

use anyhow::Result;
use futures::future::join_all;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Result of a batched operation
#[derive(Debug, Clone)]
pub struct BatchResult<T> {
    pub index: usize,
    pub result: Result<T, String>,
    pub duration_ms: u64,
}

/// High-performance batch executor for parallel-safe operations
pub struct BatchExecutor;

impl BatchExecutor {
    /// Execute multiple file reads in parallel
    ///
    /// # Example
    /// ```ignore
    /// let files = vec!["a.txt", "b.txt", "c.txt"];
    /// let results = BatchExecutor::read_files_parallel(files).await;
    /// ```
    pub async fn read_files_parallel(paths: Vec<PathBuf>) -> Vec<BatchResult<String>> {
        let start = Instant::now();

        let futures: Vec<_> = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| async move {
                let op_start = Instant::now();
                let result = tokio::fs::read_to_string(&path).await;

                BatchResult {
                    index,
                    result: result.map_err(|e| e.to_string()),
                    duration_ms: op_start.elapsed().as_millis() as u64,
                }
            })
            .collect();

        let results = join_all(futures).await;

        log::debug!(
            "Batch read {} files in {}ms",
            results.len(),
            start.elapsed().as_millis()
        );

        results
    }

    /// Execute multiple directory listings in parallel
    pub async fn list_directories_parallel(paths: Vec<PathBuf>) -> Vec<BatchResult<Vec<String>>> {
        let futures: Vec<_> = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| async move {
                let op_start = Instant::now();

                let result: Result<Vec<String>, String> = async {
                    let mut entries = tokio::fs::read_dir(&path)
                        .await
                        .map_err(|e| e.to_string())?;
                    let mut names = Vec::new();

                    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
                        names.push(entry.file_name().to_string_lossy().to_string());
                    }

                    Ok(names)
                }
                .await;

                BatchResult {
                    index,
                    result,
                    duration_ms: op_start.elapsed().as_millis() as u64,
                }
            })
            .collect();

        join_all(futures).await
    }

    /// Batch tool execution for safe operations
    ///
    /// Safe operations: read_file, list_directory, glob, grep, web_fetch
    pub async fn execute_safe_tools_batch(
        tool_calls: Vec<(String, serde_json::Value)>,
    ) -> Vec<BatchResult<String>> {
        let start = Instant::now();

        let futures: Vec<_> = tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, (tool_name, args))| async move {
                let op_start = Instant::now();

                // Check cache first
                let cache_key = format!("{}:{}", tool_name, args);
                if let Some(cached) = crate::cache::GLOBAL_CACHE.get_tool_result(&cache_key).await {
                    return BatchResult {
                        index,
                        result: Ok(cached),
                        duration_ms: 0, // Cache hit is instant
                    };
                }

                // Execute tool
                let result = match tool_name.as_str() {
                    "read_file" => execute_read_file(&args).await,
                    "list_directory" => execute_list_directory(&args).await,
                    "glob" => execute_glob(&args).await,
                    "web_fetch" => execute_web_fetch(&args).await,
                    _ => Err(format!("Tool '{}' not safe for batching", tool_name)),
                };

                // Cache successful results
                if let Ok(ref output) = result {
                    crate::cache::GLOBAL_CACHE
                        .cache_tool_result(cache_key, output.clone())
                        .await;
                }

                BatchResult {
                    index,
                    result,
                    duration_ms: op_start.elapsed().as_millis() as u64,
                }
            })
            .collect();

        let results = join_all(futures).await;

        log::debug!(
            "Batch executed {} tools in {}ms",
            results.len(),
            start.elapsed().as_millis()
        );

        results
    }

    /// Collect results into ordered map
    pub fn collect_ordered<T>(results: Vec<BatchResult<T>>) -> HashMap<usize, Result<T, String>> {
        results.into_iter().map(|r| (r.index, r.result)).collect()
    }
}

// Tool implementations for batching
async fn execute_read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'path' argument")?;

    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| e.to_string())
}

async fn execute_list_directory(args: &serde_json::Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'path' argument")?;

    let mut entries = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;

    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        names.push(entry.file_name().to_string_lossy().to_string());
    }

    Ok(serde_json::to_string(&names).unwrap_or_default())
}

async fn execute_glob(args: &serde_json::Value) -> Result<String, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'pattern' argument")?;

    let paths: Vec<String> = glob::glob(pattern)
        .map_err(|e| e.to_string())?
        .filter_map(|p| p.ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    Ok(serde_json::to_string(&paths).unwrap_or_default())
}

async fn execute_web_fetch(args: &serde_json::Value) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'url' argument")?;

    let client = crate::http_client::global_http_client();
    let _permit = client
        .acquire_permit()
        .await
        .map_err(|e| e.to_string())?;

    client
        .inner()
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())
}

/// Batch statistics for monitoring
#[derive(Debug, Default)]
pub struct BatchStats {
    pub total_operations: usize,
    pub successful: usize,
    pub failed: usize,
    pub cache_hits: usize,
    pub total_duration_ms: u64,
}

impl BatchStats {
    pub fn from_results<T>(results: &[BatchResult<T>]) -> Self {
        let total = results.len();
        let successful = results.iter().filter(|r| r.result.is_ok()).count();
        let failed = total - successful;
        let cache_hits = results.iter().filter(|r| r.duration_ms == 0).count();
        let total_duration: u64 = results.iter().map(|r| r.duration_ms).sum();

        Self {
            total_operations: total,
            successful,
            failed,
            cache_hits,
            total_duration_ms: total_duration,
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_operations == 0 {
            0.0
        } else {
            self.successful as f64 / self.total_operations as f64
        }
    }

    pub fn avg_duration_ms(&self) -> f64 {
        if self.total_operations == 0 {
            0.0
        } else {
            self.total_duration_ms as f64 / self.total_operations as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_batch_read_files() {
        // Create temp files
        let mut file1 = NamedTempFile::new().unwrap();
        let mut file2 = NamedTempFile::new().unwrap();
        writeln!(file1, "content1").unwrap();
        writeln!(file2, "content2").unwrap();

        let paths = vec![file1.path().to_path_buf(), file2.path().to_path_buf()];
        let results = BatchExecutor::read_files_parallel(paths).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].result.is_ok());
        assert!(results[1].result.is_ok());
    }

    #[tokio::test]
    async fn test_batch_stats() {
        let results = vec![
            BatchResult {
                index: 0,
                result: Ok("success".to_string()),
                duration_ms: 10,
            },
            BatchResult {
                index: 1,
                result: Err("error".to_string()),
                duration_ms: 5,
            },
        ];

        let stats = BatchStats::from_results(&results);
        assert_eq!(stats.total_operations, 2);
        assert_eq!(stats.successful, 1);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.success_rate(), 0.5);
    }
}
