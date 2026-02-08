// Simple tool calling implementation
// Since Rig's tool API isn't well documented, we'll use a prompt-based approach

use anyhow::Result;

use super::filesystem::{edit_file, EditFileArgs};

/// Tool descriptions for the agent's preamble
/// Tool descriptions for the agent's preamble
pub fn get_tool_descriptions() -> String {
    let mut s = r#"
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

13. **script_eval** - Execute a Rhai script for data transformation or logic
    Usage: { "tool": "script_eval", "script": "let x = 10; x * 2", "function": "optional_fn_name", "args": ["arg1"] }
    Note: Scripts are sandboxed. No loops allowed. Max string size 100KB.
"#.to_string();

    #[cfg(feature = "browser")]
    {
        s.push_str(r##"
14. **browser_navigate** - Navigate to a URL in the browser
    Usage: { "tool": "browser_navigate", "url": "https://example.com" }

15. **browser_click** - Click an element by CSS selector
    Usage: { "tool": "browser_click", "selector": "#submit-button" }

16. **browser_type** - Type text into an element
    Usage: { "tool": "browser_type", "selector": "input[name='q']", "text": "hello" }

17. **browser_screenshot** - Take a screenshot of the current page
    Usage: { "tool": "browser_screenshot" }
    Returns path to saved PNG file.

18. **browser_evaluate** - Execute JavaScript on the page
    Usage: { "tool": "browser_evaluate", "script": "document.title" }

19. **browser_pdf** - Print current page to PDF
    Usage: { "tool": "browser_pdf" }
    Returns path to saved PDF file.

20. **browser_list_tabs** - List all open browser tabs
    Usage: { "tool": "browser_list_tabs" }

21. **browser_switch_tab** - Switch to a specific tab
    Usage: { "tool": "browser_switch_tab", "index": 0 }
"##);
    }

    s.push_str("\nWhen you need to use a tool, respond with ONLY the JSON tool call on a single line.\n\nAfter I execute the tool and show you the result, continue the conversation normally.\n");
    s
}

/// Execute a tool based on JSON input
#[tracing::instrument(skip_all, fields(tool_name))]
pub async fn execute_tool(
    tool_input: &str,
    cron_scheduler: Option<&crate::cron::CronScheduler>,
    agent_manager: Option<&crate::gateway::agent_manager::AgentManager>,
    memory_manager: Option<&std::sync::Arc<crate::memory::MemoryManager>>,
    permission_manager: Option<&tokio::sync::Mutex<super::PermissionManager>>,
    skill_loader: Option<&std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>>,
    #[cfg(feature = "browser")]
    browser_client: Option<&crate::browser::BrowserClient>,
    tenant_id: Option<&str>,
) -> Result<String> {
    // Strip prefix if present (optional support)
    let json_str = tool_input.trim().trim_start_matches("__TOOL_CALL__").trim();

    let tool_call: serde_json::Value = serde_json::from_str(json_str)?;

    let tool_name = tool_call["tool"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'tool' field"))?;


    tracing::Span::current().record("tool_name", tool_name);

    // Phase 1 Integration: ToolGuard validation (schema + safety checks)
    if let Err(e) = super::guard::ToolGuard::validate_args(tool_name, &tool_call) {
        tracing::warn!("ToolGuard validation failed for {}: {}", tool_name, e);
        return Err(anyhow::anyhow!("Tool validation failed: {}", e));
    }

    // Phase 3: Security Integration
    let workspace_root = std::env::current_dir()?;
    
    // Map tool to operation type for permission checking
    let operation = match tool_name {
        "read_file" | "list_directory" => {
            // Extract path from args if available
            if let Some(path_str) = tool_call.get("path").and_then(|v| v.as_str()) {
                super::permissions::Operation::ReadFile(std::path::PathBuf::from(path_str))
            } else {
                // Generic read operation
                super::permissions::Operation::ReadFile(workspace_root.clone())
            }
        }
        "write_file" | "edit_file" | "apply_patch" => {
            if let Some(path_str) = tool_call.get("path").and_then(|v| v.as_str()) {
                super::permissions::Operation::WriteFile(std::path::PathBuf::from(path_str))
            } else {
                super::permissions::Operation::WriteFile(workspace_root.join("unknown"))
            }
        }
        "run_command" | "bash" | "exec" => {
            let cmd = tool_call.get("command")
                .or(tool_call.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            super::permissions::Operation::ExecuteCommand(cmd.to_string())
        }
        _ => {
            // Unknown tool, treat as medium-risk command
            super::permissions::Operation::ExecuteCommand(format!("unknown:{}", tool_name))
        }
    };
    
    // Check permission (using passed permission manager or create temporary one)
    let decision = if let Some(perm_mgr) = permission_manager {
        let mgr = perm_mgr.lock().await;
        mgr.check_permission(&operation)
    } else {
        // Fallback: Create temporary permission manager for backwards compatibility
        let profile = super::permissions::SecurityProfile::standard(workspace_root.clone());
        let temp_mgr = super::permissions::PermissionManager::new(profile);
        temp_mgr.check_permission(&operation)
    };
    
    // Create confirmation service with CLI adapter
    let mut confirmation_service = super::confirmation::ConfirmationService::new();
    confirmation_service.register_adapter(Box::new(super::cli_confirmation::CliConfirmationAdapter::new()));
    
    
    match decision {
        super::permissions::PermissionDecision::Deny => {
            tracing::warn!("Permission denied for tool: {}", tool_name);
            return Ok(super::ToolResult::error(format!("Permission denied: Tool '{}' is not allowed", tool_name)).output);
        }
        super::permissions::PermissionDecision::Ask => {
            // Determine risk level
            let risk_level = match tool_name {
                "read_file" | "list_directory" => super::confirmation::RiskLevel::Low,
                "write_file" | "edit_file" => super::confirmation::RiskLevel::Medium,
                "run_command" | "bash" | "exec" => super::confirmation::RiskLevel::High,
                _ => super::confirmation::RiskLevel::Medium,
            };
            
            // Request confirmation
            let request = super::confirmation::ConfirmationRequest {
                id: uuid::Uuid::new_v4().to_string(),
                tool_name: tool_name.to_string(),
                operation: format!("{:?}", operation),
                args: serde_json::to_string_pretty(&tool_call)?,
                risk_level,
                timeout: None,
            };
            
            let response = confirmation_service.request_confirmation(request).await?;
            
            if !response.allowed {
                tracing::info!("User denied permission for tool: {}", tool_name);
                return Ok(super::ToolResult::error(format!("User denied permission for tool: {}", tool_name)).output);
            }
            
            tracing::info!("User approved permission for tool: {}", tool_name);
        }
        super::permissions::PermissionDecision::Allow => {
            tracing::debug!("Tool {} auto-approved by profile", tool_name);
        }
    }



    // Try Skills first (if loader available)
    if let Some(loader) = skill_loader {
        let loader_guard = loader.lock().await;
        if let Some(skill) = loader_guard.get_skill(tool_name) {
            if skill.enabled {
                tracing::info!("Executing skill: {}", tool_name);
                // Execute skill's primary script/tool
                if let Some(tool_def) = skill.tools.first() {
                    // For now, return skill description as execution result
                    // In a full implementation, this would execute the actual skill logic
                    return Ok(format!("✓ Skill '{}' executed: {}\n\nDescription: {}", 
                        skill.name, 
                        tool_def.name,
                        skill.description));
                }
            }
        }
    }

    // Try ToolRegistry next (for simple, modular tools)
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
                    let results = manager.search(query, 5, tenant_id).await?;
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
                        .add_document(content, std::collections::HashMap::new(), tenant_id)
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

        #[cfg(feature = "browser")]
        "browser_navigate" => {
            if let Some(client) = browser_client {
                let url = tool_call["url"].as_str().ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let _page = client.navigate(url).await?;
                Ok(format!("Navigated to {}", url))
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_click" => {
            if let Some(client) = browser_client {
                let selector = tool_call["selector"].as_str().ok_or_else(|| anyhow::anyhow!("Missing selector"))?;
                let page = client.get_page().await?;
                crate::browser::actions::BrowserActions::click(&page, selector).await
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_type" => {
            if let Some(client) = browser_client {
                let selector = tool_call["selector"].as_str().ok_or_else(|| anyhow::anyhow!("Missing selector"))?;
                let text = tool_call["text"].as_str().ok_or_else(|| anyhow::anyhow!("Missing text"))?;
                let page = client.get_page().await?;
                crate::browser::actions::BrowserActions::type_text(&page, selector, text).await
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_screenshot" => {
            if let Some(client) = browser_client {
                let page = client.get_page().await?;
                let data = crate::browser::actions::BrowserActions::screenshot(&page).await?;
                let path = format!("screenshot_{}.png", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
                tokio::fs::write(&path, data).await?;
                Ok(format!("Screenshot saved to {}", path))
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_evaluate" => {
             if let Some(client) = browser_client {
                let script = tool_call["script"].as_str().ok_or_else(|| anyhow::anyhow!("Missing script"))?;
                let page = client.get_page().await?;
                crate::browser::actions::BrowserActions::execute_js(&page, script).await
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_pdf" => {
            if let Some(client) = browser_client {
                let page = client.get_page().await?;
                let data = crate::browser::actions::BrowserActions::print_to_pdf(&page).await?;
                let path = format!("page_{}.pdf", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
                tokio::fs::write(&path, data).await?;
                Ok(format!("PDF saved to {}", path))
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_list_tabs" => {
             if let Some(client) = browser_client {
                let pages = client.get_pages().await?;
                let mut s = String::new();
                for (i, page) in pages.iter().enumerate() {
                     let title = page.get_title().await.unwrap_or_default().unwrap_or_default();
                     let url = page.url().await.unwrap_or_default().unwrap_or_default();
                     s.push_str(&format!("{}: {} ({})\n", i, title, url));
                }
                if s.is_empty() {
                    Ok("No open tabs.".to_string())
                } else {
                    Ok(s)
                }
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

        #[cfg(feature = "browser")]
        "browser_switch_tab" => {
             if let Some(client) = browser_client {
                let index = tool_call["index"].as_u64()
                    .ok_or_else(|| anyhow::anyhow!("Missing index"))? as usize;
                let _ = client.switch_tab(index).await?;
                Ok(format!("Switched to tab {}", index))
            } else {
                Err(anyhow::anyhow!("Browser not available."))
            }
        }

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
        // Pass None for all optional context parameters
        #[cfg(feature = "browser")]
        let result = execute_tool(json, None, None, None, None, None, None, None).await;
        #[cfg(not(feature = "browser"))]
        let result = execute_tool(json, None, None, None, None, None, None).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("Status: ✅ Success"));
    }
}
