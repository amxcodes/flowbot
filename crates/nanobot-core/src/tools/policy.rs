use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

/// Policy violation error types
#[derive(Debug, Error)]
pub enum PolicyViolation {
    #[error("Tool '{0}' is not allowed by policy")]
    ToolNotAllowed(String),

    #[error("Path '{path}' is not allowed for {operation}")]
    PathNotAllowed { path: PathBuf, operation: String },

    #[error("Command '{0}' is not allowed by policy")]
    CommandNotAllowed(String),

    #[error("Tool '{0}' requires user approval")]
    ApprovalRequired(String),

    #[error("File size {actual} bytes exceeds maximum of {max} bytes")]
    FileSizeExceeded { actual: u64, max: u64 },

    #[error("Command timeout {actual:?} exceeds maximum of {max:?}")]
    TimeoutExceeded { actual: Duration, max: Duration },
}

/// Tool execution safety policy
#[derive(Debug, Clone)]
pub struct ToolPolicy {
    /// Tools that are explicitly allowed (None = all allowed)
    pub allowed_tools: Option<HashSet<String>>,

    /// Tools that are explicitly denied
    pub denied_tools: HashSet<String>,

    /// Paths allowed for read operations (glob patterns supported)
    pub allowed_read_paths: Vec<String>,

    /// Paths allowed for write operations (glob patterns supported)
    pub allowed_write_paths: Vec<String>,

    /// Commands allowed for execution (None = all allowed)
    pub allowed_commands: Option<HashSet<String>>,

    /// Tools requiring user approval before execution
    pub require_approval_for: HashSet<String>,

    /// Maximum file size for read/write operations
    pub max_file_size: u64,

    /// Maximum command execution timeout
    pub max_command_timeout: Duration,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self::permissive()
    }
}

