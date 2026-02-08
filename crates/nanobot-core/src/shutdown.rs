use std::sync::atomic::{AtomicBool, Ordering};
use tokio::signal;
use anyhow::Result;

/// Global shutdown signal
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Check if shutdown has been requested
pub fn is_shutdown() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}

/// Request shutdown
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::Relaxed);
    tracing::info!("Shutdown requested");
}

/// Setup graceful shutdown handler
/// Returns a future that resolves when Ctrl+C or SIGTERM is received
pub async fn setup_shutdown_handler() -> Result<()> {
    #[cfg(unix)]
    {
        use signal::unix::{signal, SignalKind};
        
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;
        
        tokio::select! {
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM");
            }
            _ = sigint.recv() => {
                tracing::info!("Received SIGINT (Ctrl+C)");
            }
        }
    }
    
    #[cfg(windows)]
    {
        signal::ctrl_c().await?;
        tracing::info!("Received Ctrl+C");
    }
    
    request_shutdown();
    Ok(())
}

/// Shutdown coordinator that waits for all components to finish
pub struct ShutdownCoordinator {
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl ShutdownCoordinator {
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }
    
    /// Add a task handle to track
    pub fn add_handle(&mut self, handle: tokio::task::JoinHandle<()>) {
        self.handles.push(handle);
    }
    
    /// Wait for all tasks to complete gracefully
    pub async fn shutdown(self, timeout_secs: u64) {
        tracing::info!("Beginning graceful shutdown (timeout: {}s)", timeout_secs);
        
        let shutdown_future = async {
            for handle in self.handles {
                if let Err(e) = handle.await {
                    tracing::error!("Task panicked during shutdown: {:?}", e);
                }
            }
        };
        
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(timeout_secs),
            shutdown_future
        ).await {
            Ok(_) => {
                tracing::info!("All tasks shut down successfully");
            }
            Err(_) => {
                tracing::warn!("Shutdown timeout reached, forcing exit");
            }
        }
    }
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new()
    }
}
