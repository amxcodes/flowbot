#![allow(dead_code)]
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::types::{ServiceRuntime, ServiceStatus};

const SERVICE_NAME: &str = "nanobot";

/// Get the systemd user service directory
fn get_service_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let service_dir = PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user");
    
    fs::create_dir_all(&service_dir)
        .context("Failed to create systemd user service directory")?;
    
    Ok(service_dir)
}

/// Get the path to the service file
fn get_service_file() -> Result<PathBuf> {
    Ok(get_service_dir()?.join(format!("{}.service", SERVICE_NAME)))
}

/// Generate systemd service unit content
fn generate_service_unit(binary_path: &str, working_dir: &str) -> String {
    format!(
        r#"[Unit]
Description=Nanobot AI Assistant Gateway
After=network.target

[Service]
Type=simple
ExecStart={} gateway
WorkingDirectory={}
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

# Environment variables (add your config here)
# Environment="TELEGRAM_BOT_TOKEN=your_token"
# Environment="ANTIGRAVITY_API_KEY=your_key"

[Install]
WantedBy=default.target
"#,
        binary_path, working_dir
    )
}

/// Install the systemd service
pub fn install() -> Result<()> {
    let exe_path = std::env::current_exe().context("Failed to get current executable path")?;
    let exe_path_str = exe_path
        .to_str()
        .context("Executable path contains invalid UTF-8")?;
    
    let working_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .to_str()
        .context("Working directory contains invalid UTF-8")?
        .to_string();
    
    let service_content = generate_service_unit(exe_path_str, &working_dir);
    let service_file = get_service_file()?;
    
    fs::write(&service_file, service_content)
        .context("Failed to write service file")?;
    
    println!("✅ Service file created: {}", service_file.display());
    println!();
    println!("Reloading systemd daemon...");
    
    let output = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to reload systemd daemon")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to reload systemd daemon: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Systemd daemon reloaded");
    println!();
    println!("Service installed successfully!");
    println!("To start the service:");
    println!("  nanobot service start");
    println!("To enable auto-start on boot:");
    println!("  systemctl --user enable {}", SERVICE_NAME);
    
    Ok(())
}

/// Uninstall the systemd service
pub fn uninstall() -> Result<()> {
    let service_file = get_service_file()?;
    
    if !service_file.exists() {
        println!("Service not installed");
        return Ok(());
    }
    
    // Stop the service first
    let _ = stop();
    
    // Disable the service
    let _ = Command::new("systemctl")
        .args(["--user", "disable", SERVICE_NAME])
        .output();
    
    // Remove the service file
    fs::remove_file(&service_file)
        .context("Failed to remove service file")?;
    
    // Reload daemon
    let output = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to reload systemd daemon")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to reload systemd daemon: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service uninstalled successfully");
    
    Ok(())
}

/// Start the systemd service
pub fn start() -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "start", SERVICE_NAME])
        .output()
        .context("Failed to start service")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to start service: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service started");
    Ok(())
}

/// Stop the systemd service
pub fn stop() -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "stop", SERVICE_NAME])
        .output()
        .context("Failed to stop service")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to stop service: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service stopped");
    Ok(())
}

/// Restart the systemd service
pub fn restart() -> Result<()> {
    let output = Command::new("systemctl")
        .args(["--user", "restart", SERVICE_NAME])
        .output()
        .context("Failed to restart service")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to restart service: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service restarted");
    Ok(())
}

/// Get the service status
pub fn status() -> Result<ServiceRuntime> {
    let output = Command::new("systemctl")
        .args(["--user", "status", SERVICE_NAME])
        .output()
        .context("Failed to get service status")?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Parse systemd status output
    let status = if stdout.contains("Active: active (running)") {
        ServiceStatus::Running
    } else if stdout.contains("Active: inactive") || stdout.contains("Active: failed") {
        ServiceStatus::Stopped
    } else {
        ServiceStatus::Unknown
    };
    
    // Try to extract PID
    let pid = stdout
        .lines()
        .find(|line| line.contains("Main PID:"))
        .and_then(|line| {
            line.split_whitespace()
                .nth(2)
                .and_then(|s| s.parse::<u32>().ok())
        });
    
    Ok(ServiceRuntime {
        status,
        pid,
        uptime_seconds: None, // TODO: Parse from systemd output
        last_exit_code: None,
        last_exit_reason: None,
    })
}

/// Check if the service is installed
pub fn is_installed() -> bool {
    get_service_file().ok().map(|f| f.exists()).unwrap_or(false)
}
