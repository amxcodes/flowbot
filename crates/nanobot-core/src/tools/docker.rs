// Docker execution strategy
// Wraps commands in a disposable Docker container

use super::commands::{CommandOutput, RunCommandArgs};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::process::Stdio;

/// Configuration for Docker execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    pub image: String,
    pub workdir: String,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            image: "python:3.11-slim".to_string(), // Robust default image with tools
            workdir: "/workspace".to_string(),
        }
    }
}

/// Run a command inside a Docker container
/// This mounts the current workspace to /workspace
pub async fn run_docker_command(args: RunCommandArgs) -> Result<CommandOutput> {
    let mut config = DockerConfig::default();

    // Allow image override if provided in args (we need to update RunCommandArgs to support this,
    // or parse it from env/config. For now, let's infer from the command or allow a special arg?)
    // Actually, best way is to let the tool caller specify 'image' in the JSON, but RunCommandArgs is shared.
    // Let's modify RunCommandArgs in commands.rs to include optional `image`.

    // For now, simple inference:
    if args.command == "node" || args.command == "npm" {
        config.image = "node:20-slim".to_string();
    } else if args.command == "rustc" || args.command == "cargo" {
        config.image = "rust:latest".to_string();
    } else if let Some(img) = &args.docker_image {
        config.image = img.clone();
    }

    let current_dir = std::env::current_dir()?;
    let current_dir_str = current_dir.to_string_lossy();

    // Construct Docker command args
    // docker run --rm -v "C:\Path:/workspace" -w /workspace [image] [command] ...
    let mut docker_args = vec![
        "run".to_string(),
        "--rm".to_string(), // Cleanup container after exit
        "-v".to_string(),
        format!("{}:{}", current_dir_str, config.workdir), // Mount workspace
        "-w".to_string(),
        config.workdir.clone(), // Set working directory
        config.image,
        args.command,
    ];

    // Append arguments to the command being run inside Docker
    docker_args.extend(args.args);

    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(&docker_args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Execute
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
