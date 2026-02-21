#![allow(dead_code)]
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::types::{ServiceRuntime, ServiceStatus};

const SERVICE_NAME: &str = "nanobot";

fn parse_systemd_uptime_seconds(stdout: &str) -> Option<u64> {
    // Example: Active: active (running) since ...; 2h 3min ago
    let active_line = stdout.lines().find(|line| line.contains("Active:"))?;
    let (_, tail) = active_line.split_once(';')?;
    let human = tail
        .trim()
        .strip_suffix(" ago")
        .unwrap_or(tail.trim())
        .to_ascii_lowercase();

    let mut total = 0u64;
    for part in human.split_whitespace() {
        if let Some(v) = part.strip_suffix("ms") {
            if let Ok(ms) = v.parse::<u64>() {
                total = total.saturating_add(ms / 1000);
            }
        } else if let Some(v) = part.strip_suffix('s') {
            if let Ok(sec) = v.parse::<u64>() {
                total = total.saturating_add(sec);
            }
        } else if let Some(v) = part.strip_suffix("min") {
            if let Ok(min) = v.parse::<u64>() {
                total = total.saturating_add(min.saturating_mul(60));
            }
        } else if let Some(v) = part.strip_suffix('h') {
            if let Ok(hour) = v.parse::<u64>() {
                total = total.saturating_add(hour.saturating_mul(3600));
            }
        } else if let Some(v) = part.strip_suffix('d')
            && let Ok(day) = v.parse::<u64>()
        {
            total = total.saturating_add(day.saturating_mul(86400));
        }
    }

    if total > 0 { Some(total) } else { None }
}

/// Get the systemd user service directory
fn get_service_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let service_dir = PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user");

    fs::create_dir_all(&service_dir).context("Failed to create systemd user service directory")?;

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

fn current_username() -> Option<String> {
    if let Ok(user) = std::env::var("USER") {
        let trimmed = user.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let output = Command::new("id").args(["-un"]).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if user.is_empty() { None } else { Some(user) }
}

fn ensure_linger_enabled_if_possible() {
    let Some(user) = current_username() else {
        println!("⚠️  Could not determine current user for linger check.");
        return;
    };

    let show = Command::new("loginctl")
        .args(["show-user", &user, "-p", "Linger"])
        .output();

    let Ok(show) = show else {
        println!(
            "ℹ️  'loginctl' not available. If service stops after logout, enable linger manually."
        );
        return;
    };

    if show.status.success() {
        let out = String::from_utf8_lossy(&show.stdout).to_ascii_lowercase();
        if out.contains("linger=yes") {
            println!("✅ systemd linger already enabled for user '{}'", user);
            return;
        }
    }

    let enable = Command::new("loginctl")
        .args(["enable-linger", &user])
        .output();

    match enable {
        Ok(res) if res.status.success() => {
            println!("✅ Enabled systemd linger for user '{}'", user);
        }
        Ok(res) => {
            println!(
                "⚠️  Could not enable linger automatically: {}",
                String::from_utf8_lossy(&res.stderr).trim()
            );
            println!(
                "   Run manually if needed: sudo loginctl enable-linger {}",
                user
            );
        }
        Err(_) => {
            println!(
                "ℹ️  Could not run 'loginctl enable-linger'. If service stops after logout, run: sudo loginctl enable-linger {}",
                user
            );
        }
    }
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

    fs::write(&service_file, service_content).context("Failed to write service file")?;

    println!("✅ Service file created: {}", service_file.display());
    println!();
    println!("Reloading systemd daemon...");

    let output = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to reload systemd daemon")?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to reload systemd daemon: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    println!("✅ Systemd daemon reloaded");
    println!();

    let enable_output = Command::new("systemctl")
        .args(["--user", "enable", SERVICE_NAME])
        .output()
        .context("Failed to enable service")?;

    if !enable_output.status.success() {
        anyhow::bail!(
            "Failed to enable service: {}",
            String::from_utf8_lossy(&enable_output.stderr)
        );
    }

    println!("✅ Service enabled for auto-start");
    println!();

    ensure_linger_enabled_if_possible();
    println!();

    println!("Service installed successfully!");
    println!("To start the service:");
    println!("  nanobot service start");

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
    fs::remove_file(&service_file).context("Failed to remove service file")?;

    // Reload daemon
    let output = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .context("Failed to reload systemd daemon")?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to reload systemd daemon: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        anyhow::bail!(
            "Failed to start service: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        anyhow::bail!(
            "Failed to stop service: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        anyhow::bail!(
            "Failed to restart service: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        uptime_seconds: parse_systemd_uptime_seconds(&stdout),
        last_exit_code: None,
        last_exit_reason: None,
    })
}

/// Check if the service is installed
pub fn is_installed() -> bool {
    get_service_file().ok().map(|f| f.exists()).unwrap_or(false)
}
