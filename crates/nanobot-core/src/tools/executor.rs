// Simple tool calling implementation
// Since Rig's tool API isn't well documented, we'll use a prompt-based approach

use anyhow::Result;
use futures::future::join_all;
use serde_json::json;
use std::marker::PhantomData;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use super::filesystem::{
    ApplyPatchArgs, EditFileArgs, ListDirArgs, ReadFileArgs, WriteFileArgs, apply_patch,
    edit_file, list_directory, read_file, write_file,
};
use super::search::{GlobArgs, GrepArgs, glob_files, grep_files};
use super::todos::{TodoWriteArgs, todo_write};

#[derive(Debug, Clone)]
struct RequestContext {
    request_id: String,
    tenant_id: String,
    tool_name: String,
}

#[derive(Debug, Clone)]
struct AuthContext {
    request: RequestContext,
}

#[derive(Debug, Clone)]
struct CapContext<C> {
    auth: AuthContext,
    _cap: PhantomData<C>,
}

#[derive(Debug, Clone)]
struct ToolBasic;
#[derive(Debug, Clone)]
struct FileSystemAccess;
#[derive(Debug, Clone)]
struct NetworkAccess;
#[derive(Debug, Clone)]
struct ProcessExec;
#[derive(Debug, Clone)]
struct PersistenceAccess;

pub(in crate::tools) struct ExecutorMintKey(());

pub(in crate::tools) fn new_executor_token() -> super::ExecutorToken {
    super::ExecutorToken::new(ExecutorMintKey(()))
}

fn grant_tool_basic(ctx: RequestContext) -> CapContext<ToolBasic> {
    CapContext {
        auth: AuthContext { request: ctx },
        _cap: PhantomData,
    }
}

fn grant_filesystem_cap(ctx: &CapContext<ToolBasic>) -> CapContext<FileSystemAccess> {
    CapContext {
        auth: ctx.auth.clone(),
        _cap: PhantomData,
    }
}

fn grant_network_cap(ctx: &CapContext<ToolBasic>) -> CapContext<NetworkAccess> {
    CapContext {
        auth: ctx.auth.clone(),
        _cap: PhantomData,
    }
}

fn grant_process_cap(ctx: &CapContext<ToolBasic>) -> CapContext<ProcessExec> {
    CapContext {
        auth: ctx.auth.clone(),
        _cap: PhantomData,
    }
}

fn grant_persistence_cap(ctx: &CapContext<ToolBasic>) -> CapContext<PersistenceAccess> {
    CapContext {
        auth: ctx.auth.clone(),
        _cap: PhantomData,
    }
}

async fn run_edit_file_with_cap(
    _cap: &CapContext<FileSystemAccess>,
    args: EditFileArgs,
) -> Result<String> {
    let token = new_executor_token();
    edit_file(&token, args).await
}

async fn run_read_file_with_cap(
    _cap: &CapContext<FileSystemAccess>,
    args: ReadFileArgs,
) -> Result<String> {
    let token = new_executor_token();
    read_file(&token, args).await
}

async fn run_write_file_with_cap(
    _cap: &CapContext<FileSystemAccess>,
    args: WriteFileArgs,
) -> Result<String> {
    let token = new_executor_token();
    write_file(&token, args).await
}

async fn run_list_directory_with_cap(
    _cap: &CapContext<FileSystemAccess>,
    args: ListDirArgs,
) -> Result<String> {
    let token = new_executor_token();
    let files = list_directory(&token, args).await?;
    Ok(serde_json::to_string_pretty(&files)?)
}

async fn run_apply_patch_with_cap(
    _cap: &CapContext<FileSystemAccess>,
    args: ApplyPatchArgs,
) -> Result<String> {
    let token = new_executor_token();
    apply_patch(&token, args).await
}

async fn run_glob_with_cap(_cap: &CapContext<FileSystemAccess>, args: GlobArgs) -> Result<String> {
    let token = new_executor_token();
    glob_files(&token, args).await
}

async fn run_grep_with_cap(_cap: &CapContext<FileSystemAccess>, args: GrepArgs) -> Result<String> {
    let token = new_executor_token();
    grep_files(&token, args).await
}

async fn run_web_fetch_with_cap(
    _cap: &CapContext<NetworkAccess>,
    args: super::fetch::WebFetchArgs,
) -> Result<String> {
    let token = new_executor_token();
    super::fetch::web_fetch(&token, args).await
}

async fn run_spawn_process_with_cap(
    _cap: &CapContext<ProcessExec>,
    args: super::process::SpawnArgs,
) -> Result<String> {
    let token = new_executor_token();
    super::process::spawn_process(&token, args).await
}

async fn run_read_process_with_cap(
    _cap: &CapContext<ProcessExec>,
    args: super::process::PidArgs,
) -> Result<String> {
    let token = new_executor_token();
    super::process::read_process_output(&token, args).await
}

async fn run_kill_process_with_cap(
    _cap: &CapContext<ProcessExec>,
    args: super::process::PidArgs,
) -> Result<String> {
    let token = new_executor_token();
    super::process::terminate_process(&token, args).await
}

async fn run_list_processes_with_cap(_cap: &CapContext<ProcessExec>) -> Result<String> {
    let token = new_executor_token();
    super::process::list_processes(&token).await
}

async fn run_todowrite_with_cap(
    _cap: &CapContext<PersistenceAccess>,
    args: TodoWriteArgs,
    tenant_id: Option<&str>,
) -> Result<String> {
    let token = new_executor_token();
    todo_write(&token, args, tenant_id).await
}

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

async fn command_exists_quick(cmd: &str) -> bool {
    crate::blocking::command_exists(cmd, std::time::Duration::from_secs(2)).await
}

fn deno_policy_flags(policy: Option<&str>) -> Vec<&'static str> {
    match policy.map(|p| p.trim().to_ascii_lowercase()).as_deref() {
        Some("strict") => Vec::new(),
        Some("permissive") => vec![
            "--allow-read",
            "--allow-write",
            "--allow-env",
            "--allow-net",
        ],
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

fn deno_compat_flags() -> Vec<&'static str> {
    vec![
        "--compat",
        "--unstable-node-globals",
        "--unstable-bare-node-builtins",
    ]
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

fn stable_hash(input: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn approval_fingerprint(tool_name: &str, tool_call: &serde_json::Value) -> Option<String> {
    match tool_name {
        "run_command" | "spawn_process" | "bash" | "exec" => {
            let cmd = tool_call
                .get("command")
                .or_else(|| tool_call.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let args = tool_call
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join("\u{1f}")
                })
                .unwrap_or_default();

            Some(format!("cmd:{}:{}", cmd, stable_hash(&args)))
        }
        "apply_patch" => {
            let canonical = serde_json::to_string(tool_call).ok()?;
            Some(format!("apply_patch:{}", stable_hash(&canonical)))
        }
        "browser_navigate" => {
            let url = tool_call
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("about:blank");
            let host = url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
                .unwrap_or_else(|| url.to_string());
            Some(format!("browser_navigate:{}", host))
        }
        "browser_click" | "browser_type" | "browser_evaluate" | "browser_screenshot"
        | "browser_pdf" | "browser_list_tabs" | "browser_switch_tab" => {
            Some(format!("{}:session", tool_name))
        }
        "skill" => {
            let action = tool_call
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if action.eq_ignore_ascii_case("run") {
                let name = tool_call
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let tool = tool_call
                    .get("tool_name")
                    .or_else(|| tool_call.get("toolName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let args = tool_call
                    .get("arguments")
                    .and_then(|v| serde_json::to_string(v).ok())
                    .unwrap_or_default();
                Some(format!("skill:{}:{}:{}", name, tool, stable_hash(&args)))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn build_tool_candidates(tool_name: &str) -> Vec<String> {
    match tool_name {
        "run_command" | "bash" | "exec" | "spawn_process" => {
            vec!["run_command", "bash", "exec", "read_file", "grep"]
                .into_iter()
                .map(ToString::to_string)
                .collect()
        }
        "write_file" | "edit_file" | "apply_patch" => vec![
            "write_file",
            "edit_file",
            "apply_patch",
            "read_file",
            "grep",
        ]
        .into_iter()
        .map(ToString::to_string)
        .collect(),
        "browser_navigate" | "browser_click" | "browser_type" | "browser_evaluate"
        | "browser_pdf" | "browser_screenshot" => {
            vec!["browser_navigate", "web_fetch", "web_search", "read_file"]
                .into_iter()
                .map(ToString::to_string)
                .collect()
        }
        "skill" => vec!["skill", "read_file", "grep"]
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        _ => vec![tool_name.to_string()],
    }
}

fn summarize_top_candidates(candidates: &[crate::intelligent_router::ToolPlanCandidate]) -> String {
    let mut parts = Vec::new();
    for c in candidates.iter().take(3) {
        parts.push(format!("{}({:?}/{:.0})", c.tool, c.decision, c.score));
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(", ")
    }
}

fn is_auto_fallback_compatible(
    requested_tool: &str,
    fallback_tool: &str,
    tool_call: &serde_json::Value,
) -> bool {
    if requested_tool == fallback_tool {
        return false;
    }

    let command_family = matches!(
        requested_tool,
        "run_command" | "bash" | "exec" | "spawn_process"
    ) && matches!(
        fallback_tool,
        "run_command" | "bash" | "exec" | "spawn_process"
    );

    if command_family {
        return tool_call
            .get("command")
            .or_else(|| tool_call.get("cmd"))
            .and_then(|v| v.as_str())
            .is_some();
    }

    if requested_tool == "browser_navigate" && fallback_tool == "web_fetch" {
        return tool_call
            .get("url")
            .and_then(|v| v.as_str())
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);
    }

    false
}

fn rollback_action_for_step(step: &crate::intelligent_router::TaskStep) -> Option<String> {
    match step.suggested_tool.as_str() {
        "write_file" | "edit_file" | "apply_patch" => step
            .inferred_args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| format!("restore backup for '{}'", p))
            .or_else(|| Some("restore previous file contents from backup".to_string())),
        "run_command" => Some("run diagnostic command and collect logs before retry".to_string()),
        "web_fetch" | "web_search" => Some("retry with alternate URL/source".to_string()),
        _ => None,
    }
}

fn collect_step_artifacts(
    step: &crate::intelligent_router::TaskStep,
    output: &str,
) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for expected in &step.expected_outputs {
        out.insert(expected.clone());
    }

    if let Some(path) = step.inferred_args.get("path").and_then(|v| v.as_str()) {
        let p = path.trim();
        if !p.is_empty() {
            out.insert(format!("file:{}", p));
        }
    }
    if let Some(url) = step.inferred_args.get("url").and_then(|v| v.as_str()) {
        let u = url.trim();
        if !u.is_empty() {
            out.insert(format!("url:{}", u));
        }
    }
    if let Some(cmd) = step.inferred_args.get("command").and_then(|v| v.as_str()) {
        let c = cmd.trim();
        if !c.is_empty() {
            out.insert(format!("cmd:{}", c));
        }
    }

    // Basic dynamic signal from output for downstream gating.
    let lower = output.to_ascii_lowercase();
    if lower.contains("ok") || lower.contains("success") {
        out.insert(format!("step:{}:ok", step.id));
    }

    out
}

fn output_assertion_failure(
    step: &crate::intelligent_router::TaskStep,
    output: &str,
) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    let mut parsed_json: Option<serde_json::Value> = None;
    for assertion in &step.expected_assertions {
        if let Some(needle) = assertion.strip_prefix("contains:") {
            let n = needle.trim().to_ascii_lowercase();
            if !n.is_empty() && !lower.contains(&n) {
                return Some(format!("missing expected token '{}'", needle));
            }
            continue;
        }
        if let Some(needle) = assertion.strip_prefix("not_contains:") {
            let n = needle.trim().to_ascii_lowercase();
            if !n.is_empty() && lower.contains(&n) {
                return Some(format!("unexpected token '{}' present", needle));
            }
            continue;
        }
        if let Some(raw) = assertion.strip_prefix("min_len:") {
            if let Ok(min_len) = raw.trim().parse::<usize>()
                && output.chars().count() < min_len
            {
                return Some(format!("output shorter than required min_len {}", min_len));
            }
            continue;
        }
        if let Some(pattern) = assertion.strip_prefix("regex:") {
            match regex::Regex::new(pattern.trim()) {
                Ok(re) => {
                    if !re.is_match(output) {
                        return Some(format!("regex '{}' did not match output", pattern.trim()));
                    }
                }
                Err(_) => {
                    return Some(format!("invalid regex assertion '{}'", pattern.trim()));
                }
            }
            continue;
        }
        if assertion == "json_valid" {
            if parsed_json.is_none() {
                parsed_json = serde_json::from_str::<serde_json::Value>(output).ok();
            }
            if parsed_json.is_none() {
                return Some("output is not valid JSON".to_string());
            }
            continue;
        }
        if let Some(key) = assertion.strip_prefix("json_key:") {
            if parsed_json.is_none() {
                parsed_json = serde_json::from_str::<serde_json::Value>(output).ok();
            }
            let Some(val) = parsed_json.as_ref() else {
                return Some("output is not valid JSON".to_string());
            };
            let found = if key.contains('.') {
                let mut cur = val;
                let mut ok = true;
                for part in key.split('.') {
                    if let Some(next) = cur.get(part) {
                        cur = next;
                    } else {
                        ok = false;
                        break;
                    }
                }
                ok
            } else {
                val.get(key).is_some()
            };
            if !found {
                return Some(format!("json key '{}' missing", key));
            }
            continue;
        }
    }
    None
}

fn retry_reason_for_output(
    tool_name: &str,
    tool_call: &serde_json::Value,
    output: &str,
) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Some("empty output".to_string());
    }

    match tool_name {
        "read_file" => {
            if trimmed.starts_with("Error") || trimmed.contains("File not found") {
                Some("read_file returned error payload".to_string())
            } else {
                None
            }
        }
        "web_fetch" => {
            if trimmed.len() < 20 {
                Some("web_fetch output too short".to_string())
            } else {
                None
            }
        }
        "browser_navigate" => {
            if let Some(url) = tool_call.get("url").and_then(|v| v.as_str())
                && !trimmed
                    .to_ascii_lowercase()
                    .contains(&url.to_ascii_lowercase())
            {
                return Some("navigate output missing requested URL".to_string());
            }
            None
        }
        _ => None,
    }
}

fn is_retryable_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file" | "list_directory" | "glob" | "grep" | "web_fetch" | "web_search"
    )
}

