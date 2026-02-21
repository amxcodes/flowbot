use anyhow::{Result, anyhow};
use rig::OneOrMany;
use rig::completion::message::{Text, UserContent};
use rig::completion::{CompletionRequest, Message};
use serde_json::Value;
use tokio_stream::StreamExt;

pub async fn execute_llm_task(args: &Value) -> Result<String> {
    let prompt = args["prompt"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'prompt' field"))?
        .to_string();

    let system = args["system"].as_str().map(|s| s.to_string());
    let temperature = args["temperature"].as_f64();
    let max_tokens = args["max_tokens"].as_u64();

    let config = crate::config::Config::load()?;
    let indices_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    let provider = crate::agent::AgentLoop::create_provider(&config, &indices_map).await?;

    let request = CompletionRequest {
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(Text { text: prompt })),
        }),
        preamble: system,
        max_tokens,
        temperature,
        tools: vec![],
        tool_choice: None,
        documents: vec![],
        additional_params: None,
    };

    let mut stream = provider.stream(request).await?;
    let mut output = String::new();
    while let Some(chunk_res) = stream.next().await {
        if let Ok(chunk) = chunk_res
            && let crate::agent::ProviderChunk::TextDelta(t) = chunk
        {
            output.push_str(&t);
        }
    }

    Ok(output)
}
