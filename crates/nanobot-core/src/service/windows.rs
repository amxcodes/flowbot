use anyhow::{Context, Result};
use std::fs;
use std::process::Command;

use super::types::{ServiceRuntime, ServiceStatus};

const TASK_NAME: &str = "Nanobot";

/// Get the Windows Task Scheduler task path
fn get_task_path() -> String {
    format!("\\{}", TASK_NAME)
}

/// Install Windows Task Scheduler task
pub fn install() -> Result<()> {
    let exe_path = std::env::current_exe().context("Failed to get current executable path")?;
    let exe_path_str = exe_path
        .to_str()
        .context("Executable path contains invalid UTF-8")?;
    
    // Create XML task definition
    let task_xml = generate_task_xml(exe_path_str)?;
    
    // Write to temp file
    let temp_dir = std::env::temp_dir();
    let xml_file = temp_dir.join("nanobot-task.xml");
    fs::write(&xml_file, task_xml)
        .context("Failed to write task XML file")?;
    
    // Register the task using schtasks.exe
    let output = Command::new("schtasks")
        .args([
            "/Create",
            "/TN",
            &get_task_path(),
            "/XML",
            xml_file.to_str().unwrap(),
            "/F", // Force create (overwrite if exists)
        ])
        .output()
        .context("Failed to create scheduled task")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to create scheduled task: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service installed successfully");
    println!();
    println!("To start the service:");
    println!("  nanobot service start");
    println!("Or use Task Scheduler GUI:");
    println!("  taskschd.msc");
    
    Ok(())
}

/// Generate Windows Task Scheduler XML
fn generate_task_xml(binary_path: &str) -> Result<String> {
    let username = std::env::var("USERNAME")
        .context("USERNAME environment variable not set")?;
    
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Nanobot AI Assistant Gateway - 24/7 Bot Service</Description>
    <URI>\{}</URI>
  </RegistrationInfo>
  <Triggers>
    <BootTrigger>
      <Enabled>true</Enabled>
    </BootTrigger>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{}</Command>
      <Arguments>gateway</Arguments>
    </Exec>
  </Actions>
</Task>"#,
        TASK_NAME, username, username, binary_path
    ))
}

/// Uninstall Windows Task Scheduler task
pub fn uninstall() -> Result<()> {
    let output = Command::new("schtasks")
        .args([
            "/Delete",
            "/TN",
            &get_task_path(),
            "/F", // Force delete without confirmation
        ])
        .output()
        .context("Failed to delete scheduled task")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("cannot be found") {
            println!("Service not installed");
            return Ok(());
        }
        anyhow::bail!("Failed to delete scheduled task: {}", stderr);
    }
    
    println!("✅ Service uninstalled successfully");
    
    Ok(())
}

/// Start the Windows Task Scheduler task
pub fn start() -> Result<()> {
    let output = Command::new("schtasks")
        .args([
            "/Run",
            "/TN",
            &get_task_path(),
        ])
        .output()
        .context("Failed to start scheduled task")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to start scheduled task: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service started");
    Ok(())
}

/// Stop the Windows Task Scheduler task
pub fn stop() -> Result<()> {
    let output = Command::new("schtasks")
        .args([
            "/End",
            "/TN",
            &get_task_path(),
        ])
        .output()
        .context("Failed to stop scheduled task")?;
    
    if !output.status.success() {
        anyhow::bail!("Failed to stop scheduled task: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    println!("✅ Service stopped");
    Ok(())
}

/// Restart the Windows Task Scheduler task
pub fn restart() -> Result<()> {
    stop().ok(); // Ignore errors if not running
    std::thread::sleep(std::time::Duration::from_secs(1));
    start()
}

/// Get the service status
pub fn status() -> Result<ServiceRuntime> {
    let output = Command::new("schtasks")
        .args([
            "/Query",
            "/TN",
            &get_task_path(),
            "/FO",
            "LIST",
            "/V",
        ])
        .output()
        .context("Failed to query scheduled task")?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Parse task status
    let status = if stdout.contains("Status:") {
        if stdout.contains("Running") {
            ServiceStatus::Running
        } else if stdout.contains("Ready") {
            ServiceStatus::Stopped
        } else {
            ServiceStatus::Unknown
        }
    } else {
        ServiceStatus::Unknown
    };
    
    Ok(ServiceRuntime {
        status,
        pid: None, // Task Scheduler doesn't easily expose PID
        uptime_seconds: None,
        last_exit_code: None,
        last_exit_reason: None,
    })
}

/// Check if the service is installed
pub fn is_installed() -> bool {
    Command::new("schtasks")
        .args(["/Query", "/TN", &get_task_path()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