fn skill_runtime_env_from_config(
    config: &crate::config::Config,
) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();

    if let Some(openai) = config.providers.openai.as_ref()
        && let Some(api_key) = openai.api_key.as_ref().filter(|k| !k.trim().is_empty())
    {
        env.insert("OPENAI_API_KEY".to_string(), api_key.clone());
    }

    if let Some(openrouter) = config.providers.openrouter.as_ref()
        && let Some(api_key) = openrouter.api_key.as_ref().filter(|k| !k.trim().is_empty())
    {
        env.insert("OPENROUTER_API_KEY".to_string(), api_key.clone());
    }

    if let Some(antigravity) = config.providers.antigravity.as_ref() {
        if let Some(api_key) = antigravity
            .api_key
            .as_ref()
            .filter(|k| !k.trim().is_empty())
        {
            env.insert("ANTIGRAVITY_API_KEY".to_string(), api_key.clone());
        }
        if let Some(base_url) = antigravity
            .base_url
            .as_ref()
            .filter(|u| !u.trim().is_empty())
        {
            env.insert("ANTIGRAVITY_BASE_URL".to_string(), base_url.clone());
        }
    }

    if let Some(google) = config.providers.google.as_ref()
        && let Some(api_key) = google.api_key.as_ref().filter(|k| !k.trim().is_empty())
    {
        env.insert("GOOGLE_API_KEY".to_string(), api_key.clone());
    }

    if let Ok(vault_path) = std::env::var("OBSIDIAN_VAULT_PATH")
        && !vault_path.trim().is_empty()
    {
        env.insert("OBSIDIAN_VAULT_PATH".to_string(), vault_path);
    }

    env
}

fn openclaw_compat_paths() -> Option<(String, String)> {
    let home = dirs::home_dir()?;
    let openclaw_home = home.join(".openclaw");
    let openclaw_auth = openclaw_home.join("auth");
    Some((
        openclaw_home.to_string_lossy().to_string(),
        openclaw_auth.to_string_lossy().to_string(),
    ))
}

async fn node_command_path() -> Option<String> {
    if command_exists_quick("node").await {
        return Some("node".to_string());
    }

    if cfg!(windows) {
        let fallback = "C:\\Program Files\\nodejs\\node.exe";
        if tokio::fs::metadata(fallback).await.is_ok() {
            return Some(fallback.to_string());
        }
    }

    None
}

