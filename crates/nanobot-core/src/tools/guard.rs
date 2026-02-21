use anyhow::Result;
use serde_json::Value;

/// ToolGuard provides pre-execution validation for tools
pub struct ToolGuard;

impl ToolGuard {
    pub fn guarded_tool_names() -> Vec<&'static str> {
        let names = vec![
            "run_command",
            "spawn_process",
            "write_file",
            "edit_file",
            "read_file",
            "list_directory",
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
            "web_search",
            "script_eval",
            "memory_search",
            "memory_save",
            "list_processes",
            "cron",
            "write_process_input",
            "read_process_output",
            "kill_process",
            "sessions_spawn",
            "spawn_subagent",
            "get_subagent_result",
            "list_subagents",
            "sessions_wait",
            "sessions_broadcast",
            "message",
            "gateway",
            "sessions_send",
            "sessions_cancel",
            "sessions_pause",
            "sessions_resume",
            "sessions_history",
            "session_status",
            "sessions_list",
            "agents_list",
            "memory_get",
            "llm_task",
            "tts",
            "stt",
        ];

        #[cfg(feature = "browser")]
        {
            let mut names = names;
            names.extend([
                "browser_navigate",
                "browser_click",
                "browser_type",
                "browser_screenshot",
                "browser_evaluate",
                "browser_pdf",
                "browser_list_tabs",
                "browser_switch_tab",
            ]);
            return names;
        }

