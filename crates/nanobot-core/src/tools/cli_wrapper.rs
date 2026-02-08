use super::commands::{CommandOutput, RunCommandArgs, run_command};
use anyhow::Result;

/// Configuration for command execution output formatting.
///
/// Controls how command output is truncated when it exceeds reasonable limits
/// to prevent overwhelming the LLM context window.
pub struct CliOutputConfig {
    /// Maximum characters before truncation kicks in
    pub max_chars: usize,
    /// Number of lines to preserve from the start of output
    pub head_lines: usize,
    /// Number of lines to preserve from the end of output
    pub tail_lines: usize,
}

impl Default for CliOutputConfig {
    fn default() -> Self {
        Self {
            max_chars: 2000,
            head_lines: 20,
            tail_lines: 10,
        }
    }
}

/// Run a command and return a formatted string suitable for an LLM.
///
/// This function executes the command using the standard executor and then
/// applies smart formatting with truncation to ensure the output doesn't
/// overflow the LLM's context window.
///
/// # Examples
///
/// ```ignore
/// let args = RunCommandArgs {
///     command: "cargo".to_string(),
///     args: vec!["build".to_string(), "--verbose".to_string()],
///     use_docker: false,
///     docker_image: None,
/// };
/// let output = run_and_format(args).await?;
/// ```
pub async fn run_and_format(mut args: RunCommandArgs) -> Result<String> {
    // Resolve command for Windows compatibility (Parity with OpenClaw)
    args.command = resolve_command(&args.command);

    // Check if Docker execution is requested
    // Check if Docker execution is requested
    if args.use_docker {
        // First check if Docker is available
        if super::docker_executor::is_docker_available() {
            let config = super::docker_executor::SandboxConfig {
                image: args.docker_image.clone().unwrap_or_else(|| "alpine:latest".to_string()),
                // For a general "run_command" tool, we likely need network and write access
                // to match host capabilities but within a container.
                allow_network: true, 
                writable_workspace: true,
                workdir: Some("/workspace".to_string()),
                env_vars: Vec::new(), // Pass env vars if needed?
            };

            match super::docker_executor::execute_in_container(
                &args.command,
                &args.args,
                &config,
            ).await {
                Ok(output) => {
                    return Ok(format!(
                        "Status: ✅ Success (Docker)\nCommand: {} {}\nOutput:\n{}",
                        args.command,
                        args.args.join(" "),
                        output
                    ));
                }
                Err(e) => {
                    // Strict Sandbox: Do NOT fall back to host if Docker was requested but failed.
                    return Err(anyhow::anyhow!("Docker execution failed: {}", e));
                }
            }
        } else {
             return Err(anyhow::anyhow!("Docker execution requested but Docker is not available on this system."));
        }
    }

    // Standard host execution
    let output = run_command(args.clone()).await?;
    Ok(format_output(
        output,
        &args.command,
        &CliOutputConfig::default(),
    ))
}

/// Resolves a command for Windows compatibility.
///
/// On Windows, non-.exe commands (like npm, pnpm) usually require their .cmd extension
/// to be executed via `Command::new`.
fn resolve_command(command: &str) -> String {
    #[cfg(windows)]
    {
        let cmd_lower = command.to_lowercase();

        // Skip if already has an extension (simple check for dot in last 5 chars)
        if cmd_lower.len() > 4 && cmd_lower[cmd_lower.len() - 4..].contains('.') {
            return command.to_string();
        }

        // List of commands that are implemented as batch scripts on Windows
        // and need explicit .cmd extension
        let batch_commands = ["npm", "pnpm", "yarn", "npx"];

        if batch_commands.contains(&cmd_lower.as_str()) {
            return format!("{}.cmd", command);
        }
    }

    command.to_string()
}

/// Format command output with smart truncation.
///
/// This is exposed publicly so it can also be used for Docker command output.
/// Applies consistent formatting with status indicators and intelligent truncation.
pub fn format_output(
    output: CommandOutput,
    command_name: &str,
    config: &CliOutputConfig,
) -> String {
    let status_line = if output.success {
        if output.exit_code == 0 {
            "✅ Success".to_string()
        } else {
            format!("✅ Success (Exit Code: {})", output.exit_code)
        }
    } else {
        format!("❌ Failed (Exit Code: {})", output.exit_code)
    };

    let stdout_formatted = format_stream("Stdout", &output.stdout, config);
    let stderr_formatted = format_stream("Stderr", &output.stderr, config);

    format!(
        "Command: {}\nStatus: {}\n\n{}\n\n{}",
        command_name, status_line, stdout_formatted, stderr_formatted
    )
}

fn format_stream(name: &str, content: &str, config: &CliOutputConfig) -> String {
    if content.trim().is_empty() {
        return format!("{}: (empty)", name);
    }

    if content.len() <= config.max_chars {
        return format!("{}:\n{}", name, content);
    }

    // Truncation logic
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    if total_lines <= (config.head_lines + config.tail_lines) {
        // If lines are few but content is long (very long lines), just simple char truncation
        let head = &content[..config.max_chars / 2];
        let tail = &content[content.len() - (config.max_chars / 2)..];
        return format!(
            "{}:\n{}\n\n... [Output Truncated] ...\n\n{}",
            name, head, tail
        );
    }

    // Line-based truncation
    let head = lines[..config.head_lines].join("\n");
    let tail = lines[total_lines - config.tail_lines..].join("\n");
    let skipped = total_lines - (config.head_lines + config.tail_lines);

    format!(
        "{}:\n{}\n\n... [{} lines truncated] ...\n\n{}",
        name, head, skipped, tail
    )
}
