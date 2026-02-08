use crate::cron::{CronJob, Payload};
use crate::gateway::agent_manager::{AgentManager, CleanupPolicy};
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

    let (session, _task) = agent_manager
        .spawn_subagent(
            "cron-root".to_string(),
            message.clone(),
            Some(label),
            CleanupPolicy::Delete,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to spawn subagent: {}", e))?;

    // Execute agent with timeout
    let timeout = std::time::Duration::from_secs(timeout_seconds.unwrap_or(120));

    match tokio::time::timeout(
        timeout,
        execute_agent_message(&session.id, &message, model_override),
    )
    .await
    {
        Ok(Ok(output)) => Ok(IsolatedAgentResult {
            status: "ok".to_string(),
            summary: Some(format!("Executed in session {}", session.id)),
            output_text: Some(output),
            error: None,
        }),
        Ok(Err(e)) => Ok(IsolatedAgentResult {
            status: "error".to_string(),
            summary: Some("Agent execution failed".to_string()),
            output_text: None,
            error: Some(e.to_string()),
        }),
        Err(_) => Ok(IsolatedAgentResult {
            status: "error".to_string(),
            summary: Some("Agent execution timed out".to_string()),
            output_text: None,
            error: Some(format!("Timeout after {:?}", timeout)),
        }),
    }
}

/// Execute agent message and collect response (simplified, no tools)
async fn execute_agent_message(
    session_id: &str,
    message: &str,
    _model_override: Option<String>,
) -> Result<String> {
    use crate::agent::AgentLoop;
    use futures::StreamExt;
    use rig::OneOrMany;
    use rig::completion::message::{Text, UserContent};
    use rig::completion::{CompletionRequest, Message};
    use rig::streaming::StreamedAssistantContent;

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
            Ok(StreamedAssistantContent::Text(text)) => {
                response_text.push_str(&text.text);
            }
            Ok(_) => {} // Ignore other chunk types
            Err(e) => {
                eprintln!("Stream chunk error: {}", e);
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
