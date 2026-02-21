use super::adapter::{ChannelAdapter, ChannelMessage, build_session_id};
use super::registry::ChannelRegistry;
/// Discord bot channel integration using Twilight (Production Grade)
/// This implementation uses Twilight's Gateway and HTTP clients for robust handling
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

// Twilight Imports
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt};
use twilight_http::Client as HttpClient;
use twilight_model::id::Id;
use twilight_model::id::marker::ChannelMarker;

/// Discord bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub application_id: u64,
    #[serde(default)]
    pub dm_scope: crate::config::DmScope,
}

/// Discord bot instance using Gateway Registry pattern
pub struct DiscordBot {
    config: DiscordConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
    http_client: Arc<HttpClient>,
    confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    confirmation_txs: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<u64, mpsc::Sender<crate::tools::ChannelConfirmationResponse>>,
        >,
    >,
    confirmation_ready: Arc<tokio::sync::Mutex<std::collections::HashSet<u64>>>,
    pending_questions: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<String, crate::tools::question::QuestionPayload>,
        >,
    >,
    confirmation_outbound_tx: mpsc::Sender<ChannelMessage>,
}

impl DiscordBot {
    /// Create a new Discord bot
    pub fn new(
        config: DiscordConfig,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        registry: Arc<ChannelRegistry>,
        confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    ) -> Self {
        let http_client = Arc::new(HttpClient::new(config.token.clone()));

        Self {
            config,
            agent_tx,
            registry,
            http_client,
            confirmation_service,
            confirmation_txs: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            confirmation_ready: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            pending_questions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            confirmation_outbound_tx: {
                let (tx, _rx) = mpsc::channel(1);
                tx
            },
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
                .content(&chunk)
                .await?;

            // Rate limiting: wait 200ms between chunks (conservative)
            if chunk_count > 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }
        }