        names
    }

    /// Validate tool arguments against expected schema
    pub fn validate_args(tool_name: &str, args: &Value) -> Result<()> {
        match tool_name {
            "run_command" | "spawn_process" => {
                Self::validate_command_args(args)?;
            }
            "write_file" | "edit_file" => {
                Self::validate_file_write_args(args)?;
            }
            "read_file" | "list_directory" => {
                Self::validate_file_read_args(args)?;
            }
            "web_fetch" => {
                Self::validate_string_arg(args, "url")?;
            }
            "glob" => {
                Self::validate_string_arg(args, "pattern")?;
            }
            "grep" => {
                Self::validate_string_arg(args, "pattern")?;
            }
            "question" => {
                Self::validate_string_arg(args, "question")?;
            }
            "apply_patch" => {
                let patch_text = args
                    .get("patch")
                    .and_then(|v| v.as_str())
                    .or_else(|| args.get("patch_text").and_then(|v| v.as_str()))
                    .or_else(|| args.get("patchText").and_then(|v| v.as_str()));

                if let Some(patch) = patch_text {
                    if patch.trim().is_empty() {
                        return Err(anyhow::anyhow!("patch text cannot be empty"));
                    }
                    if patch.len() > 2_000_000 {
                        return Err(anyhow::anyhow!("patch text too large (max 2MB)"));
                    }
                } else {
                    let ops = args
                        .get("operations")
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| {
                            anyhow::anyhow!("Missing 'operations' array or 'patch' text")
                        })?;
                    if ops.is_empty() {
                        return Err(anyhow::anyhow!("operations cannot be empty"));
                    }
                    for (idx, op) in ops.iter().enumerate() {
                        if let Some(v) = op.get("before_context") {
                            let s = v.as_str().ok_or_else(|| {
                                anyhow::anyhow!(
                                    "operations[{}].before_context must be a string",
                                    idx
                                )
                            })?;
                            if s.len() > 1000 {
                                return Err(anyhow::anyhow!(
                                    "operations[{}].before_context too large (max 1000 chars)",
                                    idx
                                ));
                            }
                        }
                        if let Some(v) = op.get("after_context") {
                            let s = v.as_str().ok_or_else(|| {
                                anyhow::anyhow!(
                                    "operations[{}].after_context must be a string",
                                    idx
                                )
                            })?;
                            if s.len() > 1000 {
                                return Err(anyhow::anyhow!(
                                    "operations[{}].after_context too large (max 1000 chars)",
                                    idx
                                ));
                            }
                        }
                    }
                }
                if let Some(v) = args.get("dry_run").or_else(|| args.get("dryRun"))
                    && !v.is_boolean()
                {
                    return Err(anyhow::anyhow!("dry_run must be a boolean"));
                }
                if let Some(v) = args.get("atomic")
                    && !v.is_boolean()
                {
                    return Err(anyhow::anyhow!("atomic must be a boolean"));
                }
            }
            "todowrite" => {
                let todos = args
                    .get("todos")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'todos' array"))?;
                if todos.is_empty() {
                    return Err(anyhow::anyhow!("todos cannot be empty"));
                }
                for (idx, todo) in todos.iter().enumerate() {
                    let obj = todo
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("todos[{}] must be an object", idx))?;
                    for key in ["id", "content", "status", "priority"] {
                        let val = obj.get(key).and_then(|v| v.as_str()).ok_or_else(|| {
                            anyhow::anyhow!("todos[{}].{} must be a string", idx, key)
                        })?;
                        if val.trim().is_empty() {
                            return Err(anyhow::anyhow!("todos[{}].{} cannot be empty", idx, key));
                        }
                    }

                    let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    let valid_status = ["pending", "in_progress", "completed", "cancelled"];
                    if !valid_status.iter().any(|s| s.eq_ignore_ascii_case(status)) {
                        return Err(anyhow::anyhow!(
                            "todos[{}].status invalid, expected one of: pending,in_progress,completed,cancelled",
                            idx
                        ));
                    }
                }
            }
            "parallel" => {
                let calls = args
                    .get("tool_calls")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'tool_calls' array"))?;
                if calls.is_empty() {
                    return Err(anyhow::anyhow!("tool_calls cannot be empty"));
                }
                if calls.len() > 16 {
                    return Err(anyhow::anyhow!("tool_calls max is 16"));
                }

                for (idx, call) in calls.iter().enumerate() {
                    let obj = call
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("tool_calls[{}] must be an object", idx))?;
                    let tool = obj
                        .get("tool")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow::anyhow!("tool_calls[{}].tool missing", idx))?;

                    let allowed = ["read_file", "list_directory", "glob", "grep", "web_fetch"];
                    if !allowed.iter().any(|t| t == &tool) {
                        return Err(anyhow::anyhow!(
                            "tool_calls[{}].tool '{}' not allowed in parallel mode",
                            idx,
                            tool
                        ));
                    }

                    if let Some(inner_args) = obj.get("args").and_then(|v| v.as_object())
                        && inner_args.contains_key("tool")
                    {
                        return Err(anyhow::anyhow!(
                            "tool_calls[{}].args cannot include reserved key 'tool'",
                            idx
                        ));
                    }
                }

                if let Some(v) = args.get("timeout_ms").or_else(|| args.get("timeoutMs")) {
                    let ms = v
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("timeout_ms must be a positive integer"))?;
                    if !(1_000..=120_000).contains(&ms) {
                        return Err(anyhow::anyhow!("timeout_ms out of range (1000..=120000)"));
                    }
                }
            }
            "task" => {
                Self::validate_string_arg(args, "prompt")?;
                if let Some(v) = args.get("description")
                    && !v.is_string()
                {
                    return Err(anyhow::anyhow!("description must be a string"));
                }
                if let Some(v) = args
                    .get("subagent_type")
                    .or_else(|| args.get("subagentType"))
                    && !v.is_string()
                {
                    return Err(anyhow::anyhow!("subagent_type must be a string"));
                }
                if let Some(v) = args
                    .get("timeout_seconds")
                    .or_else(|| args.get("timeoutSeconds"))
                {
                    let secs = v.as_u64().ok_or_else(|| {
                        anyhow::anyhow!("timeout_seconds must be a positive integer")
                    })?;
                    if !(1..=3600).contains(&secs) {
                        return Err(anyhow::anyhow!("timeout_seconds out of range (1..=3600)"));
                    }
                }
                if let Some(v) = args.get("max_retries").or_else(|| args.get("maxRetries")) {
                    let retries = v.as_u64().ok_or_else(|| {
                        anyhow::anyhow!("max_retries must be a non-negative integer")
                    })?;
                    if retries > 10 {
                        return Err(anyhow::anyhow!("max_retries out of range (0..=10)"));
                    }
                }
                if let Some(v) = args
                    .get("retry_backoff_ms")
                    .or_else(|| args.get("retryBackoffMs"))
                {
                    let ms = v.as_u64().ok_or_else(|| {
                        anyhow::anyhow!("retry_backoff_ms must be a non-negative integer")
                    })?;
                    if !(0..=60000).contains(&ms) {
                        return Err(anyhow::anyhow!("retry_backoff_ms out of range (0..=60000)"));
                    }
                }
            }
            "skill" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'action' field"))?;
                match action {
                    "create" => {
                        Self::validate_string_arg(args, "name")?;
                        if let Some(v) = args.get("backend") {
                            let backend = v
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("backend must be a string"))?;
                            let valid = ["native", "mcp", "deno"];
                            if !valid.iter().any(|s| s.eq_ignore_ascii_case(backend)) {
                                return Err(anyhow::anyhow!(
                                    "backend must be one of: native, mcp, deno"
                                ));
                            }
                        }
                        if let Some(v) = args.get("description")
                            && !v.is_string()
                        {
                            return Err(anyhow::anyhow!("description must be a string"));
                        }
                        if let Some(v) = args.get("auto_enable").or_else(|| args.get("autoEnable"))
                            && !v.is_boolean()
                        {
                            return Err(anyhow::anyhow!("auto_enable must be a boolean"));
                        }
                    }
                    "install" => {
                        if args.get("name").is_none() && args.get("skill").is_none() {
                            return Err(anyhow::anyhow!(
                                "Missing 'name' (or legacy 'skill') field"
                            ));
                        }
                        if let Some(v) = args.get("name")
                            && (!v.is_string() || v.as_str().unwrap_or("").trim().is_empty())
                        {
                            return Err(anyhow::anyhow!("name must be a non-empty string"));
                        }
                        if let Some(v) = args.get("skill")
                            && (!v.is_string() || v.as_str().unwrap_or("").trim().is_empty())
                        {
                            return Err(anyhow::anyhow!("skill must be a non-empty string"));
                        }
                        if let Some(v) = args.get("repo")
                            && !v.is_string()
                        {
                            return Err(anyhow::anyhow!("repo must be a string"));
                        }
                        if let Some(v) = args.get("auto_enable").or_else(|| args.get("autoEnable"))
                            && !v.is_boolean()
                        {
                            return Err(anyhow::anyhow!("auto_enable must be a boolean"));
                        }
                        if let Some(v) = args.get("bootstrap")
                            && !v.is_boolean()
                        {
                            return Err(anyhow::anyhow!("bootstrap must be a boolean"));
                        }
                        if let Some(v) = args.get("runtime") {
                            let runtime = v
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("runtime must be a string"))?;
                            let valid = ["deno", "node", "native", "mcp"];
                            if !valid.iter().any(|s| s.eq_ignore_ascii_case(runtime)) {
                                return Err(anyhow::anyhow!(
                                    "runtime must be one of: deno, node, native, mcp"
                                ));
                            }
                        }
                        if let Some(credentials) = args.get("credentials") {
                            let obj = credentials
                                .as_object()
                                .ok_or_else(|| anyhow::anyhow!("credentials must be an object"))?;
                            if !obj.values().all(|v| v.is_string()) {
                                return Err(anyhow::anyhow!("credentials values must be strings"));
                            }
                        }
                    }
                    "configure" => {
                        Self::validate_string_arg(args, "name")?;
                        if let Some(v) = args.get("enabled")
                            && !v.is_boolean()
                        {
                            return Err(anyhow::anyhow!("enabled must be a boolean"));
                        }
                        if let Some(v) = args.get("runtime") {
                            let runtime = v
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("runtime must be a string"))?;
                            let valid = ["deno", "node", "native", "mcp"];
                            if !valid.iter().any(|s| s.eq_ignore_ascii_case(runtime)) {
                                return Err(anyhow::anyhow!(
                                    "runtime must be one of: deno, node, native, mcp"
                                ));
                            }
                        }
                        if let Some(credentials) = args.get("credentials") {
                            let obj = credentials
                                .as_object()
                                .ok_or_else(|| anyhow::anyhow!("credentials must be an object"))?;
                            if !obj.values().all(|v| v.is_string()) {
                                return Err(anyhow::anyhow!("credentials values must be strings"));
                            }
                        }
                    }
                    "list" => {}
                    "show" | "enable" | "disable" => {
                        Self::validate_string_arg(args, "name")?;
                    }
                    "run" => {
                        Self::validate_string_arg(args, "name")?;
                        Self::validate_string_arg_any(args, &["tool_name", "toolName"])?;
                        if let Some(v) = args.get("arguments")
                            && !v.is_object()
                        {
                            return Err(anyhow::anyhow!("arguments must be an object"));
                        }
                    }
                    "set_runtime" => {
                        Self::validate_string_arg(args, "name")?;
                        Self::validate_string_arg(args, "runtime")?;
                        let runtime = args["runtime"].as_str().unwrap_or_default();
                        let valid = ["deno", "node", "native", "mcp"];
                        if !valid.iter().any(|s| s.eq_ignore_ascii_case(runtime)) {
                            return Err(anyhow::anyhow!(
                                "runtime must be one of: deno, node, native, mcp"
                            ));
                        }
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "action must be one of: create, install, configure, list, show, enable, disable, set_runtime, run"
                        ));
                    }
                }
            }
            "mcp_config" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'action' field"))?;

                match action {
                    "list" | "status" | "connect_all" => {}
                    "remove" => {
                        Self::validate_string_arg(args, "name")?;
                    }
                    "add" => {
                        Self::validate_string_arg(args, "name")?;
                        Self::validate_string_arg(args, "command")?;
                        let arr = args
                            .get("args")
                            .and_then(|v| v.as_array())
                            .ok_or_else(|| anyhow::anyhow!("Missing 'args' array"))?;
                        if arr.is_empty() {
                            return Err(anyhow::anyhow!("args cannot be empty"));
                        }
                        if !arr.iter().all(|v| v.is_string()) {
                            return Err(anyhow::anyhow!("All args must be strings"));
                        }
                        if let Some(env) = args.get("env") {
                            let obj = env
                                .as_object()
                                .ok_or_else(|| anyhow::anyhow!("env must be an object"))?;
                            if !obj.values().all(|v| v.is_string()) {
                                return Err(anyhow::anyhow!("env values must be strings"));
                            }
                        }
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "action must be one of: list, status, add, remove, connect_all"
                        ));
                    }
                }
            }
            "web_search" => {
                Self::validate_string_arg(args, "query")?;
            }
            "script_eval" => {
                Self::validate_string_arg(args, "script")?;
            }
            "list_processes" => {}
            "cron" => {
                Self::validate_string_arg(args, "action")?;
            }
            "memory_search" => {
                Self::validate_string_arg(args, "query")?;
            }
            "memory_save" => {
                Self::validate_string_arg(args, "content")?;
            }
            "write_process_input" => {
                Self::validate_pid_arg(args, "pid")?;
                Self::validate_string_arg(args, "input")?;
            }
            "read_process_output" | "kill_process" => {
                Self::validate_pid_arg(args, "pid")?;
            }
            "sessions_spawn" | "spawn_subagent" => {
                Self::validate_string_arg(args, "task")?;
                if let Some(v) = args
                    .get("timeout_seconds")
                    .or_else(|| args.get("timeoutSeconds"))
                    && !v.is_u64()
                {
                    return Err(anyhow::anyhow!(
                        "timeout_seconds must be a positive integer"
                    ));
                }
                if let Some(v) = args.get("max_retries").or_else(|| args.get("maxRetries"))
                    && !v.is_u64()
                {
                    return Err(anyhow::anyhow!(
                        "max_retries must be a non-negative integer"
                    ));
                }
                if let Some(v) = args
                    .get("retry_backoff_ms")
                    .or_else(|| args.get("retryBackoffMs"))
                    && !v.is_u64()
                {
                    return Err(anyhow::anyhow!(
                        "retry_backoff_ms must be a non-negative integer"
                    ));
                }
            }
            "get_subagent_result" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
            }
            "list_subagents" => {
                Self::validate_string_arg_any(args, &["parent_session_id", "parentSessionId"])?;
            }
            "sessions_wait" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
            }
            "sessions_broadcast" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
                Self::validate_string_arg(args, "message")?;
            }
            "message" => {
                Self::validate_string_arg(args, "message")?;
            }
            "gateway" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
                Self::validate_string_arg(args, "message")?;
            }
            "sessions_send" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
                Self::validate_string_arg(args, "message")?;
            }
            "sessions_cancel" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
            }
            "sessions_pause" | "sessions_resume" => {
                Self::validate_string_arg_any(args, &["session_id", "sessionId"])?;
            }
            "sessions_history" | "session_status" => {
                // Optional session_id; if provided must be string
                if let Some(val) = args.get("session_id").or_else(|| args.get("sessionId"))
                    && !val.is_string()
                {
                    return Err(anyhow::anyhow!("session_id must be a string"));
                }
            }
            "sessions_list" | "agents_list" => {}
            "memory_get" => {
                Self::validate_string_arg(args, "id")?;
            }
            "llm_task" => {
                Self::validate_string_arg(args, "prompt")?;
            }
            "tts" => {
                Self::validate_string_arg(args, "text")?;
                if let Some(val) = args.get("output_path")
                    && !val.is_string()
                {
                    return Err(anyhow::anyhow!("output_path must be a string"));
                }
            }
            "stt" => {
                Self::validate_string_arg(args, "audio_path")?;
            }
            _ => {
                // Unknown tools pass through (permissive by default)
            }
        }
        Ok(())
    }

    fn validate_string_arg(args: &Value, key: &str) -> Result<()> {
        let value = args
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("Missing '{}' argument", key))?;

        if !value.is_string() {
            return Err(anyhow::anyhow!("{} must be a string", key));
        }

        let value_str = value.as_str().unwrap_or("");
        if value_str.is_empty() {
            return Err(anyhow::anyhow!("{} cannot be empty", key));
        }

        Ok(())
    }

    fn validate_string_arg_any(args: &Value, keys: &[&str]) -> Result<()> {
        for key in keys {
            if let Some(value) = args.get(*key) {
                if !value.is_string() {
                    return Err(anyhow::anyhow!("{} must be a string", key));
                }

                let value_str = value.as_str().unwrap_or("");
                if value_str.is_empty() {
                    return Err(anyhow::anyhow!("{} cannot be empty", key));
                }

                return Ok(());
            }
        }

        Err(anyhow::anyhow!(
            "Missing one of required arguments: {}",
            keys.join(", ")
        ))
    }

    fn validate_pid_arg(args: &Value, key: &str) -> Result<()> {
        let value = args
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("Missing '{}' argument", key))?;

        if value.as_u64().is_some() {
            return Ok(());
        }

        if let Some(s) = value.as_str()
            && s.parse::<u32>().is_ok()
        {
            return Ok(());
        }

        Err(anyhow::anyhow!(
            "{} must be a number or numeric string",
            key
        ))
    }

    fn validate_command_args(args: &Value) -> Result<()> {
        let cmd = args
            .get("cmd")
            .or_else(|| args.get("command"))
            .ok_or_else(|| anyhow::anyhow!("Missing 'cmd' or 'command' argument"))?;

        if !cmd.is_string() {
            return Err(anyhow::anyhow!("Command must be a string"));
        }

        let raw_cmd = cmd.as_str().unwrap().trim();
        if raw_cmd.is_empty() {
            return Err(anyhow::anyhow!("Command cannot be empty"));
        }

        // Backward compatibility: allow `cmd` to be either a bare binary name
        // or a shell-style command string like "ls -la".
        let cmd_str = raw_cmd.split_whitespace().next().unwrap_or(raw_cmd).trim();

        if cmd_str.is_empty() {
            return Err(anyhow::anyhow!("Command cannot be empty"));
        }

        if !super::commands::command_allowed(cmd_str) {
            return Err(anyhow::anyhow!(
                "Command '{}' is not in the allowed whitelist.",
                cmd_str
            ));
        }

        let parsed_args = args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| {
                        v.as_str()
                            .map(|s| s.to_string())
                            .ok_or_else(|| anyhow::anyhow!("args must be an array of strings"))
                    })
                    .collect::<Result<Vec<String>>>()
            })
            .transpose()?
            .unwrap_or_default();

        if super::commands::dangerous_command_detected(cmd_str, &parsed_args)
            && std::env::var("NANOBOT_ALLOW_DANGEROUS_COMMANDS")
                .ok()
                .as_deref()
                != Some("1")
        {
            return Err(anyhow::anyhow!(
                "Blocked dangerous command. Set NANOBOT_ALLOW_DANGEROUS_COMMANDS=1 to override explicitly."
            ));
        }

        // Block known dangerous patterns by default
        let dangerous_patterns = ["rm -rf /", "format", "del /f /s /q"];
        for pattern in &dangerous_patterns {
            if cmd_str.contains(pattern) {
                return Err(anyhow::anyhow!(
                    "Blocked dangerous command pattern: {}",
                    pattern
                ));
            }
        }

        Ok(())
    }

    fn validate_file_write_args(args: &Value) -> Result<()> {
        let path = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

        if !path.is_string() {
            return Err(anyhow::anyhow!("Path must be a string"));
        }

        let path_str = path.as_str().unwrap();
        if path_str.is_empty() {
            return Err(anyhow::anyhow!("Path cannot be empty"));
        }

        // Validate against system-critical paths
        let critical_paths = ["/etc/", "/sys/", "/proc/", "C:\\Windows\\System32"];
        for critical in &critical_paths {
            if path_str.starts_with(critical) {
                return Err(anyhow::anyhow!(
                    "Cannot write to system-critical path: {}",
                    critical
                ));
            }
        }

        Ok(())
    }

    fn validate_file_read_args(args: &Value) -> Result<()> {
        let path = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' argument"))?;

        if !path.is_string() {
            return Err(anyhow::anyhow!("Path must be a string"));
        }

        let path_str = path.as_str().unwrap();
        if path_str.is_empty() {
            return Err(anyhow::anyhow!("Path cannot be empty"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_command_args_valid() {
        let args = json!({"cmd": "ls -la"});
        assert!(ToolGuard::validate_args("run_command", &args).is_ok());
    }

    #[test]
    fn test_validate_command_args_missing() {
        let args = json!({});
        assert!(ToolGuard::validate_args("run_command", &args).is_err());
    }

    #[test]
    fn test_validate_file_write_critical_path() {
        let args = json!({"path": "/etc/passwd"});
        let result = ToolGuard::validate_args("write_file", &args);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_file_write_valid() {
        let args = json!({"path": "/tmp/test.txt"});
        assert!(ToolGuard::validate_args("write_file", &args).is_ok());
    }
}
