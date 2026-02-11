// Simple tool calling implementation
// Since Rig's tool API isn't well documented, we'll use a prompt-based approach

use anyhow::Result;
use futures::future::join_all;
use serde_json::json;
use std::time::Duration;

use super::filesystem::{apply_patch, edit_file, ApplyPatchArgs, EditFileArgs};
use super::search::{glob_files, grep_files, GlobArgs, GrepArgs};
use super::todos::{todo_write, TodoWriteArgs};

fn task_status_str(status: &crate::gateway::agent_manager::TaskStatus) -> &'static str {
    match status {
        crate::gateway::agent_manager::TaskStatus::Pending => "pending",
        crate::gateway::agent_manager::TaskStatus::Running => "running",
        crate::gateway::agent_manager::TaskStatus::Retrying => "retrying",
        crate::gateway::agent_manager::TaskStatus::Paused => "paused",
        crate::gateway::agent_manager::TaskStatus::Completed => "completed",
        crate::gateway::agent_manager::TaskStatus::Failed => "failed",
        crate::gateway::agent_manager::TaskStatus::Cancelled => "cancelled",
        crate::gateway::agent_manager::TaskStatus::TimedOut => "timed_out",
    }
}

fn is_parallel_safe_tool(tool: &str) -> bool {
    matches!(
        tool,
        "read_file" | "list_directory" | "glob" | "grep" | "web_fetch"
    )
}

fn command_exists_quick(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .is_ok()
}

fn deno_policy_flags(policy: Option<&str>) -> Vec<&'static str> {
    match policy
        .map(|p| p.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("strict") => Vec::new(),
        Some("permissive") => vec!["--allow-read", "--allow-write", "--allow-env", "--allow-net"],
        Some("none") | Some("off") => Vec::new(),
        Some("balanced") | None => vec![
            "--allow-read",
            "--allow-write",
            "--allow-env=NANOBOT_SKILL,NANOBOT_TOOL,NANOBOT_TOOL_ARGS",
        ],
        Some(_) => vec![
            "--allow-read",
            "--allow-write",
            "--allow-env=NANOBOT_SKILL,NANOBOT_TOOL,NANOBOT_TOOL_ARGS",
        ],
    }
}

fn push_unique_args(target: &mut Vec<String>, extras: impl IntoIterator<Item = String>) {
    for arg in extras {
        if !target.iter().any(|existing| existing == &arg) {
            target.push(arg);
        }
    }
}

fn redacted_env_preview(env: &std::collections::HashMap<String, String>) -> serde_json::Value {
    let mut keys = env.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut map = serde_json::Map::new();
    for key in keys {
        map.insert(key, serde_json::Value::String("***".to_string()));
    }
    serde_json::Value::Object(map)
}

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

12. **glob** - Find files by glob pattern
    Usage: { "tool": "glob", "pattern": "src/**/*.rs", "path": ".", "max_results": 200 }

13. **grep** - Search file contents using regex
    Usage: { "tool": "grep", "pattern": "fn\\s+main", "include": "*.rs", "path": ".", "case_sensitive": true }

14. **question** - Ask a structured clarification question to user
    Usage: { "tool": "question", "question": "Choose mode", "header": "Mode", "options": ["Quick", "Thorough"], "multiple": false }

15. **apply_patch** - Apply structured file patch operations
    Usage: { "tool": "apply_patch", "operations": [{ "op": "update", "path": "src/main.rs", "old_text": "foo", "new_text": "bar", "before_context": "anchor before", "after_context": "anchor after" }], "atomic": true, "dry_run": false }

16. **write_process_input** - Send text input to a running process (stdin)
    Usage: { "tool": "write_process_input", "pid": "...", "input": "yes\n" }

17. **script_eval** - Execute a Rhai script for data transformation or logic
    Usage: { "tool": "script_eval", "script": "let x = 10; x * 2", "function": "optional_fn_name", "args": ["arg1"] }
    Note: Scripts are sandboxed. No loops allowed. Max string size 100KB.

18. **sessions_spawn** - Spawn an isolated subagent to handle a task
    Usage: { "tool": "sessions_spawn", "task": "do X", "label": "optional", "cleanup": "delete", "parent_session_id": "main", "model": "optional", "timeout_seconds": 120, "max_retries": 1, "retry_backoff_ms": 1000 }
    Note: Use **sessions_wait** to block until completion.

    Aliases: **spawn_subagent**, **get_subagent_result**, **list_subagents**

19. **sessions_wait** - Wait for a subagent session to finish
    Usage: { "tool": "sessions_wait", "session_id": "...", "timeout_seconds": 120 }

20. **sessions_broadcast** - Send an interim update from a subagent to its parent
    Usage: { "tool": "sessions_broadcast", "session_id": "...", "message": "progress update" }

21. **message** - Send a message to the current session
    Usage: { "tool": "message", "message": "text", "session_id": "optional" }

22. **gateway** - Send a message to a specific session
    Usage: { "tool": "gateway", "session_id": "...", "message": "text" }

23. **session_status** - Get status for a session
    Usage: { "tool": "session_status", "session_id": "optional" }

24. **sessions_history** - Get session message history
    Usage: { "tool": "sessions_history", "session_id": "optional" }

25. **sessions_list** - List active sessions
    Usage: { "tool": "sessions_list" }

26. **sessions_send** - Send a message to another session
    Usage: { "tool": "sessions_send", "session_id": "...", "message": "text" }

27. **sessions_cancel** - Cancel a running subagent session
    Usage: { "tool": "sessions_cancel", "session_id": "..." }

28. **sessions_pause** - Pause a subagent session
    Usage: { "tool": "sessions_pause", "session_id": "..." }

29. **sessions_resume** - Resume a paused subagent session
    Usage: { "tool": "sessions_resume", "session_id": "..." }

30. **agents_list** - List available agent manifests
    Usage: { "tool": "agents_list" }

31. **memory_get** - Retrieve a memory item by id
    Usage: { "tool": "memory_get", "id": "..." }

32. **memory_search** - Search semantic memory
    Usage: { "tool": "memory_search", "query": "..." }

33. **memory_save** - Save text into semantic memory
    Usage: { "tool": "memory_save", "content": "..." }

34. **cron** - Manage cron jobs via tool interface
    Usage: { "tool": "cron", "action": "list" }

35. **llm_task** - Run a micro-task with the configured provider
    Usage: { "tool": "llm_task", "prompt": "...", "system": "optional", "temperature": 0.2, "max_tokens": 200 }

36. **tts** - Generate speech audio from text
    Usage: { "tool": "tts", "text": "hello", "output_path": "optional.wav", "model": "optional", "voice": "optional", "extra_args": ["..."] }

37. **stt** - Transcribe audio using Whisper CLI
    Usage: { "tool": "stt", "audio_path": "file.wav", "model": "base", "output_dir": "stt_output", "format": "txt" }

