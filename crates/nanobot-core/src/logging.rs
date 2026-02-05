use anyhow::{Context, Result};
use std::path::PathBuf;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize file-based logging with rotation
pub fn init_file_logging() -> Result<()> {
    let log_dir = get_log_dir()?;
    
    // Create rolling file appender (rotates daily, keeps last 5 files)
    let file_appender = RollingFileAppender::new(
        Rotation::DAILY,
        &log_dir,
        "nanobot.log",
    );
    
    // Create file layer
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false) // No ANSI codes in log files
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true);
    
    // Create stdout layer (for console output)
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout);
    
    // Combine layers with env filter
    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    
    tracing::info!("Logging initialized: {}", log_dir.display());
    
    Ok(())
}

/// Get the log directory path
pub fn get_log_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let log_dir = home.join(".nanobot").join("logs");
    
    std::fs::create_dir_all(&log_dir)
        .context("Failed to create log directory")?;
    
    Ok(log_dir)
}

/// Clean up old log files (keep only last N files)
pub fn cleanup_old_logs(keep: usize) -> Result<()> {
    let log_dir = get_log_dir()?;
    
    let mut log_files: Vec<_> = std::fs::read_dir(&log_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();
    
    // Sort by modification time (oldest first)
    log_files.sort_by_key(|entry| {
        entry.metadata()
            .and_then(|m| m.modified())
            .ok()
    });
    
    // Remove oldest files, keeping only 'keep' most recent
    let to_remove = log_files.len().saturating_sub(keep);
    for entry in log_files.iter().take(to_remove) {
        std::fs::remove_file(entry.path())?;
        tracing::info!("Removed old log file: {:?}", entry.path());
    }
    
    Ok(())
}
