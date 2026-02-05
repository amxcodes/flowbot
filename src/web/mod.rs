use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::Result;

#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAssets;

#[derive(Clone)]
pub struct WebState {
    // Will hold agent client and persistence later
    pub sessions: Arc<Mutex<std::collections::HashMap<String, Vec<String>>>>,
}

impl WebState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
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

async fn serve_index() -> Response {
    match WebAssets::get("index.html") {
        Some(content) => {
            let body = content.data;
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html")
                .body(axum::body::Body::from(body))
                .unwrap()
        }
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

async fn chat_handler(
    State(state): State<WebState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, StatusCode> {
    // For now, echo back + simple response
    // TODO: Integrate with actual agent
    let session_id = req.session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    
    let response_text = format!("I received your message: '{}'. This is a placeholder response - agent integration coming soon!", req.message);
    
    // Store in session history
    let mut sessions = state.sessions.lock().await;
    sessions.entry(session_id.clone())
        .or_insert_with(Vec::new)
        .push(req.message.clone());
    sessions.entry(session_id.clone())
        .or_insert_with(Vec::new)
        .push(response_text.clone());
    
    Ok(Json(ChatResponse {
        response: response_text,
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

pub async fn run_server(port: u16) -> Result<()> {
    let state = WebState::new();
    let app = create_router(state);
    
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    println!("🌐 WebChat UI available at http://localhost:{}", port);
    
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    
    Ok(())
}
