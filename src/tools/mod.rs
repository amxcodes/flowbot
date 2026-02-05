// Tools module - Provides file system, web search, and command execution capabilities

pub mod filesystem;
pub mod websearch;
pub mod commands;
pub mod executor;
pub mod docker;
pub mod definitions;
pub mod process;
pub mod fetch;
pub mod cron;
pub mod sessions;
pub mod cli_wrapper;
pub mod read_file_tool;
pub mod write_file_tool;
pub mod list_directory_tool;
pub mod web_search_tool;
pub mod run_command_tool;
pub mod policy;
pub mod subagent_tools;



use anyhow::Result;

// Re-export key types
pub use policy::ToolPolicy;

use serde::{Deserialize, Serialize};

/// Common result type for all tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    pub fn success(output: String) -> Self {
        Self {
            success: true,
            output,
            error: None,
        }
    }

    pub fn error(error: String) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error),
        }
    }
}

/// Validate that a path is safe to access
/// - Must be relative or within workspace
/// - No directory traversal (../)
/// - No access to system directories
pub fn validate_path(path: &str) -> Result<std::path::PathBuf> {
    use std::path::Path;
    
    let path = Path::new(path);
    
    // Reject absolute paths to system directories
    if path.is_absolute() {
        #[cfg(windows)]
        {
            let path_str = path.to_string_lossy();
            if path_str.starts_with("C:\\Windows") 
                || path_str.starts_with("C:\\Program Files")
                || path_str.starts_with("C:\\System32") {
                return Err(anyhow::anyhow!("Access to system directories is restricted"));
            }
        }
        
        #[cfg(unix)]
        {
            let path_str = path.to_string_lossy();
            if path_str.starts_with("/etc")
                || path_str.starts_with("/sys")
                || path_str.starts_with("/proc")
                || path_str.starts_with("/root") {
                return Err(anyhow::anyhow!("Access to system directories is restricted"));
            }
        }
    }
    
    // Check for directory traversal
    for component in path.components() {
        if component == std::path::Component::ParentDir {
            return Err(anyhow::anyhow!("Path traversal (..) is not allowed"));
        }
    }
    
    // Make relative to current working directory
    let canonical = if path.is_relative() {
        std::env::current_dir()?.join(path)
    } else {
        path.to_path_buf()
    };
    
    Ok(canonical)
}
