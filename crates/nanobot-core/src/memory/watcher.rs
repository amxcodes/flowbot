use crate::memory::MemoryManager;
use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub struct WorkspaceWatcher {
    _watcher: RecommendedWatcher,
    // We keep the receiver loop handle if needed, or just let it run detached
    #[allow(dead_code)]
    tenant_id: String,
}

impl WorkspaceWatcher {
    pub fn new(
        root_path: PathBuf,
        memory_manager: Arc<MemoryManager>,
        tenant_id: Option<String>,
    ) -> Result<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel();

        // Create the watcher
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })?;

        // Start watching
        watcher.watch(&root_path, RecursiveMode::Recursive)?;
        info!("Started watching workspace at {:?}", root_path);

        // Spawn async event handler
        let root = root_path.clone();
        let tenant = tenant_id.ok_or_else(|| anyhow::anyhow!("tenant_id is required"))?;
        let tenant_clone = tenant.clone();

        tokio::spawn(async move {
            // Simple debounce logic could go here, for now processing raw events
            // Real-world: use a debounce crate or hashmap<path, instant>

            while let Some(event) = rx.recv().await {
                handle_event(event, &root, &memory_manager, &tenant_clone).await;
            }
        });

        Ok(Self {
            _watcher: watcher,
            tenant_id: tenant,
        })
    }
}

async fn handle_event(event: Event, root: &Path, memory: &Arc<MemoryManager>, tenant_id: &str) {
    // Basic filter: ignore .git, target, etc.
    // Better: use `ignore` crate to check if path is ignored.
    // For MVP transparency, we'll implement simple filtering here
    // as integrating `ignore` crate for single-file check is a bit verbose.

    for path in event.paths {
        if is_ignored(&path, root) {
            continue;
        }

        let rel_path = match path.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => continue,
        };

        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                // Read and update
                if path.is_file() {
                    match tokio::fs::read_to_string(&path).await {
                        Ok(content) => {
                            debug!("Indexing changed file: {}", rel_path);
                            if let Err(e) = memory
                                .update_document(&rel_path, &content, Some(tenant_id))
                                .await
                            {
                                error!("Failed to update document {}: {}", rel_path, e);
                            }
                        }
                        Err(e) => {
                            // File might be binary or deleted
                            debug!("Could not read file {}: {}", rel_path, e);
                        }
                    }
                }
            }
            EventKind::Remove(_) => {
                debug!("Removing deleted file: {}", rel_path);
                if let Err(e) = memory.remove_document_by_path(&rel_path, Some(tenant_id)) {
                    error!("Failed to remove document {}: {}", rel_path, e);
                }
            }
            _ => {}
        }
    }
}

fn is_ignored(path: &Path, _root: &Path) -> bool {
    let s = path.to_string_lossy();
    // Rudimentary ignore list
    if s.contains(".git") || s.contains("target") || s.contains("node_modules") {
        return true;
    }
    // Check extension
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy();
        if matches!(
            ext_str.as_ref(),
            "o" | "exe" | "dll" | "so" | "rlib" | "png" | "jpg" | "pdf"
        ) {
            return true;
        }
    }
    false
}
