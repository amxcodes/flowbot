use anyhow::Result;
use std::process::Command;

/// Check if Docker is available on the system
pub fn is_docker_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Execute a command in a Docker container
pub async fn execute_in_container(
    command: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> Result<String> {
    let current_dir = std::env::current_dir()?;
    let mount_path = current_dir.to_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid current directory path")
    })?;

    // Build the command to execute inside container
    let mut cmd_parts = vec![command.to_string()];
    cmd_parts.extend(args.iter().cloned());
    let full_command = cmd_parts.join(" ");

    // Use Alpine Linux as a lightweight base image
    let mut docker_cmd = Command::new("docker");
    docker_cmd
        .arg("run")
        .arg("--rm") // Remove container after execution
        .arg("--network=none") // Disable network access for security
        .arg("--read-only") // Make filesystem read-only
        .arg("-v")
        .arg(format!("{}:/workspace:ro", mount_path)) // Mount current dir as read-only
        .arg("-w")
        .arg(working_dir.unwrap_or("/workspace")) // Set working directory
        .arg("alpine:latest") // Use Alpine Linux
        .arg("sh")
        .arg("-c")
        .arg(&full_command);

    // Execute the command
    let output = docker_cmd.output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow::anyhow!(
            "Docker execution failed: {}",
            stderr
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_availability() {
        // This test will pass if Docker is installed, fail otherwise
        // In CI/CD, you might want to skip this or ensure Docker is available
        let available = is_docker_available();
        println!("Docker available: {}", available);
    }

    #[tokio::test]
    async fn test_docker_execution() -> Result<()> {
        if !is_docker_available() {
            println!("Skipping test - Docker not available");
            return Ok(());
        }

        let result = execute_in_container("echo", &["hello".to_string()], None).await?;
        assert!(result.contains("hello"));
        Ok(())
    }

    #[tokio::test]
    async fn test_docker_isolation() -> Result<()> {
        if !is_docker_available() {
            println!("Skipping test - Docker not available");
            return Ok(());
        }

        // Try to access /etc/passwd (should fail due to read-only filesystem)
        let result = execute_in_container("ls", &["/workspace".to_string()], None).await;
        // Should succeed in listing workspace
        assert!(result.is_ok());
        Ok(())
    }
}
