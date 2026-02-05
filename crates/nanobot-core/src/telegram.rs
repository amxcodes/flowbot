/// Telegram bot channel integration
/// Uses HTTP polling to receive messages from Telegram
use anyhow::Result;
use serde::{Deserialize, Serialize};
use teloxide::prelude::*;
use tokio::sync::mpsc;

/// Message from Telegram to be processed by the agent
#[derive(Debug, Clone)]
pub struct TelegramMessage {
    pub chat_id: ChatId,
    pub text: String,
    pub user_id: UserId,
}

/// Response from agent to send back to Telegram
#[derive(Debug, Clone)]
pub struct TelegramResponse {
    pub chat_id: ChatId,
    pub text: String,
}

/// Telegram bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub token: String,
    pub allowed_users: Option<Vec<i64>>,
}

/// Telegram bot instance
pub struct TelegramBot {
    bot: Bot,
    config: TelegramConfig,
    agent_tx: mpsc::Sender<TelegramMessage>,
    response_rx: mpsc::Receiver<TelegramResponse>,
}

impl TelegramBot {
    /// Create a new Telegram bot
    pub fn new(
        config: TelegramConfig,
        agent_tx: mpsc::Sender<TelegramMessage>,
        response_rx: mpsc::Receiver<TelegramResponse>,
    ) -> Self {
        let bot = Bot::new(&config.token);
        Self {
            bot,
            config,
            agent_tx,
            response_rx,
        }
    }

