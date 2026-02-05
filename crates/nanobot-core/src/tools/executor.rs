// Simple tool calling implementation
// Since Rig's tool API isn't well documented, we'll use a prompt-based approach

use anyhow::Result;

use super::commands::*;
use super::filesystem::*;
use super::websearch::*;

/// Tool descriptions for the agent's preamble
pub fn get_tool_descriptions() -> String {
    r#"
You have access to the following tools:

1. **read_file** - Read the contents of a file
   Usage: { "tool": "read_file", "path": "file.txt" }
   
2. **write_file** - Write content to a file
   Usage: { "tool": "write_file", "path": "file.txt", "content": "text", "overwrite": true }
   
3. **edit_file** - Find and replace text in a file
   Usage: { "tool": "edit_file", "path": "file.txt", "old_text": "old", "new_text": "new" }
   
4. **list_directory** - List files in a directory
   Usage: { "tool": "list_directory", "path": ".", "max_depth": 1 }
   
5. **web_search** - Search the web
   Usage: { "tool": "web_search", "query": "search terms", "max_results": 5 }
   
6. **run_command** - Execute a system command
   Usage: { "tool": "run_command", "command": "cargo", "args": ["--version"], "use_docker": false }
   Note: Set "use_docker": true to run safely in a container. Default is false (Host).

7. **spawn_process** - Start a background process
   Usage: { "tool": "spawn_process", "command": "ping", "args": ["google.com"] }
   Returns a PID.

8. **read_process_output** - Read output from a background process
   Usage: { "tool": "read_process_output", "pid": "..." }
   Reads and clears the buffer.

9. **kill_process** - Terminate a background process
   Usage: { "tool": "kill_process", "pid": "..." }

10. **list_processes** - List all background processes
    Usage: { "tool": "list_processes" }

11. **web_fetch** - Download and extract content from a URL
    Usage: { "tool": "web_fetch", "url": "https://example.com" }

12. **write_process_input** - Send text input to a running process (stdin)
    Usage: { "tool": "write_process_input", "pid": "...", "input": "yes\n" }

When you need to use a tool, respond with ONLY the JSON tool call on a single line.

After I execute the tool and show you the result, continue the conversation normally.
"#
    .to_string()
}

