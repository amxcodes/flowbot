use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

/// Tool for spawning subagents
pub struct SpawnSubagentTool {
    agent_manager: Arc<crate::gateway::agent_manager::AgentManager>,
}

impl SpawnSubagentTool {
    pub fn new(agent_manager: Arc<crate::gateway::agent_manager::AgentManager>) -> Self {
        Self { agent_manager }
    }
}

#[async_trait]
impl super::definitions::Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a child agent to handle a specific sub-task independently"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "spawn_subagent",
            "description": "Spawn a child agent to handle a specific sub-task independently. The child agent will run in isolation and report back when complete.",
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "The task description for the subagent to execute"
                    },
                    "parent_session_id": {
                        "type": "string",
                        "description": "The current agent's session ID"
                    }
                },
                "required": ["task", "parent_session_id"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let task = args["task"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'task' parameter"))?
            .to_string();

        let parent_session_id = args["parent_session_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'parent_session_id' parameter"))?
            .to_string();

        // Spawn the subagent
        let (session, task_obj) = self
            .agent_manager
            .spawn_subagent(
                parent_session_id,
                task.clone(),
                None,
                crate::gateway::agent_manager::CleanupPolicy::Delete,
            )
            .await?;

        Ok(json!({
            "session_id": session.id,
            "task_id": task_obj.id,
            "status": "pending",
            "message": format!("Subagent spawned successfully to handle: {}", task)
        })
        .to_string())
    }
}

/// Tool for getting subagent results
pub struct GetSubagentResultTool {
    agent_manager: Arc<crate::gateway::agent_manager::AgentManager>,
}

impl GetSubagentResultTool {
    pub fn new(agent_manager: Arc<crate::gateway::agent_manager::AgentManager>) -> Self {
        Self { agent_manager }
    }
}

#[async_trait]
impl super::definitions::Tool for GetSubagentResultTool {
    fn name(&self) -> &str {
        "get_subagent_result"
    }

    fn description(&self) -> &str {
        "Check the status and retrieve the result of a subagent task"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "get_subagent_result",
            "description": "Check the status and retrieve the result of a subagent task",
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": {
                        "type": "string",
                        "description": "The subagent's session ID"
                    }
                },
                "required": ["session_id"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let session_id = args["session_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'session_id' parameter"))?;

        // Get the task for this session
        let task = self
            .agent_manager
            .get_task_by_session(session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("No task found for session ID: {}", session_id))?;

        let status_str = match task.status {
            crate::gateway::agent_manager::TaskStatus::Pending => "pending",
            crate::gateway::agent_manager::TaskStatus::Running => "running",
            crate::gateway::agent_manager::TaskStatus::Completed => "completed",
            crate::gateway::agent_manager::TaskStatus::Failed => "failed",
        };

        Ok(json!({
            "task_id": task.id,
            "session_id": task.session_id,
            "status": status_str,
            "result": task.result,
            "task": task.task,
        })
        .to_string())
    }
}

/// Tool for listing all subagents
pub struct ListSubagentsTool {
    agent_manager: Arc<crate::gateway::agent_manager::AgentManager>,
}

impl ListSubagentsTool {
    pub fn new(agent_manager: Arc<crate::gateway::agent_manager::AgentManager>) -> Self {
        Self { agent_manager }
    }
}

#[async_trait]
impl super::definitions::Tool for ListSubagentsTool {
    fn name(&self) -> &str {
        "list_subagents"
    }

    fn description(&self) -> &str {
        "List all child agents spawned by the current agent"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "list_subagents",
            "description": "List all child agents spawned by the current agent",
            "parameters": {
                "type": "object",
                "properties": {
                    "parent_session_id": {
                        "type": "string",
                        "description": "The current agent's session ID"
                    }
                },
                "required": ["parent_session_id"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let parent_session_id = args["parent_session_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'parent_session_id' parameter"))?;

        // Get all children
        let children = self.agent_manager.get_children(parent_session_id).await;

        let mut subagents = Vec::new();
        for child_id in children {
            if let Some(task) = self.agent_manager.get_task_by_session(&child_id).await {
                let status_str = match task.status {
                    crate::gateway::agent_manager::TaskStatus::Pending => "pending",
                    crate::gateway::agent_manager::TaskStatus::Running => "running",
                    crate::gateway::agent_manager::TaskStatus::Completed => "completed",
                    crate::gateway::agent_manager::TaskStatus::Failed => "failed",
                };

                subagents.push(json!({
                    "session_id": child_id,
                    "task_id": task.id,
                    "task": task.task,
                    "status": status_str,
                }));
            }
        }

        Ok(json!({
            "count": subagents.len(),
            "subagents": subagents
        })
        .to_string())
    }
}
