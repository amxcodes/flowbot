use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::gateway::agent_manager::{AgentManager, CleanupPolicy, SubagentOptions};

/// Actions for the sessions tool
pub enum SessionAction {
    Spawn,
    List,
    Status,
    Send,
    History,
    Cancel,
    Pause,
    Resume,
}

/// Execute a sessions tool call
pub async fn execute_sessions_tool(
    agent_manager: &AgentManager,
    tool_call: &Value,
) -> Result<String> {
    let action_str = tool_call["action"]
        .as_str()
        .or_else(|| match tool_call["tool"].as_str() {
            Some("sessions_spawn") | Some("spawn_subagent") => Some("spawn"),
            Some("sessions_list") | Some("list_subagents") => Some("list"),
            Some("session_status") | Some("get_subagent_result") => Some("status"),
            Some("sessions_send") => Some("send"),
            Some("sessions_history") => Some("history"),
            Some("sessions_cancel") => Some("cancel"),
            Some("sessions_pause") => Some("pause"),
            Some("sessions_resume") => Some("resume"),
            _ => None,
        })
        .ok_or_else(|| anyhow!("Missing 'action' field"))?;

    let action = match action_str {
        "spawn" => SessionAction::Spawn,
        "list" => SessionAction::List,
        "status" => SessionAction::Status,
        "send" => SessionAction::Send,
        "history" => SessionAction::History,
        "cancel" => SessionAction::Cancel,
        "pause" => SessionAction::Pause,
        "resume" => SessionAction::Resume,
        _ => return Err(anyhow!("Unknown action: {}", action_str)),
    };

    match action {
        SessionAction::Spawn => {
            let task = tool_call["task"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing 'task' field"))?
                .to_string();

            let label = tool_call["label"].as_str().map(|s| s.to_string());
            let model = tool_call["model"].as_str().map(|s| s.to_string());

            let cleanup = match tool_call["cleanup"].as_str() {
                Some("delete") => CleanupPolicy::Delete,
                _ => CleanupPolicy::Keep,
            };

            let max_retries = tool_call["max_retries"]
                .as_u64()
                .or_else(|| tool_call["maxRetries"].as_u64())
                .unwrap_or(0) as u32;
            let retry_backoff_ms = tool_call["retry_backoff_ms"]
                .as_u64()
                .or_else(|| tool_call["retryBackoffMs"].as_u64())
                .unwrap_or(1000);
            let timeout_seconds = tool_call["timeout_seconds"]
                .as_u64()
                .or_else(|| tool_call["timeoutSeconds"].as_u64())
                .unwrap_or(120);

            let parent_session_id = tool_call["parent_session_id"]
                .as_str()
                .or_else(|| tool_call["parentSessionId"].as_str())
                .or_else(|| tool_call["session_id"].as_str())
                .or_else(|| tool_call["sessionId"].as_str())
                .unwrap_or("main")
                .to_string();

            let (session, task_obj) = agent_manager
                .spawn_subagent_with_options(
                    parent_session_id,
                    task.clone(),
                    label,
                    cleanup,
                    model,
                    SubagentOptions {
                        max_retries,
                        retry_backoff_ms,
                        timeout_seconds,
                    },
                )
                .await?;

            Ok(serde_json::to_string(&json!({
                "session_id": session.id,
                "task_id": task_obj.id,
                "status": "pending",
                "message": format!("Subagent spawned for task: {}", task),
                "attempts": task_obj.attempts,
                "max_retries": task_obj.max_retries,
                "retry_backoff_ms": task_obj.retry_backoff_ms,
                "timeout_seconds": task_obj.timeout_seconds,
                "session": {
                    "id": session.id,
                    "type": session.session_type,
                    "parentSessionId": session.parent_session_id,
                },
                "task": {
                    "id": task_obj.id,
                    "status": task_obj.status,
                }
            }))?)
        }
        SessionAction::List => {
            let sessions = agent_manager.list_sessions().await;
            Ok(serde_json::to_string(&json!({ "sessions": sessions }))?)
        }
        SessionAction::Status => {
            let session_id = tool_call["session_id"]
                .as_str()
                .or_else(|| tool_call["sessionId"].as_str())
                .ok_or_else(|| anyhow!("Missing 'session_id' field"))?;

            let session = agent_manager
                .get_session(session_id)
                .await
                .ok_or_else(|| anyhow!("Session not found: {}", session_id))?;

            Ok(serde_json::to_string(&json!({ "session": session }))?)
        }
        SessionAction::Cancel => {
            let session_id = tool_call["session_id"]
                .as_str()
                .or_else(|| tool_call["sessionId"].as_str())
                .ok_or_else(|| anyhow!("Missing 'session_id' field"))?;

            agent_manager.cancel_session(session_id).await?;

            Ok(serde_json::to_string(&json!({
                "status": "cancelled",
                "session_id": session_id,
            }))?)
        }
        SessionAction::Pause => {
            let session_id = tool_call["session_id"]
                .as_str()
                .or_else(|| tool_call["sessionId"].as_str())
                .ok_or_else(|| anyhow!("Missing 'session_id' field"))?;

            agent_manager.pause_session(session_id).await?;

            Ok(serde_json::to_string(&json!({
                "status": "paused",
                "session_id": session_id,
            }))?)
        }
        SessionAction::Resume => {
            let session_id = tool_call["session_id"]
                .as_str()
                .or_else(|| tool_call["sessionId"].as_str())
                .ok_or_else(|| anyhow!("Missing 'session_id' field"))?;

            agent_manager.resume_session(session_id).await?;

            Ok(serde_json::to_string(&json!({
                "status": "resumed",
                "session_id": session_id,
            }))?)
        }
        _ => {
            // Stub for unimplemented actions
            Err(anyhow!("Action '{}' not yet implemented", action_str))
        }
    }
}
