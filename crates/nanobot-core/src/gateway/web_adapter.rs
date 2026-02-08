// use std::sync::Arc;
use tokio::sync::mpsc;
use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures::{sink::SinkExt, stream::StreamExt};
use serde_json::json;
use anyhow::Result;
use async_trait::async_trait;

use super::adapter::ChannelAdapter;
use crate::agent::StreamChunk;

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
        let (mut ws_tx, mut ws_rx) = socket.split();
        
        // Wait for initial message - check if client provides session_id
        let session_id = if let Some(Ok(WsMessage::Text(first_msg))) = ws_rx.next().await {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&first_msg) {
                if json_val["type"] == "init" {
                    if let Some(provided_id) = json_val["session_id"].as_str() {
                        let session = provided_id.to_string();
                        tracing::info!("Resuming session: {}", session);
                        session
                    } else {
                        let new_id = uuid::Uuid::new_v4().to_string();
                        tracing::info!("New session: {}", new_id);
                        
                        let response = json!({"type": "session_init", "session_id": new_id});
                        let _ = ws_tx.send(WsMessage::Text(response.to_string())).await;
                        
                        new_id
                    }
                } else {
                    uuid::Uuid::new_v4().to_string()
                }
            } else {
                uuid::Uuid::new_v4().to_string()
            }
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        
        let span = tracing::info_span!("websocket_session", session_id = %session_id);
        let _enter = span.enter();
        
        tracing::info!("WebSocket session established");

        // Create channel for agent responses
        let (response_tx, mut response_rx) = mpsc::channel(100);

        // Spawn task to forward agent responses to WebSocket
        let send_task = tokio::spawn(async move {
            while let Some(chunk) = response_rx.recv().await {
                match chunk {
                    StreamChunk::TextDelta(text) => {
                        let msg = json!({
                            "type": "text_delta",
                            "delta": text
                        });
                        if let Err(e) = ws_tx.send(WsMessage::Text(msg.to_string())).await {
                            eprintln!("WS send error: {}", e);
                            break;
                        }
                    }
                    StreamChunk::Done => {
                        let msg = json!({ "type": "done" });
                        let _ = ws_tx.send(WsMessage::Text(msg.to_string())).await;
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
                            json["message"].as_str().unwrap_or("").to_string()
                        } else {
                            text
                        };

                        if !content.is_empty() {
                            let msg = crate::agent::AgentMessage {
                                session_id: session_id.clone(),
                                tenant_id: "default".to_string(), // Web adapter uses default tenant for now
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
}
