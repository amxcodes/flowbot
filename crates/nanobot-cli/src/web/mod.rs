use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use nanobot_core::agent::{AgentMessage, StreamChunk};

#[derive(Clone)]
pub struct WebState {
    pub sessions: Arc<Mutex<std::collections::HashMap<String, Vec<String>>>>,
    pub agent_tx: tokio::sync::mpsc::Sender<AgentMessage>,
    pub auth_secret: Vec<u8>,
    pub session_ids: Arc<Mutex<std::collections::HashMap<String, String>>>,
    pub pending_questions: Arc<
        Mutex<std::collections::HashMap<String, nanobot_core::tools::question::QuestionPayload>>,
    >,
}

impl WebState {
    pub fn new(agent_tx: tokio::sync::mpsc::Sender<AgentMessage>) -> Self {
        let secret = std::env::var("NANOBOT_WEB_TOKEN_SECRET")
            .map(|s| s.into_bytes())
            .unwrap_or_else(|_| {
                let secrets = nanobot_core::security::get_or_create_session_secrets()
                    .map(|s| s.web_token_secret)
                    .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
                secrets.into_bytes()
            });
        Self {
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            agent_tx,
            auth_secret: secret,
            session_ids: Arc::new(Mutex::new(std::collections::HashMap::new())),
            pending_questions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }
}

#[derive(Deserialize)]
pub struct ChatRequest {
    message: String,
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

#[derive(Debug, Serialize, Deserialize)]
struct WebClaims {
    sub: String,
    exp: usize,
}

#[derive(Deserialize)]
pub struct AdminTokenRequest {
    current: Option<String>,
    token: String,
}

async fn serve_login() -> Html<&'static str> {
    Html(
        r#"
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
                window.location.href = '/chat';
            } else {
                alert('Invalid credentials');
            }
        }
    </script>
</body>
</html>
    "#,
    )
}

async fn serve_index() -> Html<&'static str> {
    Html(
        r#"
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
    <details>
        <summary>Admin Token</summary>
        <div style="margin: 10px 0;">
            <input id="currentAdminSecret" type="password" placeholder="Current admin token or primary password" />
            <input id="adminToken" type="password" placeholder="Set admin token" />
            <button onclick="setAdminToken()">Save Token</button>
        </div>
    </details>
    <div id="chat"></div>
    <input id="input" type="text" placeholder="Type your message..." />
    <button onclick="send()">Send</button>
    <script>
        // Check auth
        if (!document.cookie.includes('nanobot_auth=')) {
            window.location.href = '/';
        }

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
                body: JSON.stringify({ message })
            });
            
            if (response.status === 401) {
                window.location.href = '/';
                return;
            }

            const data = await response.json();
            appendMessage('assistant', data.response);
        }

        async function setAdminToken() {
            const current = document.getElementById('currentAdminSecret').value || null;
            const token = document.getElementById('adminToken').value;
            if (!token) {
                alert('Please enter new admin token');
                return;
            }

            const res = await fetch('/api/admin/token', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ current, token })
            });

            if (res.ok) {
                alert('Admin token updated');
                document.getElementById('adminToken').value = '';
                return;
            }

            if (res.status === 401) {
                alert('Current token/password invalid (or missing)');
            } else {
                alert('Failed to update admin token');
            }
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
    "#,
    )
}

async fn login_handler(
    State(state): State<WebState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let Some(expected_pass) = nanobot_core::security::read_primary_password() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "error": "Web password not configured. Run setup and set a primary password."
            })),
        )
            .into_response();
    };

    if nanobot_core::security::secure_eq(&req.password, &expected_pass) {
        let tenant_id = req.username;

        let exp = (Utc::now() + Duration::hours(24)).timestamp() as usize;
        let claims = WebClaims {
            sub: tenant_id,
            exp,
        };
        let token = match jsonwebtoken::encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(&state.auth_secret),
        ) {
            Ok(token) => token,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::http::HeaderMap::new(),
                    Json(LoginResponse {
                        success: false,
                        token: None,
                    }),
                )
                    .into_response();
            }
        };

        let mut headers = axum::http::HeaderMap::new();
        let cookie_value = format!(
            "nanobot_auth={}; Path=/; Max-Age=86400; HttpOnly; SameSite=Strict",
            token
        );
        headers.insert(
            axum::http::header::SET_COOKIE,
            cookie_value.parse().unwrap(),
        );

        (
            StatusCode::OK,
            headers,
            Json(LoginResponse {
                success: true,
                token: None,
            }),
        )
            .into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            axum::http::HeaderMap::new(),
            Json(LoginResponse {
                success: false,
                token: None,
            }),
        )
            .into_response()
    }
}

