use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Entry in the cron job execution log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRunEntry {
    pub ts: u64,
    pub job_id: String,
    pub action: String, // "finished"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>, // "ok", "error", "skipped"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at_ms: Option<u64>,
}

/// Resolve path to run log file for a job
pub fn resolve_run_log_path(db_path: &Path, job_id: &str) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("runs")
        .join(format!("{}.jsonl", job_id))
}

/// Append entry to run log with automatic pruning
pub fn append_run_log(
    file_path: &Path,
    entry: &CronRunEntry,
    max_bytes: usize,
    keep_lines: usize,
) -> Result<()> {
    // Ensure directory exists
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Append entry
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)?;

    let json = serde_json::to_string(entry)?;
    writeln!(file, "{}", json)?;
    drop(file);

    // Check if pruning is needed
    let metadata = fs::metadata(file_path)?;
    if metadata.len() as usize <= max_bytes {
        return Ok(());
    }

    // Prune: keep last N lines
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();

    if lines.len() <= keep_lines {
        return Ok(());
    }

    // Keep last N lines
    let kept = lines
        .into_iter()
        .rev()
        .take(keep_lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();

    // Write to temp file, then rename
    let tmp_path = file_path.with_extension("tmp");
    let mut tmp_file = File::create(&tmp_path)?;
    for line in kept {
        writeln!(tmp_file, "{}", line)?;
    }
    drop(tmp_file);

    fs::rename(tmp_path, file_path)?;

    Ok(())
}

/// Read run log entries in reverse chronological order
pub fn read_run_log(file_path: &Path, limit: usize) -> Result<Vec<CronRunEntry>> {
    if !file_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut entries = Vec::new();
    let lines: Vec<String> = reader
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();

    // Parse in reverse order (most recent first)
    for line in lines.iter().rev().take(limit) {
        if let Ok(entry) = serde_json::from_str::<CronRunEntry>(line) {
            // Validate entry
            if entry.action == "finished" && !entry.job_id.is_empty() {
                entries.push(entry);
            }
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_append_and_read() {
        let temp_dir = env::temp_dir();
        let log_path = temp_dir.join("test_cron_run.jsonl");

        // Clean up from previous test
        let _ = fs::remove_file(&log_path);

        let entry1 = CronRunEntry {
            ts: 1000,
            job_id: "test-job".to_string(),
            action: "finished".to_string(),
            status: Some("ok".to_string()),
            error: None,
            summary: Some("Success".to_string()),
            run_at_ms: Some(1000),
            duration_ms: Some(100),
            next_run_at_ms: Some(2000),
        };

        let entry2 = CronRunEntry {
            ts: 2000,
            job_id: "test-job".to_string(),
            action: "finished".to_string(),
            status: Some("error".to_string()),
            error: Some("Test error".to_string()),
            summary: None,
            run_at_ms: Some(2000),
            duration_ms: Some(50),
            next_run_at_ms: None,
        };

        // Append entries
        append_run_log(&log_path, &entry1, 2_000_000, 2000).unwrap();
        append_run_log(&log_path, &entry2, 2_000_000, 2000).unwrap();

        // Read entries (should be in reverse chronological order)
        let entries = read_run_log(&log_path, 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].ts, 2000); // Most recent first
        assert_eq!(entries[1].ts, 1000);

        // Clean up
        let _ = fs::remove_file(&log_path);
    }

    #[test]
    fn test_auto_prune() {
        let temp_dir = env::temp_dir();
        let log_path = temp_dir.join("test_prune.jsonl");

        // Clean up
        let _ = fs::remove_file(&log_path);

        // Add many entries
        for i in 0..100 {
            let entry = CronRunEntry {
                ts: i as u64,
                job_id: "test".to_string(),
                action: "finished".to_string(),
                status: Some("ok".to_string()),
                error: None,
                summary: None,
                run_at_ms: Some(i as u64),
                duration_ms: Some(10),
                next_run_at_ms: None,
            };
            // Small max_bytes to trigger pruning, keep only 10 lines
            append_run_log(&log_path, &entry, 500, 10).unwrap();
        }

        // Should have pruned to 10 entries
        let entries = read_run_log(&log_path, 100).unwrap();
        assert!(entries.len() <= 10);

        // Most recent should be preserved
        assert_eq!(entries[0].ts, 99);

        // Clean up
        let _ = fs::remove_file(&log_path);
    }
}
