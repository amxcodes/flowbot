use crate::cron::{CronJob, Payload};
use crate::gateway::agent_manager::{AgentManager, CleanupPolicy, TaskStatus};
use anyhow::Result;

/// Result of running an isolated agent turn
#[derive(Debug, Clone)]
pub struct IsolatedAgentResult {
    pub status: String, // "ok", "error", "skipped"
    pub summary: Option<String>,
    pub output_text: Option<String>,
    pub error: Option<String>,
}

/// Run an isolated agent turn for a cron job
///
/// This spawns a new isolated agent session, executes the agent with the
/// provided message, and returns the result. The session is automatically
/// cleaned up after execution.
pub async fn run_isolated_agent_turn(
    job: &CronJob,
    agent_manager: &AgentManager,
    message: String,
) -> Result<IsolatedAgentResult> {
    // Extract AgentTurn payload
    let (model_override, _thinking, timeout_seconds) = match &job.payload {
        Payload::AgentTurn {
            model,
            thinking,
            timeout_seconds,
            ..
        } => (model.clone(), thinking.clone(), *timeout_seconds),
        _ => {
            return Ok(IsolatedAgentResult {
                status: "skipped".to_string(),
                summary: Some("Job payload is not AgentTurn".to_string()),
                output_text: None,
                error: Some("Expected AgentTurn payload".to_string()),
            });
        }
    };

    // Spawn isolated session
    let label = job
        .name
        .clone()
        .unwrap_or_else(|| format!("cron-{}", job.id));

    let (session, task) = agent_manager
        .spawn_subagent(
            "cron-root".to_string(),
            message.clone(),
            Some(label),
            CleanupPolicy::Delete,
            model_override.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn subagent: {}", e))?;

    let _ = agent_manager
        .update_task_status(&task.id, TaskStatus::Running, None)
        .await;

    let _ = agent_manager
        .broadcast_to_parent(
            &session.id,
            format!("[Subagent {}] Thinking...", session.id),
        )
        .await;

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let progress_manager = agent_manager.clone();
    let progress_session_id = session.id.clone();
    let progress_handle = tokio::spawn(async move {
        let mut buffer = String::new();
        let mut last_flush = std::time::Instant::now();

        while let Some(chunk) = progress_rx.recv().await {
            buffer.push_str(&chunk);
            if buffer.len() >= 256 || last_flush.elapsed() >= std::time::Duration::from_secs(2) {
                let snippet = truncate_utf8(&buffer, 512);
                if !snippet.is_empty() {
                    let _ = progress_manager
                        .broadcast_to_parent(
                            &progress_session_id,
                            format!("[Subagent {}] Progress: {}", progress_session_id, snippet),
                        )
                        .await;
                }
                buffer.clear();
                last_flush = std::time::Instant::now();
            }
        }

        if !buffer.is_empty() {
            let snippet = truncate_utf8(&buffer, 512);
            let _ = progress_manager
                .broadcast_to_parent(
                    &progress_session_id,
                    format!("[Subagent {}] Progress: {}", progress_session_id, snippet),
                )
                .await;
        }
    });

    // Execute agent with timeout
    let timeout = std::time::Duration::from_secs(timeout_seconds.unwrap_or(120));

    let exec_progress_tx = progress_tx.clone();
    match tokio::time::timeout(
        timeout,
        execute_agent_message(
            &session.id,
            &message,
            model_override,
            Some(exec_progress_tx),
        ),
    )
    .await
    {
        Ok(Ok(output)) => {
            drop(progress_tx);
            let _ = progress_handle.await;
            let summary = truncate_utf8(&output, 2000);
            let _ = agent_manager
                .update_task_status(&task.id, TaskStatus::Completed, Some(output.clone()))
                .await;
            let _ = agent_manager
                .broadcast_to_parent(
                    &session.id,
                    format!("[Subagent {}] Completed\n{}", session.id, summary),
                )
                .await;
            Ok(IsolatedAgentResult {
                status: "ok".to_string(),
                summary: Some(format!("Executed in session {}", session.id)),
                output_text: Some(output),
                error: None,
            })
        }
        Ok(Err(e)) => {
            drop(progress_tx);
            let _ = progress_handle.await;
            let _ = agent_manager
                .update_task_status(&task.id, TaskStatus::Failed, Some(e.to_string()))
                .await;
            let _ = agent_manager
                .broadcast_to_parent(
                    &session.id,
                    format!("[Subagent {}] Failed: {}", session.id, e),
                )
                .await;
            Ok(IsolatedAgentResult {
                status: "error".to_string(),
                summary: Some("Agent execution failed".to_string()),
                output_text: None,
                error: Some(e.to_string()),
            })
        }
        Err(_) => {
            drop(progress_tx);
            let _ = progress_handle.await;
            let timeout_msg = format!("Timeout after {:?}", timeout);
            let _ = agent_manager
                .update_task_status(&task.id, TaskStatus::Failed, Some(timeout_msg.clone()))
                .await;
            let _ = agent_manager
                .broadcast_to_parent(
                    &session.id,
                    format!("[Subagent {}] Failed: {}", session.id, timeout_msg),
                )
                .await;
            Ok(IsolatedAgentResult {
                status: "error".to_string(),
                summary: Some("Agent execution timed out".to_string()),
                output_text: None,
                error: Some(timeout_msg),
            })
        }
    }
}

/// Execute agent message and collect response (simplified, no tools)
pub async fn execute_agent_message(
    session_id: &str,
    message: &str,
    _model_override: Option<String>,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<String> {
    use crate::agent::AgentLoop;
    use futures::StreamExt;
    use rig::OneOrMany;
    use rig::completion::message::{Text, UserContent};
    use rig::completion::{CompletionRequest, Message};

    // Load Config and Create Provider
    // This ensures we respect the global configuration (provider selection, API keys, limits)
    let config = crate::config::Config::load()?;
    let indices = std::collections::HashMap::new(); // Default to first key for isolated runs
    let provider = AgentLoop::create_provider(&config, &indices).await?;

    // Build chat history with just the user message
    let chat_history = vec![Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: message.to_string(),
        })),
    }];

    // Simple system message (no tools for isolated execution)
    let system_msg = format!(
        "You are Flowbot, a helpful AI assistant executing a scheduled task.\nSession ID: {}",
        session_id
    );

    let request = CompletionRequest {
        chat_history: OneOrMany::many(chat_history).unwrap(),
        preamble: Some(system_msg),
        max_tokens: Some(2048), // Shorter for isolated tasks
        temperature: Some(0.7),
        tools: vec![], // No tools in isolated execution
        tool_choice: None,
        documents: vec![],
        additional_params: Some(serde_json::json!({})),
    };

    // Stream and collect response
    let mut stream = provider
        .stream(request)
        .await
        .map_err(|e| anyhow::anyhow!("Stream error: {}", e))?;

    let mut response_text = String::new();

    while let Some(chunk_res) = stream.next().await {
        match chunk_res {
            Ok(crate::agent::ProviderChunk::TextDelta(text)) => {
                response_text.push_str(&text);
                if let Some(tx) = progress_tx.as_ref() {
                    let _ = tx.send(text);
                }
            }
            Ok(crate::agent::ProviderChunk::ToolCall { .. }) => {
                tracing::warn!("isolated agent received unexpected tool call chunk")
            }
            Ok(crate::agent::ProviderChunk::Error(err)) => {
                tracing::warn!("isolated agent stream chunk error: {}", err);
            }
            Ok(crate::agent::ProviderChunk::End) => {}
            Err(e) => {
                tracing::warn!("Stream chunk error: {}", e);
            }
        }
    }

    Ok(response_text)
}

/// Truncate text to a maximum number of UTF-8 characters
pub fn truncate_utf8(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    text.chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_utf8() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
        assert_eq!(truncate_utf8("hello world", 5), "hell…");
        assert_eq!(truncate_utf8("🎉🎊🎈", 2), "🎉…");
    }
}
