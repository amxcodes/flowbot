use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub uptime_seconds: u64,
    pub version: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Clone)]
pub struct HealthState {
    start_time: Arc<RwLock<SystemTime>>,
}

impl HealthState {
    pub fn new() -> Self {
        Self {
            start_time: Arc::new(RwLock::new(SystemTime::now())),
        }
    }

    pub async fn get_uptime(&self) -> u64 {
        let start = *self.start_time.read().await;
        SystemTime::now()
            .duration_since(start)
            .unwrap_or(Duration::ZERO)
            .as_secs()
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

async fn health_check(State(state): State<HealthState>) -> Json<HealthStatus> {
    let uptime = state.get_uptime().await;

    Json(HealthStatus {
        healthy: true,
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
        status: "running".to_string(),
        details: None,
    })
}

async fn liveness() -> StatusCode {
    StatusCode::OK
}

async fn readiness() -> impl IntoResponse {
    let config_ok = crate::config::Config::load().is_ok();

    let storage_ok = {
        let db_path = std::path::PathBuf::from(".")
            .join(".nanobot")
            .join("sessions.db");
        if let Some(parent) = db_path.parent() {
            std::fs::metadata(parent)
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
        } else {
            false
        }
    };

    if config_ok && storage_ok {
        (StatusCode::OK, Json(serde_json::json!({"ready": true})))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ready": false,
                "config_ok": config_ok,
                "storage_ok": storage_ok,
            })),
        )
    }
}

pub fn create_health_router() -> Router {
    let state = HealthState::new();

    Router::new()
        .route("/health", get(health_check))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .with_state(state)
}

/// Start the health check server on the given port
pub async fn start_health_server(port: u16) -> anyhow::Result<()> {
    let app = create_health_router();

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Health check server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
