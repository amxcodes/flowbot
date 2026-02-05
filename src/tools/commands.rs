// Command execution tool (Basic Revert)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::process::Stdio;

/// Arguments for running a command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub use_docker: bool,
    #[serde(default)]
    pub docker_image: Option<String>,
}

/// Command output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
}

// Whitelist of allowed commands (Expanded for VPS Admin - keeping this!)
const ALLOWED_COMMANDS: &[&str] = &[
    "cargo", "rustc", "git", 
    "npm", "node", "yarn", "pnpm",
    "python", "python3", "pip", "pip3", 
    "ls", "dir", "pwd", "cd", 
    "echo", "cat", "type", "grep", "find",
    "mkdir", "rm", "cp", "mv", "touch",
    "systemctl", "journalctl", "service", // VPS Management
    "docker", "docker-compose", // Container Management
    "curl", "wget", "ping", "netstat", // Network
    "apt", "apt-get", "yum", "dnf", // Package Management
    "whoami", "id", "uptime", "df", "free", "ps", "top", "htop" // System Info
];

/// Run a system command
pub async fn run_command(args: RunCommandArgs) -> Result<CommandOutput> {
    // Validate command is in whitelist
    if !ALLOWED_COMMANDS.contains(&args.command.as_str()) {
        return Err(anyhow::anyhow!(
            "Command '{}' is not in the allowed whitelist.",
            args.command
        ));
    }
    
    // Build command
    let mut cmd = tokio::process::Command::new(&args.command);
    cmd.args(&args.args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    
    let output = cmd.output().await?;
    
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);
    let success = output.status.success();
    
    Ok(CommandOutput {
        stdout,
        stderr,
        exit_code,
        success,
    })
}
