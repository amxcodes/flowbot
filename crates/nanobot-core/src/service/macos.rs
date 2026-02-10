use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::types::{ServiceRuntime, ServiceStatus};

const SERVICE_LABEL: &str = "com.nanobot.gateway";

fn get_launch_agents_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let dir = PathBuf::from(home).join("Library").join("LaunchAgents");
    fs::create_dir_all(&dir).context("Failed to create LaunchAgents directory")?;
    Ok(dir)
}

fn get_plist_path() -> Result<PathBuf> {
    Ok(get_launch_agents_dir()?.join(format!("{}.plist", SERVICE_LABEL)))
}

fn generate_plist(binary_path: &str, working_dir: &str) -> Result<String> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let log_dir = PathBuf::from(home).join("Library").join("Logs");
    let stdout_path = log_dir.join("nanobot.log");
    let stderr_path = log_dir.join("nanobot.error.log");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>gateway</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{working_dir}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        binary = binary_path,
        working_dir = working_dir,
        stdout = stdout_path.display(),
        stderr = stderr_path.display(),
    ))
}

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

    let plist_content = generate_plist(exe_path_str, &working_dir)?;
    let plist_path = get_plist_path()?;
    fs::write(&plist_path, plist_content).context("Failed to write launchd plist")?;

    let output = Command::new("launchctl")
        .args(["load", "-w", plist_path.to_str().unwrap()])
        .output()
        .context("Failed to load launchd plist")?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to load launchd plist: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    println!("✅ Service installed and loaded: {}", plist_path.display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let plist_path = get_plist_path()?;
    if !plist_path.exists() {
        println!("Service not installed");
        return Ok(());
    }

    let _ = Command::new("launchctl")
        .args(["unload", "-w", plist_path.to_str().unwrap()])
        .output();

    fs::remove_file(&plist_path).context("Failed to remove launchd plist")?;
    println!("✅ Service uninstalled");
    Ok(())
}

pub fn start() -> Result<()> {
    let output = Command::new("launchctl")
        .args(["start", SERVICE_LABEL])
        .output()
        .context("Failed to start launchd service")?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to start launchd service: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    println!("✅ Service started");
    Ok(())
}

pub fn stop() -> Result<()> {
    let output = Command::new("launchctl")
        .args(["stop", SERVICE_LABEL])
        .output()
        .context("Failed to stop launchd service")?;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to stop launchd service: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    println!("✅ Service stopped");
    Ok(())
}

pub fn restart() -> Result<()> {
    stop().ok();
    std::thread::sleep(std::time::Duration::from_secs(1));
    start()
}

pub fn status() -> Result<ServiceRuntime> {
    let output = Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()
        .context("Failed to get launchd status")?;

    let status = if output.status.success() {
        ServiceStatus::Running
    } else {
        ServiceStatus::Stopped
    };

    Ok(ServiceRuntime {
        status,
        pid: None,
        uptime_seconds: None,
        last_exit_code: None,
        last_exit_reason: None,
    })
}

pub fn is_installed() -> bool {
    get_plist_path().map(|p| p.exists()).unwrap_or(false)
}