impl ToolPolicy {
    /// Create a permissive policy (current Nanobot behavior)
    pub fn permissive() -> Self {
        Self {
            allowed_tools: None, // All tools allowed
            denied_tools: HashSet::new(),
            allowed_read_paths: vec!["**".to_string()], // All paths allowed
            allowed_write_paths: vec!["**".to_string()],
            allowed_commands: None, // All commands allowed
            require_approval_for: HashSet::new(),
            max_file_size: 100 * 1024 * 1024,              // 100MB
            max_command_timeout: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Create a restrictive policy (safe defaults)
    pub fn restrictive() -> Self {
        Self {
            allowed_tools: Some(HashSet::new()), // No tools allowed by default
            denied_tools: HashSet::new(),
            allowed_read_paths: Vec::new(), // No paths allowed
            allowed_write_paths: Vec::new(),
            allowed_commands: Some(HashSet::new()), // No commands allowed
            require_approval_for: HashSet::new(),
            max_file_size: 10 * 1024 * 1024,              // 10MB
            max_command_timeout: Duration::from_secs(60), // 1 minute
        }
    }

    /// Builder: Allow a specific tool
    pub fn allow_tool(mut self, tool: impl Into<String>) -> Self {
        if self.allowed_tools.is_none() {
            self.allowed_tools = Some(HashSet::new());
        }
        self.allowed_tools.as_mut().unwrap().insert(tool.into());
        self
    }

    /// Builder: Deny a specific tool
    pub fn deny_tool(mut self, tool: impl Into<String>) -> Self {
        self.denied_tools.insert(tool.into());
        self
    }

    /// Builder: Allow reading from a path pattern
    pub fn allow_read_path(mut self, path: impl Into<String>) -> Self {
        self.allowed_read_paths.push(path.into());
        self
    }

    /// Builder: Allow writing to a path pattern
    pub fn allow_write_path(mut self, path: impl Into<String>) -> Self {
        self.allowed_write_paths.push(path.into());
        self
    }

    /// Builder: Deny all write operations
    pub fn deny_write(mut self) -> Self {
        self.allowed_write_paths.clear();
        self
    }

    /// Builder: Allow a specific command
    pub fn allow_command(mut self, command: impl Into<String>) -> Self {
        if self.allowed_commands.is_none() {
            self.allowed_commands = Some(HashSet::new());
        }
        self.allowed_commands
            .as_mut()
            .unwrap()
            .insert(command.into());
        self
    }

    /// Builder: Require approval for a tool
    pub fn require_approval(mut self, tool: impl Into<String>) -> Self {
        self.require_approval_for.insert(tool.into());
        self
    }

    /// Check if a tool is allowed to execute
    pub fn check_tool_allowed(&self, tool_name: &str) -> Result<(), PolicyViolation> {
        // Check explicit denials first
        if self.denied_tools.contains(tool_name) {
            return Err(PolicyViolation::ToolNotAllowed(tool_name.to_string()));
        }

        // If allowlist exists, check if tool is in it
        if let Some(ref allowed) = self.allowed_tools
            && !allowed.contains(tool_name)
        {
            return Err(PolicyViolation::ToolNotAllowed(tool_name.to_string()));
        }

        // Check if approval is required
        if self.require_approval_for.contains(tool_name) {
            return Err(PolicyViolation::ApprovalRequired(tool_name.to_string()));
        }

        Ok(())
    }

    /// Check if a path is allowed for reading
    pub fn check_read_path(&self, path: &str) -> Result<(), PolicyViolation> {
        if self.allowed_read_paths.is_empty() {
            return Err(PolicyViolation::PathNotAllowed {
                path: PathBuf::from(path),
                operation: "read".to_string(),
            });
        }

        // Simple wildcard matching for now (can be enhanced with glob crate)
        let allowed = self.allowed_read_paths.iter().any(|pattern| {
            pattern == "**" || pattern == path || path.starts_with(pattern.trim_end_matches("/**"))
        });

        if !allowed {
            return Err(PolicyViolation::PathNotAllowed {
                path: PathBuf::from(path),
                operation: "read".to_string(),
            });
        }

        Ok(())
    }

    /// Check if a path is allowed for writing
    pub fn check_write_path(&self, path: &str) -> Result<(), PolicyViolation> {
        if self.allowed_write_paths.is_empty() {
            return Err(PolicyViolation::PathNotAllowed {
                path: PathBuf::from(path),
                operation: "write".to_string(),
            });
        }

        let allowed = self.allowed_write_paths.iter().any(|pattern| {
            pattern == "**" || pattern == path || path.starts_with(pattern.trim_end_matches("/**"))
        });

        if !allowed {
            return Err(PolicyViolation::PathNotAllowed {
                path: PathBuf::from(path),
                operation: "write".to_string(),
            });
        }

        Ok(())
    }

    /// Check if a command is allowed for execution
    pub fn check_command_allowed(&self, command: &str) -> Result<(), PolicyViolation> {
        if let Some(ref allowed) = self.allowed_commands
            && !allowed.contains(command)
        {
            return Err(PolicyViolation::CommandNotAllowed(command.to_string()));
        }

        Ok(())
    }

    /// Check if file size is within limits
    pub fn check_file_size(&self, size: u64) -> Result<(), PolicyViolation> {
        if size > self.max_file_size {
            return Err(PolicyViolation::FileSizeExceeded {
                actual: size,
                max: self.max_file_size,
            });
        }
        Ok(())
    }

    /// Production-oriented default: broad functionality with explicit approvals
    /// for high-risk operations.
    pub fn ask_me_default() -> Self {
        Self::permissive()
            .require_approval("run_command")
            .require_approval("spawn_process")
            .require_approval("bash")
            .require_approval("exec")
            .require_approval("apply_patch")
            .require_approval("browser_navigate")
            .require_approval("browser_click")
            .require_approval("browser_type")
            .require_approval("browser_screenshot")
            .require_approval("browser_evaluate")
            .require_approval("browser_pdf")
            .require_approval("browser_list_tabs")
            .require_approval("browser_switch_tab")
    }

    /// Headless deny profile: block command and browser execution entirely.
    pub fn headless_deny_default() -> Self {
        Self::permissive()
            .deny_tool("run_command")
            .deny_tool("spawn_process")
            .deny_tool("bash")
            .deny_tool("exec")
            .deny_tool("browser_navigate")
            .deny_tool("browser_click")
            .deny_tool("browser_type")
            .deny_tool("browser_screenshot")
            .deny_tool("browser_evaluate")
            .deny_tool("browser_pdf")
            .deny_tool("browser_list_tabs")
            .deny_tool("browser_switch_tab")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissive_policy() {
        let policy = ToolPolicy::permissive();
        assert!(policy.check_tool_allowed("read_file").is_ok());
        assert!(policy.check_read_path("/any/path").is_ok());
        assert!(policy.check_write_path("/any/path").is_ok());
        assert!(policy.check_command_allowed("any_command").is_ok());
    }

    #[test]
    fn test_restrictive_policy() {
        let policy = ToolPolicy::restrictive();
        assert!(policy.check_tool_allowed("read_file").is_err());
        assert!(policy.check_read_path("/any/path").is_err());
        assert!(policy.check_write_path("/any/path").is_err());
        assert!(policy.check_command_allowed("any_command").is_err());
    }

    #[test]
    fn test_builder_pattern() {
        let policy = ToolPolicy::restrictive()
            .allow_tool("read_file")
            .allow_read_path("/home/user/**")
            .deny_write();

        assert!(policy.check_tool_allowed("read_file").is_ok());
        assert!(policy.check_tool_allowed("write_file").is_err());
        assert!(policy.check_read_path("/home/user/doc.txt").is_ok());
        assert!(policy.check_write_path("/any/path").is_err());
    }
}