async fn chat_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    let ingress_at = std::time::Instant::now();
    // Extract tenant_id from cookie
    let cookie_header = headers
        .get("cookie")
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_str()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let token = cookie_header
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

    let claims = jsonwebtoken::decode::<WebClaims>(
        &token,
        &DecodingKey::from_secret(&state.auth_secret),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let tenant_id = claims.claims.sub;

    let session_id = {
        let mut sessions = state.session_ids.lock().await;
        sessions
            .entry(tenant_id.clone())
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone()
    };

    let mut user_message = req.message.clone();
    if let Some(pending) = {
        let pending_map = state.pending_questions.lock().await;
        pending_map.get(&session_id).cloned()
    } {
        match nanobot_core::tools::question::normalize_question_answer(&pending, &user_message) {
            Ok(normalized) => {
                user_message = normalized;
                let mut pending_map = state.pending_questions.lock().await;
                pending_map.remove(&session_id);
            }
            Err(err_msg) => {
                let prompt = nanobot_core::tools::question::format_question_prompt(&pending);
                return Ok(Json(ChatResponse {
                    response: format!("{}\n{}", err_msg, prompt),
                    session_id,
                }));
            }
        }
    }

    // Create channel for agent responses
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel::<StreamChunk>(100);

    // Send message to agent
    let agent_msg = AgentMessage {
        session_id: session_id.clone(),
        tenant_id, // Use validated tenant_id
        request_id: uuid::Uuid::new_v4().to_string(),
        content: user_message,
        response_tx,
        ingress_at,
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
                if let Some(payload) =
                    nanobot_core::tools::question::parse_question_payload(&result)
                {
                    let prompt = nanobot_core::tools::question::format_question_prompt(&payload);
                    {
                        let mut pending_map = state.pending_questions.lock().await;
                        pending_map.insert(session_id.clone(), payload);
                    }
                    if !full_response.is_empty() {
                        full_response.push_str("\n\n");
                    }
                    full_response.push_str(&prompt);
                } else {
                    full_response.push_str(&format!("\n[Result: {}]\n", result));
                }
            }
            StreamChunk::Thinking(text) => {
                // For web simple view, we just append it as a block?
                // Or maybe ignore it if we want clean output?
                // Let's append it but marked.
                full_response.push_str(&format!("\n<think>{}</think>\n", text));
            }
            StreamChunk::Done { .. } => break,
        }
    }

    // Store in session history
    let mut sessions = state.sessions.lock().await;
    sessions
        .entry(session_id.clone())
        .or_insert_with(Vec::new)
        .push(req.message.clone());
    sessions
        .entry(session_id.clone())
        .or_insert_with(Vec::new)
        .push(full_response.clone());

    Ok(Json(ChatResponse {
        response: full_response,
        session_id,
    }))
}

async fn admin_token_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<AdminTokenRequest>,
) -> Result<StatusCode, StatusCode> {
    let cookie_header = headers
        .get("cookie")
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_str()
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let token = cookie_header
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

    let claims = jsonwebtoken::decode::<WebClaims>(
        &token,
        &DecodingKey::from_secret(&state.auth_secret),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    if claims.claims.sub.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    if req.token.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if nanobot_core::security::read_admin_auth_secret().is_some() {
        let Some(current) = req.current.as_deref() else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if !nanobot_core::security::verify_admin_rotation_secret(current) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    nanobot_core::security::write_admin_token(req.token.trim())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
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
        .route("/api/admin/token", post(admin_token_handler))
        .route("/api/status", get(status_handler))
        .with_state(state)
}

pub async fn run_server(
    port: u16,
    agent_tx: tokio::sync::mpsc::Sender<AgentMessage>,
) -> Result<()> {
    let state = WebState::new(agent_tx);
    let app = create_router(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    println!("🌐 WebChat UI available at http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