/// Execute a tool based on JSON input
pub async fn execute_tool(
    tool_input: &str,
    cron_scheduler: Option<&crate::cron::CronScheduler>,
    agent_manager: Option<&crate::gateway::agent_manager::AgentManager>,
    memory_manager: Option<&std::sync::Arc<crate::memory::MemoryManager>>,
) -> Result<String> {
    // Strip prefix if present (optional support)
    let json_str = tool_input.trim().trim_start_matches("__TOOL_CALL__").trim();

    let tool_call: serde_json::Value = serde_json::from_str(json_str)?;

    let tool_name = tool_call["tool"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'tool' field"))?;

    // Try ToolRegistry first (for simple, modular tools)
    let registry = super::definitions::get_tool_registry();
    if let Some(tool) = registry.get(tool_name) {
        // Extract args (everything except "tool" field)
        let args = if let Some(obj) = tool_call.as_object() {
            let mut args_obj = obj.clone();
            args_obj.remove("tool");
            serde_json::Value::Object(args_obj)
        } else {
            tool_call.clone()
        };

        return tool.execute(args).await;
    }

    // Fall back to legacy match for complex tools that need context
    match tool_name {
        // Simple tools (read_file, write_file, list_directory, web_search, run_command)
        // are now handled by the registry above
        "edit_file" => {
            let args = EditFileArgs {
                path: tool_call["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' field"))?
                    .to_string(),
                old_text: tool_call["old_text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'old_text' field"))?
                    .to_string(),
                new_text: tool_call["new_text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'new_text' field"))?
                    .to_string(),
                all_occurrences: tool_call["all_occurrences"].as_bool().unwrap_or(false),
            };
            edit_file(args).await
        }

        "spawn_process" => {
            let args = super::process::SpawnArgs {
                command: tool_call["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'command' field"))?
                    .to_string(),
                args: tool_call["args"].as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                }),
            };
            super::process::spawn_process(args).await
        }

        "read_process_output" => {
            // PID in JSON might be string or number, handle both
            let pid_val = &tool_call["pid"];
            let pid = if let Some(n) = pid_val.as_u64() {
                n as u32
            } else if let Some(s) = pid_val.as_str() {
                s.parse::<u32>()
                    .map_err(|_| anyhow::anyhow!("Invalid PID format"))?
            } else {
                return Err(anyhow::anyhow!("Missing 'pid' field"));
            };

            let args = super::process::PidArgs { pid };
            super::process::read_process_output(args).await
        }

        "kill_process" => {
            let pid_val = &tool_call["pid"];
            let pid = if let Some(n) = pid_val.as_u64() {
                n as u32
            } else if let Some(s) = pid_val.as_str() {
                s.parse::<u32>()
                    .map_err(|_| anyhow::anyhow!("Invalid PID format"))?
            } else {
                return Err(anyhow::anyhow!("Missing 'pid' field"));
            };

            let args = super::process::PidArgs { pid };
            super::process::terminate_process(args).await
        }

        "list_processes" => {
            // Not implemented in process.rs yet, let's skip or implement stub
            Err(anyhow::anyhow!("list_processes not implemented yet"))
        }

        "web_fetch" => {
            let args = super::fetch::WebFetchArgs {
                url: tool_call["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'url' field"))?
                    .to_string(),
                extract_mode: tool_call["extract_mode"].as_str().map(|s| s.to_string()),
            };
            super::fetch::web_fetch(args).await
        }

        "write_process_input" => {
            let pid_val = &tool_call["pid"];
            let pid = if let Some(n) = pid_val.as_u64() {
                n as u32
            } else if let Some(s) = pid_val.as_str() {
                s.parse::<u32>()
                    .map_err(|_| anyhow::anyhow!("Invalid PID format"))?
            } else {
                return Err(anyhow::anyhow!("Missing 'pid' field"));
            };

            let args = super::process::WriteInputArgs {
                pid,
                input: tool_call["input"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'input' field"))?
                    .to_string(),
            };
            super::process::write_process_input(args).await
        }

        "memory_search" => {
            let query = tool_call["query"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' field"))?;

            match memory_manager {
                Some(manager) => {
                    let results = manager.search(query, 5).await?;
                    let mut response = String::new();
                    for (score, entry) in results {
                        response.push_str(&format!("[Score: {:.2}] {}\n", score, entry.content));
                    }
                    if response.is_empty() {
                        Ok("No relevant memories found.".to_string())
                    } else {
                        Ok(response)
                    }
                }
                None => Ok("Memory manager not initialized.".to_string()),
            }
        }

        "memory_save" => {
            let content = tool_call["content"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'content' field"))?;

            match memory_manager {
                Some(manager) => {
                    manager
                        .add_document(content, std::collections::HashMap::new())
                        .await?;
                    Ok("Memory saved.".to_string())
                }
                None => Ok("Memory manager not initialized.".to_string()),
            }
        }

        "cron" => match cron_scheduler {
            Some(scheduler) => crate::tools::cron::execute_cron_tool(scheduler, &tool_call).await,
            None => {
                Ok("Cron scheduler not initialized. Available in gateway/server mode.".to_string())
            }
        },

        "sessions_spawn" => match agent_manager {
            Some(manager) => {
                crate::tools::sessions::execute_sessions_tool(manager, &tool_call).await
            }
            None => {
                Ok("Agent manager not initialized. Available in gateway/server mode.".to_string())
            }
        },

        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    }
}

/// Check if a response contains a tool call
pub fn is_tool_call(response: &str) -> bool {
    let trimmed = response.trim();
    trimmed.starts_with("__TOOL_CALL__")
        || (trimmed.starts_with('{') && trimmed.contains(r#""tool""#))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tool_parsing() {
        let json =
            r#"{"tool": "run_command", "command": "echo", "args": ["hello"], "use_docker": false}"#;
        // We can't easily execute in unit test environment without real commands,
        // but we can check if it parses and tries to execute.
        // Actually, "echo" is safe to run on host.

        // Note: This test requires the binary to be built/run where 'echo' exists.
        // Windows 'echo' is a shell builtin, might fail with Command::new("echo").
        // We should use "cmd" /C "echo" on Windows or "sh" -c "echo" on Unix.
        // But run_command implementation uses Command::new(command).
        // let's try "whoami" or "rustc --version" which is in our whitelist.

        let json = r#"{"tool": "run_command", "command": "cargo", "args": ["--version"]}"#;
        let result = execute_tool(json, None, None).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("Status: Success"));
    }
}
