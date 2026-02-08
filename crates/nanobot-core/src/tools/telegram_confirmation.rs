use super::confirmation::{ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse, RiskLevel};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::json;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

/// Telegram confirmation adapter using inline keyboard buttons
pub struct TelegramConfirmationAdapter {
    bot_token: String,
    chat_id: i64,
    /// Channel to receive callback responses (wrapped in Mutex for interior mutability)
    callback_rx: Mutex<mpsc::Receiver<CallbackResponse>>,
    /// Channel to send pending requests
    pending_tx: mpsc::Sender<String>, // request_id -> for tracking
}

#[derive(Debug, Clone)]
struct CallbackResponse {
    request_id: String,
    allowed: bool,
}

impl TelegramConfirmationAdapter {
    pub fn new(
        bot_token: String,
        chat_id: i64,
        callback_rx: mpsc::Receiver<CallbackResponse>,
        pending_tx: mpsc::Sender<String>,
    ) -> Self {
        Self {
            bot_token,
            chat_id,
            callback_rx: Mutex::new(callback_rx),
            pending_tx,
        }
    }

    fn format_risk_emoji(risk: RiskLevel) -> &'static str {
        match risk {
            RiskLevel::Low => "ℹ️",
            RiskLevel::Medium => "⚠️",
            RiskLevel::High => "🚨",
            RiskLevel::Critical => "💀",
        }
    }

    async fn send_inline_keyboard(&self, request: &ConfirmationRequest) -> Result<()> {
        let client = reqwest::Client::new();
        
        let message_text = format!(
            "{} *Security Permission Request*\n\n\
            *Tool:* `{}`\n\
            *Risk:* {:?}\n\
            *Operation:* {}\n\n\
            ```\n{}\n```",
            Self::format_risk_emoji(request.risk_level),
            request.tool_name,
            request.risk_level,
            request.operation,
            request.args
        );

        let inline_keyboard = json!({
            "inline_keyboard": [[
                {
                    "text": "✅ Allow",
                    "callback_data": format!("allow:{}", request.id)
                },
                {
                    "text": "❌ Deny",
                    "callback_data": format!("deny:{}", request.id)
                }
            ]]
        });

        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        
        client
            .post(&url)
            .json(&json!({
                "chat_id": self.chat_id,
                "text": message_text,
                "parse_mode": "Markdown",
                "reply_markup": inline_keyboard
            }))
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}

#[async_trait]
impl ConfirmationAdapter for TelegramConfirmationAdapter {
    async fn request_confirmation(&self, request: &ConfirmationRequest) -> Result<ConfirmationResponse> {
        // Send inline keyboard to Telegram
        self.send_inline_keyboard(request).await?;

        // Register as pending
        self.pending_tx.send(request.id.clone()).await?;

        // Wait for callback with timeout
        let timeout = request.timeout.unwrap_or(Duration::from_secs(300));
        
        match tokio::time::timeout(timeout, async {
            // Wait for callback matching this request ID
            loop {
                let mut rx = self.callback_rx.lock().await;
                if let Some(callback) = rx.recv().await {
                    if callback.request_id == request.id {
                        return Ok(callback);
                    }
                    // Not our request, ignore
                }
            }
        })
        .await
        {
            Ok(Ok(callback)) => Ok(ConfirmationResponse {
                id: request.id.clone(),
                allowed: callback.allowed,
                remember: false,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Timeout - deny by default
                tracing::warn!("Telegram confirmation timeout for request {}", request.id);
                Ok(ConfirmationResponse {
                    id: request.id.clone(),
                    allowed: false,
                    remember: false,
                })
            }
        }
    }

    fn name(&self) -> &str {
        "Telegram"
    }

    async fn is_available(&self) -> bool {
        // Could ping Telegram API, but for now just check if we have credentials
        !self.bot_token.is_empty()
    }
}

// Helper for processing Telegram callbacks
// This would be called by the main Telegram bot when a callback_query is received
pub async fn handle_telegram_callback(
    callback_data: &str,
    pending_tx: &mpsc::Sender<CallbackResponse>,
) -> Result<()> {
    // Parse callback_data: "allow:req_123" or "deny:req_123"
    let parts: Vec<&str> = callback_data.split(':').collect();
    if parts.len() != 2 {
        return Err(anyhow!("Invalid callback data format"));
    }

    let action = parts[0];
    let request_id = parts[1].to_string();

    let response = CallbackResponse {
        request_id,
        allowed: action == "allow",
    };

    pending_tx.send(response).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_emoji_formatting() {
        assert_eq!(
            TelegramConfirmationAdapter::format_risk_emoji(RiskLevel::Low),
            "ℹ️"
        );
        assert_eq!(
            TelegramConfirmationAdapter::format_risk_emoji(RiskLevel::Critical),
            "💀"
        );
    }
}
