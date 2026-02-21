// Command execution tool (Basic Revert)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::process::Stdio;

pub(crate) fn dangerous_command_detected(command: &str, args: &[String]) -> bool {
    let lower_cmd = command.to_ascii_lowercase();
    let lower_joined = args
        .iter()
        .map(|a| a.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    if lower_cmd == "rm" && lower_joined.contains("-rf") {
        return true;
    }

    if lower_cmd == "del" && lower_joined.contains("/f") && lower_joined.contains("/s") {
        return true;
    }

    let full = format!("{} {}", lower_cmd, lower_joined);
    full.contains("rm -rf /") || full.contains("del /f /s /q") || full.contains("format")
}

pub(crate) fn command_allowed(command: &str) -> bool {
    ALLOWED_COMMANDS.contains(&command)
}

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
    "cargo",
    "rustc",
    "git",
    "npm",
    "node",
    "yarn",
    "pnpm",
    "python",
    "python3",
    "pip",
    "pip3",
    "ls",
    "dir",
    "pwd",
    "cd",
    "echo",
    "cat",
    "type",
    "grep",
    "find",
    "mkdir",
    "rm",
    "cp",
    "mv",
    "touch",
    "systemctl",
    "journalctl",
    "service", // VPS Management
    "docker",
    "docker-compose", // Container Management
    "curl",
    "wget",
    "ping",
    "netstat", // Network
    "apt",
    "apt-get",
    "yum",
    "dnf", // Package Management
    "whoami",
    "id",
    "uptime",
    "df",
    "free",
    "ps",
    "top",
    "htop", // System Info
];

/// Run a system command
pub(super) async fn run_command(
    _token: &super::ExecutorToken,
    args: RunCommandArgs,
) -> Result<CommandOutput> {
    // Validate command is in whitelist
    if !command_allowed(&args.command) {
        return Err(anyhow::anyhow!(
            "Command '{}' is not in the allowed whitelist.",
            args.command
        ));
    }

    if dangerous_command_detected(&args.command, &args.args)
        && std::env::var("NANOBOT_ALLOW_DANGEROUS_COMMANDS")
            .ok()
            .as_deref()
            != Some("1")
    {
        return Err(anyhow::anyhow!(
            "Blocked dangerous command. Set NANOBOT_ALLOW_DANGEROUS_COMMANDS=1 to override explicitly."
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
