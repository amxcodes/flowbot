// Tools module - Provides file system, web search, and command execution capabilities

pub mod batch;
pub mod channel_confirmation;
pub mod cli_confirmation;
pub mod cli_wrapper;
pub mod commands;
pub mod confirmation;
pub mod cron;
pub mod definitions;
pub mod docker;
pub mod docker_executor;
pub mod executor;
pub mod gateway_confirmation;
pub mod guard;
pub mod permissions;
pub mod telegram_confirmation;

pub use channel_confirmation::ChannelConfirmationResponse;
pub use confirmation::{
    ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse, ConfirmationService, RiskLevel,
};
pub use permissions::{Operation, PermissionDecision, PermissionManager, SecurityProfile};
mod fetch;
mod filesystem;
pub mod llm_task;
pub mod policy;
mod process;
pub mod question;
pub mod run_command_tool;
pub mod script_eval_tool;
mod search;
pub mod sessions;
pub mod stt;
pub mod subagent_tools;
mod todos;
pub mod tts;
pub mod web_search_tool;
pub mod websearch;

use anyhow::Result;

mod sealed {
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct ExecutorToken(());

    impl ExecutorToken {
        pub(in crate::tools) const fn new(_key: super::executor::ExecutorMintKey) -> Self {
            Self(())
        }
    }
}

pub(crate) use sealed::ExecutorToken;

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
                || path_str.starts_with("C:\\System32")
            {
                return Err(anyhow::anyhow!(
                    "Access to system directories is restricted"
                ));
            }
        }

        #[cfg(unix)]
        {
            let path_str = path.to_string_lossy();
            if path_str.starts_with("/etc")
                || path_str.starts_with("/sys")
                || path_str.starts_with("/proc")
                || path_str.starts_with("/root")
            {
                return Err(anyhow::anyhow!(
                    "Access to system directories is restricted"
                ));
            }
        }
    }

    // Check for directory traversal
    for component in path.components() {
        if component == std::path::Component::ParentDir {
            return Err(anyhow::anyhow!("Path traversal (..) is not allowed"));
        }
    }

    let candidate = if path.is_relative() {
        std::env::current_dir()?.join(path)
    } else {
        path.to_path_buf()
    };

    let normalized = normalize_for_scope(&candidate)?;
    Ok(normalized)
}

fn normalize_for_scope(path: &std::path::Path) -> Result<std::path::PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }

    let mut missing_parts = Vec::new();
    let mut cursor = path;

    while !cursor.exists() {
        if let Some(name) = cursor.file_name() {
            missing_parts.push(name.to_os_string());
        }
        cursor = cursor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Path has no existing ancestor: {}", path.display()))?;
    }

    let mut resolved = cursor.canonicalize()?;
    for part in missing_parts.iter().rev() {
        resolved.push(part);
    }

    Ok(resolved)
}
