use anyhow::Result;
use async_trait::async_trait;
use nanobot_core::tools::{
    ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse, ConfirmationService,
    PermissionManager, SecurityProfile,
};
use once_cell::sync::Lazy;
use serde_json::json;
use tempfile::tempdir;

static CWD_TEST_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

// Mock confirmation adapter that always allows (for testing only)
struct AlwaysAllowAdapter;

#[async_trait]
impl ConfirmationAdapter for AlwaysAllowAdapter {
    async fn request_confirmation(
        &self,
        request: &ConfirmationRequest,
    ) -> Result<ConfirmationResponse> {
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

fn create_test_confirmation_service() -> ConfirmationService {
    let mut service = ConfirmationService::new();
    service.register_adapter(Box::new(AlwaysAllowAdapter));
    service
}

async fn exec_tool(json_input: &str) -> Result<String> {
    let permission_manager =
        tokio::sync::Mutex::new(PermissionManager::new(SecurityProfile::trust()));
    let confirmation_service = tokio::sync::Mutex::new(create_test_confirmation_service());

    #[cfg(feature = "browser")]
    {
        nanobot_core::tools::executor::execute_tool(
            json_input,
            nanobot_core::tools::executor::ExecuteToolContext {
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
        .await
    }

    #[cfg(not(feature = "browser"))]
    {
        nanobot_core::tools::executor::execute_tool(
            json_input,
            nanobot_core::tools::executor::ExecuteToolContext {
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
        .await
    }
}

async fn exec_tool_with_skill_loader(
    json_input: &str,
    skill_loader: &std::sync::Arc<tokio::sync::Mutex<nanobot_core::skills::SkillLoader>>,
) -> Result<String> {
    let permission_manager =
        tokio::sync::Mutex::new(PermissionManager::new(SecurityProfile::trust()));
    let confirmation_service = tokio::sync::Mutex::new(create_test_confirmation_service());

    #[cfg(feature = "browser")]
    {
        nanobot_core::tools::executor::execute_tool(
            json_input,
            nanobot_core::tools::executor::ExecuteToolContext {
                cron_scheduler: None,
                agent_manager: None,
                memory_manager: None,
                persistence: None,
                permission_manager: Some(&permission_manager),
                tool_policy: None,
                confirmation_service: Some(&confirmation_service),
                skill_loader: Some(skill_loader),
                browser_client: None,
                tenant_id: Some("test-tenant"),
                mcp_manager: None,
            },
        )
        .await
    }

    #[cfg(not(feature = "browser"))]
    {
        nanobot_core::tools::executor::execute_tool(
            json_input,
            nanobot_core::tools::executor::ExecuteToolContext {
                cron_scheduler: None,
                agent_manager: None,
                memory_manager: None,
                persistence: None,
                permission_manager: Some(&permission_manager),
                tool_policy: None,
                confirmation_service: Some(&confirmation_service),
                skill_loader: Some(skill_loader),
                tenant_id: Some("test-tenant"),
                mcp_manager: None,
            },
        )
        .await
    }
}

async fn exec_tool_with_tenant(json_input: &str, tenant_id: &str) -> Result<String> {
    let permission_manager =
        tokio::sync::Mutex::new(PermissionManager::new(SecurityProfile::trust()));

    #[cfg(feature = "browser")]
    {
        nanobot_core::tools::executor::execute_tool(
            json_input,
            nanobot_core::tools::executor::ExecuteToolContext {
                cron_scheduler: None,
                agent_manager: None,
                memory_manager: None,
                persistence: None,
                permission_manager: Some(&permission_manager),
                tool_policy: None,
                confirmation_service: None,
                skill_loader: None,
                browser_client: None,
                tenant_id: Some(tenant_id),
                mcp_manager: None,
            },
        )
        .await
    }

    #[cfg(not(feature = "browser"))]
    {
        nanobot_core::tools::executor::execute_tool(
            json_input,
            nanobot_core::tools::executor::ExecuteToolContext {
                cron_scheduler: None,
                agent_manager: None,
                memory_manager: None,
                persistence: None,
                permission_manager: Some(&permission_manager),
                tool_policy: None,
                confirmation_service: None,
                skill_loader: None,
                tenant_id: Some(tenant_id),
                mcp_manager: None,
            },
        )
        .await
    }
}

#[tokio::test]
async fn glob_tool_finds_files_in_path() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(root.join("src").join("a.rs"), "fn a() {}\n")?;
    std::fs::write(root.join("src").join("b.txt"), "hello\n")?;

    let input = json!({
        "tool": "glob",
        "pattern": "src/**/*.rs",
        "path": root.to_string_lossy(),
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    let count = parsed["count"].as_u64().unwrap_or(0);
    assert!(count >= 1);
    let joined = parsed["paths"].to_string();
    assert!(joined.contains("src/a.rs"));
    Ok(())
}

#[tokio::test]
async fn grep_tool_matches_content() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )?;

    let input = json!({
        "tool": "grep",
        "pattern": "fn\\s+main",
        "path": root.to_string_lossy(),
        "include": "*.rs",
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    let count = parsed["count"].as_u64().unwrap_or(0);
    assert!(count >= 1);
    let matches = parsed["matches"].to_string();
    assert!(matches.contains("main.rs"));
    Ok(())
}

#[tokio::test]
async fn apply_patch_tool_updates_and_adds_file() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    let file = root.join("note.txt");
    std::fs::write(&file, "hello world\n")?;

    let input = json!({
        "tool": "apply_patch",
        "operations": [
            {
                "op": "update",
                "path": file.to_string_lossy(),
                "old_text": "world",
                "new_text": "nanobot",
                "all_occurrences": false
            },
            {
                "op": "add",
                "path": root.join("new.txt").to_string_lossy(),
                "content": "created\n",
                "overwrite": true
            }
        ]
    })
    .to_string();

    let output = exec_tool(&input).await?;
    assert!(output.contains("update"));
    assert!(output.contains("add"));

    let updated = std::fs::read_to_string(&file)?;
    assert!(updated.contains("hello nanobot"));
    assert!(root.join("new.txt").exists());
    Ok(())
}

#[tokio::test]
async fn apply_patch_dry_run_does_not_modify_files() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    let file = root.join("dry.txt");
    std::fs::write(&file, "alpha beta\n")?;

    let input = json!({
        "tool": "apply_patch",
        "dry_run": true,
        "operations": [
            {
                "op": "update",
                "path": file.to_string_lossy(),
                "old_text": "beta",
                "new_text": "gamma"
            }
        ]
    })
    .to_string();

    let output = exec_tool(&input).await?;
    assert!(output.contains("dry-run"));
    let unchanged = std::fs::read_to_string(&file)?;
    assert!(unchanged.contains("alpha beta"));
    Ok(())
}

#[tokio::test]
async fn apply_patch_atomic_rolls_back_on_failure() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    let file = root.join("atomic.txt");
    std::fs::write(&file, "hello world\n")?;

    let input = json!({
        "tool": "apply_patch",
        "atomic": true,
        "operations": [
            {
                "op": "update",
                "path": file.to_string_lossy(),
                "old_text": "world",
                "new_text": "nanobot"
            },
            {
                "op": "update",
                "path": file.to_string_lossy(),
                "old_text": "__missing__",
                "new_text": "x"
            }
        ]
    })
    .to_string();

    let err = exec_tool(&input).await.expect_err("should fail");
    assert!(err.to_string().contains("rolled back"));
    let content = std::fs::read_to_string(&file)?;
    assert!(content.contains("hello world"));
    Ok(())
}

#[tokio::test]
async fn apply_patch_context_aware_update_targets_window() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    let file = root.join("ctx.txt");
    std::fs::write(&file, "alpha\nBEGIN\nvalue=old\nEND\nomega\nvalue=old\n")?;

    let input = json!({
        "tool": "apply_patch",
        "operations": [
            {
                "op": "update",
                "path": file.to_string_lossy(),
                "old_text": "value=old",
                "new_text": "value=new",
                "before_context": "BEGIN",
                "after_context": "END"
            }
        ]
    })
    .to_string();

    let output = exec_tool(&input).await?;
    assert!(output.contains("constrained context"));

    let content = std::fs::read_to_string(&file)?;
    let lines: Vec<&str> = content.lines().collect();
    assert!(lines.contains(&"value=new"));
    let old_count = content.matches("value=old").count();
    assert_eq!(old_count, 1);
    Ok(())
}

#[tokio::test]
async fn question_tool_returns_structured_payload() -> Result<()> {
    let input = json!({
        "tool": "question",
        "question": "Select mode",
        "header": "Mode",
        "options": ["Quick", "Thorough"],
        "multiple": false
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["type"], "question");
    assert_eq!(parsed["header"], "Mode");
    assert_eq!(parsed["question"], "Select mode");
    Ok(())
}

#[tokio::test]
async fn todowrite_tool_persists_structured_state() -> Result<()> {
    let _dir = tempdir()?;
    let tenant = "todo_test_tenant";

    let input = json!({
        "tool": "todowrite",
        "todos": [
            {
                "id": "1",
                "content": "Implement feature",
                "status": "in_progress",
                "priority": "high"
            },
            {
                "id": "2",
                "content": "Run tests",
                "status": "pending",
                "priority": "medium"
            }
        ]
    })
    .to_string();

    let output = exec_tool_with_tenant(&input, tenant).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["counts"]["total"], 2);

    let path = parsed["path"].as_str().unwrap_or("");
    let todo_file = std::path::PathBuf::from(path);
    assert!(todo_file.exists());
    let saved = std::fs::read_to_string(&todo_file)?;
    assert!(saved.contains("Implement feature"));
    Ok(())
}

#[tokio::test]
async fn parallel_tool_runs_safe_calls() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path();
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n")?;

    let input = json!({
        "tool": "parallel",
        "tool_calls": [
            {
                "tool": "glob",
                "args": {
                    "pattern": "src/**/*.rs",
                    "path": root.to_string_lossy()
                }
            },
            {
                "tool": "grep",
                "args": {
                    "pattern": "fn\\s+main",
                    "include": "*.rs",
                    "path": root.to_string_lossy()
                }
            }
        ]
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["count"], 2);
    assert_eq!(parsed["results"][0]["status"], "ok");
    assert_eq!(parsed["results"][1]["status"], "ok");
    Ok(())
}

#[tokio::test]
async fn parallel_tool_rejects_unsafe_tool_calls() -> Result<()> {
    let input = json!({
        "tool": "parallel",
        "tool_calls": [
            {
                "tool": "write_file",
                "args": { "path": "x.txt", "content": "bad" }
            }
        ]
    })
    .to_string();

    let err = exec_tool(&input)
        .await
        .expect_err("should reject unsafe call");
    assert!(err.to_string().contains("not allowed in parallel mode"));
    Ok(())
}

#[tokio::test]
async fn task_tool_requires_agent_manager_context() -> Result<()> {
    let input = json!({
        "tool": "task",
        "description": "test",
        "prompt": "do something",
        "subagent_type": "general"
    })
    .to_string();

    let output = exec_tool(&input).await?;
    assert!(output.contains("Agent manager not initialized"));
    Ok(())
}

#[tokio::test]
async fn sessions_send_requires_agent_manager_context() -> Result<()> {
    let input = json!({
        "tool": "sessions_send",
        "session_id": "main",
        "message": "ping"
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["code"], "AGENT_MANAGER_UNAVAILABLE");
    assert!(
        parsed["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Agent manager not initialized")
    );
    Ok(())
}

#[tokio::test]
async fn sessions_history_requires_persistence_without_manager() -> Result<()> {
    let input = json!({
        "tool": "sessions_history",
        "session_id": "main"
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["code"], "PERSISTENCE_UNAVAILABLE");
    assert!(
        parsed["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Persistence manager not initialized")
    );
    Ok(())
}

#[tokio::test]
async fn cron_tool_requires_scheduler_context() -> Result<()> {
    let input = json!({
        "tool": "cron",
        "action": "list"
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["code"], "CRON_SCHEDULER_UNAVAILABLE");
    Ok(())
}

#[tokio::test]
async fn memory_tool_requires_manager_context() -> Result<()> {
    let input = json!({
        "tool": "memory_save",
        "content": "remember this"
    })
    .to_string();

    let output = exec_tool(&input).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["code"], "MEMORY_MANAGER_UNAVAILABLE");
    Ok(())
}

#[tokio::test]
async fn skill_tool_lists_loaded_skills() -> Result<()> {
    let dir = tempdir()?;
    let bundled = dir.path().join("bundled");
    let managed = dir.path().join("managed");
    std::fs::create_dir_all(&bundled)?;
    std::fs::create_dir_all(&managed)?;
    unsafe {
        std::env::set_var(
            "NANOBOT_BUNDLED_SKILLS_DIR",
            bundled.to_string_lossy().to_string(),
        );
        std::env::set_var(
            "NANOBOT_MANAGED_SKILLS_DIR",
            managed.to_string_lossy().to_string(),
        );
    }

    let skill_dir = dir.path().join("skills").join("github");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: github\ndescription: \"GitHub helper\"\ncategory: integration\nstatus: active\n---\n\n# GitHub\n\n## Tools Provided\n\n- `gh_issue`: create issue\n",
    )?;

    let loader = std::sync::Arc::new(tokio::sync::Mutex::new(
        nanobot_core::skills::SkillLoader::new(dir.path().to_path_buf()),
    ));

    let input = json!({
        "tool": "skill",
        "action": "list"
    })
    .to_string();

    let output = exec_tool_with_skill_loader(&input, &loader).await?;
    let parsed: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(parsed["count"], 1);
    assert!(parsed["skills"].to_string().contains("github"));

    unsafe {
        std::env::remove_var("NANOBOT_BUNDLED_SKILLS_DIR");
        std::env::remove_var("NANOBOT_MANAGED_SKILLS_DIR");
    }

    Ok(())
}

#[tokio::test]
async fn skill_tool_create_deno_scaffold() -> Result<()> {
    let dir = tempdir()?;
    let loader = std::sync::Arc::new(tokio::sync::Mutex::new(
        nanobot_core::skills::SkillLoader::new(dir.path().to_path_buf()),
    ));

    let create = json!({
        "tool": "skill",
        "action": "create",
        "name": "my_deno_skill",
        "backend": "deno",
        "description": "Deno generated skill",
        "auto_enable": false
    })
    .to_string();

    let out = exec_tool_with_skill_loader(&create, &loader).await?;
    let parsed: serde_json::Value = serde_json::from_str(&out)?;
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["backend"], "deno");

    let skill_dir = dir.path().join("skills").join("my_deno_skill");
    let skill_md = skill_dir.join("SKILL.md");
    let main_ts = skill_dir.join("main.ts");
    assert!(skill_md.exists());
    assert!(main_ts.exists());

    let md = std::fs::read_to_string(skill_md)?;
    assert!(md.contains("backend: deno"));
    assert!(md.contains("deno_script: skills/my_deno_skill/main.ts"));
    Ok(())
}

#[tokio::test]
async fn skill_tool_run_mcp_requires_manager() -> Result<()> {
    let dir = tempdir()?;

    let skill_dir = dir.path().join("skills").join("mcp_demo");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: mcp_demo\ndescription: \"MCP demo\"\nbackend: mcp\nmcp_server_name: demo-server\n---\n\n# MCP Demo\n\n## Tools Provided\n\n- `demo_tool`: run demo\n",
    )?;

    let loader = std::sync::Arc::new(tokio::sync::Mutex::new(
        nanobot_core::skills::SkillLoader::new(dir.path().to_path_buf()),
    ));

    let input = json!({
        "tool": "skill",
        "action": "enable",
        "name": "mcp_demo"
    })
    .to_string();
    let _ = exec_tool_with_skill_loader(&input, &loader).await?;

    let input = json!({
        "tool": "skill",
        "action": "run",
        "name": "mcp_demo",
        "tool_name": "demo_tool",
        "arguments": {}
    })
    .to_string();

    let err = exec_tool_with_skill_loader(&input, &loader)
        .await
        .expect_err("expected error without MCP manager");
    assert!(err.to_string().contains("MCP manager not initialized"));
    assert!(err.to_string().contains("SKILL_MCP_MANAGER_UNAVAILABLE"));

    Ok(())
}

#[tokio::test]
async fn mcp_config_add_list_remove_persists_config() -> Result<()> {
    let _lock = CWD_TEST_LOCK.lock().await;
    let dir = tempdir()?;
    let prev = std::env::current_dir()?;
    std::env::set_current_dir(dir.path())?;

    // Minimal valid config.toml required by Config::load
    std::fs::write(
        dir.path().join("config.toml"),
        r#"
default_provider = "openai"
context_token_limit = 32000

[providers.openai]
api_key = "test-key"

[session]
dm_scope = "main"
"#,
    )?;

    let add = json!({
        "tool": "mcp_config",
        "action": "add",
        "name": "fs",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
        "env": {
            "API_KEY": "super-secret",
            "TOKEN": "another-secret"
        }
    })
    .to_string();

    let add_out = exec_tool(&add).await?;
    let add_json: serde_json::Value = serde_json::from_str(&add_out)?;
    assert_eq!(add_json["status"], "ok");
    assert_eq!(add_json["secrets_redacted"], true);
    assert_eq!(add_json["env_count"], 2);
    assert_eq!(add_json["env_preview"]["API_KEY"], "***");
    assert_eq!(add_json["env_preview"]["TOKEN"], "***");

    let list = json!({
        "tool": "mcp_config",
        "action": "list"
    })
    .to_string();

    let status = json!({
        "tool": "mcp_config",
        "action": "status"
    })
    .to_string();

    let status_out = exec_tool(&status).await?;
    let status_json: serde_json::Value = serde_json::from_str(&status_out)?;
    assert_eq!(status_json["enabled"], true);
    assert_eq!(status_json["configured_servers"], 1);

    let list_out = exec_tool(&list).await?;
    let list_json: serde_json::Value = serde_json::from_str(&list_out)?;
    assert_eq!(list_json["count"], 1);
    assert!(list_json["servers"].to_string().contains("fs"));

    let remove = json!({
        "tool": "mcp_config",
        "action": "remove",
        "name": "fs"
    })
    .to_string();

    let remove_out = exec_tool(&remove).await?;
    let remove_json: serde_json::Value = serde_json::from_str(&remove_out)?;
    assert_eq!(remove_json["status"], "ok");

    let list2_out = exec_tool(&list).await?;
    let list2_json: serde_json::Value = serde_json::from_str(&list2_out)?;
    assert_eq!(list2_json["count"], 0);

    std::env::set_current_dir(prev)?;
    Ok(())
}

#[tokio::test]
async fn skill_tool_run_deno_requires_script() -> Result<()> {
    let dir = tempdir()?;

    let skill_dir = dir.path().join("skills").join("deno_demo");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: deno_demo\ndescription: \"Deno demo\"\nbackend: deno\n---\n\n# Deno Demo\n\n## Tools Provided\n\n- `demo_tool`: run demo\n",
    )?;

    let loader = std::sync::Arc::new(tokio::sync::Mutex::new(
        nanobot_core::skills::SkillLoader::new(dir.path().to_path_buf()),
    ));

    let enable = json!({
        "tool": "skill",
        "action": "enable",
        "name": "deno_demo"
    })
    .to_string();
    let _ = exec_tool_with_skill_loader(&enable, &loader).await?;

    let run = json!({
        "tool": "skill",
        "action": "run",
        "name": "deno_demo",
        "tool_name": "demo_tool",
        "arguments": {}
    })
    .to_string();

    let err = exec_tool_with_skill_loader(&run, &loader)
        .await
        .expect_err("expected deno config error");
    assert!(err.to_string().contains("missing deno_script"));
    assert!(err.to_string().contains("SKILL_DENO_SCRIPT_MISSING"));
    Ok(())
}

#[tokio::test]
async fn skill_tool_run_native_requires_command() -> Result<()> {
    let dir = tempdir()?;

    let skill_dir = dir.path().join("skills").join("native_demo");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: native_demo\ndescription: \"Native demo\"\nbackend: native\n---\n\n# Native Demo\n\n## Tools Provided\n\n- `demo_tool`: run demo\n",
    )?;