38. **todowrite** - Store structured todo list state
    Usage: { "tool": "todowrite", "todos": [{ "id": "1", "content": "task", "status": "pending", "priority": "high" }] }

39. **parallel** - Execute multiple read-only tool calls concurrently
    Usage: { "tool": "parallel", "tool_calls": [{ "tool": "glob", "args": { "pattern": "src/**/*.rs" } }, { "tool": "grep", "args": { "pattern": "fn\\s+main", "include": "*.rs" } }], "timeout_ms": 30000 }

40. **task** - Create a subagent task using the existing sessions runtime
    Usage: { "tool": "task", "description": "short", "prompt": "do X", "subagent_type": "general", "timeout_seconds": 120, "max_retries": 0, "retry_backoff_ms": 1000 }

41. **skill** - Manage loaded skills (list/show/enable/disable)
    Usage: { "tool": "skill", "action": "create", "name": "my_skill", "backend": "native", "description": "optional", "auto_enable": true }
    Usage: { "tool": "skill", "action": "list" }
    Usage: { "tool": "skill", "action": "show", "name": "github" }
    Usage: { "tool": "skill", "action": "enable", "name": "github" }
    Usage: { "tool": "skill", "action": "disable", "name": "github" }
    Usage: { "tool": "skill", "action": "run", "name": "my_skill", "tool_name": "my_tool", "arguments": { } }

42. **mcp_config** - Manage MCP server configuration (manual + agent-driven)
    Usage: { "tool": "mcp_config", "action": "list" }
    Usage: { "tool": "mcp_config", "action": "status" }
    Usage: { "tool": "mcp_config", "action": "add", "name": "server", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] }
    Usage: { "tool": "mcp_config", "action": "remove", "name": "server" }
    Usage: { "tool": "mcp_config", "action": "connect_all" }
"#.to_string();

    #[cfg(feature = "browser")]
    {
        s.push_str(r##"
28. **browser_navigate** - Navigate to a URL in the browser
    Usage: { "tool": "browser_navigate", "url": "https://example.com" }

29. **browser_click** - Click an element by CSS selector
    Usage: { "tool": "browser_click", "selector": "#submit-button" }

30. **browser_type** - Type text into an element
    Usage: { "tool": "browser_type", "selector": "input[name='q']", "text": "hello" }

31. **browser_screenshot** - Take a screenshot of the current page
    Usage: { "tool": "browser_screenshot" }
    Returns path to saved PNG file.

32. **browser_evaluate** - Execute JavaScript on the page
    Usage: { "tool": "browser_evaluate", "script": "document.title" }

33. **browser_pdf** - Print current page to PDF
    Usage: { "tool": "browser_pdf" }
    Returns path to saved PDF file.

34. **browser_list_tabs** - List all open browser tabs
    Usage: { "tool": "browser_list_tabs" }

35. **browser_switch_tab** - Switch to a specific tab
    Usage: { "tool": "browser_switch_tab", "index": 0 }
"##);
    }

    s.push_str("\nWhen you need to use a tool, respond with ONLY the JSON tool call on a single line.\n\nAfter I execute the tool and show you the result, continue the conversation normally.\n");
    s
}

pub fn supported_tool_names() -> Vec<&'static str> {
    let tools = vec![
        "read_file",
        "write_file",
        "edit_file",
        "list_directory",
        "run_command",
        "spawn_process",
        "read_process_output",
        "kill_process",
        "list_processes",
        "web_fetch",
        "glob",
        "grep",
        "question",
        "apply_patch",
        "todowrite",
        "parallel",
        "task",
        "skill",
        "mcp_config",
        "write_process_input",
        "web_search",
        "script_eval",
        "sessions_spawn",
        "spawn_subagent",
        "get_subagent_result",
        "list_subagents",
        "sessions_wait",
        "sessions_broadcast",
        "message",
        "gateway",
        "session_status",
        "sessions_history",
        "sessions_list",
        "sessions_send",
        "sessions_cancel",
        "sessions_pause",
        "sessions_resume",
        "agents_list",
        "memory_get",
        "llm_task",
        "tts",
        "stt",
        "cron",
        "memory_search",
        "memory_save",
    ];

    #[cfg(feature = "browser")]
    {
        tools.extend([
            "browser_navigate",
            "browser_click",
            "browser_type",
            "browser_screenshot",
            "browser_evaluate",
            "browser_pdf",
            "browser_list_tabs",
            "browser_switch_tab",
        ]);
    }

    tools
}

