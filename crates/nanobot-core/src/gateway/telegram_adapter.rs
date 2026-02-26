use super::adapter::{ChannelAdapter, ChannelMessage, build_session_id};
use super::registry::ChannelRegistry;
/// Telegram bot channel integration
/// Uses HTTP polling to receive messages from Telegram
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

/// Telegram bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub token: String,
    pub allowed_users: Option<Vec<i64>>,
    #[serde(default)]
    pub dm_scope: crate::config::DmScope,
}

/// Telegram bot instance (Refactored to use Actor Model)
pub struct TelegramBot {
    bot: Bot,
    config: TelegramConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
    confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    confirmation_txs: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<
                i64,
                mpsc::Sender<crate::tools::telegram_confirmation::CallbackResponse>,
            >,
        >,
    >,
    confirmation_ready: Arc<tokio::sync::Mutex<std::collections::HashSet<i64>>>,
    pending_questions: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<String, crate::tools::question::QuestionPayload>,
        >,
    >,
}

impl TelegramBot {
    /// Create a new Telegram bot
    pub fn new(
        config: TelegramConfig,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        registry: Arc<ChannelRegistry>,
        confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    ) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            bot,
            config,
            agent_tx,
            registry,
            confirmation_service,
            confirmation_txs: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            confirmation_ready: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            pending_questions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Start the bot with dual-task architecture (Inbound + Outbound)
    pub async fn run(self) -> Result<()> {
        let bot = self.bot.clone();
        let agent_tx = self.agent_tx.clone();
        let allowed_users = self.config.allowed_users.clone();
        let dm_scope = self.config.dm_scope;
        let registry = self.registry.clone();
        let confirmation_service = self.confirmation_service.clone();
        let confirmation_txs = self.confirmation_txs.clone();
        let confirmation_ready = self.confirmation_ready.clone();
        let pending_questions = self.pending_questions.clone();
        let bot_token = self.config.token.clone();

        // 1. Create Inbox
        let (inbox_tx, mut inbox_rx) = mpsc::channel::<ChannelMessage>(100);
        registry.register("telegram", inbox_tx).await;

        // 2. Spawn Outbound Handler (Inbox -> Telegram)
        let bot_clone = bot.clone();
        let _outbound_handle = tokio::spawn(async move {
            tracing::info!("📤 Telegram Outbound Actor started");
            while let Some(msg) = inbox_rx.recv().await {
                let chat_id = msg.user_id.replace("telegram:", "").parse::<i64>();
                match chat_id {
                    Ok(id) => {
                        if let Err(e) = bot_clone
                            .send_message(teloxide::types::ChatId(id), msg.content)
                            .await
                        {
                            tracing::error!("Failed to send Telegram message: {}", e);
                        }
                    }
                    Err(e) => tracing::error!("Invalid Telegram Chat ID {}: {}", msg.user_id, e),
                }
            }
        });

        // 3. Run Inbound Poller (Telegram -> Agent) with resilient restart/backoff
        let mut retry_attempt: u32 = 0;
        loop {
            tracing::info!("📥 Telegram Inbound Poller started");
            let agent_tx_loop = agent_tx.clone();
            let allowed_users_loop = allowed_users.clone();
            let confirmation_service_loop = confirmation_service.clone();
            let confirmation_txs_loop = confirmation_txs.clone();
            let confirmation_ready_loop = confirmation_ready.clone();
            let pending_questions_loop = pending_questions.clone();
            let bot_token_loop = bot_token.clone();

            teloxide::repl(bot.clone(), move |bot: Bot, msg: Message| {
                let agent_tx = agent_tx_loop.clone();
                let allowed_users = allowed_users_loop.clone();
                let confirmation_service = confirmation_service_loop.clone();
                let confirmation_txs = confirmation_txs_loop.clone();
                let confirmation_ready = confirmation_ready_loop.clone();
                let pending_questions = pending_questions_loop.clone();
                let bot_token = bot_token_loop.clone();

                async move {
                let ingress_at = std::time::Instant::now();
                let user_id = msg.chat.id.0.to_string();
                let username = msg.chat.username().map(|s| s.to_string());
                let chat_id = msg.chat.id.0;

                let sender_id = msg
                    .from
                    .as_ref()
                    .map(|u| u.id.0 as i64)
                    .unwrap_or(chat_id);

                if let Some(allowlist) = allowed_users.as_ref()
                    && !allowlist.contains(&sender_id)
                {
                    tracing::warn!(
                        "Blocked Telegram message from unauthorized user {}",
                        sender_id
                    );
                    return Ok(());
                }

                // Ensure confirmation adapter for this chat
                {
                    let mut ready = confirmation_ready.lock().await;
                    if !ready.contains(&chat_id) {
                        let (callback_tx, callback_rx) = mpsc::channel(10);
                        let (pending_tx, _pending_rx) = mpsc::channel(10);
                        let channel = format!("telegram:{}", chat_id);

                        let adapter = crate::tools::telegram_confirmation::TelegramConfirmationAdapter::new(
                            bot_token.clone(),
                            chat_id,
                            callback_rx,
                            pending_tx,
                            channel,
                        );

                        {
                            let mut service = confirmation_service.lock().await;
                            service.register_adapter(Box::new(adapter));
                        }

                        let mut txs = confirmation_txs.lock().await;
                        txs.insert(chat_id, callback_tx);
                        ready.insert(chat_id);
                    }
                }

                // Pairing Logic
                match crate::pairing::is_authorized("telegram", &user_id).await {
                    Ok(authorized) => {
                        if !authorized {
                            match crate::pairing::get_user_code("telegram", &user_id).await {
                                Ok(Some(code)) => {
                                    bot.send_message(msg.chat.id, format!("⏳ Pending code: **{}**", code)).await?;
                                }
                                Ok(None) => {
                                    if let Ok(code) = crate::pairing::create_pairing_request("telegram", user_id.clone(), username.clone()).await {
                                         bot.send_message(msg.chat.id, format!("🔐 Auth Code: **{}**", code)).await?;
                                    }
                                }
                                _ => {}
                            }
                            return Ok(());
                        }
                    }
                    Err(_) => return Ok(()),
                }

                // Normal Message Handling
                let mut text = match msg.text() { Some(t) => t.to_string(), None => return Ok(()) };
                let user_id = msg
                    .from
                    .as_ref()
                    .map(|u| u.id.0.to_string())
                    .unwrap_or_else(|| msg.chat.id.0.to_string());

                match crate::gateway::onboarding::process_onboarding_message("telegram", &user_id, &text).await {
                    Ok(crate::gateway::onboarding::OnboardingOutcome::ReplyOnly(reply)) => {
                        let _ = bot.send_message(msg.chat.id, reply).await;
                        return Ok(());
                    }
                    Ok(crate::gateway::onboarding::OnboardingOutcome::NotNeeded) => {}
                    Err(e) => {
                        let _ = bot
                            .send_message(msg.chat.id, format!("❌ Setup error: {}", e))
                            .await;
                        return Ok(());
                    }
                }

                if let Some((allowed, request_id)) = parse_confirmation_response(&text) {
                    let tx = {
                        let txs = confirmation_txs.lock().await;
                        txs.get(&chat_id).cloned()
                    };

                    if let Some(sender) = tx {
                        let _ = sender
                            .send(crate::tools::telegram_confirmation::CallbackResponse {
                                request_id,
                                allowed,
                            })
                            .await;
                        let _ = bot.send_message(msg.chat.id, "✅ Confirmation received.").await;
                    } else {
                        let _ = bot.send_message(msg.chat.id, "❌ No pending confirmation.").await;
                    }
                    return Ok(());
                }

                let skill_scope = format!("telegram:{}:{}", chat_id, user_id);
                match crate::gateway::skill_chat::handle_skill_slash_command(&skill_scope, &text).await {
                    Ok(Some(reply)) => {
                        let _ = bot.send_message(msg.chat.id, reply).await;
                        return Ok(());
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = bot
                            .send_message(msg.chat.id, format!("❌ Skill command error: {}", e))
                            .await;
                        return Ok(());
                    }
                }

                let session_id = build_session_id(
                    "telegram",
                    &msg.chat.id.0.to_string(),
                    &user_id,
                    dm_scope,
                    msg.chat.is_private(),
                );

                if let Some(pending) = {
                    let pending_map = pending_questions.lock().await;
                    pending_map.get(&session_id).cloned()
                } {
                    match crate::tools::question::normalize_question_answer(&pending, &text) {
                        Ok(normalized) => {
                            text = normalized;
                            let mut pending_map = pending_questions.lock().await;
                            pending_map.remove(&session_id);
                        }
                        Err(err_msg) => {
                            let prompt = crate::tools::question::format_question_prompt(&pending);
                            let _ = bot
                                .send_message(msg.chat.id, format!("{}\n{}", err_msg, prompt))
                                .await;
                            return Ok(());
                        }
                    }
                }

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
                            let _ = bot
                                .send_message(
                                    msg.chat.id,
                                    "❌ First-time setup: /set_admin_token <new_token>",
                                )
                                .await;
                            return Ok(());
                        }
                        if let Err(e) = crate::security::write_admin_token(new_token) {
                            let _ = bot.send_message(msg.chat.id, format!("❌ Failed to save token: {}", e)).await;
                            return Ok(());
                        }
                        let _ = bot.send_message(msg.chat.id, "✅ Admin token saved (first-time setup)").await;
                        return Ok(());
                    }

                    if parts.len() < 2 {
                        let _ = bot
                            .send_message(
                                msg.chat.id,
                                "❌ Usage: /set_admin_token <current_token_or_primary_password> <new_token>\nExample: /set_admin_token mypassword newtoken123",
                            )
                            .await;
                        return Ok(());
                    }
                    let current = parts[0].trim();
                    let new_token = parts[1].trim();
                    if new_token.is_empty() {
                        let _ = bot
                            .send_message(msg.chat.id, "❌ New token cannot be empty")
                            .await;
                        return Ok(());
                    }

                    if !crate::security::verify_admin_rotation_secret(current) {
                        let _ = bot
                            .send_message(msg.chat.id, "❌ Current token/password is invalid")
                            .await;
                        return Ok(());
                    }

                    if let Err(e) = crate::security::write_admin_token(new_token) {
                        let _ = bot.send_message(msg.chat.id, format!("❌ Failed to save token: {}", e)).await;
                        return Ok(());
                    }
                    let _ = bot.send_message(msg.chat.id, "✅ Admin token saved").await;
                    return Ok(());
                }

                // Typing
                let _ = bot.send_chat_action(msg.chat.id, teloxide::types::ChatAction::Typing).await;

                // Send to Agent
                let (response_tx, mut response_rx) = mpsc::channel(100);
                let agent_msg = crate::agent::AgentMessage {
                    session_id: session_id.clone(),
                    tenant_id: session_id.clone(),
                    request_id: uuid::Uuid::new_v4().to_string(),
                    content: text,
                    response_tx,
                    ingress_at,
                };

                if agent_tx.send(agent_msg).await.is_err() {
                    let _ = bot.send_message(msg.chat.id, "❌ Agent Error").await;
                    return Ok(());
                }

                // Stream response back
                let mut full = String::new();
                while let Some(chunk) = response_rx.recv().await {
                    match chunk {
                        crate::agent::StreamChunk::TextDelta(d) => full.push_str(&d),
                        crate::agent::StreamChunk::ToolResult(result) => {
                            if let Some(payload) = crate::tools::question::parse_question_payload(&result) {
                                let prompt = crate::tools::question::format_question_prompt(&payload);
                                {
                                    let mut pending_map = pending_questions.lock().await;
                                    pending_map.insert(session_id.clone(), payload);
                                }
                                let _ = bot.send_message(msg.chat.id, prompt).await;
                            }
                        }
                        _ => {}
                    }
                }
                if !full.is_empty() { let _ = bot.send_message(msg.chat.id, full).await; }

                Ok(())
                }
            })
            .await;

            let delay = next_poll_retry_delay(retry_attempt);
            retry_attempt = retry_attempt.saturating_add(1);
            tracing::warn!(
                "Telegram poller exited unexpectedly; restarting in {}s",
                delay.as_secs()
            );
            sleep(delay).await;
        }
    }
}

fn next_poll_retry_delay(attempt: u32) -> Duration {
    let capped_attempt = attempt.min(6);
    let base_secs = 1u64 << capped_attempt;
    let jitter_ms = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis())
        .unwrap_or(0)
        % 600) as u64;
    Duration::from_secs(base_secs.min(30)) + Duration::from_millis(jitter_ms)
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
impl ChannelAdapter for TelegramBot {
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        let chat_id = user_id.replace("telegram:", "").parse::<i64>()?;
        self.bot
            .send_message(teloxide::types::ChatId(chat_id), content)
            .await?;
        Ok(())
    }

    async fn send_stream_chunk(&self, _user_id: &str, _chunk: &str) -> Result<()> {
        Ok(())
    }

    fn channel_name(&self) -> &str {
        "telegram"
    }

    fn format_user_id(&self, raw_id: &str) -> String {
        format!("telegram:{}", raw_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_config() {
        let config = TelegramConfig {
            token: "test_token".to_string(),
            allowed_users: Some(vec![123456789]),
            dm_scope: crate::config::DmScope::Main,
        };
        assert_eq!(config.token, "test_token");
        assert!(config.allowed_users.unwrap().contains(&123456789));
    }
}
