// use std::sync::Arc;
use tokio::sync::mpsc;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures::{sink::SinkExt, stream::StreamExt};
use serde_json::json;
use anyhow::Result;
use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use chrono::{Duration, Utc};

use super::adapter::ChannelAdapter;
use crate::agent::StreamChunk;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct GatewayClaims {
    sid: String,
    exp: usize,
}

fn encode_session_token(secret: &[u8], session_id: &str) -> String {
    let exp = (Utc::now() + Duration::hours(24)).timestamp() as usize;
    jsonwebtoken::encode(
        &Header::default(),
        &GatewayClaims {
            sid: session_id.to_string(),
            exp,
        },
        &EncodingKey::from_secret(secret),
    )
    .unwrap_or_default()
}

fn validate_session_token(secret: &[u8], token: &str, session_id: &str) -> bool {
    let claims = jsonwebtoken::decode::<GatewayClaims>(
        token,
        &DecodingKey::from_secret(secret),
        &Validation::default(),
    );

    matches!(claims, Ok(decoded) if decoded.claims.sid == session_id)
}

/// WebSocket implementation of ChannelAdapter
pub struct WebAdapter {
    session_id: String,
}


impl WebAdapter {
    pub fn new(session_id: String) -> Self {
        Self { session_id }
    }

    /// Handle a WebSocket connection (extracted from gateway/mod.rs)
    pub async fn handle_socket(
        socket: WebSocket,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    ) -> Result<()> {
        let (ws_tx, mut ws_rx) = socket.split();
        let ws_tx = std::sync::Arc::new(tokio::sync::Mutex::new(ws_tx));
        
        // Wait for initial message (optional), but always generate a server-side session_id
        let _ = ws_rx.next().await;
        let session_id = uuid::Uuid::new_v4().to_string();
        tracing::info!("New session: {}", session_id);

        let secret = std::env::var("NANOBOT_GATEWAY_SESSION_SECRET")
            .map(|s| s.into_bytes())
            .unwrap_or_else(|_| {
                let secrets = crate::security::get_or_create_session_secrets()
                    .map(|s| s.gateway_session_secret)
                    .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
                secrets.into_bytes()
            });
        let token = encode_session_token(&secret, &session_id);

        let response = json!({"type": "session_init", "session_id": session_id, "token": token});
        let _ = ws_tx.lock().await.send(WsMessage::Text(response.to_string())).await;
        
        let span = tracing::info_span!("websocket_session", session_id = %session_id);
        let _enter = span.enter();
        
        tracing::info!("WebSocket session established");

        // Create channel for agent responses
        let (response_tx, mut response_rx) = mpsc::channel(100);

        // Spawn task to forward agent responses to WebSocket
        let ws_tx_clone = ws_tx.clone();
        let send_task = tokio::spawn(async move {
            while let Some(chunk) = response_rx.recv().await {
                match chunk {
                    StreamChunk::TextDelta(text) => {
                        let msg = json!({
                            "type": "text_delta",
                            "delta": text
                        });
                        if let Err(e) = ws_tx_clone.lock().await.send(WsMessage::Text(msg.to_string())).await {
                            eprintln!("WS send error: {}", e);
                            break;
                        }
                    }
                    StreamChunk::Done => {
                        let msg = json!({ "type": "done" });
                        let _ = ws_tx_clone.lock().await.send(WsMessage::Text(msg.to_string())).await;
                    }
                    _ => {}
                }
            }
        });

        // Handle incoming messages
        while let Some(result) = ws_rx.next().await {
            match result {
                Ok(msg) => {
                    if let WsMessage::Text(text) = msg {
                        let content = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if json["type"] == "refresh_token" {
                                let exp = (Utc::now() + Duration::hours(24)).timestamp() as usize;
                                let new_token = jsonwebtoken::encode(
                                    &Header::default(),
                                    &GatewayClaims { sid: session_id.clone(), exp },
                                    &EncodingKey::from_secret(&secret),
                                )
                                .unwrap_or_default();
                                let msg = json!({"type": "session_refresh", "token": new_token});
                                let _ = ws_tx.lock().await.send(WsMessage::Text(msg.to_string())).await;
                                continue;
                            }
                            if json["type"] == "refresh_token" {
                                let new_token = encode_session_token(&secret, &session_id);
                                let msg = json!({"type": "session_refresh", "token": new_token});
                                let _ = ws_tx.lock().await.send(WsMessage::Text(msg.to_string())).await;
                                continue;
                            }

                            let token = json["token"].as_str().unwrap_or("");
                            if !validate_session_token(&secret, token, &session_id) {
                                let msg = json!({"type": "error", "error": "invalid_token", "action": "refresh_token"});
                                let _ = ws_tx.lock().await.send(WsMessage::Text(msg.to_string())).await;
                                continue;
                            }

                            json["message"].as_str().unwrap_or("").to_string()
                        } else {
                            let msg = json!({"type": "error", "error": "invalid_payload"});
                            let _ = ws_tx.lock().await.send(WsMessage::Text(msg.to_string())).await;
                            continue;
                        };

                        if !content.is_empty() {
                            let msg = crate::agent::AgentMessage {
                                session_id: session_id.clone(),
                                tenant_id: format!("web:{}", session_id),
                                content,
                                response_tx: response_tx.clone(),
                            };
                            if let Err(e) = agent_tx.send(msg).await {
                                eprintln!("Failed to send to agent: {}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("WS receive error: {}", e);
                    break;
                }
            }
        }

        send_task.abort();
        tracing::info!("WebSocket disconnected: {}", session_id);
        
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for WebAdapter {
    async fn send_message(&self, _user_id: &str, content: &str) -> Result<()> {
        // Note: In a real implementation, this would need to maintain WebSocket connections
        // For now, this is a placeholder showing the interface
        tracing::info!("[WebAdapter] Would send message to {}: {}", self.session_id, content);
        Ok(())
    }

    async fn send_stream_chunk(&self, _user_id: &str, chunk: &str) -> Result<()> {
        tracing::debug!("[WebAdapter] Would stream chunk to {}: {}", self.session_id, chunk);
        Ok(())
    }

    fn channel_name(&self) -> &str {
        "web"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_adapter_creation() {
        let adapter = WebAdapter::new("test-session".to_string());
        assert_eq!(adapter.channel_name(), "web");
    }

    #[test]
    fn test_format_user_id() {
        let adapter = WebAdapter::new("test-session".to_string());
        assert_eq!(adapter.format_user_id("user123"), "web:user123");
    }

    #[test]
    fn test_session_token_roundtrip() {
        let secret = b"test-secret";
        let session_id = "session-1";
        let token = encode_session_token(secret, session_id);
        assert!(validate_session_token(secret, &token, session_id));
        assert!(!validate_session_token(secret, &token, "other"));
    }
}
