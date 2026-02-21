use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Admin API state
#[derive(Clone)]
pub struct AdminState {
    pub status: Arc<RwLock<ServerStatus>>,
    pub permission_manager: Arc<tokio::sync::Mutex<crate::tools::PermissionManager>>,
    pub tool_policy: Arc<crate::tools::ToolPolicy>,
    pub confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    pub rate_limit: Arc<tokio::sync::Mutex<RateLimiter>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerStatus {
    pub uptime_secs: u64,
    pub active_agents: usize,
    pub tools_registered: usize,
}

/// Create admin API router
pub fn create_admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/state", get(get_server_state))
        .route("/tools", get(list_tools))
        .route("/eval", post(eval_tool))
        .with_state(state)
}

/// Start admin API server
pub async fn start_admin_server(port: u16) -> Result<()> {
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let permission_manager = Arc::new(tokio::sync::Mutex::new(
        crate::tools::PermissionManager::new(crate::tools::permissions::SecurityProfile::standard(
            workspace,
        )),
    ));
    let tool_policy = Arc::new(
        crate::tools::ToolPolicy::restrictive()
            .allow_tool("read_file")
            .allow_tool("list_directory")
            .allow_tool("glob")
            .allow_tool("grep")
            .allow_tool("doctor")
            .allow_tool("skill")
            .allow_tool("mcp_config")
            .allow_tool("session_status")
            .allow_tool("sessions_wait"),
    );
    let confirmation_service = Arc::new(tokio::sync::Mutex::new(
        crate::tools::ConfirmationService::new(),
    ));

    if crate::security::read_admin_auth_secret().is_none() {
        tracing::warn!("Admin auth secret not set; /eval will require a token and deny requests");
    }

    let state = AdminState {
        status: Arc::new(RwLock::new(ServerStatus::default())),
        permission_manager,
        tool_policy,
        confirmation_service,
        rate_limit: Arc::new(tokio::sync::Mutex::new(RateLimiter::new(
            30,
            Duration::from_secs(60),
        ))),
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

#[derive(Debug, Deserialize)]
struct EvalRequest {
    tool: String,
    #[serde(default)]
    args: Option<Value>,
}

#[derive(Debug, Serialize)]
struct EvalResponse {
    output: String,
}

async fn eval_tool(
    State(state): State<AdminState>,
    headers: HeaderMap,
    Json(payload): Json<EvalRequest>,
) -> std::result::Result<Json<EvalResponse>, (StatusCode, String)> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tracing::debug!(request_id = %request_id, "Admin eval request received");

    let token = crate::security::read_admin_auth_secret();

    let token = match token {
        Some(value) => value,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                "Admin secret not set. Set primary password in setup, or run nanobot admin-token set".to_string(),
            ));
        }
    };

    let expected = format!("Bearer {}", token);
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    if !crate::security::secure_eq(auth, &expected) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Missing or invalid admin token".to_string(),
        ));
    }

    {
        let mut limiter = state.rate_limit.lock().await;
        if !limiter.allow(auth) {
            tracing::warn!("Admin eval rate limit exceeded");
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded".to_string(),
            ));
        }
    }

    let mut call = match payload.args {
        Some(Value::Object(obj)) => Value::Object(obj),
        Some(_) => Value::Object(serde_json::Map::new()),
        None => Value::Object(serde_json::Map::new()),
    };
    let tool_name = payload.tool.clone();
    if let Value::Object(ref mut obj) = call {
        obj.insert("tool".to_string(), Value::String(payload.tool));
    }

    let tool_input =
        serde_json::to_string(&call).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    #[cfg(feature = "browser")]
    let result = crate::tools::executor::execute_tool(
        &tool_input,
        crate::tools::executor::ExecuteToolContext {
            cron_scheduler: None,
            agent_manager: None,
            memory_manager: None,
            persistence: None,
            permission_manager: Some(&*state.permission_manager),
            tool_policy: Some(&state.tool_policy),
            confirmation_service: Some(&*state.confirmation_service),
            skill_loader: None,
            browser_client: None,
            tenant_id: None,
            mcp_manager: None,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    #[cfg(not(feature = "browser"))]
    let result = crate::tools::executor::execute_tool(
        &tool_input,
        crate::tools::executor::ExecuteToolContext {
            cron_scheduler: None,
            agent_manager: None,
            memory_manager: None,
            persistence: None,
            permission_manager: Some(&*state.permission_manager),
            tool_policy: Some(&state.tool_policy),
            confirmation_service: Some(&*state.confirmation_service),
            skill_loader: None,
            tenant_id: None,
            mcp_manager: None,
        },
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    tracing::info!("Admin eval executed tool {}", tool_name);

    Ok(Json(EvalResponse { output: result }))
}

pub struct RateLimiter {
    limit: usize,
    window: Duration,
    entries: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            entries: HashMap::new(),
        }
    }

    fn allow(&mut self, key: &str) -> bool {
        let now = Instant::now();
        let window = self.window;
        let list = self.entries.entry(key.to_string()).or_default();
        list.retain(|t| now.duration_since(*t) <= window);
        if list.len() >= self.limit {
            return false;
        }
        list.push(now);
        true
    }
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
