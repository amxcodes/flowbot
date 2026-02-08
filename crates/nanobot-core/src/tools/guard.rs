use serde_json::Value;
use anyhow::Result;

/// ToolGuard provides pre-execution validation for tools
pub struct ToolGuard;

impl ToolGuard {
    /// Validate tool arguments against expected schema
    pub fn validate_args(tool_name: &str, args: &Value) -> Result<()> {
        match tool_name {
            "run_command" | "spawn_process" => {
                Self::validate_command_args(args)?;
            }
            "write_file" | "edit_file" => {
                Self::validate_file_write_args(args)?;
            }
            "read_file" | "list_directory" => {
                Self::validate_file_read_args(args)?;
            }
            _ => {
                // Unknown tools pass through (permissive by default)
            }
        }
        Ok(())
    }

    fn validate_command_args(args: &Value) -> Result<()> {
        let cmd = args.get("cmd")
            .or_else(|| args.get("command"))
            .ok_or_else(|| anyhow::anyhow!("Missing 'cmd' or 'command' argument"))?;

        if !cmd.is_string() {
            return Err(anyhow::anyhow!("Command must be a string"));
        }

        let cmd_str = cmd.as_str().unwrap();
        if cmd_str.is_empty() {
            return Err(anyhow::anyhow!("Command cannot be empty"));
        }

        // Warn about dangerous patterns (but don't block)
        let dangerous_patterns = ["rm -rf /", "format", "del /f /s /q"];
        for pattern in &dangerous_patterns {
            if cmd_str.contains(pattern) {
                tracing::warn!("⚠️ Detected potentially dangerous command: {}", pattern);
            }
        }

        Ok(())
    }

    fn validate_file_write_args(args: &Value) -> Result<()> {
        let path = args.get("path")
            .or_else(|| args.get("file_path"))
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

        if !path.is_string() {
            return Err(anyhow::anyhow!("Path must be a string"));
        }

        let path_str = path.as_str().unwrap();
        if path_str.is_empty() {
            return Err(anyhow::anyhow!("Path cannot be empty"));
        }

        // Validate against system-critical paths
        let critical_paths = ["/etc/", "/sys/", "/proc/", "C:\\Windows\\System32"];
        for critical in &critical_paths {
            if path_str.starts_with(critical) {
                return Err(anyhow::anyhow!(
                    "Cannot write to system-critical path: {}",
                    critical
                ));
            }
        }

        Ok(())
    }

    fn validate_file_read_args(args: &Value) -> Result<()> {
        let path = args.get("path")
            .or_else(|| args.get("file_path"))
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

        if !path.is_string() {
            return Err(anyhow::anyhow!("Path must be a string"));
        }

        let path_str = path.as_str().unwrap();
        if path_str.is_empty() {
            return Err(anyhow::anyhow!("Path cannot be empty"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_command_args_valid() {
        let args = json!({"cmd": "ls -la"});
        assert!(ToolGuard::validate_args("run_command", &args).is_ok());
    }

    #[test]
    fn test_validate_command_args_missing() {
        let args = json!({});
        assert!(ToolGuard::validate_args("run_command", &args).is_err());
    }

    #[test]
    fn test_validate_file_write_critical_path() {
        let args = json!({"path": "/etc/passwd"});
        let result = ToolGuard::validate_args("write_file", &args);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_file_write_valid() {
        let args = json!({"path": "/tmp/test.txt"});
        assert!(ToolGuard::validate_args("write_file", &args).is_ok());
    }
}
