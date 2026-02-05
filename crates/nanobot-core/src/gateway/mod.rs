// Gateway module - entry point for the API server
pub mod agent_manager;

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

use crate::agent::{AgentMessage, StreamChunk};

#[derive(Clone)]
pub struct GatewayConfig {
    pub port: u16,
}

#[derive(Clone)]
pub struct Gateway {
    config: GatewayConfig,
    agent_tx: mpsc::Sender<AgentMessage>,
}

impl Gateway {
    pub fn new(config: GatewayConfig, agent_tx: mpsc::Sender<AgentMessage>) -> Self {
        Self { config, agent_tx }
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
    let (mut ws_tx, mut ws_rx) = socket.split();
    let session_id = uuid::Uuid::new_v4().to_string(); // Simple session ID
    println!("New WebSocket connection: {}", session_id);

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
                _ => {} // Ignore others for now
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

                    let content = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                    {
                        json["message"].as_str().unwrap_or("").to_string()
                    } else {
                        text // Fallback
                    };

                    if !content.is_empty() {
                        let msg = AgentMessage {
                            session_id: session_id.clone(),
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