/// Execute a tool based on JSON input
#[tracing::instrument(skip_all, fields(tool_name))]
pub async fn execute_tool(
    tool_input: &str,
    cron_scheduler: Option<&crate::cron::CronScheduler>,
    agent_manager: Option<&crate::gateway::agent_manager::AgentManager>,
    memory_manager: Option<&std::sync::Arc<crate::memory::MemoryManager>>,
    persistence: Option<&crate::persistence::PersistenceManager>,
    permission_manager: Option<&tokio::sync::Mutex<super::PermissionManager>>,
    tool_policy: Option<&super::policy::ToolPolicy>,
    confirmation_service: Option<&tokio::sync::Mutex<super::confirmation::ConfirmationService>>,
    skill_loader: Option<&std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>>,
    #[cfg(feature = "browser")]
    browser_client: Option<&crate::browser::BrowserClient>,
    tenant_id: Option<&str>,
    mcp_manager: Option<&std::sync::Arc<crate::mcp::McpManager>>,
) -> Result<String> {
    // Strip prefix if present (optional support)
    let json_str = tool_input.trim().trim_start_matches("__TOOL_CALL__").trim();

    let tool_call: serde_json::Value = serde_json::from_str(json_str)?;

    let runtime_config = crate::config::Config::load().ok();
    let interaction_policy = runtime_config
        .as_ref()
        .map(|c| c.interaction_policy)
        .unwrap_or_default();
    let audit_logger = runtime_config
        .as_ref()
        .and_then(|c| c.audit_log_path.as_ref())
        .map(|p| crate::system::audit::AuditLogger::new(std::path::PathBuf::from(p)));

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
    let default_policy = super::policy::ToolPolicy::permissive();
    let policy = tool_policy.unwrap_or(&default_policy);

    policy
        .check_tool_allowed(tool_name)
        .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;
    
    // Map tool to operation type for permission checking
    let operation = match tool_name {
        "read_file" | "list_directory" | "glob" | "grep" | "parallel" => {
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
        "todowrite" => {
            super::permissions::Operation::WriteFile(workspace_root.join(".nanobot").join("todos"))
        }
        "web_fetch" => tool_call
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| super::permissions::Operation::NetworkRequest(url.to_string()))
            .unwrap_or_else(|| super::permissions::Operation::NetworkRequest("web_fetch".to_string())),
        "web_search" => {
            super::permissions::Operation::NetworkRequest("web_search".to_string())
        }
        "browser_navigate" | "browser_click" | "browser_type" | "browser_screenshot"
        | "browser_evaluate" | "browser_pdf" | "browser_list_tabs" | "browser_switch_tab" => {
            super::permissions::Operation::NetworkRequest("browser".to_string())
        }
        "run_command" | "bash" | "exec" => {
            let cmd = tool_call.get("command")
                .or(tool_call.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            super::permissions::Operation::ExecuteCommand(cmd.to_string())
        }
        "question" => {
            super::permissions::Operation::ReadFile(workspace_root.clone())
        }
        "task" => {
            super::permissions::Operation::ExecuteCommand("sessions_spawn".to_string())
        }
        "skill" => {
            super::permissions::Operation::ReadFile(workspace_root.join("skills"))
        }
        "mcp_config" => {
            super::permissions::Operation::WriteFile(workspace_root.join("config.toml"))
        }
        _ => {
            // Unknown tool, treat as medium-risk command
            super::permissions::Operation::ExecuteCommand(format!("unknown:{}", tool_name))
        }
    };

    match tool_name {
        "read_file" | "list_directory" | "glob" | "grep" | "parallel" => {
            if let Some(path_str) = tool_call.get("path").and_then(|v| v.as_str()) {
                policy
                    .check_read_path(path_str)
                    .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;
            }
        }
        "write_file" | "edit_file" | "apply_patch" => {
            if let Some(path_str) = tool_call.get("path").and_then(|v| v.as_str()) {
                policy
                    .check_write_path(path_str)
                    .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;
            }
        }
        "run_command" | "spawn_process" | "tts" => {
            if let Some(cmd) = tool_call
                .get("command")
                .or(tool_call.get("cmd"))
                .and_then(|v| v.as_str())
            {
                policy
                    .check_command_allowed(cmd)
                    .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;
            }
        }
        _ => {}
    }
    
    // Check permission (using passed permission manager or create temporary one)
    let channel_key = tenant_id.unwrap_or("default");
    let operation_key = format!("{}:{}:{:?}", channel_key, tool_name, operation);
    let cached_decision = if let Some(perm_mgr) = permission_manager {
        let mgr = perm_mgr.lock().await;
        mgr.get_cached_decision(&operation_key)
    } else {
        None
    };

    let decision = if let Some(cached) = cached_decision {
        if cached {
            super::permissions::PermissionDecision::Allow
        } else {
            super::permissions::PermissionDecision::Deny
        }
    } else if let Some(perm_mgr) = permission_manager {
        let mgr = perm_mgr.lock().await;
        mgr.check_permission(&operation)
    } else {
        // Fallback: Create temporary permission manager for backwards compatibility
        let profile = super::permissions::SecurityProfile::standard(workspace_root.clone());
        let temp_mgr = super::permissions::PermissionManager::new(profile);
        temp_mgr.check_permission(&operation)
    };
    
    match decision {
        super::permissions::PermissionDecision::Deny => {
            tracing::warn!("Permission denied for tool: {}", tool_name);
            if let Some(logger) = &audit_logger {
                logger.log_deny(
                    tool_name,
                    tenant_id.unwrap_or("default"),
                    "permission-manager",
                    tool_call.clone(),
                    "permission denied",
                );
            }
            return Ok(super::ToolResult::error(format!("Permission denied: Tool '{}' is not allowed", tool_name)).output);
        }
        super::permissions::PermissionDecision::Ask => {
            if interaction_policy == crate::config::InteractionPolicy::HeadlessDeny {
                tracing::warn!("HeadlessDeny active, auto-denying tool: {}", tool_name);
                if let Some(logger) = &audit_logger {
                    logger.log_deny(
                        tool_name,
                        tenant_id.unwrap_or("default"),
                        "headless-deny",
                        tool_call.clone(),
                        "interaction_policy=headlessdeny",
                    );
                }
                return Ok(super::ToolResult::error(format!(
                    "Permission denied by interaction policy: '{}'",
                    tool_name
                ))
                .output);
            } else if interaction_policy == crate::config::InteractionPolicy::HeadlessAllowLog {
                tracing::info!("HeadlessAllowLog active, auto-allowing tool: {}", tool_name);
                if let Some(logger) = &audit_logger {
                    logger.log_allow(
                        tool_name,
                        tenant_id.unwrap_or("default"),
                        "headless-allow-log",
                        tool_call.clone(),
                    );
                }
            } else {
                let risk_level = match tool_name {
                    "read_file" | "list_directory" => super::confirmation::RiskLevel::Low,
                    "write_file" | "edit_file" => super::confirmation::RiskLevel::Medium,
                    "run_command" | "bash" | "exec" => super::confirmation::RiskLevel::High,
                    _ => super::confirmation::RiskLevel::Medium,
                };

                let request = super::confirmation::ConfirmationRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    tool_name: tool_name.to_string(),
                    operation: format!("{:?}", operation),
                    args: serde_json::to_string_pretty(&tool_call)?,
                    risk_level,
                    timeout: None,
                    channel: tenant_id.map(|id| id.to_string()),
                };

                let response = if let Some(service) = confirmation_service {
                    service.lock().await.request_confirmation(request).await?
                } else {
                    let mut local_service = super::confirmation::ConfirmationService::new();
                    local_service
                        .register_adapter(Box::new(super::cli_confirmation::CliConfirmationAdapter::new()));
                    local_service.request_confirmation(request).await?
                };

                if !response.allowed {
                    tracing::info!("User denied permission for tool: {}", tool_name);
                    if let Some(logger) = &audit_logger {
                        logger.log_deny(
                            tool_name,
                            tenant_id.unwrap_or("default"),
                            "user-denied",
                            tool_call.clone(),
                            "confirmation denied",
                        );
                    }
                    return Ok(super::ToolResult::error(format!("User denied permission for tool: {}", tool_name)).output);
                }

                if response.remember {
                    if let Some(perm_mgr) = permission_manager {
                        let mut mgr = perm_mgr.lock().await;
                        mgr.cache_decision(operation_key, true);
                    }
                }

                tracing::info!("User approved permission for tool: {}", tool_name);
                if let Some(logger) = &audit_logger {
                    logger.log_allow(
                        tool_name,
                        tenant_id.unwrap_or("default"),
                        "user-approved",
                        tool_call.clone(),
                    );
                }
            }
        }
        super::permissions::PermissionDecision::Allow => {
            tracing::debug!("Tool {} auto-approved by profile", tool_name);
            if let Some(logger) = &audit_logger {
                logger.log_allow(
                    tool_name,
                    tenant_id.unwrap_or("default"),
                    "permission-allow",
                    tool_call.clone(),
                );
            }
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

    // Try MCP Tools
    if let Some(manager) = mcp_manager {
        // Prepare args (remove "tool" field)
        let args = if let Some(obj) = tool_call.as_object() {
            let mut args_obj = obj.clone();
            args_obj.remove("tool");
            serde_json::Value::Object(args_obj)
        } else {
            tool_call.clone()
        };

        if let Some(result) = manager.execute_tool_by_name(tool_name, args).await {
            match result {
                Ok(tool_res) => {
                     let output = tool_res.content.iter()
                        .map(|c| match c {
                            crate::mcp::types::ToolCallContent::Text { text } => text.clone(),
                            _ => "[Non-text content]".to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                        
                    if tool_res.is_error.unwrap_or(false) {
                         return Ok(format!("❌ Tool Error: {}", output));
                    } else {
                         return Ok(output);
                    }
                }
                Err(e) => return Err(anyhow::anyhow!("MCP Tool execution failed: {}", e)),
            }
        }
    }

    // Try ToolRegistry next (for simple, modular tools)
    let registry = super::definitions::get_tool_registry();
    if registry.get(tool_name).is_some() {
        // Extract args (everything except "tool" field)
        let args = if let Some(obj) = tool_call.as_object() {
            let mut args_obj = obj.clone();
            args_obj.remove("tool");
            serde_json::Value::Object(args_obj)
        } else {
            tool_call.clone()
        };

        return registry.execute_with_policy(tool_name, args, policy).await;
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
            super::process::list_processes().await
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

        "glob" => {
            let args = GlobArgs {
                pattern: tool_call["pattern"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' field"))?
                    .to_string(),
                path: tool_call["path"].as_str().map(|s| s.to_string()),
                max_results: tool_call["max_results"]
                    .as_u64()
                    .or_else(|| tool_call["maxResults"].as_u64())
                    .map(|n| n as usize),
            };
            glob_files(args).await
        }

        "grep" => {
            let args = GrepArgs {
                pattern: tool_call["pattern"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'pattern' field"))?
                    .to_string(),
                path: tool_call["path"].as_str().map(|s| s.to_string()),
                include: tool_call["include"].as_str().map(|s| s.to_string()),
                case_sensitive: tool_call["case_sensitive"]
                    .as_bool()
                    .or_else(|| tool_call["caseSensitive"].as_bool()),
                max_results: tool_call["max_results"]
                    .as_u64()
                    .or_else(|| tool_call["maxResults"].as_u64())
                    .map(|n| n as usize),
            };
            grep_files(args).await
        }

        "question" => {
            let question = tool_call["question"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'question' field"))?;
            let header = tool_call["header"].as_str().unwrap_or("Question");
            let multiple = tool_call["multiple"].as_bool().unwrap_or(false);
            let options: Vec<String> = tool_call["options"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            Ok(json!({
                "type": "question",
                "header": header,
                "question": question,
                "options": options,
                "multiple": multiple,
            }).to_string())
        }

        "apply_patch" => {
            let operations = tool_call["operations"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing 'operations' field"))?
                .clone();

            let args = ApplyPatchArgs {
                operations: serde_json::from_value(serde_json::Value::Array(operations))?,
                dry_run: tool_call["dry_run"]
                    .as_bool()
                    .or_else(|| tool_call["dryRun"].as_bool())
                    .unwrap_or(false),
                atomic: tool_call["atomic"].as_bool().unwrap_or(true),
            };
            apply_patch(args).await
        }

        "todowrite" => {
            let todos = tool_call["todos"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing 'todos' field"))?
                .clone();

            let args = TodoWriteArgs {
                todos: serde_json::from_value(serde_json::Value::Array(todos))?,
            };
            todo_write(args, tenant_id).await
        }

        "parallel" => {
            let calls = tool_call["tool_calls"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing 'tool_calls' field"))?;
            if calls.is_empty() {
                return Err(anyhow::anyhow!("tool_calls cannot be empty"));
            }
            if calls.len() > 16 {
                return Err(anyhow::anyhow!("tool_calls max is 16"));
            }

            let timeout_ms = tool_call["timeout_ms"]
                .as_u64()
                .or_else(|| tool_call["timeoutMs"].as_u64())
                .unwrap_or(30_000)
                .clamp(1_000, 120_000);
            let timeout = Duration::from_millis(timeout_ms);

            let mut prepared = Vec::with_capacity(calls.len());
            for (idx, call) in calls.iter().enumerate() {
                let obj = call
                    .as_object()
                    .ok_or_else(|| anyhow::anyhow!("tool_calls[{}] must be an object", idx))?;
                let tool = obj
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("tool_calls[{}].tool missing", idx))?;

                if tool == "parallel" {
                    return Err(anyhow::anyhow!("Nested parallel calls are not allowed"));
                }
                if !is_parallel_safe_tool(tool) {
                    return Err(anyhow::anyhow!(
                        "tool_calls[{}].tool '{}' is not allowed in parallel mode",
                        idx,
                        tool
                    ));
                }

                let mut merged = serde_json::Map::new();
                merged.insert("tool".to_string(), serde_json::Value::String(tool.to_string()));

                if let Some(args) = obj.get("args").and_then(|v| v.as_object()) {
                    if args.contains_key("tool") {
                        return Err(anyhow::anyhow!(
                            "tool_calls[{}].args cannot include reserved key 'tool'",
                            idx
                        ));
                    }
                    for (k, v) in args {
                        merged.insert(k.clone(), v.clone());
                    }
                } else {
                    for (k, v) in obj {
                        if k != "tool" {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                }

                prepared.push((idx, tool.to_string(), serde_json::Value::Object(merged).to_string()));
            }

            let results = join_all(prepared.into_iter().map(|(idx, tool, input)| async move {
                let started = std::time::Instant::now();
                let call = std::boxed::Box::pin(execute_tool(
                    &input,
                    cron_scheduler,
                    agent_manager,
                    memory_manager,
                    persistence,
                    permission_manager,
                    tool_policy,
                    confirmation_service,
                    skill_loader,
                    #[cfg(feature = "browser")]
                    browser_client,
                    tenant_id,
                    mcp_manager,
                ));
                let res = tokio::time::timeout(timeout, call).await;
                let duration_ms = started.elapsed().as_millis();

                match res {
                    Ok(Ok(output)) => json!({
                        "index": idx,
                        "tool": tool,
                        "status": "ok",
                        "duration_ms": duration_ms,
                        "output": output,
                    }),
                    Ok(Err(err)) => json!({
                        "index": idx,
                        "tool": tool,
                        "status": "error",
                        "duration_ms": duration_ms,
                        "error": err.to_string(),
                    }),
                    Err(_) => json!({
                        "index": idx,
                        "tool": tool,
                        "status": "timeout",
                        "duration_ms": duration_ms,
                        "error": format!("call exceeded timeout_ms={}", timeout_ms),
                    }),
                }
            }))
            .await;

            Ok(json!({
                "status": "ok",
                "count": results.len(),
                "results": results,
            })
            .to_string())
        }

        "task" => match agent_manager {
            Some(manager) => {
                let prompt = tool_call["prompt"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' field"))?;
                let description = tool_call["description"].as_str().unwrap_or("task");
                let subagent_type = tool_call["subagent_type"]
                    .as_str()
                    .or_else(|| tool_call["subagentType"].as_str())
                    .unwrap_or("general");

                let timeout_seconds = tool_call["timeout_seconds"]
                    .as_u64()
                    .or_else(|| tool_call["timeoutSeconds"].as_u64())
                    .unwrap_or(120);
                let max_retries = tool_call["max_retries"]
                    .as_u64()
                    .or_else(|| tool_call["maxRetries"].as_u64())
                    .unwrap_or(0) as u32;
                let retry_backoff_ms = tool_call["retry_backoff_ms"]
                    .as_u64()
                    .or_else(|| tool_call["retryBackoffMs"].as_u64())
                    .unwrap_or(1000);

                let parent_session_id = tool_call["parent_session_id"]
                    .as_str()
                    .or_else(|| tool_call["parentSessionId"].as_str())
                    .or(tenant_id)
                    .unwrap_or("main")
                    .to_string();

                let label = Some(format!("{}:{}", subagent_type, description));

                let (session, task_obj) = manager
                    .spawn_subagent_with_options(
                        parent_session_id,
                        prompt.to_string(),
                        label,
                        crate::gateway::agent_manager::CleanupPolicy::Keep,
                        None,
                        crate::gateway::agent_manager::SubagentOptions {
                            max_retries,
                            retry_backoff_ms,
                            timeout_seconds,
                        },
                    )
                    .await?;

                Ok(json!({
                    "status": "pending",
                    "session_id": session.id,
                    "task_id": task_obj.id,
                    "description": description,
                    "subagent_type": subagent_type,
                    "hint": "Use sessions_wait/session_status/sessions_cancel for lifecycle control",
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "skill" => {
            let action = tool_call["action"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' field"))?;

            let loader = skill_loader
                .ok_or_else(|| anyhow::anyhow!("Skill loader not initialized"))?;

            match action {
                "create" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?
                        .trim()
                        .to_string();
                    let backend = tool_call["backend"]
                        .as_str()
                        .unwrap_or("native")
                        .to_lowercase();
                    let description = tool_call["description"]
                        .as_str()
                        .unwrap_or("Custom skill")
                        .to_string();
                    let auto_enable = tool_call["auto_enable"]
                        .as_bool()
                        .or_else(|| tool_call["autoEnable"].as_bool())
                        .unwrap_or(true);

                    let mut loader_guard = loader.lock().await;
                    loader_guard.scan()?;
                    if loader_guard.get_skill(&name).is_some() {
                        return Err(anyhow::anyhow!("Skill already exists: {}", name));
                    }
                    let workspace = loader_guard.workspace_dir().to_path_buf();
                    drop(loader_guard);

                    let skill_dir = workspace.join("skills").join(&name);
                    tokio::fs::create_dir_all(&skill_dir).await?;

                    let tool_name = "run";
                    let mut frontmatter = vec![
                        "---".to_string(),
                        format!("name: {}", name),
                        format!("description: \"{}\"", description),
                        "category: custom".to_string(),
                        "status: active".to_string(),
                        format!("backend: {}", backend),
                    ];

                    match backend.as_str() {
                        "mcp" => {
                            frontmatter.push(format!("mcp_server_name: {}-server", name));
                            frontmatter.push("mcp_command: npx".to_string());
                            frontmatter.push(
                                "mcp_args: [\"-y\", \"@modelcontextprotocol/server-filesystem\", \".\"]"
                                    .to_string(),
                            );
                        }
                        "deno" => {
                            frontmatter.push("deno_command: deno".to_string());
                            frontmatter.push(format!("deno_script: skills/{}/main.ts", name));
                            frontmatter.push(
                                "deno_args: [\"run\", \"--allow-read\", \"--allow-write\"]"
                                    .to_string(),
                            );
                            frontmatter.push("deno_sandbox: balanced".to_string());

                            let script = r#"const tool = Deno.args[0] ?? "run";
const raw = Deno.args[1] ?? "{}";
let args = {};
try { args = JSON.parse(raw); } catch (_) {}

if (tool === "run") {
  console.log(JSON.stringify({ status: "ok", tool, args }));
} else {
  console.error(`unknown tool: ${tool}`);
  Deno.exit(1);
}
"#;
                            tokio::fs::write(skill_dir.join("main.ts"), script).await?;
                        }
                        "native" => {
                            frontmatter.push("native_command: __set_me__".to_string());
                            frontmatter.push("native_args: []".to_string());
                        }
                        other => {
                            return Err(anyhow::anyhow!(
                                "Unsupported backend '{}'. expected: native|mcp|deno",
                                other
                            ));
                        }
                    }

                    frontmatter.push("---".to_string());
                    let body = format!(
                        "\n# {}\n\n{}\n\n## Tools Provided\n\n- `{}`: Primary skill action\n",
                        name, description, tool_name
                    );
                    let content = format!("{}{}", frontmatter.join("\n"), body);
                    tokio::fs::write(skill_dir.join("SKILL.md"), content).await?;

                    if auto_enable {
                        let mut loader_guard = loader.lock().await;
                        loader_guard.scan()?;
                        let _ = loader_guard.enable_skill(&name);
                        drop(loader_guard);

                        let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
                        cfg.enable_skill(&name);
                        let _ = cfg.save();
                    }

                    Ok(json!({
                        "status": "ok",
                        "action": "create",
                        "name": name,
                        "backend": backend,
                        "enabled": auto_enable,
                        "path": skill_dir.to_string_lossy(),
                        "next": if auto_enable { "Use skill run" } else { "Use skill enable then skill run" },
                    })
                    .to_string())
                }
                "list" => {
                    let mut loader = loader.lock().await;
                    loader.scan()?;
                    let mut names: Vec<_> = loader
                        .skills()
                        .values()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "description": s.description,
                                "enabled": s.enabled,
                                "category": s.category,
                                "status": s.status,
                            })
                        })
                        .collect();
                    names.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                    Ok(json!({"skills": names, "count": names.len()}).to_string())
                }
                "show" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?;
                    let mut loader = loader.lock().await;
                    loader.scan()?;
                    if let Some(skill) = loader.get_skill(name) {
                        Ok(json!({
                            "name": skill.name,
                            "description": skill.description,
                            "category": skill.category,
                            "status": skill.status,
                            "enabled": skill.enabled,
                            "backend": skill.backend,
                            "tools": skill.tools,
                            "dependencies": skill.dependencies,
                            "mcp_server_name": skill.mcp_server_name,
                            "mcp_command": skill.mcp_command,
                            "mcp_args": skill.mcp_args,
                            "mcp_env": skill.mcp_env,
                            "deno_command": skill.deno_command,
                            "deno_script": skill.deno_script,
                            "deno_args": skill.deno_args,
                            "deno_sandbox": skill.deno_sandbox,
                            "deno_permissions": skill.deno_permissions,
                            "deno_env": skill.deno_env,
                            "native_command": skill.native_command,
                            "native_args": skill.native_args,
                            "native_env": skill.native_env,
                            "author": skill.author,
                            "skill_path": skill.skill_path,
                        })
                        .to_string())
                    } else {
                        Err(anyhow::anyhow!("Skill not found: {}", name))
                    }
                }
                "enable" | "disable" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?;

                    let mut loader = loader.lock().await;
                    loader.scan()?;
                    if action == "enable" {
                        loader.enable_skill(name)?;
                    } else {
                        loader.disable_skill(name)?;
                    }

                    let mut cfg = crate::skills::config::SkillsConfig::load()?;
                    if action == "enable" {
                        cfg.enable_skill(name);
                    } else {
                        cfg.disable_skill(name);
                    }
                    cfg.save()?;

                    Ok(json!({
                        "status": "ok",
                        "action": action,
                        "name": name,
                    })
                    .to_string())
                }
                "run" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?;
                    let tool_name = tool_call["tool_name"]
                        .as_str()
                        .or_else(|| tool_call["toolName"].as_str())
                        .ok_or_else(|| anyhow::anyhow!("Missing 'tool_name' field"))?;
                    let arguments = tool_call["arguments"].clone();
                    let arguments = if arguments.is_null() {
                        serde_json::json!({})
                    } else {
                        arguments
                    };

                    let mut loader = loader.lock().await;
                    loader.scan()?;
                    let skill = loader
                        .get_skill(name)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", name))?;
                    drop(loader);

                    let skills_cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
                    if !skills_cfg.is_enabled(name) {
                        return Err(anyhow::anyhow!(
                            "Skill '{}' is disabled. Enable it first with {{ \"tool\": \"skill\", \"action\": \"enable\", \"name\": \"{}\" }}",
                            name,
                            name
                        ));
                    }

                    match skill.backend.to_lowercase().as_str() {
                        "mcp" => {
                            let manager = mcp_manager.ok_or_else(|| {
                                anyhow::anyhow!("MCP manager not initialized")
                            })?;

                            let server_name = skill
                                .mcp_server_name
                                .clone()
                                .unwrap_or_else(|| format!("skill-{}", name));

                            if let Some(command) = skill.mcp_command.clone() {
                                manager
                                    .add_server(crate::mcp::types::McpServerConfig {
                                        name: server_name.clone(),
                                        command,
                                        args: skill.mcp_args.clone(),
                                        env: skill.mcp_env.clone(),
                                    })
                                    .await?;
                            }

                            let result = match manager.call_tool(&server_name, tool_name, arguments.clone()).await {
                                Ok(r) => r,
                                Err(e) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to call MCP tool '{}' on server '{}': {}. Configure with mcp_config add/connect_all or provide mcp_command in SKILL.md",
                                        tool_name,
                                        server_name,
                                        e
                                    ));
                                }
                            };
                            let content_text = result
                                .content
                                .iter()
                                .filter_map(|c| match c {
                                    crate::mcp::types::ToolCallContent::Text { text } => Some(text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");

                            Ok(json!({
                                "status": "ok",
                                "backend": "mcp",
                                "skill": name,
                                "server": server_name,
                                "tool": tool_name,
                                "content": content_text,
                                "raw": result,
                            })
                            .to_string())
                        }
                        "deno" => {
                            let script = skill
                                .deno_script
                                .clone()
                                .ok_or_else(|| anyhow::anyhow!(
                                    "Deno skill '{}' missing deno_script in SKILL.md",
                                    name
                                ))?;
                            let deno_command = skill
                                .deno_command
                                .clone()
                                .unwrap_or_else(|| "deno".to_string());

                            if !command_exists_quick(&deno_command) {
                                return Err(anyhow::anyhow!(
                                    "Deno command '{}' not found. Install Deno or set deno_command in SKILL.md",
                                    deno_command
                                ));
                            }

                            let mut cmd = tokio::process::Command::new(&deno_command);
                            let mut deno_args = if skill.deno_args.is_empty() {
                                vec!["run".to_string()]
                            } else {
                                skill.deno_args.clone()
                            };
                            push_unique_args(
                                &mut deno_args,
                                deno_policy_flags(skill.deno_sandbox.as_deref())
                                    .into_iter()
                                    .map(|s| s.to_string()),
                            );
                            push_unique_args(&mut deno_args, skill.deno_permissions.clone());

                            cmd.args(&deno_args);
                            cmd.arg(&script);
                            cmd.arg(tool_name);
                            cmd.arg(arguments.to_string());
                            cmd.stdin(std::process::Stdio::null());
                            cmd.stdout(std::process::Stdio::piped());
                            cmd.stderr(std::process::Stdio::piped());
                            cmd.env("NANOBOT_SKILL", name);
                            cmd.env("NANOBOT_TOOL", tool_name);
                            cmd.env("NANOBOT_TOOL_ARGS", arguments.to_string());
                            for (k, v) in &skill.deno_env {
                                cmd.env(k, v);
                            }

                            let output = match tokio::time::timeout(
                                std::time::Duration::from_secs(60),
                                cmd.output(),
                            )
                            .await
                            {
                                Ok(Ok(o)) => o,
                                Ok(Err(e)) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to start deno command '{}': {}",
                                        deno_command,
                                        e
                                    ));
                                }
                                Err(_) => {
                                    return Err(anyhow::anyhow!(
                                        "Deno skill '{}' timed out after 60s",
                                        name
                                    ));
                                }
                            };

                            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

                            if !output.status.success() {
                                return Err(anyhow::anyhow!(
                                    "Deno skill '{}' failed (command '{}'): {}",
                                    name,
                                    deno_command,
                                    if stderr.is_empty() { "no stderr output" } else { &stderr }
                                ));
                            }

                            let parsed: Option<serde_json::Value> = serde_json::from_str(&stdout).ok();
                            Ok(json!({
                                "status": "ok",
                                "backend": "deno",
                                "skill": name,
                                "tool": tool_name,
                                "sandbox": skill.deno_sandbox.clone().unwrap_or_else(|| "balanced".to_string()),
                                "applied_permissions": deno_args,
                                "script": script,
                                "output": stdout,
                                "json": parsed,
                            })
                            .to_string())
                        }
                        "native" => {
                            let native_command = skill
                                .native_command
                                .clone()
                                .ok_or_else(|| anyhow::anyhow!(
                                    "Native skill '{}' missing native_command in SKILL.md",
                                    name
                                ))?;

                            let mut cmd = tokio::process::Command::new(&native_command);
                            if !skill.native_args.is_empty() {
                                cmd.args(&skill.native_args);
                            }
                            cmd.arg(tool_name);
                            cmd.arg(arguments.to_string());
                            cmd.stdin(std::process::Stdio::null());
                            cmd.stdout(std::process::Stdio::piped());
                            cmd.stderr(std::process::Stdio::piped());
                            cmd.env("NANOBOT_SKILL", name);
                            cmd.env("NANOBOT_TOOL", tool_name);
                            cmd.env("NANOBOT_TOOL_ARGS", arguments.to_string());
                            for (k, v) in &skill.native_env {
                                cmd.env(k, v);
                            }

                            let output = match tokio::time::timeout(
                                std::time::Duration::from_secs(60),
                                cmd.output(),
                            )
                            .await
                            {
                                Ok(Ok(o)) => o,
                                Ok(Err(e)) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to start native command '{}': {}",
                                        native_command,
                                        e
                                    ));
                                }
                                Err(_) => {
                                    return Err(anyhow::anyhow!(
                                        "Native skill '{}' timed out after 60s",
                                        name
                                    ));
                                }
                            };

                            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

                            if !output.status.success() {
                                return Err(anyhow::anyhow!(
                                    "Native skill '{}' failed (command '{}'): {}",
                                    name,
                                    native_command,
                                    if stderr.is_empty() { "no stderr output" } else { &stderr }
                                ));
                            }

                            let parsed: Option<serde_json::Value> = serde_json::from_str(&stdout).ok();
                            Ok(json!({
                                "status": "ok",
                                "backend": "native",
                                "skill": name,
                                "tool": tool_name,
                                "command": native_command,
                                "output": stdout,
                                "json": parsed,
                            })
                            .to_string())
                        }
                        _ => Err(anyhow::anyhow!(
                            "Unsupported backend '{}' in skill.run",
                            skill.backend
                        )),
                    }
                }
                _ => Err(anyhow::anyhow!("Unknown skill action: {}", action)),
            }
        }

        "mcp_config" => {
            let action = tool_call["action"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' field"))?;

            let mut cfg = crate::config::Config::load()?;
            let mcp_cfg = cfg.mcp.get_or_insert(crate::config::McpConfig {
                enabled: false,
                servers: Vec::new(),
            });

            match action {
                "list" => {
                    let servers = mcp_cfg
                        .servers
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "command": s.command,
                                "args": s.args,
                                "env_keys": s.env.keys().cloned().collect::<Vec<_>>(),
                            })
                        })
                        .collect::<Vec<_>>();

                    let connected_servers = if let Some(manager) = mcp_manager {
                        manager.list_servers().await
                    } else {
                        Vec::new()
                    };

                    Ok(json!({
                        "enabled": mcp_cfg.enabled,
                        "count": servers.len(),
                        "servers": servers,
                        "connected_servers": connected_servers,
                    })
                    .to_string())
                }
                "status" => {
                    let connected_servers = if let Some(manager) = mcp_manager {
                        manager.list_servers().await
                    } else {
                        Vec::new()
                    };
                    Ok(json!({
                        "enabled": mcp_cfg.enabled,
                        "configured_servers": mcp_cfg.servers.len(),
                        "connected_servers": connected_servers,
                        "manager_active": mcp_manager.is_some(),
                        "hint": "Use mcp_config add/remove/connect_all to manage MCP servers",
                    })
                    .to_string())
                }
                "add" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?
                        .to_string();
                    let command = tool_call["command"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'command' field"))?
                        .to_string();
                    let args = tool_call["args"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'args' field"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .ok_or_else(|| anyhow::anyhow!("All args must be strings"))
                        })
                        .collect::<Result<Vec<String>>>()?;

                    let env = tool_call["env"]
                        .as_object()
                        .map(|map| {
                            map.iter()
                                .map(|(k, v)| {
                                    v.as_str()
                                        .map(|s| (k.clone(), s.to_string()))
                                        .ok_or_else(|| anyhow::anyhow!("env values must be strings"))
                                })
                                .collect::<Result<std::collections::HashMap<String, String>>>()
                        })
                        .transpose()?
                        .unwrap_or_default();

                    let server = crate::mcp::types::McpServerConfig {
                        name: name.clone(),
                        command,
                        args,
                        env,
                    };
                    let env_keys = server.env.keys().cloned().collect::<Vec<_>>();
                    let env_count = server.env.len();
                    let env_preview = redacted_env_preview(&server.env);

                    mcp_cfg.enabled = true;
                    mcp_cfg.servers.retain(|s| s.name != name);
                    mcp_cfg.servers.push(server.clone());
                    cfg.save()?;

                    if let Some(manager) = mcp_manager {
                        manager.add_server(server).await?;
                    }

                    Ok(json!({
                        "status": "ok",
                        "action": "add",
                        "name": name,
                        "connected_now": mcp_manager.is_some(),
                        "env_keys": env_keys,
                        "env_count": env_count,
                        "env_preview": env_preview,
                        "secrets_redacted": true,
                        "redaction_note": "Secret values are hidden. Only env key names are shown.",
                    })
                    .to_string())
                }
                "remove" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?
                        .to_string();

                    let before = mcp_cfg.servers.len();
                    mcp_cfg.servers.retain(|s| s.name != name);
                    let removed = before != mcp_cfg.servers.len();
                    cfg.save()?;

                    if let Some(manager) = mcp_manager {
                        let _ = manager.remove_server(&name).await;
                    }

                    Ok(json!({
                        "status": "ok",
                        "action": "remove",
                        "name": name,
                        "removed": removed,
                    })
                    .to_string())
                }
                "connect_all" => {
                    if let Some(manager) = mcp_manager {
                        let mut connected = Vec::new();
                        let mut failed = Vec::new();
                        for server in mcp_cfg.servers.clone() {
                            match manager.add_server(server.clone()).await {
                                Ok(_) => connected.push(server.name),
                                Err(e) => failed.push(json!({"name": server.name, "error": e.to_string()})),
                            }
                        }
                        Ok(json!({
                            "status": "ok",
                            "connected": connected,
                            "failed": failed,
                        })
                        .to_string())
                    } else {
                        Ok(json!({
                            "status": "ok",
                            "note": "MCP manager not active in this mode. Configuration saved for next startup.",
                            "configured_servers": mcp_cfg.servers.len(),
                        })
                        .to_string())
                    }
                }
                _ => Err(anyhow::anyhow!(
                    "Unknown mcp_config action: {}",
                    action
                )),
            }
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

        "memory_get" => {
            let id = tool_call["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'id' field"))?;

            match memory_manager {
                Some(manager) => {
                    let entry = manager.get_document(id, tenant_id)?;
                    if let Some(entry) = entry {
                        Ok(json!({
                            "id": entry.id,
                            "content": entry.content,
                            "metadata": entry.metadata,
                            "tenant_id": entry.tenant_id,
                        })
                        .to_string())
                    } else {
                        Ok("Memory item not found.".to_string())
                    }
                }
                None => Ok("Memory manager not initialized.".to_string()),
            }
        }

        "llm_task" => {
            crate::tools::llm_task::execute_llm_task(&tool_call).await
        }

        "tts" => {
            crate::tools::tts::execute_tts(&tool_call).await
        }

        "stt" => {
            crate::tools::stt::execute_stt(&tool_call).await
        }

        "cron" => match cron_scheduler {
            Some(scheduler) => crate::tools::cron::execute_cron_tool(scheduler, &tool_call).await,
            None => {
                Ok("Cron scheduler not initialized. Available in gateway/server mode.".to_string())
            }
        },

        "sessions_spawn" | "spawn_subagent" => match agent_manager {
            Some(manager) => {
                crate::tools::sessions::execute_sessions_tool(manager, &tool_call).await
            }
            None => {
                Ok("Agent manager not initialized. Available in gateway/server mode.".to_string())
            }
        },

        "get_subagent_result" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                if let Some(task) = manager.get_task_by_session(session_id).await {
                    let status_str = task_status_str(&task.status);
                    Ok(json!({
                        "task_id": task.id,
                        "session_id": task.session_id,
                        "status": status_str,
                        "result": task.result,
                        "task": task.task,
                    })
                    .to_string())
                } else {
                    Ok(json!({
                        "session_id": session_id,
                        "status": "not_found",
                        "error": format!("No task found for session ID: {}", session_id),
                    })
                    .to_string())
                }
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "list_subagents" => match agent_manager {
            Some(manager) => {
                let parent_session_id = tool_call["parent_session_id"]
                    .as_str()
                    .or_else(|| tool_call["parentSessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'parent_session_id' field"))?;

                let children = manager.get_children(parent_session_id).await;
                let mut subagents = Vec::new();
                for child_id in children {
                    if let Some(task) = manager.get_task_by_session(&child_id).await {
                        let status_str = task_status_str(&task.status);
                        subagents.push(json!({
                            "session_id": child_id,
                            "task_id": task.id,
                            "task": task.task,
                            "status": status_str,
                        }));
                    }
                }

                Ok(json!({
                    "parent_session_id": parent_session_id,
                    "count": subagents.len(),
                    "subagents": subagents,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "sessions_wait" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;
                let timeout_seconds = tool_call["timeout_seconds"]
                    .as_u64()
                    .or_else(|| tool_call["timeoutSeconds"].as_u64())
                    .unwrap_or(120);

                match manager
                    .wait_for_task(session_id, std::time::Duration::from_secs(timeout_seconds))
                    .await
                {
                    Ok(task) => {
                        let status_str = task_status_str(&task.status);
                        Ok(json!({
                            "session_id": task.session_id,
                            "task_id": task.id,
                            "status": status_str,
                            "result": task.result,
                        })
                        .to_string())
                    }
                    Err(e) => Ok(json!({
                        "session_id": session_id,
                        "status": "timeout",
                        "error": e.to_string(),
                    })
                    .to_string()),
                }
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "sessions_broadcast" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;
                let message = tool_call["message"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' field"))?;
                manager
                    .broadcast_to_parent(session_id, message.to_string())
                    .await?;
                Ok(json!({
                    "status": "sent",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "message" => match agent_manager {
            Some(manager) => {
                let message = tool_call["message"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' field"))?;
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .or(tenant_id)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                manager
                    .broadcast_to_session(session_id, message.to_string())
                    .await?;

                Ok(json!({
                    "status": "sent",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "gateway" => match agent_manager {
            Some(manager) => {
                let message = tool_call["message"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' field"))?;
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                manager
                    .broadcast_to_session(session_id, message.to_string())
                    .await?;

                Ok(json!({
                    "status": "sent",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "session_status" => match persistence {
            Some(store) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .or(tenant_id)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                let (count, last) = store.get_session_stats(session_id)?;
                Ok(json!({
                    "session_id": session_id,
                    "message_count": count,
                    "last_message_at": last,
                })
                .to_string())
            }
            None => Ok("Persistence manager not initialized.".to_string()),
        },

        "sessions_history" => match persistence {
            Some(store) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .or(tenant_id)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                let history = store.get_history(session_id)?;
                let rendered = history
                    .into_iter()
                    .map(message_to_simple)
                    .collect::<Vec<_>>();
                Ok(json!({
                    "session_id": session_id,
                    "messages": rendered,
                })
                .to_string())
            }
            None => Ok("Persistence manager not initialized.".to_string()),
        },

        "sessions_list" => match agent_manager {
            Some(manager) => {
                let sessions = manager.list_sessions().await;
                Ok(json!({ "sessions": sessions }).to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "sessions_send" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;
                let message = tool_call["message"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' field"))?;
                manager
                    .broadcast_to_session(session_id, message.to_string())
                    .await?;
                Ok(json!({
                    "status": "sent",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "sessions_cancel" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                manager.cancel_session(session_id).await?;

                Ok(json!({
                    "status": "cancelled",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "sessions_pause" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                manager.pause_session(session_id).await?;

                Ok(json!({
                    "status": "paused",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "sessions_resume" => match agent_manager {
            Some(manager) => {
                let session_id = tool_call["session_id"]
                    .as_str()
                    .or_else(|| tool_call["sessionId"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' field"))?;

                manager.resume_session(session_id).await?;

                Ok(json!({
                    "status": "resumed",
                    "session_id": session_id,
                })
                .to_string())
            }
            None => Ok("Agent manager not initialized. Available in gateway/server mode.".to_string()),
        },

        "agents_list" => {
            let mut agents = Vec::new();
            for path in ["./agents", "./.nanobot/agents"] {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.extension().and_then(|s| s.to_str()) == Some("toml") {
                            agents.push(p.display().to_string());
                        }
                    }
                }
            }
            Ok(json!({ "agents": agents }).to_string())
        }

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

fn message_to_simple(msg: rig::completion::Message) -> serde_json::Value {
    match msg {
        rig::completion::Message::User { content } => {
            let text = content
                .iter()
                .filter_map(|part| match part {
                    rig::completion::message::UserContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            json!({ "role": "user", "content": text })
        }
        rig::completion::Message::Assistant { content, .. } => {
            let text = content
                .iter()
                .filter_map(|part| match part {
                    rig::completion::message::AssistantContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            json!({ "role": "assistant", "content": text })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{PermissionManager, SecurityProfile};

    #[tokio::test]
    async fn test_tool_parsing() {
        let _json =
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
        let permission_manager = tokio::sync::Mutex::new(
            PermissionManager::new(SecurityProfile::trust()),
        );

        // Pass None for all optional context parameters
        #[cfg(feature = "browser")]
        let result = execute_tool(
            json,
            None,
            None,
            None,
            None,
            Some(&permission_manager),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        #[cfg(not(feature = "browser"))]
        let result = execute_tool(
            json,
            None,
            None,
            None,
            None,
            Some(&permission_manager),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("Status: ✅ Success"));
    }
}
