use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Html},
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

#[derive(Deserialize)]
pub struct LoginRequest {
    username: String, // Acts as tenant_id
    password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    success: bool,
    token: Option<String>,
}

async fn serve_login() -> Html<&'static str> {
    Html(r#"
<!DOCTYPE html>
<html>
<head>
    <title>Nanobot Login</title>
    <style>
        body { font-family: system-ui; max-width: 400px; margin: 100px auto; padding: 20px; text-align: center; }
        input {  width: 100%; padding: 10px; margin: 10px 0; box-sizing: border-box; }
        button { width: 100%; padding: 10px; background: #333; color: white; border: none; cursor: pointer; }
    </style>
</head>
<body>
    <h1>🤖 Nanobot Login</h1>
    <input id="username" type="text" placeholder="Workspace / User ID" />
    <input id="password" type="password" placeholder="Password" />
    <button onclick="login()">Login</button>
    <script>
        async function login() {
            const username = document.getElementById('username').value;
            const password = document.getElementById('password').value;
            const res = await fetch('/api/login', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ username, password })
            });
            const data = await res.json();
            if (data.success) {
                // Simple session storage for demo (In prod use HttpOnly cookies)
                // We'll just append it to headers in the chat client or use a simple cookie
                document.cookie = `nanobot_auth=${data.token}; path=/; max-age=86400`;
                window.location.href = '/chat';
            } else {
                alert('Invalid credentials');
            }
        }
    </script>
</body>
</html>
    "#)
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
        .logout { float: right; font-size: 0.8em; color: #666; text-decoration: none; }
    </style>
</head>
<body>
    <a href="/" class="logout" onclick="document.cookie='nanobot_auth=; path=/; expires=Thu, 01 Jan 1970 00:00:00 GMT';">Logout</a>
    <h1>🤖 Nanobot WebChat</h1>
    <div id="chat"></div>
    <input id="input" type="text" placeholder="Type your message..." />
    <button onclick="send()">Send</button>
    <script>
        // Check auth
        if (!document.cookie.includes('nanobot_auth=')) {
            window.location.href = '/';
        }

        let sessionId = null;
        async function send() {
            const input = document.getElementById('input');
            const message = input.value;
            if (!message) return;
            appendMessage('user', message);
            input.value = '';
            
            // Get token from cookie manually for simplicity (or trust browser to send it)
            // We just need the backend to read it.
            
            const response = await fetch('/api/chat', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ message, session_id: sessionId })
            });
            
            if (response.status === 401) {
                window.location.href = '/';
                return;
            }

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

async fn login_handler(
    Json(req): Json<LoginRequest>,
) ->  impl IntoResponse {
    let expected_pass = std::env::var("NANOBOT_WEB_PASSWORD").unwrap_or_else(|_| "admin".to_string());
    
    if req.password == expected_pass {
        // Token = username for simplicity (or sign it if we had jwt)
        // Format: username:signature? No, just username for now as tenant_id.
        // SECURITY NOTE: This is weak. In prod, use real JWT.
        let token = req.username; 
        
        Json(LoginResponse {
            success: true,
            token: Some(token),
        })
    } else {
        Json(LoginResponse {
            success: false,
            token: None,
        })
    }
}

async fn chat_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    
    // Extract tenant_id from cookie
    let cookie_header = headers.get("cookie").ok_or(StatusCode::UNAUTHORIZED)?.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
    let tenant_id = cookie_header
        .split(';')
        .find_map(|p| {
            let p = p.trim();
            if p.starts_with("nanobot_auth=") {
                Some(p.trim_start_matches("nanobot_auth=").to_string())
            } else {
                None
            }
        })
        .ok_or(StatusCode::UNAUTHORIZED)?;
    
    if tenant_id.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Create channel for agent responses
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<StreamChunk>(100);
    
    // Send message to agent
    let agent_msg = AgentMessage {
        session_id: session_id.clone(),
        tenant_id, // Use validated tenant_id
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
            StreamChunk::Thinking(text) => {
                // For web simple view, we just append it as a block?
                // Or maybe ignore it if we want clean output?
                // Let's append it but marked.
                full_response.push_str(&format!("\n<think>{}</think>\n", text));
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
        .route("/", get(serve_login))
        .route("/chat", get(serve_index))
        .route("/api/login", post(login_handler))
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
