use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::gateway::agent_manager::{AgentManager, CleanupPolicy};

/// Actions for the sessions tool
pub enum SessionAction {
    Spawn,
    List,
    Status,
    Send,
    History,
}

/// Execute a sessions tool call
pub async fn execute_sessions_tool(
    agent_manager: &AgentManager,
    tool_call: &Value,
) -> Result<String> {
    let action_str = tool_call["action"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'action' field"))?;

    let action = match action_str {
        "spawn" => SessionAction::Spawn,
        "list" => SessionAction::List,
        "status" => SessionAction::Status,
        "send" => SessionAction::Send,
        "history" => SessionAction::History,
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

            // For now, we'll assume parent session is "main"
            // In a full implementation, this would be passed from the agent loop
            let parent_session_id = tool_call["parentSessionId"]
                .as_str()
                .unwrap_or("main")
                .to_string();

            let (session, task_obj) = agent_manager
                .spawn_subagent(parent_session_id, task, label, cleanup, model)
                .await?;

            Ok(serde_json::to_string(&json!({
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
            let session_id = tool_call["sessionId"]
                .as_str()
                .ok_or_else(|| anyhow!("Missing 'sessionId' field"))?;

            let session = agent_manager
                .get_session(session_id)
                .await
                .ok_or_else(|| anyhow!("Session not found: {}", session_id))?;

            Ok(serde_json::to_string(&json!({ "session": session }))?)
        }
        _ => {
            // Stub for unimplemented actions
            Err(anyhow!("Action '{}' not yet implemented", action_str))
        }
    }
}
