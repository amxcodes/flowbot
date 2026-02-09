/// Discord bot channel integration using Twilight (Production Grade)
/// This implementation uses Twilight's Gateway and HTTP clients for robust handling
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use async_trait::async_trait;
use std::sync::Arc;
use super::adapter::{ChannelAdapter, ChannelMessage};
use super::registry::ChannelRegistry;
use futures::StreamExt;

// Twilight Imports
use twilight_gateway::{Shard, ShardId, Event, Intents};
use twilight_http::Client as HttpClient;
use twilight_model::id::Id;
use twilight_model::id::marker::ChannelMarker;

/// Discord bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub application_id: u64,
}

/// Discord bot instance using Gateway Registry pattern
pub struct DiscordBot {
    config: DiscordConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
    http_client: Arc<HttpClient>,
}

impl DiscordBot {
    /// Create a new Discord bot
    pub fn new(
        config: DiscordConfig,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        registry: Arc<ChannelRegistry>,
    ) -> Self {
        let http_client = Arc::new(HttpClient::new(config.token.clone()));
        
        Self {
            config,
            agent_tx,
            registry,
            http_client,
        }
    }

    /// Send a message to Discord using Twilight HTTP Client
    async fn post_message(&self, channel_id: Id<ChannelMarker>, content: &str) -> Result<()> {
        // Discord has 2000 char limit, split if needed
        let chunks: Vec<String> = content
            .chars()
            .collect::<Vec<_>>()
            .chunks(1900)
            .map(|c| c.iter().collect::<String>())
            .collect();

        let chunk_count = chunks.len();
        
        for chunk in chunks {
            self.http_client
                .create_message(channel_id)
                .content(&chunk)?
                .await?;

            // Rate limiting: wait 200ms between chunks (conservative)
            if chunk_count > 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }
        }

        Ok(())
    }

    /// Handle incoming Discord message
    async fn handle_message(&self, msg: twilight_model::gateway::payload::incoming::MessageCreate) -> Result<()> {
        // Skip bot messages
        if msg.author.bot {
            return Ok(());
        }

        let user_id = msg.author.id.to_string();
        let channel_id = msg.channel_id;
        let text = msg.content.clone();

        if text.is_empty() {
            return Ok(());
        }
        
        // --- Pairing Authorization Logic ---
        match crate::pairing::is_authorized("discord", &user_id).await {
            Ok(authorized) => {
                if !authorized {
                    match crate::pairing::get_user_code("discord", &user_id).await {
                        Ok(Some(code)) => {
                            self.post_message(channel_id, &format!("⏳ Pending authorization. Code: **{}**", code)).await?;
                        }
                        Ok(None) => {
                            let username = Some(msg.author.name.clone());
                            if let Ok(code) = crate::pairing::create_pairing_request("discord", user_id.clone(), username).await {
                                self.post_message(channel_id, &format!("🔐 Authorization Code: **{}**\n\nRun `nanobot pair discord {}` to authorize", code, code)).await?;
                            }
                        }
                        _ => {}
                    }
                    return Ok(());
                }
            }
            Err(_) => return Ok(()),
        }
        // -----------------------------------

        // Forward to AgentLoop
        let (response_tx, mut response_rx) = mpsc::channel(100);
        let agent_msg = crate::agent::AgentMessage {
            session_id: format!("discord:{}", channel_id),
            tenant_id: format!("discord:{}", channel_id),
            content: text,
            response_tx,
        };

        if self.agent_tx.send(agent_msg).await.is_err() {
            self.post_message(channel_id, "❌ Agent service unavailable").await?;
            return Ok(());
        }

        // Collect streaming response
        let mut full_response = String::new();
        while let Some(chunk) = response_rx.recv().await {
            match chunk {
                crate::agent::StreamChunk::TextDelta(delta) => {
                    full_response.push_str(&delta);
                }
                _ => {} // Ignore other chunks for basic implementation
            }
        }

        if !full_response.is_empty() {
            self.post_message(channel_id, &full_response).await?;
        }

        Ok(())
    }

    /// Run the Discord Gateway and Outbound Handler
    pub async fn run(self) -> Result<()> {
        let registry = self.registry.clone();

        // 1. Create Inbox and register with Gateway Registry (for outbound)
        let (inbox_tx, mut inbox_rx) = mpsc::channel::<ChannelMessage>(100);
        registry.register("discord", inbox_tx).await;
        tracing::info!("✅ Discord adapter registered");

        // 2. Start Gateway Shard (Inbound)
        let intents = Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES | Intents::MESSAGE_CONTENT;
        let mut shard = Shard::new(ShardId::ONE, self.config.token.clone(), intents);

        // 3. Spawn Outbound Handler
        let http_client = self.http_client.clone();
        tokio::spawn(async move {
            tracing::info!("📤 Discord Outbound Actor started");
            while let Some(msg) = inbox_rx.recv().await {
                // Parse channel_id from "discord:123456"
                let raw_id = msg.user_id.replace("discord:", "");
                if let Ok(channel_id_u64) = raw_id.parse::<u64>() {
                    let channel_id = Id::<ChannelMarker>::new(channel_id_u64);
                    
                    // Simple send (could be improved with splitting)
                    let _ = http_client.create_message(channel_id)
                        .content(&msg.content)
                        .unwrap()
                        .await;
                }
            }
        });

        tracing::info!("📥 Discord Gateway connecting...");

        // 4. Gateway Event Loop
        loop {
            let event = match shard.next_event().await {
                Ok(event) => event,
                Err(source) => {
                    tracing::warn!("Gateway error: {:?}", source);
                    if source.is_fatal() {
                        break;
                    }
                    continue;
                }
            };

            match event {
                Event::MessageCreate(msg) => {
                    if let Err(e) = self.handle_message(*msg).await {
                        tracing::error!("Failed to handle Discord message: {:?}", e);
                    }
                }
                Event::Ready(_) => {
                    tracing::info!("✅ Discord Gateway READY");
                }
                _ => {}
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for DiscordBot {
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        let raw_id = user_id.replace("discord:", "");
        if let Ok(channel_id_u64) = raw_id.parse::<u64>() {
            let channel_id = Id::<ChannelMarker>::new(channel_id_u64);
            self.post_message(channel_id, content).await
        } else {
             Err(anyhow::anyhow!("Invalid channel ID format"))
        }
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
