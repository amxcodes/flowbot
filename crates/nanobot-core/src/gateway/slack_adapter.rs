/// Slack bot channel integration using direct HTTP API
/// This implementation uses Slack's Web API directly for maximum reliability
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use async_trait::async_trait;
use std::sync::Arc;
use super::adapter::{ChannelAdapter, ChannelMessage};
use super::registry::ChannelRegistry;

/// Slack bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub bot_token: String,      // xoxb-...
    pub app_token: Option<String>,      // xapp-... (for Socket Mode, optional)
}

/// Slack message event from Events API
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SlackEventWrapper {
    #[serde(rename = "type")]
    event_type: String,
    event: Option<SlackEvent>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    user: Option<String>,
    text: Option<String>,
    channel: Option<String>,
    bot_id: Option<String>,
}

/// Slack bot instance using Gateway Registry pattern
pub struct SlackBot {
    config: SlackConfig,
    #[allow(dead_code)]
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
    http_client: reqwest::Client,
}

impl SlackBot {
    /// Create a new Slack bot
    pub fn new(
        config: SlackConfig,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        registry: Arc<ChannelRegistry>,
    ) -> Self {
        Self {
            config,
            agent_tx,
            registry,
            http_client: reqwest::Client::new(),
        }
    }

    /// Send a message to Slack using Web API
    async fn post_message(&self, channel: &str, text: &str) -> Result<()> {
        let url = "https://slack.com/api/chat.postMessage";
        
        let response = self.http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&json!({
                "channel": channel,
                "text": text,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Slack API error: {}", response.status()));
        }

        Ok(())
    }

    /// Handle incoming Slack event
    #[allow(dead_code)]
    async fn handle_event(&self, event: SlackEvent) -> Result<()> {
        // Skip bot messages
        if event.bot_id.is_some() {
            return Ok(());
        }

        let user_id = event.user.unwrap_or_default();
        let channel_id = event.channel.unwrap_or_default();
        let text = event.text.unwrap_or_default();

        if text.is_empty() || user_id.is_empty() || channel_id.is_empty() {
            return Ok(());
        }

        // Pairing Logic
        match crate::pairing::is_authorized("slack", &user_id).await {
            Ok(authorized) => {
                if !authorized {
                    match crate::pairing::get_user_code("slack", &user_id).await {
                        Ok(Some(code)) => {
                            self.post_message(&channel_id, &format!("⏳ Pending authorization. Code: **{}**", code)).await?;
                        }
                        Ok(None) => {
                            if let Ok(code) = crate::pairing::create_pairing_request("slack", user_id.clone(), None).await {
                                self.post_message(&channel_id, &format!("🔐 Authorization Code: **{}**\n\nRun `nanobot pair slack {}` to authorize", code, code)).await?;
                            }
                        }
                        _ => {}
                    }
                    return Ok(());
                }
            }
            Err(_) => return Ok(()),
        }

        // Forward to AgentLoop
        let (response_tx, mut response_rx) = mpsc::channel(100);
        let agent_msg = crate::agent::AgentMessage {
            session_id: format!("slack:{}", channel_id),
            tenant_id: format!("slack:{}", channel_id), // Use Channel ID as Tenant ID
            content: text,
            response_tx,
        };

        if self.agent_tx.send(agent_msg).await.is_err() {
            self.post_message(&channel_id, "❌ Agent service unavailable").await?;
            return Ok(());
        }

        // Collect streaming response
        let mut full_response = String::new();
        while let Some(chunk) = response_rx.recv().await {
            if let crate::agent::StreamChunk::TextDelta(delta) = chunk {
                full_response.push_str(&delta);
            }
        }

        if !full_response.is_empty() {
            self.post_message(&channel_id, &full_response).await?;
        }

        Ok(())
    }

    /// Start the bot with dual-task architecture
    /// Note: This requires an HTTP endpoint for Events API
    /// For production, you would set up an Axum/Actix server to receive events
    pub async fn run(self) -> Result<()> {
        let registry = self.registry.clone();

        // 1. Create Inbox and register with Gateway Registry
        let (inbox_tx, mut inbox_rx) = mpsc::channel::<ChannelMessage>(100);
        registry.register("slack", inbox_tx).await;
        tracing::info!("✅ Slack adapter registered with Gateway Registry");

        // 2. Spawn Outbound Handler (Inbox -> Slack HTTP API)
        let http_client = self.http_client.clone();
        let bot_token = self.config.bot_token.clone();
        tokio::spawn(async move {
            tracing::info!("📤 Slack Outbound Actor started");
            
            while let Some(msg) = inbox_rx.recv().await {
                let channel = msg.user_id.replace("slack:", "");
                
                let url = "https://slack.com/api/chat.postMessage";
                let result = http_client
                    .post(url)
                    .header("Authorization", format!("Bearer {}", bot_token))
                    .header("Content-Type", "application/json")
                    .json(&json!({
                        "channel": channel,
                        "text": msg.content,
                    }))
                    .send()
                    .await;

                if let Err(e) = result {
                    tracing::error!("Failed to send Slack message: {:?}", e);
                }
            }
        });

        // 3. Event listener stub
        // TODO: Set up HTTP endpoint to receive Slack Events API callbacks
        // This would typically be done via Axum in the main server
        tracing::info!("📥 Slack adapter ready (Events API endpoint required)");
        tracing::warn!("Set up POST /slack/events endpoint to forward events to this adapter");

        // Keep alive
        tokio::signal::ctrl_c().await?;
        
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for SlackBot {
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        let channel = user_id.replace("slack:", "");
        self.post_message(&channel, content).await
    }

    async fn send_stream_chunk(&self, _user_id: &str, _chunk: &str) -> Result<()> {
        Ok(())
    }

    fn channel_name(&self) -> &str {
        "slack"
    }

    fn format_user_id(&self, raw_id: &str) -> String {
        format!("slack:{}", raw_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_config() {
        let config = SlackConfig {
            bot_token: "xoxb-test".to_string(),
            app_token: Some("xapp-test".to_string()),
        };
        assert!(config.bot_token.starts_with("xoxb-"));
    }
}
