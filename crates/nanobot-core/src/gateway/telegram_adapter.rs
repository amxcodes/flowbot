/// Telegram bot channel integration
/// Uses HTTP polling to receive messages from Telegram
use anyhow::Result;
use serde::{Deserialize, Serialize};
use teloxide::prelude::*;
use tokio::sync::mpsc;
use async_trait::async_trait;
use std::sync::Arc;
use super::adapter::{ChannelAdapter, ChannelMessage};
use super::registry::ChannelRegistry;

/// Telegram bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub token: String,
    pub allowed_users: Option<Vec<i64>>,
}

/// Telegram bot instance (Refactored to use Actor Model)
pub struct TelegramBot {
    bot: Bot,
    config: TelegramConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
}

impl TelegramBot {
    /// Create a new Telegram bot
    pub fn new(
        config: TelegramConfig,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        registry: Arc<ChannelRegistry>,
    ) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            bot,
            config,
            agent_tx,
            registry,
        }
    }

    /// Start the bot with dual-task architecture (Inbound + Outbound)
    pub async fn run(self) -> Result<()> {
        let bot = self.bot.clone();
        let agent_tx = self.agent_tx.clone();
        let allowed_users = self.config.allowed_users.clone();
        let registry = self.registry.clone();

        // 1. Create Inbox
        let (inbox_tx, mut inbox_rx) = mpsc::channel::<ChannelMessage>(100);
        registry.register("telegram", inbox_tx).await;

        // 2. Spawn Outbound Handler (Inbox -> Telegram)
        let bot_clone = bot.clone();
        let outbound_handle = tokio::spawn(async move {
            tracing::info!("📤 Telegram Outbound Actor started");
            while let Some(msg) = inbox_rx.recv().await {
                 let chat_id = msg.user_id.replace("telegram:", "").parse::<i64>();
                 match chat_id {
                    Ok(id) => {
                        if let Err(e) = bot_clone.send_message(teloxide::types::ChatId(id), msg.content).await {
                            tracing::error!("Failed to send Telegram message: {}", e);
                        }
                    }
                     Err(e) => tracing::error!("Invalid Telegram Chat ID {}: {}", msg.user_id, e),
                 }
            }
        });

        // 3. Run Inbound Poller (Telegram -> Agent)
        tracing::info!("📥 Telegram Inbound Poller started");
        teloxide::repl(bot, move |bot: Bot, msg: Message| {
            let agent_tx = agent_tx.clone();
            let _allowed_users = allowed_users.clone();
            
            async move {
                let user_id = msg.chat.id.0.to_string();
                let username = msg.chat.username().map(|s| s.to_string());

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
                let text = match msg.text() { Some(t) => t.to_string(), None => return Ok(()) };
                
                // Typing
                let _ = bot.send_chat_action(msg.chat.id, teloxide::types::ChatAction::Typing).await;

                // Send to Agent
                let (response_tx, mut response_rx) = mpsc::channel(100);
                let agent_msg = crate::agent::AgentMessage {
                    session_id: format!("telegram:{}", msg.chat.id),
                    content: text,
                    response_tx,
                };

                if agent_tx.send(agent_msg).await.is_err() {
                    let _ = bot.send_message(msg.chat.id, "❌ Agent Error").await;
                    return Ok(());
                }

                // Stream response back
                let mut full = String::new();
                while let Some(chunk) = response_rx.recv().await {
                    if let crate::agent::StreamChunk::TextDelta(d) = chunk { full.push_str(&d); }
                }
                if !full.is_empty() { let _ = bot.send_message(msg.chat.id, full).await; }

                Ok(())
            }
        }).await;

        // If repl exits, abort outbound
        outbound_handle.abort();
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramBot {
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        let chat_id = user_id.replace("telegram:", "").parse::<i64>()?;
        self.bot.send_message(teloxide::types::ChatId(chat_id), content).await?;
        Ok(())
    }

    async fn send_stream_chunk(&self, _user_id: &str, _chunk: &str) -> Result<()> {
        Ok(())
    }

    fn channel_name(&self) -> &str { "telegram" }
    
    fn format_user_id(&self, raw_id: &str) -> String { format!("telegram:{}", raw_id) }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegram_config() {
        let config = TelegramConfig {
            token: "test_token".to_string(),
            allowed_users: Some(vec![123456789]),
        };
        assert_eq!(config.token, "test_token");
        assert!(config.allowed_users.unwrap().contains(&123456789));
    }
}