    let loader = std::sync::Arc::new(tokio::sync::Mutex::new(
        nanobot_core::skills::SkillLoader::new(dir.path().to_path_buf()),
    ));

    let enable = json!({
        "tool": "skill",
        "action": "enable",
        "name": "native_demo"
    })
    .to_string();
    let _ = exec_tool_with_skill_loader(&enable, &loader).await?;

    let run = json!({
        "tool": "skill",
        "action": "run",
        "name": "native_demo",
        "tool_name": "demo_tool",
        "arguments": {}
    })
    .to_string();

    let err = exec_tool_with_skill_loader(&run, &loader)
        .await
        .expect_err("expected native config error");
    assert!(err.to_string().contains("missing native_command"));
    assert!(err.to_string().contains("SKILL_NATIVE_COMMAND_MISSING"));
    Ok(())
}

#[tokio::test]
async fn skill_tool_deno_missing_uses_node_fallback_reason_code() -> Result<()> {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(());
    }

    let dir = tempdir()?;
    let skill_dir = dir.path().join("skills").join("fallback_demo");
    std::fs::create_dir_all(&skill_dir)?;

    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: fallback_demo\ndescription: \"Fallback demo\"\nbackend: deno\ndeno_script: skills/fallback_demo/main.js\ndeno_command: missing-deno-binary\n---\n\n# Fallback Demo\n\n## Tools Provided\n\n- `demo_tool`: run demo\n",
    )?;
    std::fs::write(
        skill_dir.join("main.js"),
        "console.log(JSON.stringify({ok:true, tool: process.argv[2] || null}));\n",
    )?;

    let loader = std::sync::Arc::new(tokio::sync::Mutex::new(
        nanobot_core::skills::SkillLoader::new(dir.path().to_path_buf()),
    ));

    let enable = json!({
        "tool": "skill",
        "action": "enable",
        "name": "fallback_demo"
    })
    .to_string();
    let _ = exec_tool_with_skill_loader(&enable, &loader).await?;

    let run = json!({
        "tool": "skill",
        "action": "run",
        "name": "fallback_demo",
        "tool_name": "demo_tool",
        "arguments": {"a": 1}
    })
    .to_string();

    match exec_tool_with_skill_loader(&run, &loader).await {
        Ok(output) => {
            let parsed: serde_json::Value = serde_json::from_str(&output)?;
            assert_eq!(parsed["backend"], "node-fallback");
            assert_eq!(parsed["reason_code"], "SKILL_FALLBACK_DENO_MISSING");
        }
        Err(err) => {
            let text = err.to_string();
            assert!(
                text.contains("SKILL_NODE_FALLBACK_FAILED")
                    || text.contains("SKILL_NODE_FALLBACK_PERMISSION_BLOCKED")
            );
        }
    }
    Ok(())
}
