// Gateway module - entry point for the API server
pub mod agent_manager;
pub mod adapter;
pub mod router;
pub mod web_adapter;
pub mod telegram_adapter;
pub mod registry;
pub mod slack_adapter;
pub mod discord_adapter;



use anyhow::Result;
use axum::{
    Router,
    extract::{
        State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use chrono::{Duration, Utc};

use crate::agent::{AgentMessage, StreamChunk};

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

#[derive(Clone)]
pub struct GatewayConfig {
    pub port: u16,
}

#[derive(Clone)]
pub struct Gateway {
    config: GatewayConfig,
    agent_tx: mpsc::Sender<AgentMessage>,
    confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
}

impl Gateway {
    pub fn new(
        config: GatewayConfig,
        agent_tx: mpsc::Sender<AgentMessage>,
        confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    ) -> Self {
        Self {
            config,
            agent_tx,
            confirmation_service,
        }
    }

pub async fn start(&self) -> Result<()> {
        let app = Router::new()
            .route("/health", get(health_check))
            .route("/ws", get(ws_handler))
            .with_state(Arc::new(self.clone())); // Share state

        let addr = SocketAddr::from(([0, 0, 0, 0], self.config.port));
        println!("🚀 Gateway listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_token_roundtrip() {
        let secret = b"test-secret";
        let session_id = "session-1";
        let token = encode_session_token(secret, session_id);
        assert!(validate_session_token(secret, &token, session_id));
        assert!(!validate_session_token(secret, &token, "other"));
    }
}

async fn health_check() -> &'static str {
    "OK"
}

// WebSocket Handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(gateway): State<Arc<Gateway>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, gateway))
}

async fn handle_socket(socket: WebSocket, gateway: Arc<Gateway>) {
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

    // Send session_id back to client
    let response = json!({"type": "session_init", "session_id": session_id, "token": token});
    let _ = ws_tx.lock().await.send(WsMessage::Text(response.to_string())).await;
    
    let span = tracing::info_span!("websocket_session", session_id = %session_id);
    let _enter = span.enter();
    
    
    tracing::info!("WebSocket session established");
    

    let (confirm_req_tx, mut confirm_req_rx) = mpsc::channel::<crate::tools::gateway_confirmation::GatewayConfirmationEvent>(10);
    let (confirm_resp_tx, confirm_resp_rx) = mpsc::channel::<crate::tools::gateway_confirmation::GatewayConfirmationEvent>(10);
    let confirm_channel = format!("web:{}", session_id);

    {
        let mut service = gateway.confirmation_service.lock().await;
        service.register_adapter(Box::new(crate::tools::gateway_confirmation::GatewayConfirmationAdapter::new(
            confirm_req_tx,
            confirm_resp_rx,
            confirm_channel,
        )));
    }

    // Create channel for agent responses
    let (response_tx, mut response_rx) = mpsc::channel(100);

    // Spawn task to forward agent responses to WebSocket
    let ws_tx_clone = ws_tx.clone();
    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(chunk) = response_rx.recv() => {
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
                Some(event) = confirm_req_rx.recv() => {
                    if let Ok(text) = serde_json::to_string(&event) {
                        if let Err(e) = ws_tx_clone.lock().await.send(WsMessage::Text(text)).await {
                            eprintln!("WS send error: {}", e);
                            break;
                        }
                    }
                }
                else => break,
            }
        }
    });

    // Handle incoming messages
    while let Some(result) = ws_rx.next().await {
        match result {
            Ok(msg) => {
                if let WsMessage::Text(text) = msg {
                    println!("Received: {}", text);
                    // Parse as JSON or assumes raw text in MVP?
                    // Let's assume raw text for "chat" for now, or JSON object.
                    // Basic protocol: {"message": "hello"}

                    let parsed_json = serde_json::from_str::<serde_json::Value>(&text).ok();

                    if let Some(json) = parsed_json.as_ref() {
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

                        if json["type"] == "confirmation_response" {
                            let id = json["id"].as_str().unwrap_or("").to_string();
                            let allowed = json["allowed"].as_bool().unwrap_or(false);
                            if !id.is_empty() {
                                let _ = confirm_resp_tx
                                    .send(crate::tools::gateway_confirmation::GatewayConfirmationEvent::Response {
                                        id,
                                        allowed,
                                        remember: false,
                                    })
                                    .await;
                                continue;
                            }
                        }
                    }

                    let content = if let Some(json) = parsed_json {
                        json["message"].as_str().unwrap_or("").to_string()
                    } else {
                        let msg = json!({"type": "error", "error": "invalid_payload"});
                        let _ = ws_tx.lock().await.send(WsMessage::Text(msg.to_string())).await;
                        continue;
                    };

                    if !content.is_empty() {
                        let msg = AgentMessage {
                            session_id: session_id.clone(),
                            tenant_id: format!("web:{}", session_id),
                            content,
                            response_tx: response_tx.clone(),
                        };
                        if let Err(e) = gateway.agent_tx.send(msg).await {
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
    println!("WebSocket disconnected: {}", session_id);
}