async fn node_supports_permission_model(node_cmd: &str) -> bool {
    let output = crate::blocking::process_output(
        node_cmd.to_string(),
        vec!["--help".to_string()],
        std::time::Duration::from_secs(3),
    )
    .await;

    let Ok(output) = output else {
        return false;
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let help = format!("{}\n{}", stdout, stderr);

    help.contains("--permission")
}

fn node_allow_env_list() -> String {
    [
        "NANOBOT_SKILL",
        "NANOBOT_TOOL",
        "NANOBOT_TOOL_ARGS",
        "OPENCLAW_HOME",
        "OPENCLAW_AUTH_DIR",
        "PATH",
        "HOME",
        "USERPROFILE",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATHEXT",
        "ComSpec",
        "LANG",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "GOOGLE_API_KEY",
        "ANTIGRAVITY_API_KEY",
        "ANTIGRAVITY_BASE_URL",
        "OBSIDIAN_VAULT_PATH",
    ]
    .join(",")
}

fn node_fallback_allows_network() -> bool {
    std::env::var("NANOBOT_NODE_FALLBACK_ALLOW_NET")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn node_unsandboxed_fallback_allowed() -> bool {
    std::env::var("NANOBOT_ALLOW_UNSANDBOXED_NODE_FALLBACK")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn is_mandatory_approval_tool(tool_name: &str, tool_call: &serde_json::Value) -> bool {
    match tool_name {
        "run_command" | "spawn_process" | "bash" | "exec" | "apply_patch" => true,
        "browser_navigate" | "browser_click" | "browser_type" | "browser_screenshot"
        | "browser_evaluate" | "browser_pdf" | "browser_list_tabs" | "browser_switch_tab" => true,
        "skill" => tool_call
            .get("action")
            .and_then(|v| v.as_str())
            .map(|a| {
                a.eq_ignore_ascii_case("run")
                    || a.eq_ignore_ascii_case("install")
                    || a.eq_ignore_ascii_case("configure")
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn is_deno_compatibility_error(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("cannot find module")
        || s.contains("node-gyp")
        || s.contains("not yet implemented")
        || s.contains("unsupported")
        || s.contains("ffi")
        || s.contains("napi")
        || s.contains("native module")
}

fn apply_skill_env_allowlist(cmd: &mut tokio::process::Command) {
    cmd.env_clear();
    for key in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATHEXT",
        "ComSpec",
        "LANG",
    ] {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct GitHubSkillTreeEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    download_url: Option<String>,
    url: String,
}

async fn download_clawhub_skill_tree(
    client: &reqwest::Client,
    repo: &str,
    skill_name: &str,
    destination: &std::path::Path,
) -> Result<()> {
    let root = format!("skills/{}", skill_name);
    let mut stack = vec![format!(
        "https://api.github.com/repos/{}/contents/{}",
        repo, root
    )];

    while let Some(api_url) = stack.pop() {
        let resp = client
            .get(&api_url)
            .header("User-Agent", "nanobot-skill-install")
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Failed to fetch skill tree from {} (status {})",
                api_url,
                resp.status()
            );
        }

        let entries: Vec<GitHubSkillTreeEntry> = resp.json().await?;
        for entry in entries {
            match entry.entry_type.as_str() {
                "dir" => stack.push(entry.url),
                "file" => {
                    let download_url = entry.download_url.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("Missing download_url for {}", entry.path)
                    })?;

                    let prefix = format!("skills/{}/", skill_name);
                    let relative = entry.path.strip_prefix(&prefix).unwrap_or(&entry.name);
                    let output = destination.join(relative);
                    if let Some(parent) = output.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }

                    let bytes = client
                        .get(download_url)
                        .header("User-Agent", "nanobot-skill-install")
                        .send()
                        .await?
                        .bytes()
                        .await?;
                    tokio::fs::write(output, &bytes).await?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

async fn bootstrap_skill_dependencies_if_present(skill_dir: &std::path::Path) -> Vec<String> {
    let mut notes = Vec::new();

    if tokio::fs::metadata(skill_dir.join("package.json")).await.is_ok() {
        if !command_exists_quick("npm").await {
            notes.push("package.json found but npm is not installed".to_string());
        } else {
            let status = crate::blocking::process_output_in_dir(
                "npm".to_string(),
                vec!["install".to_string(), "--omit=dev".to_string()],
                std::time::Duration::from_secs(120),
                Some(skill_dir.to_path_buf()),
            )
            .await;
            match status {
                Ok(s) if s.status.success() => {
                    notes.push("npm dependencies installed".to_string())
                }
                Ok(s) => notes.push(format!(
                    "npm install exited with status {}",
                    s.status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".to_string())
                )),
                Err(e) => notes.push(format!("failed to run npm install: {}", e)),
            }
        }
    }

    notes
}

async fn run_node_fallback_skill(
    name: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    skill: &crate::skills::metadata::SkillMetadata,
    runtime_config: Option<&crate::config::Config>,
    reason: &str,
) -> Result<String> {
    let node_cmd = node_command_path()
        .await
        .ok_or_else(|| skill_run_error("SKILL_NODE_NOT_FOUND", "Node.js not found"))?;
    let script = skill
        .deno_script
        .clone()
        .or_else(|| skill.native_args.first().cloned())
        .ok_or_else(|| {
            skill_run_error(
                "SKILL_NODE_FALLBACK_MISSING_SCRIPT",
                "No script available for node fallback",
            )
        })?;

    let mut cmd = tokio::process::Command::new(&node_cmd);
    apply_skill_env_allowlist(&mut cmd);

    let permission_model_supported = node_supports_permission_model(&node_cmd).await;
    if permission_model_supported {
        let mut fs_scopes = vec![".".to_string()];
        if let Some(parent) = std::path::Path::new(&script).parent() {
            let scope = parent.to_string_lossy().to_string();
            if !scope.trim().is_empty() && !fs_scopes.iter().any(|s| s == &scope) {
                fs_scopes.push(scope);
            }
        }

        cmd.arg("--permission");
        cmd.arg(format!("--allow-fs-read={}", fs_scopes.join(",")));
        cmd.arg(format!("--allow-fs-write={}", fs_scopes.join(",")));
        cmd.arg(format!("--allow-env={}", node_allow_env_list()));
        if node_fallback_allows_network() {
            cmd.arg("--allow-net");
        }
    } else if !node_unsandboxed_fallback_allowed() {
        return Err(skill_run_error(
            "SKILL_NODE_FALLBACK_PERMISSION_BLOCKED",
            "Node fallback blocked: this Node runtime does not support --permission. Upgrade Node or set NANOBOT_ALLOW_UNSANDBOXED_NODE_FALLBACK=1 (unsafe override).",
        ));
    }

    if script.ends_with(".ts") || script.ends_with(".mts") || script.ends_with(".cts") {
        cmd.arg("--experimental-strip-types");
    }

    cmd.arg(&script);
    cmd.arg(tool_name);
    cmd.arg(arguments.to_string());
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.env("NANOBOT_SKILL", name);
    cmd.env("NANOBOT_TOOL", tool_name);
    cmd.env("NANOBOT_TOOL_ARGS", arguments.to_string());
    if let Some((openclaw_home, openclaw_auth)) = openclaw_compat_paths() {
        cmd.env("OPENCLAW_HOME", openclaw_home);
        cmd.env("OPENCLAW_AUTH_DIR", openclaw_auth);
    }
    if let Some(cfg) = runtime_config {
        for (k, v) in skill_runtime_env_from_config(cfg) {
            cmd.env(k, v);
        }
    }
    for (k, v) in &skill.deno_env {
        cmd.env(k, v);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| {
            skill_run_error(
                "SKILL_NODE_FALLBACK_TIMEOUT",
                "Node fallback timed out after 60s",
            )
        })??;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        return Err(skill_run_error(
            "SKILL_NODE_FALLBACK_FAILED",
            format!(
                "Node fallback failed ({}): {}",
                node_cmd,
                if stderr.is_empty() {
                    "no stderr output"
                } else {
                    &stderr
                }
            ),
        ));
    }

    let parsed: Option<serde_json::Value> = serde_json::from_str(&stdout).ok();
    let reason_code = match reason {
        "deno-missing" => "SKILL_FALLBACK_DENO_MISSING",
        "deno-compatibility-fallback" => "SKILL_FALLBACK_DENO_COMPATIBILITY",
        "runtime-override" => "SKILL_FALLBACK_RUNTIME_OVERRIDE",
        _ => "SKILL_FALLBACK_OTHER",
    };
    Ok(json!({
        "status": "ok",
        "backend": "node-fallback",
        "reason": reason,
        "reason_code": reason_code,
        "permission_model": if permission_model_supported {
            "enabled"
        } else {
            "disabled-unsafe-override"
        },
        "network_allowed": node_fallback_allows_network(),
        "skill": name,
        "tool": tool_name,
        "command": node_cmd,
        "script": script,
        "output": stdout,
        "json": parsed,
    })
    .to_string())
}

fn skill_run_error(code: &str, message: impl Into<String>) -> anyhow::Error {
    let message = message.into();
    anyhow::anyhow!(
        "{}",
        json!({
            "status": "error",
            "code": code,
            "message": message,
        })
    )
}

fn tool_runtime_error_payload(code: &str, message: impl Into<String>) -> String {
    let message = message.into();
    json!({
        "status": "error",
        "code": code,
        "message": message,
    })
    .to_string()
}

fn tool_runtime_error(code: &str, message: impl Into<String>) -> anyhow::Error {
    anyhow::anyhow!("{}", tool_runtime_error_payload(code, message))
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

15. **apply_patch** - Apply patch operations (structured or unified diff)
    Usage (structured): { "tool": "apply_patch", "operations": [{ "op": "update", "path": "src/main.rs", "old_text": "foo", "new_text": "bar", "before_context": "anchor before", "after_context": "anchor after" }], "atomic": true, "dry_run": false }
    Usage (unified diff): { "tool": "apply_patch", "patch": "--- a/file\n+++ b/file\n@@ ..." }

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

41. **skill** - Manage loaded skills (install/create/list/show/enable/disable/set_runtime/run)
    Usage: { "tool": "skill", "action": "create", "name": "my_skill", "backend": "native", "description": "optional", "auto_enable": true }
    Usage: { "tool": "skill", "action": "install", "name": "github", "repo": "openclaw/openclaw", "auto_enable": true, "bootstrap": true, "runtime": "deno", "credentials": { "api_key": "..." } }
    Usage: { "tool": "skill", "action": "list" }
    Usage: { "tool": "skill", "action": "show", "name": "github" }
    Usage: { "tool": "skill", "action": "enable", "name": "github" }
    Usage: { "tool": "skill", "action": "disable", "name": "github" }
    Usage: { "tool": "skill", "action": "set_runtime", "name": "gog", "runtime": "node" }
    Usage: { "tool": "skill", "action": "configure", "name": "weather", "enabled": true, "runtime": "deno", "credentials": { "api_key": "..." } }
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
        s.push_str(
            r##"
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
"##,
        );
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
        "web_search",
        "web_fetch",
        "spawn_process",
        "read_process_output",
        "write_process_input",
        "kill_process",
        "list_processes",
        "glob",
        "grep",
        "question",
        "apply_patch",
        "script_eval",
        "todowrite",
        "parallel",
        "task",
        "skill",
        "mcp_config",
        "memory_search",
        "memory_save",
        "memory_get",
        "llm_task",
        "tts",
        "stt",
        "cron",
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
    ];

    #[cfg(feature = "browser")]
    {
        let mut tools = tools;
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
        return tools;
    }

    tools
}

/// Execute a skill by name with given tool and arguments
/// This is the internal implementation used by both direct skill calls and skill tool action
async fn execute_skill_by_name(
    skill_name: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    skill_loader: &std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>,
    mcp_manager: Option<&std::sync::Arc<crate::mcp::McpManager>>,
) -> Result<String> {
    let mut loader = skill_loader.lock().await;
    loader.scan()?;
    let skill = loader.get_skill(skill_name).cloned().ok_or_else(|| {
        skill_run_error(
            "SKILL_NOT_FOUND",
            format!("Skill not found: {}", skill_name),
        )
    })?;
    drop(loader);

    let skills_cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    if !skills_cfg.is_enabled(skill_name) {
        return Err(skill_run_error(
            "SKILL_DISABLED",
            format!(
                "Skill '{}' is disabled. Enable it first with {{ \"tool\": \"skill\", \"action\": \"enable\", \"name\": \"{}\" }}",
                skill_name, skill_name
            ),
        ));
    }

    let runtime_override = skills_cfg
        .runtime_override(skill_name)
        .map(|s| s.to_string());
    let effective_backend = runtime_override.unwrap_or_else(|| skill.backend.to_lowercase());

    match effective_backend.as_str() {
        "mcp" => {
            let manager = mcp_manager.ok_or_else(|| {
                skill_run_error(
                    "SKILL_MCP_MANAGER_UNAVAILABLE",
                    "MCP manager not initialized",
                )
            })?;

            let server_name = skill
                .mcp_server_name
                .clone()
                .unwrap_or_else(|| format!("skill-{}", skill_name));

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

            let result = match manager
                .call_tool(&server_name, tool_name, arguments.clone())
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "{}",
                        json!({
                            "status": "error",
                            "code": "SKILL_MCP_TOOL_CALL_FAILED",
                            "message": format!(
                                "Failed to call MCP tool '{}' on server '{}': {}. Configure with mcp_config add/connect_all or provide mcp_command in SKILL.md",
                                tool_name,
                                server_name,
                                e
                            ),
                        })
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
                "skill": skill_name,
                "server": server_name,
                "tool": tool_name,
                "content": content_text,
                "raw": result,
            })
            .to_string())
        }
        "deno" => {
            let script = skill.deno_script.clone().ok_or_else(|| {
                skill_run_error(
                    "SKILL_DENO_SCRIPT_MISSING",
                    format!(
                        "Deno skill '{}' missing deno_script in SKILL.md",
                        skill_name
                    ),
                )
            })?;
            let deno_command = skill
                .deno_command
                .clone()
                .unwrap_or_else(|| "deno".to_string());

            if !command_exists_quick(&deno_command).await {
                if node_command_path().await.is_some() {
                    tracing::warn!(
                        "Deno command '{}' missing for skill '{}'; using node fallback",
                        deno_command,
                        skill_name
                    );
                    return run_node_fallback_skill(
                        skill_name,
                        tool_name,
                        arguments,
                        &skill,
                        None,
                        "deno-missing",
                    )
                    .await;
                }

                return Err(skill_run_error(
                    "SKILL_DENO_COMMAND_NOT_FOUND",
                    format!(
                        "Deno command '{}' not found. Install Deno or set deno_command in SKILL.md",
                        deno_command
                    ),
                ));
            }

            let mut cmd = tokio::process::Command::new(&deno_command);
            apply_skill_env_allowlist(&mut cmd);
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
            push_unique_args(
                &mut deno_args,
                deno_compat_flags().into_iter().map(|s| s.to_string()),
            );
            push_unique_args(&mut deno_args, skill.deno_permissions.clone());

            cmd.args(&deno_args);
            cmd.arg(&script);
            cmd.arg(tool_name);
            cmd.arg(arguments.to_string());
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            cmd.env("NANOBOT_SKILL", skill_name);
            cmd.env("NANOBOT_TOOL", tool_name);
            cmd.env("NANOBOT_TOOL_ARGS", arguments.to_string());
            if let Some((openclaw_home, openclaw_auth)) = openclaw_compat_paths() {
                cmd.env("OPENCLAW_HOME", openclaw_home);
                cmd.env("OPENCLAW_AUTH_DIR", openclaw_auth);
            }
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
                    return Err(skill_run_error(
                        "SKILL_DENO_START_FAILED",
                        format!("Failed to start deno command '{}': {}", deno_command, e),
                    ));
                }
                Err(_) => {
                    return Err(skill_run_error(
                        "SKILL_DENO_TIMEOUT",
                        format!("Deno skill '{}' timed out after 60s", skill_name),
                    ));
                }
            };

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if !output.status.success() {
                if is_deno_compatibility_error(&stderr) && node_command_path().await.is_some() {
                    tracing::warn!(
                        "Skill '{}' failed in deno with compatibility error; falling back to node",
                        skill_name
                    );
                    return run_node_fallback_skill(
                        skill_name,
                        tool_name,
                        arguments,
                        &skill,
                        None,
                        "deno-compatibility-fallback",
                    )
                    .await;
                }

                return Err(skill_run_error(
                    "SKILL_DENO_FAILED",
                    format!(
                        "Deno skill '{}' failed (command '{}'): {}",
                        skill_name,
                        deno_command,
                        if stderr.is_empty() {
                            "no stderr output"
                        } else {
                            &stderr
                        }
                    ),
                ));
            }

            let parsed: Option<serde_json::Value> = serde_json::from_str(&stdout).ok();
            Ok(json!({
                "status": "ok",
                "backend": "deno",
                "skill": skill_name,
                "tool": tool_name,
                "sandbox": skill.deno_sandbox.clone().unwrap_or_else(|| "balanced".to_string()),
                "applied_permissions": deno_args,
                "script": script,
                "output": stdout,
                "json": parsed,
            })
            .to_string())
        }
        "node" => {
            run_node_fallback_skill(
                skill_name,
                tool_name,
                arguments,
                &skill,
                None,
                "runtime-override",
            )
            .await
        }
        "native" => {
            let native_command = skill.native_command.clone().ok_or_else(|| {
                skill_run_error(
                    "SKILL_NATIVE_COMMAND_MISSING",
                    format!(
                        "Native skill '{}' missing native_command in SKILL.md",
                        skill_name
                    ),
                )
            })?;

            let mut cmd = tokio::process::Command::new(&native_command);
            apply_skill_env_allowlist(&mut cmd);
            if !skill.native_args.is_empty() {
                cmd.args(&skill.native_args);
            }
            cmd.arg(tool_name);
            cmd.arg(arguments.to_string());
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            cmd.env("NANOBOT_SKILL", skill_name);
            cmd.env("NANOBOT_TOOL", tool_name);
            cmd.env("NANOBOT_TOOL_ARGS", arguments.to_string());
            if let Some((openclaw_home, openclaw_auth)) = openclaw_compat_paths() {
                cmd.env("OPENCLAW_HOME", openclaw_home);
                cmd.env("OPENCLAW_AUTH_DIR", openclaw_auth);
            }
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
                    return Err(skill_run_error(
                        "SKILL_NATIVE_START_FAILED",
                        format!("Failed to start native command '{}': {}", native_command, e),
                    ));
                }
                Err(_) => {
                    return Err(skill_run_error(
                        "SKILL_NATIVE_TIMEOUT",
                        format!("Native skill '{}' timed out after 60s", skill_name),
                    ));
                }
            };

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if !output.status.success() {
                return Err(skill_run_error(
                    "SKILL_NATIVE_FAILED",
                    format!(
                        "Native skill '{}' failed (command '{}'): {}",
                        skill_name,
                        native_command,
                        if stderr.is_empty() {
                            "no stderr output"
                        } else {
                            &stderr
                        }
                    ),
                ));
            }

            let parsed: Option<serde_json::Value> = serde_json::from_str(&stdout).ok();
            Ok(json!({
                "status": "ok",
                "backend": "native",
                "skill": skill_name,
                "tool": tool_name,
                "command": native_command,
                "output": stdout,
                "json": parsed,
            })
            .to_string())
        }
        _ => Err(skill_run_error(
            "SKILL_BACKEND_UNSUPPORTED",
            format!("Unsupported backend '{}' in skill.run", effective_backend),
        )),
    }
}