    /// Start the bot (polling loop)
    pub async fn run(self) -> Result<()> {
        let bot = self.bot.clone();
        let agent_tx = self.agent_tx.clone();
        let allowed_users = self.config.allowed_users.clone();

        // Spawn response sender task
        let response_bot = bot.clone();
        let mut response_rx = self.response_rx;
        tokio::spawn(async move {
            while let Some(response) = response_rx.recv().await {
                if let Err(e) = response_bot
                    .send_message(response.chat_id, response.text)
                    .await
                {
                    eprintln!("Failed to send Telegram message: {}", e);
                }
            }
        });

        // Message handler
        teloxide::repl(bot, move |bot: Bot, msg: Message| {
            let agent_tx = agent_tx.clone();
            let _allowed_users = allowed_users.clone();

            async move {
                let user_id = msg.chat.id.0.to_string();
                let username = msg.chat.username().map(|s| s.to_string());
                
                // Check if authorized via pairing system
                match crate::pairing::is_authorized("telegram", &user_id).await {
                    Ok(authorized) => {
                        if !authorized {
                            // Check if already has pending request
                            match crate::pairing::get_user_code("telegram", &user_id).await {
                                Ok(Some(code)) => {
                                    bot.send_message(
                                        msg.chat.id,
                                        format!("⏳ Waiting for approval.\n\nYour pairing code: **{}**\n\nContact the bot owner to approve access.", code)
                                    ).await?;
                                }
                                Ok(None) => {
                                    // Generate new pairing code
                                    match crate::pairing::create_pairing_request(
                                        "telegram",
                                        user_id.clone(),
                                        username.clone()
                                    ).await {
                                        Ok(code) => {
                                            bot.send_message(
                                                msg.chat.id,
                                                format!("🔐 Authorization required.\n\nYour pairing code: **{}**\n\nContact the bot owner to approve access.\n\nApprove with: `flowbot pairing approve telegram {}`", code, code)
                                            ).await?;
                                            
                                            tracing::info!(
                                                "New pairing request from {} ({}): {}",
                                                username.unwrap_or_else(|| user_id.clone()),
                                                user_id,
                                                code
                                            );
                                        }
                                        Err(e) => {
                                            eprintln!("Failed to create pairing request: {}", e);
                                            bot.send_message(
                                                msg.chat.id,
                                                "⚠️ Failed to create pairing request. Please try again later."
                                            ).await?;
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Failed to check pairing status: {}", e);
                                }
                            }
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to check authorization: {}", e);
                        bot.send_message(
                            msg.chat.id,
                            "⚠️ System error. Please try again later."
                        ).await?;
                        return Ok(());
                    }
                }

                // Get message text
                let text = match msg.text() {
                    Some(t) => t.to_string(),
                    None => {
                        bot.send_message(msg.chat.id, "Please send text messages only.")
                            .await?;
                        return Ok(());
                    }
                };

                // Show typing indicator
                bot.send_chat_action(msg.chat.id, teloxide::types::ChatAction::Typing)
                    .await?;

                // Send to agent
                let telegram_msg = TelegramMessage {
                    chat_id: msg.chat.id,
                    text: text.clone(),
                    user_id: msg.from().map(|u| u.id).unwrap_or(UserId(0)),
                };

                if let Err(e) = agent_tx.send(telegram_msg).await {
                    eprintln!("Failed to send message to agent: {}", e);
                    bot.send_message(
                        msg.chat.id,
                        "❌ Internal error. Please try again later.",
                    )
                    .await?;
                }

                Ok(())
            }
        })
        .await;

        Ok(())
    }
}

/// Agent loop with Antigravity integration
pub struct SimpleAgent {
    telegram_rx: mpsc::Receiver<TelegramMessage>,
    response_tx: mpsc::Sender<TelegramResponse>,
}

impl SimpleAgent {
    pub fn new(
        telegram_rx: mpsc::Receiver<TelegramMessage>,
        response_tx: mpsc::Sender<TelegramResponse>,
    ) -> Self {
        Self {
            telegram_rx,
            response_tx,
        }
    }

    /// Run the agent loop with Antigravity
    pub async fn run(mut self) {
        // Initialize Antigravity client once
        let antigravity_client = match crate::antigravity::AntigravityClient::from_env().await {
            Ok(client) => {
                eprintln!("✅ Antigravity client initialized");
                Some(client)
            }
            Err(e) => {
                eprintln!("⚠️ Antigravity not available: {}", e);
                eprintln!("ℹ️ Falling back to echo mode");
                None
            }
        };

        while let Some(msg) = self.telegram_rx.recv().await {
            let response_text = if let Some(ref client) = antigravity_client {
                // Use Antigravity for real AI responses
                match self.process_with_antigravity(client, &msg.text).await {
                    Ok(text) => text,
                    Err(e) => {
                        eprintln!("❌ Antigravity error: {}", e);
                        format!("❌ Error: {}", e)
                    }
                }
            } else {
                // Fallback to echo
                format!("🤖 Echo: {}", msg.text)
            };

            let response = TelegramResponse {
                chat_id: msg.chat_id,
                text: response_text,
            };

            if let Err(e) = self.response_tx.send(response).await {
                eprintln!("Failed to send response: {}", e);
            }
        }
    }

    /// Process message with Antigravity and handle tool calls
    async fn process_with_antigravity(
        &self,
        client: &crate::antigravity::AntigravityClient,
        message: &str,
    ) -> Result<String> {
        use crate::tools::executor;
        use rig::completion::Prompt;

        // 1. Setup Preamble with Tool Descriptions
        let preamble = format!(
            "You are FlowBot, a helpful AI assistant accessible via Telegram.\n\
             You can read/write files and execute system commands to help the user.\n\
             Keep responses concise and friendly.\n\n\
             {}",
            executor::get_tool_descriptions()
        );

        // 2. Create agent
        let agent = client.agent("gemini-2.5-flash").preamble(&preamble).build();

        // 3. Initial Prompt
        // We append the user message to the conversation history (handled by rig agent implicitly for single turn,
        // but here we are doing a manual loop for tool use. Rig's Agent maintains history?
        // rig::agent::Agent does NOT maintain history across `prompt` calls unless we use `chat`.
        // However, for this simple implementation, we'll just do a "Tool Use Loop" for this single turn.

        // Note: Rig's `prompt` method is stateless.
        // To do multi-turn tool use properly without a full Chat interface, we construct a prompt chain.
        // Or we just append the previous turns to the prompt string.

        let current_prompt = message.to_string();
        let mut conversation_log = String::new(); // To keep context of tool usage

        // Max turns to prevent infinite loops
        for _ in 0..5 {
            // Send prompt to LLM
            // If we have conversation log (previous tool outputs), prepend it or append it?
            // A simple strategy: prompt = initial_msg + \n\nContext:\n + conversation_log

            let full_prompt = if conversation_log.is_empty() {
                current_prompt.clone()
            } else {
                format!("{}\n\nPrevious steps:\n{}", message, conversation_log)
            };

            let response = agent.prompt(&full_prompt).await?;

            // 4. Check if response is a tool call
            if executor::is_tool_call(&response) {
                // It's a tool call!
                // Notify user we are running a tool (Optional, via typing status or intermediate message?
                // For now, let's just log it and run it. Telegram bot doesn't easily support multiple partial messages cleanly without spamming.
                // We could send a "Thinking..." generic message, but let's keep it simple.)

                eprintln!("🛠️ Executing tool: {}", response.trim());

                // Execute tool
                let tool_output = match executor::execute_tool(&response, None, None, None).await {
                    Ok(output) => output,
                    Err(e) => format!("Error executing tool: {}", e),
                };

                eprintln!("✅ Tool output length: {}", tool_output.len());

                // Append to conversation log
                conversation_log.push_str(&format!(
                    "\n> Agent: {}\n> Tool Result: {}\n",
                    response.trim(),
                    tool_output
                ));

                // Prepare next prompt: The tool result is now part of the history.
                // We loop back to let the agent process the result.
                continue;
            } else {
                // It's a final response (or at least not a tool call)
                return Ok(response);
            }
        }

        Ok("❌ Agent stuck in a loop or exceeded max tool turns.".to_string())
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
        };
        assert_eq!(config.token, "test_token");
        assert!(config.allowed_users.unwrap().contains(&123456789));
    }
}