        Ok(())
    }

    /// Handle incoming Discord message
    async fn handle_message(
        &self,
        msg: twilight_model::gateway::payload::incoming::MessageCreate,
    ) -> Result<()> {
        let ingress_at = std::time::Instant::now();
        // Skip bot messages
        if msg.author.bot {
            return Ok(());
        }

        let user_id = msg.author.id.to_string();
        let channel_id = msg.channel_id;
        let mut text = msg.content.clone();

        if text.is_empty() {
            return Ok(());
        }

        self.ensure_confirmation_adapter(channel_id.get()).await;

        if let Some((allowed, request_id)) = parse_confirmation_response(&text) {
            let tx = {
                let txs = self.confirmation_txs.lock().await;
                txs.get(&channel_id.get()).cloned()
            };

            if let Some(sender) = tx {
                let _ = sender
                    .send(crate::tools::ChannelConfirmationResponse {
                        request_id,
                        allowed,
                    })
                    .await;
                let _ = self
                    .post_message(channel_id, "Confirmation received.")
                    .await;
            } else {
                let _ = self
                    .post_message(channel_id, "No pending confirmation.")
                    .await;
            }
            return Ok(());
        }

        // --- Pairing Authorization Logic ---
        match crate::pairing::is_authorized("discord", &user_id).await {
            Ok(authorized) => {
                if !authorized {
                    match crate::pairing::get_user_code("discord", &user_id).await {
                        Ok(Some(code)) => {
                            self.post_message(
                                channel_id,
                                &format!("⏳ Pending authorization. Code: **{}**", code),
                            )
                            .await?;
                        }
                        Ok(None) => {
                            let username = Some(msg.author.name.clone());
                            if let Ok(code) = crate::pairing::create_pairing_request(
                                "discord",
                                user_id.clone(),
                                username,
                            )
                            .await
                            {
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

        if let Some(token) = text.strip_prefix("/set_admin_token ") {
            let parts: Vec<&str> = token.split_whitespace().collect();
            let has_existing_admin = std::env::var("NANOBOT_ADMIN_TOKEN")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .is_some()
                || crate::security::read_admin_token()
                    .ok()
                    .flatten()
                    .map(|v| !v.trim().is_empty())
                    .unwrap_or(false);

            if !has_existing_admin {
                let new_token = parts.first().map(|s| s.trim()).unwrap_or("");
                if new_token.is_empty() {
                    let _ = self
                        .post_message(channel_id, "First-time setup: /set_admin_token <new_token>")
                        .await;
                    return Ok(());
                }
                if let Err(e) = crate::security::write_admin_token(new_token) {
                    let _ = self
                        .post_message(channel_id, &format!("Failed to save token: {}", e))
                        .await;
                    return Ok(());
                }
                let _ = self
                    .post_message(channel_id, "Admin token saved (first-time setup)")
                    .await;
                return Ok(());
            }

            if parts.len() < 2 {
                let _ = self
                    .post_message(
                        channel_id,
                        "Usage: /set_admin_token <current_token_or_primary_password> <new_token>. Example: /set_admin_token mypassword newtoken123",
                    )
                    .await;
                return Ok(());
            }
            let current = parts[0].trim();
            let new_token = parts[1].trim();
            if new_token.is_empty() {
                let _ = self
                    .post_message(channel_id, "New token cannot be empty")
                    .await;
                return Ok(());
            }

            if !crate::security::verify_admin_rotation_secret(current) {
                let _ = self
                    .post_message(channel_id, "Current token/password is invalid")
                    .await;
                return Ok(());
            }

            if let Err(e) = crate::security::write_admin_token(new_token) {
                let _ = self
                    .post_message(channel_id, &format!("Failed to save token: {}", e))
                    .await;
                return Ok(());
            }
            let _ = self.post_message(channel_id, "Admin token saved").await;
            return Ok(());
        }

        match crate::gateway::onboarding::process_onboarding_message("discord", &user_id, &text)
            .await
        {
            Ok(crate::gateway::onboarding::OnboardingOutcome::ReplyOnly(reply)) => {
                let _ = self.post_message(channel_id, &reply).await;
                return Ok(());
            }
            Ok(crate::gateway::onboarding::OnboardingOutcome::NotNeeded) => {}
            Err(e) => {
                let _ = self
                    .post_message(channel_id, &format!("Setup error: {}", e))
                    .await;
                return Ok(());
            }
        }

        let skill_scope = format!("discord:{}:{}", channel_id.get(), user_id);
        match crate::gateway::skill_chat::handle_skill_slash_command(&skill_scope, &text).await {
            Ok(Some(reply)) => {
                let _ = self.post_message(channel_id, &reply).await;
                return Ok(());
            }
            Ok(None) => {}
            Err(e) => {
                let _ = self
                    .post_message(channel_id, &format!("Skill command error: {}", e))
                    .await;
                return Ok(());
            }
        }

        let is_dm = msg.guild_id.is_none();
        let session_id = build_session_id(
            "discord",
            &channel_id.get().to_string(),
            &user_id,
            self.config.dm_scope,
            is_dm,
        );

        if let Some(pending) = {
            let pending_map = self.pending_questions.lock().await;
            pending_map.get(&session_id).cloned()
        } {
            match crate::tools::question::normalize_question_answer(&pending, &text) {
                Ok(normalized) => {
                    text = normalized;
                    let mut pending_map = self.pending_questions.lock().await;
                    pending_map.remove(&session_id);
                }
                Err(err_msg) => {
                    let prompt = crate::tools::question::format_question_prompt(&pending);
                    let _ = self
                        .post_message(channel_id, &format!("{}\n{}", err_msg, prompt))
                        .await;
                    return Ok(());
                }
            }
        }

        // Forward to AgentLoop
        let (response_tx, mut response_rx) = mpsc::channel(100);
        let agent_msg = crate::agent::AgentMessage {
            session_id: session_id.clone(),
            tenant_id: session_id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            content: text,
            response_tx,
            ingress_at,
        };

        if self.agent_tx.send(agent_msg).await.is_err() {
            self.post_message(channel_id, "❌ Agent service unavailable")
                .await?;
            return Ok(());
        }

        // Collect streaming response
        let mut full_response = String::new();
        while let Some(chunk) = response_rx.recv().await {
            match chunk {
                crate::agent::StreamChunk::TextDelta(delta) => {
                    full_response.push_str(&delta);
                }
                crate::agent::StreamChunk::ToolResult(result) => {
                    if let Some(payload) = crate::tools::question::parse_question_payload(&result) {
                        let prompt = crate::tools::question::format_question_prompt(&payload);
                        {
                            let mut pending_map = self.pending_questions.lock().await;
                            pending_map.insert(session_id.clone(), payload);
                        }
                        let _ = self.post_message(channel_id, &prompt).await;
                    }
                }
                _ => {}
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
        registry.register("discord", inbox_tx.clone()).await;
        tracing::info!("✅ Discord adapter registered");

        let mut bot = self;
        bot.confirmation_outbound_tx = inbox_tx.clone();

        // 2. Start Gateway Shard (Inbound)
        let intents = Intents::GUILD_MESSAGES | Intents::DIRECT_MESSAGES | Intents::MESSAGE_CONTENT;
        let mut shard = Shard::new(ShardId::ONE, bot.config.token.clone(), intents);

        // 3. Spawn Outbound Handler
        let http_client = bot.http_client.clone();
        tokio::spawn(async move {
            tracing::info!("📤 Discord Outbound Actor started");
            while let Some(msg) = inbox_rx.recv().await {
                // Parse channel_id from "discord:123456"
                let raw_id = msg.user_id.replace("discord:", "");
                if let Ok(channel_id_u64) = raw_id.parse::<u64>() {
                    let channel_id = Id::<ChannelMarker>::new(channel_id_u64);

                    // Simple send (could be improved with splitting)
                    let _ = http_client
                        .create_message(channel_id)
                        .content(&msg.content)
                        .await;
                }
            }
        });

        tracing::info!("📥 Discord Gateway connecting...");

        // 4. Gateway Event Loop
        loop {
            let event = match shard.next_event(EventTypeFlags::all()).await {
                Some(Ok(event)) => event,
                Some(Err(source)) => {
                    tracing::warn!("Gateway error: {:?}", source);
                    continue;
                }
                None => {
                    tracing::warn!("Discord gateway stream ended");
                    break;
                }
            };

            match event {
                Event::MessageCreate(msg) => {
                    if let Err(e) = bot.handle_message(*msg).await {
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

    async fn ensure_confirmation_adapter(&self, channel_id: u64) {
        let mut ready = self.confirmation_ready.lock().await;
        if ready.contains(&channel_id) {
            return;
        }

        let (response_tx, response_rx) = mpsc::channel(10);
        let channel = format!("discord:{}", channel_id);
        let adapter = crate::tools::channel_confirmation::ChannelConfirmationAdapter::new(
            channel,
            self.confirmation_outbound_tx.clone(),
            response_rx,
        );

        {
            let mut service = self.confirmation_service.lock().await;
            service.register_adapter(Box::new(adapter));
        }

        let mut txs = self.confirmation_txs.lock().await;
        txs.insert(channel_id, response_tx);
        ready.insert(channel_id);
    }
}

fn parse_confirmation_response(text: &str) -> Option<(bool, String)> {
    let trimmed = text.trim();
    let (allowed, rest) = if let Some(rest) = trimmed.strip_prefix("/allow ") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("/deny ") {
        (false, rest)
    } else {
        return None;
    };

    let request_id = rest.trim();
    if request_id.is_empty() {
        None
    } else {
        Some((allowed, request_id.to_string()))
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