#[derive(Clone, Copy)]
pub struct ExecuteToolContext<'a> {
    pub cron_scheduler: Option<&'a crate::cron::CronScheduler>,
    pub agent_manager: Option<&'a crate::gateway::agent_manager::AgentManager>,
    pub memory_manager: Option<&'a std::sync::Arc<crate::memory::MemoryManager>>,
    pub persistence: Option<&'a crate::persistence::PersistenceManager>,
    pub permission_manager: Option<&'a tokio::sync::Mutex<super::PermissionManager>>,
    pub tool_policy: Option<&'a super::policy::ToolPolicy>,
    pub confirmation_service:
        Option<&'a tokio::sync::Mutex<super::confirmation::ConfirmationService>>,
    pub skill_loader: Option<&'a std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>>,
    #[cfg(feature = "browser")]
    pub browser_client: Option<&'a crate::browser::BrowserClient>,
    pub tenant_id: Option<&'a str>,
    pub mcp_manager: Option<&'a std::sync::Arc<crate::mcp::McpManager>>,
}

/// Execute a tool based on JSON input
#[tracing::instrument(skip_all, fields(tool_name))]
pub async fn execute_tool(tool_input: &str, ctx: ExecuteToolContext<'_>) -> Result<String> {
    let ExecuteToolContext {
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
    } = ctx;

    // Strip prefix if present (optional support)
    let json_str = tool_input.trim().trim_start_matches("__TOOL_CALL__").trim();

    let mut tool_call: serde_json::Value = serde_json::from_str(json_str)?;

    let runtime_config = crate::config::Config::load().ok();
    let interaction_policy = runtime_config
        .as_ref()
        .map(|c| c.interaction_policy)
        .unwrap_or_default();
    let audit_logger = runtime_config
        .as_ref()
        .and_then(|c| c.audit_log_path.as_ref())
        .map(|p| crate::system::audit::AuditLogger::new(std::path::PathBuf::from(p)));

    let requested_tool_name = tool_call["tool"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'tool' field"))?
        .to_string();
    let tool_name = requested_tool_name.as_str();
    let mut selected_tool_name = requested_tool_name.clone();

    tracing::Span::current().record("tool_name", tool_name);

    // Phase 1 Integration: ToolGuard validation (schema + safety checks)
    if let Err(e) = super::guard::ToolGuard::validate_args(tool_name, &tool_call) {
        tracing::warn!("ToolGuard validation failed for {}: {}", tool_name, e);
        return Err(anyhow::anyhow!("Tool validation failed: {}", e));
    }

    // Phase 3: Security Integration with Intelligent Systems

    // Initialize policy approval flag
    let mut policy_force_approval = false;
    let policy_user = tenant_id.unwrap_or("default");

    // Policy-aware candidate planning (requested tool + safer alternatives)
    let tool_candidates = build_tool_candidates(tool_name);
    let candidate_refs = tool_candidates
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>();
    let ranked_candidates = crate::intelligent_router::INTELLIGENT_ROUTER
        .rank_tool_candidates(policy_user, &candidate_refs)
        .await;
    let requested_candidate = ranked_candidates
        .iter()
        .find(|c| c.tool == tool_name)
        .cloned();

    // NEW: Intelligent Policy Check
    let intelligent_policy_result = if let Some(candidate) = requested_candidate {
        crate::intelligent_policy::PolicyCheck {
            decision: candidate.decision,
            reason: candidate.reason,
            risk_level: candidate.risk,
        }
    } else {
        crate::intelligent_policy::INTELLIGENT_POLICY
            .check_tool(tool_name, policy_user)
            .await
    };

    match intelligent_policy_result.decision {
        crate::intelligent_policy::Decision::Deny => {
            if let Some(fallback) = ranked_candidates.iter().find(|c| {
                c.tool != tool_name
                    && c.decision == crate::intelligent_policy::Decision::Allow
                    && c.score >= 40.0
                    && is_auto_fallback_compatible(tool_name, &c.tool, &tool_call)
            }) {
                selected_tool_name = fallback.tool.clone();
                tool_call["tool"] = serde_json::Value::String(selected_tool_name.clone());
                crate::intelligent_router::INTELLIGENT_ROUTER.record_fallback_auto_selected();
                tracing::warn!(
                    "Intelligent policy denied '{}' and auto-selected compatible fallback '{}'",
                    tool_name,
                    selected_tool_name
                );
            } else {
                let fallback_hint = ranked_candidates
                    .iter()
                    .find(|c| {
                        c.tool != tool_name
                            && c.decision == crate::intelligent_policy::Decision::Allow
                            && c.score >= 40.0
                    })
                    .map(|c| format!(" Suggested safer fallback: '{}'", c.tool))
                    .unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "Intelligent policy denied tool '{}': {}.{}",
                    tool_name,
                    intelligent_policy_result.reason,
                    fallback_hint
                ));
            }
        }
        crate::intelligent_policy::Decision::Escalate => {
            policy_force_approval = true;
            tracing::info!(
                "Tool '{}' requires approval due to intelligent policy (alternatives: {})",
                tool_name,
                summarize_top_candidates(&ranked_candidates)
            );
        }
        _ => {} // Allow continues normally
    }

    // If requested tool is allowed but a clearly better compatible alternative exists,
    // proactively switch to improve execution reliability.
    if selected_tool_name == tool_name {
        let requested_score = ranked_candidates
            .iter()
            .find(|c| c.tool == tool_name)
            .map(|c| c.score)
            .unwrap_or(0.0);

        if let Some(top) = ranked_candidates.iter().find(|c| {
            c.tool != tool_name
                && c.decision == crate::intelligent_policy::Decision::Allow
                && c.score > requested_score + 20.0
                && is_auto_fallback_compatible(tool_name, &c.tool, &tool_call)
        }) {
            selected_tool_name = top.tool.clone();
            tool_call["tool"] = serde_json::Value::String(selected_tool_name.clone());
            crate::intelligent_router::INTELLIGENT_ROUTER.record_fallback_auto_selected();
            tracing::info!(
                "Planner proactively switched '{}' -> '{}' (score {:.1} -> {:.1})",
                tool_name,
                selected_tool_name,
                requested_score,
                top.score
            );
        }
    }

    let tool_name = selected_tool_name.as_str();

    // NEW: Proactive Security Scan
    let tool_json = serde_json::to_string(&tool_call)?;
    let security_violations = crate::proactive_security::PROACTIVE_SECURITY
        .scan_input(&tool_json)
        .await;

    if !security_violations.is_empty() {
        let violations_desc: Vec<String> = security_violations
            .iter()
            .map(|v| format!("{:?}: {}", v.category, v.description))
            .collect();

        tracing::warn!(
            "Security violations detected for tool '{}': {:?}",
            tool_name,
            violations_desc
        );

        // Check if any critical violations
        let has_critical = security_violations
            .iter()
            .any(|v| matches!(v.severity, crate::proactive_security::Severity::Critical));
        let has_high_or_critical = security_violations.iter().any(|v| {
            matches!(
                v.severity,
                crate::proactive_security::Severity::High
                    | crate::proactive_security::Severity::Critical
            )
        });

        if has_high_or_critical {
            crate::intelligent_policy::INTELLIGENT_POLICY
                .mark_suspicious(tenant_id.unwrap_or("default"))
                .await;
            policy_force_approval = true;
        }

        if has_critical {
            return Err(anyhow::anyhow!(
                "CRITICAL security violations detected: {}",
                violations_desc.join(", ")
            ));
        }
    }

    let workspace_root = std::env::current_dir()?;
    let default_policy = super::policy::ToolPolicy::permissive();
    let policy = tool_policy.unwrap_or(&default_policy);

    if let Err(e) = policy.check_tool_allowed(tool_name) {
        match e {
            super::policy::PolicyViolation::ApprovalRequired(_) => {
                policy_force_approval = true;
            }
            other => return Err(anyhow::anyhow!("Policy violation: {}", other)),
        }
    }

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
            .unwrap_or_else(|| {
                super::permissions::Operation::NetworkRequest("web_fetch".to_string())
            }),
        "web_search" => super::permissions::Operation::NetworkRequest("web_search".to_string()),
        "browser_navigate" | "browser_click" | "browser_type" | "browser_screenshot"
        | "browser_evaluate" | "browser_pdf" | "browser_list_tabs" | "browser_switch_tab" => {
            super::permissions::Operation::NetworkRequest("browser".to_string())
        }
        "run_command" | "spawn_process" | "bash" | "exec" => {
            let cmd = tool_call
                .get("command")
                .or(tool_call.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            super::permissions::Operation::ExecuteCommand(cmd.to_string())
        }
        "question" => super::permissions::Operation::ReadFile(workspace_root.clone()),
        "task" => super::permissions::Operation::ExecuteCommand("sessions_spawn".to_string()),
        "skill" => {
            let action = tool_call
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if action == "run" {
                let skill_name = tool_call
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                super::permissions::Operation::ExecuteCommand(format!("skill.run:{}", skill_name))
            } else {
                super::permissions::Operation::ReadFile(workspace_root.join("skills"))
            }
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

                // NEW: Proactive security path verification
                let path = std::path::Path::new(path_str);
                match crate::proactive_security::PROACTIVE_SECURITY
                    .verify_path(path, &workspace_root)
                    .await
                {
                    crate::proactive_security::SecurityCheck::Safe => {}
                    crate::proactive_security::SecurityCheck::Unsafe(violations) => {
                        return Err(anyhow::anyhow!(
                            "Security violation for path '{}': {:?}",
                            path_str,
                            violations
                        ));
                    }
                }
            }
        }
        "write_file" | "edit_file" | "apply_patch" => {
            if let Some(path_str) = tool_call.get("path").and_then(|v| v.as_str()) {
                policy
                    .check_write_path(path_str)
                    .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;

                // NEW: Proactive security path verification for writes
                let path = std::path::Path::new(path_str);
                match crate::proactive_security::PROACTIVE_SECURITY
                    .verify_path(path, &workspace_root)
                    .await
                {
                    crate::proactive_security::SecurityCheck::Safe => {}
                    crate::proactive_security::SecurityCheck::Unsafe(violations) => {
                        return Err(anyhow::anyhow!(
                            "Security violation for write path '{}': {:?}",
                            path_str,
                            violations
                        ));
                    }
                }
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

                // NEW: Proactive security command verification
                let allowed_commands = std::collections::HashSet::new(); // Could load from config
                match crate::proactive_security::PROACTIVE_SECURITY
                    .verify_command(cmd, &allowed_commands)
                    .await
                {
                    crate::proactive_security::SecurityCheck::Safe => {}
                    crate::proactive_security::SecurityCheck::Unsafe(violations) => {
                        return Err(anyhow::anyhow!(
                            "Security violation for command '{}': {:?}",
                            cmd,
                            violations
                        ));
                    }
                }
            }
        }
        "web_fetch" | "web_search" => {
            // NEW: Proactive security URL verification for network operations
            if let Some(url) = tool_call.get("url").and_then(|v| v.as_str()) {
                match crate::proactive_security::PROACTIVE_SECURITY
                    .verify_url(url)
                    .await
                {
                    crate::proactive_security::SecurityCheck::Safe => {}
                    crate::proactive_security::SecurityCheck::Unsafe(violations) => {
                        return Err(anyhow::anyhow!(
                            "Security violation for URL '{}': {:?}",
                            url,
                            violations
                        ));
                    }
                }
            }
        }
        _ => {}
    }

    // Check permission (using passed permission manager or create temporary one)
    let channel_key = tenant_id.unwrap_or("default");
    let operation_key = if let Some(fp) = approval_fingerprint(tool_name, &tool_call) {
        format!("{}:{}:{}", channel_key, tool_name, fp)
    } else {
        format!("{}:{}:{:?}", channel_key, tool_name, operation)
    };
    let force_approval = is_mandatory_approval_tool(tool_name, &tool_call);

    let cached_decision = if force_approval {
        None
    } else if let Some(perm_mgr) = permission_manager {
        let mgr = perm_mgr.lock().await;
        mgr.get_cached_decision(&operation_key)
    } else {
        None
    };

    let decision_from_cache = cached_decision.is_some();

    let mut decision = if let Some(cached) = cached_decision {
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

    if (force_approval || policy_force_approval)
        && decision == super::permissions::PermissionDecision::Allow
        && !decision_from_cache
    {
        decision = super::permissions::PermissionDecision::Ask;
    }

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
            return Ok(super::ToolResult::error(format!(
                "Permission denied: Tool '{}' is not allowed",
                tool_name
            ))
            .output);
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
                if force_approval {
                    tracing::warn!(
                        "HeadlessAllowLog active but mandatory approval required for {}. Denying by default.",
                        tool_name
                    );
                    if let Some(logger) = &audit_logger {
                        logger.log_deny(
                            tool_name,
                            tenant_id.unwrap_or("default"),
                            "headless-allow-log-mandatory-deny",
                            tool_call.clone(),
                            "mandatory approval required for high-risk tool in headless mode",
                        );
                    }
                    return Ok(super::ToolResult::error(format!(
                        "Permission denied in headless mode for high-risk tool '{}'. Mandatory approval is required.",
                        tool_name
                    ))
                    .output);
                }

                tracing::info!("HeadlessAllowLog active, auto-allowing tool: {}", tool_name);
                if let Some(logger) = &audit_logger {
                    logger.log_allow(
                        tool_name,
                        tenant_id.unwrap_or("default"),
                        "headless-allow-log",
                        tool_call.clone(),
                    );
                }
            } else if confirmation_service.is_none() && tenant_id.is_none() {
                // Hardened boundary: When there's no confirmation service AND no tenant context,
                // we're in an internal/unattended context. Apply strict rules:
                // - Allow low-risk read-only operations
                // - Deny high and medium risk operations
                let risk_level = match tool_name {
                    "read_file" | "list_directory" | "glob" | "grep" | "web_fetch"
                    | "web_search" => super::confirmation::RiskLevel::Low,
                    "write_file" | "edit_file" | "apply_patch" | "run_command"
                    | "spawn_process" | "bash" | "exec" | "skill" | "browser_navigate"
                    | "browser_click" | "browser_type" | "browser_screenshot"
                    | "browser_evaluate" | "browser_pdf" => super::confirmation::RiskLevel::High,
                    _ => super::confirmation::RiskLevel::Medium,
                };

                match risk_level {
                    super::confirmation::RiskLevel::Low => {
                        tracing::debug!(
                            "Internal context: Allowing low-risk tool '{}' without confirmation",
                            tool_name
                        );
                    }
                    _ => {
                        tracing::error!(
                            "Internal context: Denying {:?}-risk tool '{}' without confirmation service",
                            risk_level,
                            tool_name
                        );
                        if let Some(logger) = &audit_logger {
                            logger.log_deny(
                                tool_name,
                                "internal",
                                "internal-context-no-confirmation",
                                tool_call.clone(),
                                "High/medium risk tool denied in internal context without confirmation service",
                            );
                        }
                        return Ok(super::ToolResult::error(format!(
                            "Permission denied: Tool '{}' requires confirmation service in internal context. \
                             High and medium risk operations are not allowed without proper confirmation infrastructure.",
                            tool_name
                        ))
                        .output);
                    }
                }
            } else {
                let risk_level = match tool_name {
                    "read_file" | "list_directory" => super::confirmation::RiskLevel::Low,
                    "write_file" | "edit_file" => super::confirmation::RiskLevel::Medium,
                    "run_command" | "spawn_process" | "bash" | "exec" | "apply_patch" => {
                        super::confirmation::RiskLevel::High
                    }
                    "browser_navigate" | "browser_click" | "browser_type"
                    | "browser_screenshot" | "browser_evaluate" | "browser_pdf"
                    | "browser_list_tabs" | "browser_switch_tab" => {
                        super::confirmation::RiskLevel::High
                    }
                    "skill"
                        if tool_call
                            .get("action")
                            .and_then(|v| v.as_str())
                            .map(|a| a.eq_ignore_ascii_case("run"))
                            .unwrap_or(false) =>
                    {
                        super::confirmation::RiskLevel::High
                    }
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
                    local_service.register_adapter(Box::new(
                        super::cli_confirmation::CliConfirmationAdapter::new(),
                    ));
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
                    return Ok(super::ToolResult::error(format!(
                        "User denied permission for tool: {}",
                        tool_name
                    ))
                    .output);
                }

                if response.remember && !force_approval {
                    if let Some(perm_mgr) = permission_manager {
                        let mut mgr = perm_mgr.lock().await;
                        mgr.cache_decision(operation_key, true);
                    }
                } else if response.remember && force_approval {
                    tracing::info!(
                        "Ignoring remember=true for mandatory approval tool {}",
                        tool_name
                    );
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
        if let Some(skill) = loader_guard.get_skill(tool_name)
            && skill.enabled
        {
            tracing::info!("Executing skill directly: {}", tool_name);
            // Get the primary tool name from the skill, or use the skill name itself
            // Clone it before dropping the guard to avoid borrow issues
            let primary_tool = skill
                .tools
                .first()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| tool_name.to_string());
            drop(loader_guard); // Release lock before await
            // Execute the skill using the proper backend
            return execute_skill_by_name(
                tool_name,
                &primary_tool,
                &tool_call,
                loader,
                mcp_manager,
            )
            .await;
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
                    let output = tool_res
                        .content
                        .iter()
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

        let started = std::time::Instant::now();
        let first = registry
            .execute_with_policy(tool_name, args.clone(), policy)
            .await;
        match first {
            Ok(output) => {
                if is_retryable_tool(tool_name)
                    && let Some(reason) = retry_reason_for_output(tool_name, &tool_call, &output)
                {
                    tracing::warn!(
                        "Planner verify requested retry for tool '{}' due to: {}",
                        tool_name,
                        reason
                    );
                    let retried = registry
                        .execute_with_policy(tool_name, args.clone(), policy)
                        .await;
                    match retried {
                        Ok(retry_output) => {
                            let verified =
                                retry_reason_for_output(tool_name, &tool_call, &retry_output)
                                    .is_none();
                            crate::intelligent_router::INTELLIGENT_ROUTER.record_tool_outcome(
                                policy_user,
                                tool_name,
                                verified,
                                started.elapsed(),
                            );
                            if verified {
                                return Ok(retry_output);
                            }
                            if let Some(fallback) = ranked_candidates.iter().find(|c| {
                                c.tool != tool_name
                                    && c.decision == crate::intelligent_policy::Decision::Allow
                                    && c.score >= 45.0
                                    && registry.get(&c.tool).is_some()
                                    && is_auto_fallback_compatible(tool_name, &c.tool, &tool_call)
                            }) {
                                tracing::warn!(
                                    "Planner switching to fallback '{}' after retry verify miss for '{}'",
                                    fallback.tool,
                                    tool_name
                                );
                                if let Ok(fallback_output) = registry
                                    .execute_with_policy(&fallback.tool, args.clone(), policy)
                                    .await
                                {
                                    crate::intelligent_router::INTELLIGENT_ROUTER
                                        .record_fallback_auto_selected();
                                    crate::intelligent_router::INTELLIGENT_ROUTER
                                        .record_tool_outcome(
                                            policy_user,
                                            &fallback.tool,
                                            true,
                                            started.elapsed(),
                                        );
                                    return Ok(fallback_output);
                                }
                            }
                            return Ok(output);
                        }
                        Err(e) => {
                            if let Some(fallback) = ranked_candidates.iter().find(|c| {
                                c.tool != tool_name
                                    && c.decision == crate::intelligent_policy::Decision::Allow
                                    && c.score >= 45.0
                                    && registry.get(&c.tool).is_some()
                                    && is_auto_fallback_compatible(tool_name, &c.tool, &tool_call)
                            }) {
                                tracing::warn!(
                                    "Planner switching to fallback '{}' after retry error for '{}'",
                                    fallback.tool,
                                    tool_name
                                );
                                if let Ok(fallback_output) = registry
                                    .execute_with_policy(&fallback.tool, args.clone(), policy)
                                    .await
                                {
                                    crate::intelligent_router::INTELLIGENT_ROUTER
                                        .record_fallback_auto_selected();
                                    crate::intelligent_router::INTELLIGENT_ROUTER
                                        .record_tool_outcome(
                                            policy_user,
                                            &fallback.tool,
                                            true,
                                            started.elapsed(),
                                        );
                                    return Ok(fallback_output);
                                }
                            }
                            crate::intelligent_router::INTELLIGENT_ROUTER.record_tool_outcome(
                                policy_user,
                                tool_name,
                                false,
                                started.elapsed(),
                            );
                            return Err(e);
                        }
                    }
                }

                crate::intelligent_router::INTELLIGENT_ROUTER.record_tool_outcome(
                    policy_user,
                    tool_name,
                    true,
                    started.elapsed(),
                );
                return Ok(output);
            }
            Err(e) => {
                if let Some(fallback) = ranked_candidates.iter().find(|c| {
                    c.tool != tool_name
                        && c.decision == crate::intelligent_policy::Decision::Allow
                        && c.score >= 45.0
                        && registry.get(&c.tool).is_some()
                        && is_auto_fallback_compatible(tool_name, &c.tool, &tool_call)
                }) {
                    tracing::warn!(
                        "Planner switching to fallback '{}' after primary error for '{}'",
                        fallback.tool,
                        tool_name
                    );
                    if let Ok(fallback_output) = registry
                        .execute_with_policy(&fallback.tool, args.clone(), policy)
                        .await
                    {
                        crate::intelligent_router::INTELLIGENT_ROUTER
                            .record_fallback_auto_selected();
                        crate::intelligent_router::INTELLIGENT_ROUTER.record_tool_outcome(
                            policy_user,
                            &fallback.tool,
                            true,
                            started.elapsed(),
                        );
                        return Ok(fallback_output);
                    }
                }
                crate::intelligent_router::INTELLIGENT_ROUTER.record_tool_outcome(
                    policy_user,
                    tool_name,
                    false,
                    started.elapsed(),
                );
                return Err(e);
            }
        }
    }

    // Fall back to legacy match for complex tools that need context
    let legacy_started = std::time::Instant::now();
    let request_context = RequestContext {
        request_id: tool_call["request_id"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        tenant_id: tenant_id.unwrap_or("default").to_string(),
        tool_name: tool_name.to_string(),
    };
    let basic_cap = grant_tool_basic(request_context);
    let filesystem_cap = grant_filesystem_cap(&basic_cap);
    let network_cap = grant_network_cap(&basic_cap);
    let process_cap = grant_process_cap(&basic_cap);
    let persistence_cap = grant_persistence_cap(&basic_cap);
    tracing::debug!(
        request_id = %basic_cap.auth.request.request_id,
        tenant_id = %basic_cap.auth.request.tenant_id,
        tool_name = %basic_cap.auth.request.tool_name,
        "Executing tool under typed capability gate"
    );

    let legacy_result = match tool_name {
        "read_file" => {
            let args = ReadFileArgs {
                path: tool_call["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' field"))?
                    .to_string(),
            };
            run_read_file_with_cap(&filesystem_cap, args).await
        }

        "write_file" => {
            let args = WriteFileArgs {
                path: tool_call["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' field"))?
                    .to_string(),
                content: tool_call["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'content' field"))?
                    .to_string(),
                overwrite: tool_call["overwrite"].as_bool().unwrap_or(false),
            };
            run_write_file_with_cap(&filesystem_cap, args).await
        }

        "list_directory" => {
            let args = ListDirArgs {
                path: tool_call["path"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'path' field"))?
                    .to_string(),
                max_depth: tool_call["max_depth"]
                    .as_u64()
                    .or_else(|| tool_call["maxDepth"].as_u64())
                    .map(|n| n as usize),
            };
            run_list_directory_with_cap(&filesystem_cap, args).await
        }

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
            run_edit_file_with_cap(&filesystem_cap, args).await
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
            run_spawn_process_with_cap(&process_cap, args).await
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
            run_read_process_with_cap(&process_cap, args).await
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
            run_kill_process_with_cap(&process_cap, args).await
        }

        "list_processes" => run_list_processes_with_cap(&process_cap).await,

        "web_fetch" => {
            let args = super::fetch::WebFetchArgs {
                url: tool_call["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing 'url' field"))?
                    .to_string(),
                extract_mode: tool_call["extract_mode"].as_str().map(|s| s.to_string()),
            };
            run_web_fetch_with_cap(&network_cap, args).await
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
            run_glob_with_cap(&filesystem_cap, args).await
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
            run_grep_with_cap(&filesystem_cap, args).await
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
            })
            .to_string())
        }

        "apply_patch" => {
            let patch = tool_call["patch"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| tool_call["patch_text"].as_str().map(|s| s.to_string()))
                .or_else(|| tool_call["patchText"].as_str().map(|s| s.to_string()));

            let operations = tool_call["operations"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            if patch.is_none() && operations.is_empty() {
                return Err(anyhow::anyhow!(
                    "Missing 'operations' or unified diff 'patch'/'patch_text'"
                ));
            }

            let args = ApplyPatchArgs {
                patch,
                patch_text: None,
                operations: serde_json::from_value(serde_json::Value::Array(operations))?,
                dry_run: tool_call["dry_run"]
                    .as_bool()
                    .or_else(|| tool_call["dryRun"].as_bool())
                    .unwrap_or(false),
                atomic: tool_call["atomic"].as_bool().unwrap_or(true),
            };
            run_apply_patch_with_cap(&filesystem_cap, args).await
        }

        "todowrite" => {
            let todos = tool_call["todos"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing 'todos' field"))?
                .clone();

            let args = TodoWriteArgs {
                todos: serde_json::from_value(serde_json::Value::Array(todos))?,
            };
            run_todowrite_with_cap(&persistence_cap, args, tenant_id).await
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
                merged.insert(
                    "tool".to_string(),
                    serde_json::Value::String(tool.to_string()),
                );

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

                prepared.push((
                    idx,
                    tool.to_string(),
                    serde_json::Value::Object(merged).to_string(),
                ));
            }

            let results = join_all(prepared.into_iter().map(|(idx, tool, input)| async move {
                let started = std::time::Instant::now();
                let call = std::boxed::Box::pin(execute_tool(&input, ctx));
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

                let task_plan =
                    crate::intelligent_router::INTELLIGENT_ROUTER.decompose_task(prompt);
                let supported_tools = supported_tool_names()
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect::<std::collections::HashSet<_>>();
                let critique = crate::intelligent_router::INTELLIGENT_ROUTER
                    .critique_task_plan(&task_plan, &supported_tools);
                let refined_task_plan = crate::intelligent_router::INTELLIGENT_ROUTER
                    .refine_task_plan(&task_plan, &supported_tools);

                let plan_validation = crate::intelligent_router::INTELLIGENT_ROUTER
                    .validate_task_plan(&refined_task_plan)
                    .map(|_| "valid".to_string())
                    .unwrap_or_else(|e| format!("invalid: {}", e));
                let execution_order = crate::intelligent_router::INTELLIGENT_ROUTER
                    .execution_order(&refined_task_plan)
                    .unwrap_or_else(|_| refined_task_plan.steps.clone());
                let rollback_hints = crate::intelligent_router::INTELLIGENT_ROUTER
                    .rollback_hints(&refined_task_plan);
                let execution_preview = crate::intelligent_router::INTELLIGENT_ROUTER
                    .build_execution_preview(&refined_task_plan, &supported_tools);

                let execute_plan = tool_call["execute_plan"]
                    .as_bool()
                    .or_else(|| tool_call["executePlan"].as_bool())
                    .unwrap_or(false);
                let strict_plan = tool_call["strict_plan"]
                    .as_bool()
                    .or_else(|| tool_call["strictPlan"].as_bool())
                    .unwrap_or(true);
                let continue_on_failure = tool_call["continue_on_failure"]
                    .as_bool()
                    .or_else(|| tool_call["continueOnFailure"].as_bool())
                    .unwrap_or(false);
                let max_plan_steps = tool_call["max_plan_steps"]
                    .as_u64()
                    .or_else(|| tool_call["maxPlanSteps"].as_u64())
                    .unwrap_or(8)
                    .clamp(1, 24) as usize;

                if execute_plan {
                    if strict_plan && !execution_preview.ready_to_run {
                        return Ok(json!({
                            "status": "blocked",
                            "mode": "planner_execute",
                            "reason": "plan_not_ready",
                            "planner": {
                                "validation": plan_validation,
                                "execution_preview": execution_preview,
                                "rollback_hints": rollback_hints,
                            }
                        })
                        .to_string());
                    }

                    let mut completed = std::collections::HashSet::new();
                    let mut produced_artifacts = std::collections::HashSet::new();
                    let mut step_results = Vec::new();
                    let mut overall_ok = true;

                    for step in execution_order.iter().take(max_plan_steps) {
                        let original_tool = step.suggested_tool.clone();
                        let step_run = crate::intelligent_router::INTELLIGENT_ROUTER
                            .rewrite_step_for_reliability(step, &supported_tools);
                        let step = &step_run;

                        if step.suggested_tool != original_tool {
                            tracing::info!(
                                "Planner rewrote step '{}' tool '{}' -> '{}' for reliability",
                                step.id,
                                original_tool,
                                step.suggested_tool
                            );
                        }

                        if strict_plan && step.confidence < 0.35 {
                            overall_ok = false;
                            step_results.push(json!({
                                "id": step.id,
                                "tool": step.suggested_tool,
                                "status": "blocked",
                                "reason": format!("low_confidence:{:.2}", step.confidence),
                                "rollback_action": rollback_action_for_step(step),
                            }));
                            if !continue_on_failure {
                                break;
                            }
                            continue;
                        }

                        let mut blocked = Vec::new();
                        for dep in &step.dependencies {
                            if !completed.contains(dep) {
                                blocked.push(dep.clone());
                            }
                        }
                        for artifact in &step.expected_inputs {
                            if artifact != "step:previous_output"
                                && !produced_artifacts.contains(artifact)
                            {
                                blocked.push(format!("artifact:{}", artifact));
                            }
                        }

                        if !blocked.is_empty() {
                            overall_ok = false;
                            step_results.push(json!({
                                "id": step.id,
                                "tool": step.suggested_tool,
                                "status": "blocked",
                                "blocked_by": blocked,
                                "rollback_action": rollback_action_for_step(step),
                            }));
                            if !continue_on_failure {
                                break;
                            }
                            continue;
                        }

                        if !supported_tools.contains(&step.suggested_tool)
                            || step.suggested_tool == "task"
                        {
                            overall_ok = false;
                            step_results.push(json!({
                                "id": step.id,
                                "tool": step.suggested_tool,
                                "status": "skipped",
                                "reason": "unsupported_or_recursive_tool",
                                "rollback_action": rollback_action_for_step(step),
                            }));
                            if !continue_on_failure {
                                break;
                            }
                            continue;
                        }

                        let mut obj = serde_json::Map::new();
                        obj.insert(
                            "tool".to_string(),
                            serde_json::Value::String(step.suggested_tool.clone()),
                        );
                        if let Some(args) = step.inferred_args.as_object() {
                            for (k, v) in args {
                                obj.insert(k.clone(), v.clone());
                            }
                        }
                        let call_value = serde_json::Value::Object(obj);
                        let call_json = call_value.to_string();

                        let started = std::time::Instant::now();
                        let call = std::boxed::Box::pin(execute_tool(&call_json, ctx));

                        match call.await {
                            Ok(output) => {
                                if let Some(assertion_error) =
                                    output_assertion_failure(step, &output)
                                {
                                    let alt_candidates =
                                        crate::intelligent_router::INTELLIGENT_ROUTER
                                            .rank_tool_candidates(
                                                policy_user,
                                                &[
                                                    step.suggested_tool.as_str(),
                                                    "read_file",
                                                    "grep",
                                                    "web_fetch",
                                                    "run_command",
                                                ],
                                            )
                                            .await;

                                    let mut recovered = false;
                                    for candidate in alt_candidates {
                                        if candidate.tool == step.suggested_tool
                                            || candidate.decision
                                                != crate::intelligent_policy::Decision::Allow
                                            || !supported_tools.contains(&candidate.tool)
                                            || !is_auto_fallback_compatible(
                                                &step.suggested_tool,
                                                &candidate.tool,
                                                &call_value,
                                            )
                                        {
                                            continue;
                                        }

                                        let mut alt_map = serde_json::Map::new();
                                        alt_map.insert(
                                            "tool".to_string(),
                                            serde_json::Value::String(candidate.tool.clone()),
                                        );
                                        if let Some(args) = step.inferred_args.as_object() {
                                            for (k, v) in args {
                                                alt_map.insert(k.clone(), v.clone());
                                            }
                                        }
                                        let alt_json =
                                            serde_json::Value::Object(alt_map).to_string();
                                        let alt_call =
                                            std::boxed::Box::pin(execute_tool(&alt_json, ctx));

                                        if let Ok(alt_output) = alt_call.await
                                            && output_assertion_failure(step, &alt_output).is_none()
                                        {
                                            crate::intelligent_router::INTELLIGENT_ROUTER
                                                .record_fallback_auto_selected();
                                            crate::intelligent_router::INTELLIGENT_ROUTER
                                                .record_step_pattern_outcome(step, true);
                                            completed.insert(step.id.clone());
                                            for artifact in
                                                collect_step_artifacts(step, &alt_output)
                                            {
                                                produced_artifacts.insert(artifact);
                                            }
                                            step_results.push(json!({
                                                "id": step.id,
                                                "tool": step.suggested_tool,
                                                "status": "recovered_after_assertion",
                                                "fallback_tool": candidate.tool,
                                                "duration_ms": started.elapsed().as_millis(),
                                                "output": alt_output,
                                            }));
                                            recovered = true;
                                            break;
                                        }
                                    }

                                    if !recovered {
                                        overall_ok = false;
                                        crate::intelligent_router::INTELLIGENT_ROUTER
                                            .record_step_pattern_outcome(step, false);
                                        step_results.push(json!({
                                            "id": step.id,
                                            "tool": step.suggested_tool,
                                            "status": "assertion_failed",
                                            "duration_ms": started.elapsed().as_millis(),
                                            "error": assertion_error,
                                            "rollback_action": rollback_action_for_step(step),
                                        }));
                                        if !continue_on_failure {
                                            break;
                                        }
                                    }
                                    continue;
                                }

                                crate::intelligent_router::INTELLIGENT_ROUTER
                                    .record_step_pattern_outcome(step, true);
                                completed.insert(step.id.clone());
                                for artifact in collect_step_artifacts(step, &output) {
                                    produced_artifacts.insert(artifact);
                                }
                                step_results.push(json!({
                                    "id": step.id,
                                    "tool": step.suggested_tool,
                                    "status": "ok",
                                    "duration_ms": started.elapsed().as_millis(),
                                    "output": output,
                                }));
                            }
                            Err(e) => {
                                let alt_candidates = crate::intelligent_router::INTELLIGENT_ROUTER
                                    .rank_tool_candidates(
                                        policy_user,
                                        &[
                                            step.suggested_tool.as_str(),
                                            "read_file",
                                            "grep",
                                            "web_fetch",
                                            "run_command",
                                        ],
                                    )
                                    .await;

                                let mut recovered = false;
                                for candidate in alt_candidates {
                                    if candidate.tool == step.suggested_tool
                                        || candidate.decision
                                            != crate::intelligent_policy::Decision::Allow
                                        || !supported_tools.contains(&candidate.tool)
                                    {
                                        continue;
                                    }

                                    if !is_auto_fallback_compatible(
                                        &step.suggested_tool,
                                        &candidate.tool,
                                        &call_value,
                                    ) {
                                        continue;
                                    }

                                    let mut alt_map = serde_json::Map::new();
                                    alt_map.insert(
                                        "tool".to_string(),
                                        serde_json::Value::String(candidate.tool.clone()),
                                    );
                                    if let Some(args) = step.inferred_args.as_object() {
                                        for (k, v) in args {
                                            alt_map.insert(k.clone(), v.clone());
                                        }
                                    }
                                    let alt_json = serde_json::Value::Object(alt_map).to_string();
                                    let alt_call =
                                        std::boxed::Box::pin(execute_tool(&alt_json, ctx));

                                    if let Ok(alt_output) = alt_call.await {
                                        crate::intelligent_router::INTELLIGENT_ROUTER
                                            .record_fallback_auto_selected();
                                        crate::intelligent_router::INTELLIGENT_ROUTER
                                            .record_step_pattern_outcome(step, true);
                                        completed.insert(step.id.clone());
                                        for artifact in collect_step_artifacts(step, &alt_output) {
                                            produced_artifacts.insert(artifact);
                                        }
                                        step_results.push(json!({
                                            "id": step.id,
                                            "tool": step.suggested_tool,
                                            "status": "recovered",
                                            "fallback_tool": candidate.tool,
                                            "duration_ms": started.elapsed().as_millis(),
                                            "output": alt_output,
                                        }));
                                        recovered = true;
                                        break;
                                    }
                                }

                                if !recovered {
                                    overall_ok = false;
                                    crate::intelligent_router::INTELLIGENT_ROUTER
                                        .record_step_pattern_outcome(step, false);
                                    step_results.push(json!({
                                        "id": step.id,
                                        "tool": step.suggested_tool,
                                        "status": "error",
                                        "duration_ms": started.elapsed().as_millis(),
                                        "error": e.to_string(),
                                        "rollback_action": rollback_action_for_step(step),
                                    }));
                                    if !continue_on_failure {
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if overall_ok {
                        crate::intelligent_router::INTELLIGENT_ROUTER
                            .record_successful_sequence(&refined_task_plan);
                    }

                    return Ok(json!({
                        "status": if overall_ok { "completed" } else { "failed" },
                        "mode": "planner_execute",
                        "description": description,
                        "subagent_type": subagent_type,
                        "planner": {
                            "validation": plan_validation,
                            "step_count": refined_task_plan.steps.len(),
                            "steps": refined_task_plan.steps,
                            "issues": critique.issues.clone(),
                            "confidence_average": critique.confidence_average,
                            "execution_order": execution_order,
                            "execution_preview": execution_preview,
                            "rollback_hints": rollback_hints,
                        },
                        "results": step_results,
                    })
                    .to_string());
                }

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
                    "planner": {
                        "validation": plan_validation,
                        "step_count": refined_task_plan.steps.len(),
                        "steps": refined_task_plan.steps,
                        "issues": critique.issues,
                        "confidence_average": critique.confidence_average,
                        "execution_order": execution_order,
                        "execution_preview": execution_preview,
                        "rollback_hints": rollback_hints,
                    },
                    "hint": "Use sessions_wait/session_status/sessions_cancel for lifecycle control",
                })
                .to_string())
            }
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
        },

        "skill" => {
            let action = tool_call["action"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'action' field"))?;

            let loader = skill_loader.ok_or_else(|| {
                tool_runtime_error("SKILL_LOADER_UNAVAILABLE", "Skill loader not initialized")
            })?;

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
                                "deno_args: [\"run\", \"--compat\", \"--unstable-node-globals\", \"--unstable-bare-node-builtins\", \"--allow-read\", \"--allow-write\"]"
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

                        let mut cfg =
                            crate::skills::config::SkillsConfig::load().unwrap_or_default();
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
                "install" => {
                    let name = tool_call["name"]
                        .as_str()
                        .or_else(|| tool_call["skill"].as_str())
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' (or 'skill') field"))?
                        .trim()
                        .to_ascii_lowercase();
                    if name.is_empty() {
                        return Err(anyhow::anyhow!("Skill name cannot be empty"));
                    }

                    let repo = tool_call["repo"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("openclaw/openclaw")
                        .to_string();
                    let auto_enable = tool_call["auto_enable"]
                        .as_bool()
                        .or_else(|| tool_call["autoEnable"].as_bool())
                        .unwrap_or(true);
                    let bootstrap = tool_call["bootstrap"].as_bool().unwrap_or(true);
                    let runtime_override = tool_call["runtime"]
                        .as_str()
                        .map(|s| s.trim().to_ascii_lowercase())
                        .filter(|s| matches!(s.as_str(), "deno" | "node" | "native" | "mcp"));

                    let workspace = {
                        let mut loader_guard = loader.lock().await;
                        loader_guard.scan()?;
                        loader_guard.workspace_dir().to_path_buf()
                    };

                    let skill_dir = workspace.join("skills").join(&name);
                    if tokio::fs::metadata(&skill_dir).await.is_ok() {
                        let _ = tokio::fs::remove_dir_all(&skill_dir).await;
                    }
                    tokio::fs::create_dir_all(&skill_dir).await?;

                    let client = reqwest::Client::new();
                    download_clawhub_skill_tree(&client, &repo, &name, &skill_dir).await?;

                    let skill_md = tokio::fs::read_to_string(skill_dir.join("SKILL.md"))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Downloaded skill '{}' but SKILL.md is missing/invalid: {}",
                                name,
                                e
                            )
                        })?;

                    let parsed = crate::skills::metadata::SkillMetadata::from_markdown(
                        std::path::PathBuf::from(format!("/skills/{}/SKILL.md", name)),
                        &skill_md,
                    )?;

                    let mut bootstrap_notes = if bootstrap {
                        bootstrap_skill_dependencies_if_present(&skill_dir).await
                    } else {
                        Vec::new()
                    };

                    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
                    if auto_enable {
                        cfg.enable_skill(&name);
                    }
                    if let Some(runtime) = runtime_override.as_deref() {
                        cfg.set_runtime_override(&name, runtime);
                    }
                    if let Some(credentials) = tool_call["credentials"].as_object() {
                        for (k, v) in credentials {
                            if let Some(value) = v.as_str() {
                                cfg.set_credential(&name, k, value.to_string());
                            }
                        }
                    }
                    cfg.save()?;

                    if auto_enable {
                        let mut loader_guard = loader.lock().await;
                        loader_guard.scan()?;
                        let _ = loader_guard.enable_skill(&name);

                        bootstrap_notes.push("skill enabled in skills.toml".to_string());
                    }

                    let mut required = crate::skills::config::known_required_credentials(&name);
                    required.extend(crate::skills::config::required_credentials_from_schema(
                        parsed.config_schema.as_deref(),
                    ));
                    required.sort();
                    required.dedup();
                    let missing_credentials = required
                        .into_iter()
                        .filter(|k| cfg.get_credential(&name, k).is_none())
                        .collect::<Vec<_>>();

                    Ok(json!({
                        "status": "ok",
                        "action": "install",
                        "name": name,
                        "repo": repo,
                        "path": skill_dir.to_string_lossy(),
                        "backend": parsed.backend,
                        "tools": parsed.tools.into_iter().map(|t| t.name).collect::<Vec<_>>(),
                        "runtime": runtime_override,
                        "enabled": auto_enable,
                        "bootstrap": bootstrap,
                        "bootstrap_notes": bootstrap_notes,
                        "missing_credentials": missing_credentials,
                        "next": if auto_enable { "Use skill run" } else { "Use skill enable then skill run" },
                    }).to_string())
                }
                "configure" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?
                        .trim()
                        .to_ascii_lowercase();
                    if name.is_empty() {
                        return Err(anyhow::anyhow!("Skill name cannot be empty"));
                    }

                    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();

                    if let Some(enabled) = tool_call["enabled"].as_bool() {
                        if enabled {
                            cfg.enable_skill(&name);
                        } else {
                            cfg.disable_skill(&name);
                        }
                    }

                    if let Some(runtime) = tool_call["runtime"]
                        .as_str()
                        .map(|s| s.trim().to_ascii_lowercase())
                        && matches!(runtime.as_str(), "deno" | "node" | "native" | "mcp")
                    {
                        cfg.set_runtime_override(&name, &runtime);
                    }

                    let mut stored_keys = Vec::new();
                    if let Some(credentials) = tool_call["credentials"].as_object() {
                        for (k, v) in credentials {
                            if let Some(value) = v.as_str() {
                                cfg.set_credential(&name, k, value.to_string());
                                stored_keys.push(k.clone());
                            }
                        }
                    }

                    cfg.save()?;

                    let mut required = crate::skills::config::known_required_credentials(&name);
                    {
                        let mut loader_guard = loader.lock().await;
                        loader_guard.scan()?;
                        if let Some(skill) = loader_guard.get_skill(&name) {
                            required.extend(
                                crate::skills::config::required_credentials_from_schema(
                                    skill.config_schema.as_deref(),
                                ),
                            );
                        }
                    }
                    required.sort();
                    required.dedup();

                    let missing_credentials = required
                        .into_iter()
                        .filter(|k| cfg.get_credential(&name, k).is_none())
                        .collect::<Vec<_>>();

                    Ok(json!({
                        "status": "ok",
                        "action": "configure",
                        "name": name,
                        "stored_credential_keys": stored_keys,
                        "enabled": cfg.is_enabled(&name),
                        "runtime": cfg.runtime_override(&name),
                        "missing_credentials": missing_credentials,
                    })
                    .to_string())
                }
                "list" => {
                    let skills_cfg =
                        crate::skills::config::SkillsConfig::load().unwrap_or_default();
                    let mut loader = loader.lock().await;
                    loader.scan()?;
                    let mut names: Vec<_> = loader
                        .skills()
                        .values()
                        .map(|s| {
                            let runtime_override =
                                skills_cfg.runtime_override(&s.name).map(|v| v.to_string());
                            json!({
                                "name": s.name,
                                "description": s.description,
                                "enabled": s.enabled,
                                "category": s.category,
                                "status": s.status,
                                "runtime_override": runtime_override,
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
                            "requires_bins": skill.requires_bins,
                            "requires_any_bins": skill.requires_any_bins,
                            "requires_env": skill.requires_env,
                            "requires_config": skill.requires_config,
                            "openclaw_install": skill.openclaw_install,
                            "allowed_os": skill.allowed_os,
                            "always": skill.always,
                            "homepage": skill.homepage,
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
                "set_runtime" => {
                    let name = tool_call["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?;
                    let runtime = tool_call["runtime"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing 'runtime' field"))?
                        .trim()
                        .to_ascii_lowercase();

                    if !matches!(runtime.as_str(), "deno" | "node" | "native" | "mcp") {
                        return Err(anyhow::anyhow!(
                            "Unsupported runtime '{}'. Use: deno|node|native|mcp",
                            runtime
                        ));
                    }

                    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
                    cfg.set_runtime_override(name, &runtime);
                    cfg.save()?;

                    Ok(json!({
                        "status": "ok",
                        "action": "set_runtime",
                        "name": name,
                        "runtime": runtime,
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

                    // Use the unified skill execution function
                    execute_skill_by_name(name, tool_name, &arguments, loader, mcp_manager).await
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
                                    v.as_str().map(|s| (k.clone(), s.to_string())).ok_or_else(
                                        || anyhow::anyhow!("env values must be strings"),
                                    )
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
                                Err(e) => failed
                                    .push(json!({"name": server.name, "error": e.to_string()})),
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
                _ => Err(anyhow::anyhow!("Unknown mcp_config action: {}", action)),
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
            let token = new_executor_token();
            super::process::write_process_input(&token, args).await
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
                None => Ok(tool_runtime_error_payload(
                    "MEMORY_MANAGER_UNAVAILABLE",
                    "Memory manager not initialized.",
                )),
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
                None => Ok(tool_runtime_error_payload(
                    "MEMORY_MANAGER_UNAVAILABLE",
                    "Memory manager not initialized.",
                )),
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
                None => Ok(tool_runtime_error_payload(
                    "MEMORY_MANAGER_UNAVAILABLE",
                    "Memory manager not initialized.",
                )),
            }
        }

        "llm_task" => crate::tools::llm_task::execute_llm_task(&tool_call).await,

        "tts" => crate::tools::tts::execute_tts(&tool_call).await,

        "stt" => crate::tools::stt::execute_stt(&tool_call).await,

        "cron" => match cron_scheduler {
            Some(scheduler) => crate::tools::cron::execute_cron_tool(scheduler, &tool_call).await,
            None => Ok(tool_runtime_error_payload(
                "CRON_SCHEDULER_UNAVAILABLE",
                "Cron scheduler not initialized. Available in gateway/server mode.",
            )),
        },

        "sessions_spawn" | "spawn_subagent" => match agent_manager {
            Some(manager) => {
                crate::tools::sessions::execute_sessions_tool(manager, &tool_call, persistence)
                    .await
            }
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
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
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
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
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
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
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
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
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
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
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
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
            None => Ok(tool_runtime_error_payload(
                "AGENT_MANAGER_UNAVAILABLE",
                "Agent manager not initialized. Available in gateway/server mode.",
            )),
        },

        "session_status" => match agent_manager {
            Some(manager) => {
                crate::tools::sessions::execute_sessions_tool(manager, &tool_call, persistence)
                    .await
            }
            None => match persistence {
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
                None => Ok(tool_runtime_error_payload(
                    "PERSISTENCE_UNAVAILABLE",
                    "Persistence manager not initialized.",
                )),
            },
        },

        "sessions_history" | "sessions_list" | "sessions_send" | "sessions_cancel"
        | "sessions_pause" | "sessions_resume" => match agent_manager {
            Some(manager) => {
                crate::tools::sessions::execute_sessions_tool(manager, &tool_call, persistence)
                    .await
            }
            None => {
                if tool_name == "sessions_history" {
                    match persistence {
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
                        None => Ok(tool_runtime_error_payload(
                            "PERSISTENCE_UNAVAILABLE",
                            "Persistence manager not initialized.",
                        )),
                    }
                } else {
                    Ok(tool_runtime_error_payload(
                        "AGENT_MANAGER_UNAVAILABLE",
                        "Agent manager not initialized. Available in gateway/server mode.",
                    ))
                }
            }
        },

        "agents_list" => {
            let mut agents = Vec::new();
            for path in ["./agents", "./.nanobot/agents"] {
                if let Ok(mut entries) = tokio::fs::read_dir(path).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
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
                let url = tool_call["url"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing url"))?;
                let _page = client.navigate(url).await?;
                Ok(format!("Navigated to {}", url))
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        #[cfg(feature = "browser")]
        "browser_click" => {
            if let Some(client) = browser_client {
                let selector = tool_call["selector"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing selector"))?;
                let page = client.get_page().await?;
                crate::browser::actions::BrowserActions::click(&page, selector).await
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        #[cfg(feature = "browser")]
        "browser_type" => {
            if let Some(client) = browser_client {
                let selector = tool_call["selector"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing selector"))?;
                let text = tool_call["text"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing text"))?;
                let page = client.get_page().await?;
                crate::browser::actions::BrowserActions::type_text(&page, selector, text).await
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        #[cfg(feature = "browser")]
        "browser_screenshot" => {
            if let Some(client) = browser_client {
                let page = client.get_page().await?;
                let data = crate::browser::actions::BrowserActions::screenshot(&page).await?;
                let path = format!(
                    "screenshot_{}.png",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                );
                tokio::fs::write(&path, data).await?;
                Ok(format!("Screenshot saved to {}", path))
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        #[cfg(feature = "browser")]
        "browser_evaluate" => {
            if let Some(client) = browser_client {
                let script = tool_call["script"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Missing script"))?;
                let page = client.get_page().await?;
                crate::browser::actions::BrowserActions::execute_js(&page, script).await
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
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
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        #[cfg(feature = "browser")]
        "browser_list_tabs" => {
            if let Some(client) = browser_client {
                let pages = client.get_pages().await?;
                let mut s = String::new();
                for (i, page) in pages.iter().enumerate() {
                    let title = page
                        .get_title()
                        .await
                        .unwrap_or_default()
                        .unwrap_or_default();
                    let url = page.url().await.unwrap_or_default().unwrap_or_default();
                    s.push_str(&format!("{}: {} ({})\n", i, title, url));
                }
                if s.is_empty() {
                    Ok("No open tabs.".to_string())
                } else {
                    Ok(s)
                }
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        #[cfg(feature = "browser")]
        "browser_switch_tab" => {
            if let Some(client) = browser_client {
                let index = tool_call["index"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("Missing index"))?
                    as usize;
                let _ = client.switch_tab(index).await?;
                Ok(format!("Switched to tab {}", index))
            } else {
                Err(tool_runtime_error(
                    "BROWSER_UNAVAILABLE",
                    "Browser not available.",
                ))
            }
        }

        _ => Err(anyhow::anyhow!("Unknown tool: {}", tool_name)),
    };

    crate::intelligent_router::INTELLIGENT_ROUTER.record_tool_outcome(
        policy_user,
        tool_name,
        legacy_result.is_ok(),
        legacy_started.elapsed(),
    );

    legacy_result
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
    use crate::tools::{
        ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse, ConfirmationService,
        PermissionManager, SecurityProfile,
    };

    #[tokio::test]
    #[ignore = "Requires security configuration - skipping for now"]
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
        let permission_manager =
            tokio::sync::Mutex::new(PermissionManager::new(SecurityProfile::trust()));

        // Create a mock confirmation service for testing with an always-allow adapter
        let mut confirmation_service = ConfirmationService::new();
        // Register a mock adapter that always allows (for unit testing only)
        struct AlwaysAllowAdapter;
        #[async_trait::async_trait]
        impl ConfirmationAdapter for AlwaysAllowAdapter {
            async fn request_confirmation(
                &self,
                request: &ConfirmationRequest,
            ) -> anyhow::Result<ConfirmationResponse> {
                Ok(ConfirmationResponse {
                    id: request.id.clone(),
                    allowed: true,
                    remember: false,
                })
            }
            fn name(&self) -> &str {
                "test-always-allow"
            }
        }
        confirmation_service.register_adapter(Box::new(AlwaysAllowAdapter));
        let confirmation_service = tokio::sync::Mutex::new(confirmation_service);

        // Pass test context with confirmation service to avoid internal context restrictions
        #[cfg(feature = "browser")]
        let result = execute_tool(
            json,
            ExecuteToolContext {
                cron_scheduler: None,
                agent_manager: None,
                memory_manager: None,
                persistence: None,
                permission_manager: Some(&permission_manager),
                tool_policy: None,
                confirmation_service: Some(&confirmation_service),
                skill_loader: None,
                browser_client: None,
                tenant_id: Some("test-tenant"),
                mcp_manager: None,
            },
        )
        .await;
        #[cfg(not(feature = "browser"))]
        let result = execute_tool(
            json,
            ExecuteToolContext {
                cron_scheduler: None,
                agent_manager: None,
                memory_manager: None,
                persistence: None,
                permission_manager: Some(&permission_manager),
                tool_policy: None,
                confirmation_service: Some(&confirmation_service),
                skill_loader: None,
                tenant_id: Some("test-tenant"),
                mcp_manager: None,
            },
        )
        .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.trim().is_empty());
    }
}
