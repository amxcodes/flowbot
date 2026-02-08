use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response, Html},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::Result;

use nanobot_core::agent::{AgentMessage, StreamChunk};

#[derive(Clone)]
pub struct WebState {
    pub sessions: Arc<Mutex<std::collections::HashMap<String, Vec<String>>>>,
    pub agent_tx: tokio::sync::mpsc::Sender<AgentMessage>,
}

impl WebState {
    pub fn new(agent_tx: tokio::sync::mpsc::Sender<AgentMessage>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            agent_tx,
        }
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    message: String,
    session_id: Option<String>,
}

#[derive(Serialize)]
pub struct ChatResponse {
    response: String,
    session_id: String,
}

async fn serve_index() -> Html<&'static str> {
    Html(r#"
<!DOCTYPE html>
<html>
<head>
    <title>Nanobot WebChat</title>
    <style>
        body { font-family: system-ui; max-width: 800px; margin: 50px auto; padding: 20px; }
        h1 { color: #333; }
        #chat { border: 1px solid #ddd; height: 400px; overflow-y: auto; padding: 10px; margin: 20px 0; }
        .message { margin: 10px 0; padding: 8px; border-radius: 4px; }
        .user { background: #e3f2fd; }
        .assistant { background: #f5f5f5; }
        input { width: 70%; padding: 10px; }
        button { padding: 10px 20px; margin-left: 10px; }
    </style>
</head>
<body>
    <h1>🤖 Nanobot WebChat</h1>
    <div id="chat"></div>
    <input id="input" type="text" placeholder="Type your message..." />
    <button onclick="send()">Send</button>
    <script>
        let sessionId = null;
        async function send() {
            const input = document.getElementById('input');
            const message = input.value;
            if (!message) return;
            appendMessage('user', message);
            input.value = '';
            const response = await fetch('/api/chat', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ message, session_id: sessionId })
            });
            const data = await response.json();
            sessionId = data.session_id;
            appendMessage('assistant', data.response);
        }
        function appendMessage(role, text) {
            const chat = document.getElementById('chat');
            const div = document.createElement('div');
            div.className = 'message ' + role;
            div.textContent = (role === 'user' ? 'You' : 'Assistant') + ': ' + text;
            chat.appendChild(div);
            chat.scrollTop = chat.scrollHeight;
        }
        document.getElementById('input').addEventListener('keypress', (e) => {
            if (e.key === 'Enter') send();
        });
    </script>
</body>
</html>
    "#)
}

async fn chat_handler(
    State(state): State<WebState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    
    // Create channel for agent responses
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<StreamChunk>(100);
    
    // Send message to agent
    let agent_msg = AgentMessage {
        session_id: session_id.clone(),
        content: req.message.clone(),
        response_tx,
    };
    
    if let Err(e) = state.agent_tx.send(agent_msg).await {
        eprintln!("Failed to send to agent: {}", e);
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    
    // Collect the full response from the stream
    let mut full_response = String::new();
    while let Some(chunk) = response_rx.recv().await {
        match chunk {
            StreamChunk::TextDelta(text) => {
                full_response.push_str(&text);
            }
            StreamChunk::ToolCall(tool_name) => {
                full_response.push_str(&format!("\n[Calling tool: {}]\n", tool_name));
            }
            StreamChunk::ToolResult(result) => {
                full_response.push_str(&format!("\n[Result: {}]\n", result));
            }
            StreamChunk::Done => break,
        }
    }
    
    // Store in session history
    let mut sessions = state.sessions.lock().await;
    sessions.entry(session_id.clone())
        .or_insert_with(Vec::new)
        .push(req.message.clone());
    sessions.entry(session_id.clone())
        .or_insert_with(Vec::new)
        .push(full_response.clone());
    
    Ok(Json(ChatResponse {
        response: full_response,
        session_id,
    }))
}

async fn status_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "features": ["memory", "browser", "webchat"]
    }))
}

pub fn create_router(state: WebState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/api/chat", post(chat_handler))
        .route("/api/status", get(status_handler))
        .with_state(state)
}

pub async fn run_server(port: u16, agent_tx: tokio::sync::mpsc::Sender<AgentMessage>) -> Result<()> {
    let state = WebState::new(agent_tx);
    let app = create_router(state);
    
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    println!("🌐 WebChat UI available at http://localhost:{}", port);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}
