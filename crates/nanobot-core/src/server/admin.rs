use anyhow::Result;
use axum::{
    Router,
    routing::get,
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Admin API state
#[derive(Clone)]
pub struct AdminState {
    pub status: Arc<RwLock<ServerStatus>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub uptime_secs: u64,
    pub active_agents: usize,
    pub tools_registered: usize,
}

impl Default for ServerStatus {
    fn default() -> Self {
        Self {
            uptime_secs: 0,
            active_agents: 0,
            tools_registered: 0,
        }
    }
}

/// Create admin API router
pub fn create_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/state", get(get_server_state))
        .route("/tools", get(list_tools))
        .with_state(state)
}

/// Start admin API server
pub async fn start_admin_server(port: u16) -> Result<()> {
    let state = AdminState {
        status: Arc::new(RwLock::new(ServerStatus::default())),
    };
    
    let app = create_admin_router(state);
    
    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    
    println!("🔧 Admin API listening on http://{}", addr);
    println!("   - Health: http://{}/health", addr);
    println!("   - State:  http://{}/state", addr);
    println!("   - Tools:  http://{}/tools", addr);
    
    axum::serve(listener, app).await?;
    Ok(())
}

// === Handlers ===

async fn health_check() -> &'static str {
    "OK"
}

async fn get_server_state(State(state): State<AdminState>) -> Json<ServerStatus> {
    let status = state.status.read().await;
    Json(status.clone())
}

async fn list_tools() -> Json<Value> {
    use crate::tools::definitions::get_tool_registry;
    
    let registry = get_tool_registry();
    let tools = registry.list_tools();
    
    Json(json!({
        "tools": tools,
        "count": tools.len()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_health_check() {
        let result = health_check().await;
        assert_eq!(result, "OK");
    }
}
