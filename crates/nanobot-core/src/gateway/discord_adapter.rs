/// Discord bot channel integration using direct HTTP API
/// This implementation uses Discord's REST API directly for maximum reliability
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use async_trait::async_trait;
use std::sync::Arc;
use super::adapter::{ChannelAdapter, ChannelMessage};
use super::registry::ChannelRegistry;

/// Discord bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub application_id: u64,
}

/// Discord message event from Gateway
#[derive(Debug, Deserialize)]
struct DiscordMessage {
    id: String,
    channel_id: String,
    author: DiscordUser,
    content: String,
}

#[derive(Debug, Deserialize)]
struct DiscordUser {
    id: String,
    username: String,
    bot: Option<bool>,
}

/// Discord bot instance using Gateway Registry pattern
pub struct DiscordBot {
    config: DiscordConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
    http_client: reqwest::Client,
}

impl DiscordBot {
    /// Create a new Discord bot
    pub fn new(
        config: DiscordConfig,
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

    /// Send a message to Discord using REST API
    async fn post_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);
        
        // Discord has 2000 char limit, split if needed
        let chunks: Vec<String> = content
            .chars()
            .collect::<Vec<_>>()
            .chunks(1900)
            .map(|c| c.iter().collect::<String>())
            .collect();

        let chunk_count = chunks.len();
        
        for chunk in chunks {
            let response = self.http_client
                .post(&url)
                .header("Authorization", format!("Bot {}", self.config.token))
                .header("Content-Type", "application/json")
                .json(&json!({
                    "content": chunk,
                }))
                .send()
                .await?;

            if !response.status().is_success() {
                let error_text = response.text().await.unwrap_or_default();
                tracing::error!("Discord API error: {}", error_text);
                return Err(anyhow::anyhow!("Discord API error"));
            }

            // Rate limiting: wait 500ms between chunks
            if chunk_count > 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }

        Ok(())
    }

    /// Handle incoming Discord message
    async fn handle_message(&self, msg: DiscordMessage) -> Result<()> {
        // Skip bot messages
        if msg.author.bot.unwrap_or(false) {
            return Ok(());
        }

        let user_id = msg.author.id.clone();
        let channel_id = msg.channel_id.clone();
        let text = msg.content.clone();

        if text.is_empty() {
            return Ok(());
        }

        // Pairing Logic
        match crate::pairing::is_authorized("discord", &user_id).await {
            Ok(authorized) => {
                if !authorized {
                    match crate::pairing::get_user_code("discord", &user_id).await {
                        Ok(Some(code)) => {
                            self.post_message(&channel_id, &format!("⏳ Pending authorization. Code: **{}**", code)).await?;
                        }
                        Ok(None) => {
                            let username = Some(msg.author.username.clone());
                            if let Ok(code) = crate::pairing::create_pairing_request("discord", user_id.clone(), username).await {
                                self.post_message(&channel_id, &format!("🔐 Authorization Code: **{}**\n\nRun `nanobot pair discord {}` to authorize", code, code)).await?;
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
            session_id: format!("discord:{}", channel_id),
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
    /// Note: This requires a Discord Gateway WebSocket connection
    /// For production, implement WebSocket client to receive events
    pub async fn run(self) -> Result<()> {
        let registry = self.registry.clone();

        // 1. Create Inbox and register with Gateway Registry
        let (inbox_tx, mut inbox_rx) = mpsc::channel::<ChannelMessage>(100);
        registry.register("discord", inbox_tx).await;
        tracing::info!("✅ Discord adapter registered with Gateway Registry");

        // 2. Spawn Outbound Handler (Inbox -> Discord REST API)
        let http_client = self.http_client.clone();
        let bot_token = self.config.token.clone();
        tokio::spawn(async move {
            tracing::info!("📤 Discord Outbound Actor started");
            
            while let Some(msg) = inbox_rx.recv().await {
                let channel_id = msg.user_id.replace("discord:", "");
                let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_id);
                
                // Split messages if too long (2000 char limit)
                let chunks: Vec<String> = msg.content
                    .chars()
                    .collect::<Vec<_>>()
                    .chunks(1900)
                    .map(|c| c.iter().collect::<String>())
                    .collect();

                let chunk_count = chunks.len();

                for chunk in chunks {
                    let result = http_client
                        .post(&url)
                        .header("Authorization", format!("Bot {}", bot_token))
                        .header("Content-Type", "application/json")
                        .json(&json!({
                            "content": chunk,
                        }))
                        .send()
                        .await;

                    if let Err(e) = result {
                        tracing::error!("Failed to send Discord message: {:?}", e);
                    }

                    // Rate limiting
                    if chunk_count > 1 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    }
                }
            }
        });

        // 3. Gateway listener stub
        // TODO: Implement Discord Gateway WebSocket client
        // This requires handling Discord's Gateway protocol (heartbeats, resume, etc.)
        tracing::info!("📥 Discord adapter ready (Gateway WebSocket required)");
        tracing::warn!("Implement Discord Gateway WebSocket client for inbound messages");

        // Keep alive
        tokio::signal::ctrl_c().await?;
        
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for DiscordBot {
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        let channel_id = user_id.replace("discord:", "");
        self.post_message(&channel_id, content).await
    }

    async fn send_stream_chunk(&self, _user_id: &str, _chunk: &str) -> Result<()> {
        Ok(())
    }

    fn channel_name(&self) -> &str {
        "discord"
    }

    fn format_user_id(&self, raw_id: &str) -> String {
        format!("discord:{}", raw_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discord_config() {
        let config = DiscordConfig {
            token: "test_token".to_string(),
            application_id: 123456789,
        };
        assert_eq!(config.token, "test_token");
        assert_eq!(config.application_id, 123456789);
    }
}
