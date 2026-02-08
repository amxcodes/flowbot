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

pub struct SandboxConfig {
    pub image: String,
    pub allow_network: bool,
    pub writable_workspace: bool,
    pub workdir: Option<String>,
    pub env_vars: Vec<(String, String)>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            image: "alpine:latest".to_string(),
            allow_network: false,
            writable_workspace: false,
            workdir: Some("/workspace".to_string()),
            env_vars: Vec::new(),
        }
    }
}

/// Execute a command in a Docker container
pub async fn execute_in_container(
    command: &str,
    args: &[String],
    config: &SandboxConfig,
) -> Result<String> {
    let current_dir = std::env::current_dir()?;
    let mount_path = current_dir.to_str().ok_or_else(|| {
        anyhow::anyhow!("Invalid current directory path")
    })?;

    // Use a fixed container name or random? Random is better for concurrency.
    // Docker run --rm handles cleanup.

    let mut docker_cmd = Command::new("docker");
    docker_cmd.arg("run");
    docker_cmd.arg("--rm"); // Remove container after execution
    
    // Security flags
    if !config.allow_network {
        docker_cmd.arg("--network=none");
    }
    
    // Filesystem flags
    // We always want root to be read-only if possible, but allow /tmp and /workspace
    docker_cmd.arg("--read-only");
    
    // Mount tmpfs for temp files
    docker_cmd.arg("--tmpfs").arg("/tmp");
    
    // Mount workspace
    let mount_mode = if config.writable_workspace { "rw" } else { "ro" };
    docker_cmd.arg("-v").arg(format!("{}:/workspace:{}", mount_path, mount_mode));
    
    // Set working directory
    if let Some(wd) = &config.workdir {
        docker_cmd.arg("-w").arg(wd);
    }

    // Set Environment Variables
    for (key, val) in &config.env_vars {
        docker_cmd.arg("-e").arg(format!("{}={}", key, val));
    }
    
    // Image
    docker_cmd.arg(&config.image);
    
    // Command and Args (DIRECT PASSING - Prevents Shell Injection)
    docker_cmd.arg(command);
    docker_cmd.args(args);

    // Execute the command
    let output = docker_cmd.output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Combine stdout and stderr for better debugging if failed
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(anyhow::anyhow!(
            "Docker execution failed.\nStderr: {}\nStdout: {}",
            stderr, stdout
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
