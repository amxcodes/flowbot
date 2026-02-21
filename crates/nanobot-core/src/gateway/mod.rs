// Gateway module - entry point for the API server
pub mod adapter;
pub mod agent_manager;
pub mod discord_adapter;
pub mod google_chat_adapter;
pub mod onboarding;
pub mod registry;
pub mod router;
pub mod skill_chat;
pub mod slack_adapter;
pub mod teams_adapter;
pub mod telegram_adapter;

use anyhow::Result;
use axum::{
    body::Bytes,
    Json, Router,
    extract::{
        Path as AxumPath, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{MethodRouter, get, post},
};
use chrono::{Duration, Utc};
use futures::{FutureExt, sink::SinkExt, stream::StreamExt};
use hmac::{Hmac, Mac};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use rusqlite::params;
use serde::Serialize;
use serde_json::json;
use sha2::Digest;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration as StdDuration, Instant};
use tokio::sync::{mpsc, oneshot};

use crate::agent::{AgentMessage, StreamChunk, TerminalKind};
use crate::config::Config;
use crate::gateway::adapter::build_session_id;

async fn load_config_async() -> anyhow::Result<Config> {
    crate::blocking::fs("config_load", crate::config::Config::load).await
}

async fn command_exists_async(command: &str) -> bool {
    crate::blocking::command_exists(command, std::time::Duration::from_secs(2)).await
}

async fn openclaw_auth_writable_async() -> bool {
    crate::blocking::fs("openclaw_auth_create_dir", || {
        if let Some(home) = dirs::home_dir() {
            std::fs::create_dir_all(home.join(".openclaw").join("auth"))?;
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .await
    .unwrap_or(false)
}

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

fn require_ws_token_per_message() -> bool {
    let requested = std::env::var("NANOBOT_GATEWAY_REQUIRE_TOKEN")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true);

    if requested {
        return true;
    }

    let insecure_override = std::env::var("NANOBOT_ALLOW_INSECURE_WS")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    if !insecure_override {
        tracing::warn!(
            "Ignoring NANOBOT_GATEWAY_REQUIRE_TOKEN=false because NANOBOT_ALLOW_INSECURE_WS is not enabled"
        );
        return true;
    }

    false
}

fn ws_max_inflight_per_session() -> usize {
    std::env::var("NANOBOT_MAX_INFLIGHT_PER_SESSION")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1)
}

#[cfg(test)]
fn onboarding_bypass_for_tests() -> bool {
    std::env::var("NANOBOT_TEST_BYPASS_ONBOARDING")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(not(test))]
fn onboarding_bypass_for_tests() -> bool {
    false
}

type HmacSha256 = Hmac<sha2::Sha256>;

static WEBHOOK_NONCE_CACHE: once_cell::sync::Lazy<tokio::sync::Mutex<std::collections::HashMap<String, i64>>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));
static LAST_NONCE_DB_CLEANUP_TS: AtomicI64 = AtomicI64::new(0);
static LAST_STICKY_VIOLATION_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const SETTINGS_AUTH_CACHE_TTL: StdDuration = StdDuration::from_secs(5);

#[derive(Default)]
struct SettingsAuthCache {
    loaded_at: Option<Instant>,
    candidates: Vec<String>,
}

static SETTINGS_AUTH_CACHE: once_cell::sync::Lazy<std::sync::RwLock<SettingsAuthCache>> =
    once_cell::sync::Lazy::new(|| std::sync::RwLock::new(SettingsAuthCache::default()));

fn header_string(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn metric_name_with_tags(base: &str, tags: &[(&str, &str)]) -> String {
    if tags.is_empty() {
        return base.to_string();
    }
    let suffix = tags
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(",");
    format!("{}{{{}}}", base, suffix)
}

fn incr_counter(base: &str, tags: &[(&str, &str)]) {
    crate::metrics::GLOBAL_METRICS.increment_counter(&metric_name_with_tags(base, tags), 1);
}

fn record_duration(base: &str, tags: &[(&str, &str)], started_at: Instant, success: bool) {
    crate::metrics::GLOBAL_METRICS.record_duration(
        &metric_name_with_tags(base, tags),
        started_at.elapsed(),
        success,
    );
}

fn now_epoch_ms_u64() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn strict_multi_replica_sticky_mode() -> bool {
    let strict = std::env::var("NANOBOT_SCALING_STRICT")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let replicas = std::env::var("NANOBOT_REPLICA_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);
    strict && replicas > 1
}

fn sticky_violation_grace_ms() -> u64 {
    std::env::var("NANOBOT_STICKY_VIOLATION_GRACE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(60_000)
}

fn record_sticky_violation() {
    LAST_STICKY_VIOLATION_MS.store(now_epoch_ms_u64(), std::sync::atomic::Ordering::Relaxed);
}

fn sticky_health_degraded_now() -> bool {
    if !strict_multi_replica_sticky_mode() {
        crate::metrics::GLOBAL_METRICS.set_gauge("gateway_health_degraded", 0.0);
        return false;
    }
    let last = LAST_STICKY_VIOLATION_MS.load(std::sync::atomic::Ordering::Relaxed);
    if last == 0 {
        crate::metrics::GLOBAL_METRICS.set_gauge("gateway_health_degraded", 0.0);
        return false;
    }
    let now = now_epoch_ms_u64();
    let degraded = now.saturating_sub(last) <= sticky_violation_grace_ms();
    crate::metrics::GLOBAL_METRICS.set_gauge("gateway_health_degraded", if degraded { 1.0 } else { 0.0 });
    degraded
}

fn record_sticky_signal_missing() {
    crate::metrics::GLOBAL_METRICS.increment_counter("distributed_sticky_signal_missing_total", 1);
    record_sticky_violation();
}

fn record_sticky_signal_conflict() {
    crate::metrics::GLOBAL_METRICS.increment_counter("distributed_sticky_signal_conflict_total", 1);
    record_sticky_violation();
}

#[cfg(test)]
fn reset_sticky_violation_state() {
    LAST_STICKY_VIOLATION_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::metrics::GLOBAL_METRICS.set_gauge("gateway_health_degraded", 0.0);
}

fn signing_secret_candidates(signing_secret_env_key: &str, key_id: Option<&str>) -> Vec<(String, String)> {
    let mut candidates = Vec::new();

    let active_key = format!("{}_ACTIVE", signing_secret_env_key);
    let previous_key = format!("{}_PREVIOUS", signing_secret_env_key);
    let active = std::env::var(&active_key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let previous = std::env::var(&previous_key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let fallback = std::env::var(signing_secret_env_key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    match key_id.map(|k| k.to_ascii_lowercase()) {
        Some(id) if id == "active" => {
            if let Some(s) = active {
                candidates.push(("active".to_string(), s));
            }
        }
        Some(id) if id == "previous" => {
            if let Some(s) = previous {
                candidates.push(("previous".to_string(), s));
            }
        }
        Some(_) => {}
        _ => {
            if let Some(s) = active {
                candidates.push(("active".to_string(), s));
            }
            if let Some(s) = previous {
                candidates.push(("previous".to_string(), s));
            }
            if let Some(s) = fallback {
                candidates.push(("legacy".to_string(), s));
            }
        }
    }

    candidates.dedup_by(|a, b| a.1 == b.1);
    candidates
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

async fn check_and_store_nonce(provider: &str, nonce: &str, now_ts: i64, window_secs: i64) -> bool {
    if let Some(db_path) = std::env::var("NANOBOT_WEBHOOK_NONCE_DB_PATH")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        let provider = provider.to_string();
        let nonce_hash = format!(
            "{:x}",
            sha2::Sha256::digest(format!("{}:{}", provider, nonce).as_bytes())
        );
        let op_started = Instant::now();
        let nonce_ok = crate::blocking::sqlite("webhook_nonce_check_and_store", move || -> anyhow::Result<bool> {
            let path = std::path::PathBuf::from(db_path);
            if let Some(parent) = path.parent()
                && std::fs::create_dir_all(parent).is_err()
            {
                return Ok(false);
            }

            let Ok(conn) = rusqlite::Connection::open(path) else {
                return Ok(false);
            };
            if conn.pragma_update(None, "journal_mode", "WAL").is_err() {
                return Ok(false);
            }
            if conn.busy_timeout(std::time::Duration::from_millis(250)).is_err() {
                return Ok(false);
            }
            if conn
                .execute(
                    "CREATE TABLE IF NOT EXISTS webhook_nonces (
                        key TEXT PRIMARY KEY,
                        seen_at INTEGER NOT NULL
                    )",
                    [],
                )
                .is_err()
            {
                return Ok(false);
            }

            let _ = conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_webhook_nonces_seen_at ON webhook_nonces(seen_at)",
                [],
            );

            let last_cleanup = LAST_NONCE_DB_CLEANUP_TS.load(Ordering::Relaxed);
            if now_ts.saturating_sub(last_cleanup) >= 60 {
                let cutoff = now_ts.saturating_sub(window_secs);
                let cleanup_started = std::time::Instant::now();
                let deleted = conn
                    .execute("DELETE FROM webhook_nonces WHERE seen_at < ?1", params![cutoff])
                    .unwrap_or(0);
                crate::metrics::GLOBAL_METRICS.increment_counter(
                    "webhook_nonce_cleanup_rows_total{backend=sqlite}",
                    deleted as u64,
                );
                crate::metrics::GLOBAL_METRICS.record_duration(
                    "webhook_nonce_cleanup_duration_seconds{backend=sqlite}",
                    cleanup_started.elapsed(),
                    true,
                );
                LAST_NONCE_DB_CLEANUP_TS.store(now_ts, Ordering::Relaxed);
            }

            let inserted = conn.execute(
                "INSERT INTO webhook_nonces (key, seen_at) VALUES (?1, ?2)
                 ON CONFLICT(key) DO NOTHING",
                params![nonce_hash, now_ts],
            )
            .map(|rows| rows == 1)
            .unwrap_or(false);

            Ok(inserted)
        })
        .await
        .ok()
        .unwrap_or(false);

        record_duration(
            "webhook_nonce_store_duration_seconds",
            &[("backend", "sqlite")],
            op_started,
            nonce_ok,
        );

        if !nonce_ok {
            incr_counter(
                "webhook_nonce_store_errors_total",
                &[("backend", "sqlite")],
            );
        }

        return nonce_ok;
    }

    let mut cache = WEBHOOK_NONCE_CACHE.lock().await;
    cache.retain(|_, seen_ts| now_ts.saturating_sub(*seen_ts) <= window_secs);

    let key = format!("{}:{}", provider, nonce);
    if cache.contains_key(&key) {
        return false;
    }

    cache.insert(key, now_ts);
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "webhook_nonce_cache_size{backend=memory}",
        cache.len() as f64,
    );
    true
}

async fn verify_webhook_request(
    provider: &str,
    headers: &HeaderMap,
    body: &[u8],
    token_env_key: &str,
    signing_secret_env_key: &str,
) -> Result<(), &'static str> {
    let token_secret = std::env::var(token_env_key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let key_id = header_string(headers, "x-nanobot-signing-key-id")
        .or_else(|| header_string(headers, "x-signing-key-id"));
    if let Some(id) = key_id.as_deref() {
        let normalized = id.trim().to_ascii_lowercase();
        if normalized != "active" && normalized != "previous" {
            return Err("unsupported signing key id");
        }
    }
    let signing_candidates = signing_secret_candidates(signing_secret_env_key, key_id.as_deref());

    if signing_candidates.iter().any(|(_, secret)| secret.len() < 32) {
        return Err("webhook signing secret too short");
    }

    if token_secret.is_none() && signing_candidates.is_empty() {
        return Err("webhook auth is not configured");
    }

    if let Some(secret) = token_secret {
        let candidate = header_string(headers, "x-nanobot-webhook-token").unwrap_or_default();
        if !crate::security::secure_eq(&candidate, &secret) {
            return Err("invalid webhook token");
        }
    }

    if !signing_candidates.is_empty() {
        let signature = header_string(headers, "x-nanobot-signature")
            .or_else(|| header_string(headers, "x-signature"))
            .ok_or("missing webhook signature")?;

        let timestamp_raw = header_string(headers, "x-nanobot-timestamp")
            .or_else(|| header_string(headers, "x-timestamp"))
            .ok_or("missing webhook timestamp")?;

        let nonce = header_string(headers, "x-nanobot-nonce")
            .or_else(|| header_string(headers, "x-nonce"))
            .ok_or("missing webhook nonce")?;

        let timestamp = timestamp_raw
            .parse::<i64>()
            .map_err(|_| "invalid webhook timestamp")?;

        let now_ts = chrono::Utc::now().timestamp();
        let window_secs = std::env::var("NANOBOT_WEBHOOK_MAX_SKEW_SECONDS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0 && *v <= 3600)
            .unwrap_or(300);

        if timestamp > now_ts.saturating_add(window_secs) {
            return Err("webhook timestamp too far in future");
        }

        if now_ts.saturating_sub(timestamp) > window_secs {
            return Err("webhook timestamp too old");
        }

        if !check_and_store_nonce(provider, &nonce, now_ts, window_secs).await {
            return Err("replayed webhook nonce");
        }

        let payload = format!("{}.{}.", timestamp, nonce);
        let mut matched_key: Option<String> = None;
        let signature_ok = signing_candidates.iter().any(|(key_name, secret)| {
            let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
                return false;
            };
            mac.update(payload.as_bytes());
            mac.update(body);
            let expected = to_hex(&mac.finalize().into_bytes());
            let ok = crate::security::secure_eq(&signature.to_ascii_lowercase(), &expected);
            if ok {
                matched_key = Some(key_name.clone());
            }
            ok
        });

        if !signature_ok {
            return Err("invalid webhook signature");
        }

        if let Some(key_name) = matched_key {
            tracing::debug!("Validated webhook signature using {} key", key_name);
        }
    }

    Ok(())
}

fn compat_extract_token(params: &serde_json::Value) -> String {
    params
        .get("token")
        .and_then(|v| v.as_str())
        .or_else(|| {
            params
                .get("auth")
                .and_then(|v| v.get("token"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("")
        .trim()
        .to_string()
}

const MAX_WS_REQUEST_ID_LEN: usize = 64;

fn is_valid_ws_request_id(request_id: &str) -> bool {
    if request_id.is_empty() || request_id.len() > MAX_WS_REQUEST_ID_LEN {
        return false;
    }

    let mut chars = request_id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.'))
}

fn parse_compat_request_id(req: &serde_json::Value) -> Result<String, &'static str> {
    let Some(raw_id) = req.get("id") else {
        return Err("invalid_request_id");
    };
    let Some(request_id) = raw_id.as_str() else {
        return Err("invalid_request_id");
    };
    if !is_valid_ws_request_id(request_id) {
        return Err("invalid_request_id");
    }
    Ok(request_id.to_string())
}

fn parse_non_compat_request_id(req: &serde_json::Value) -> Result<Option<String>, &'static str> {
    let Some(raw_id) = req.get("request_id") else {
        return Ok(None);
    };
    let Some(request_id) = raw_id.as_str() else {
        return Err("invalid_request_id");
    };
    if !is_valid_ws_request_id(request_id) {
        return Err("invalid_request_id");
    }
    Ok(Some(request_id.to_string()))
}

fn is_compat_send_method(method: &str) -> bool {
    matches!(
        method,
        "agent"
            | "send"
            | "message"
            | "agent.send"
            | "agent/send"
            | "agent/message"
            | "agent.send.message"
            | "agent/send/message"
    )
}

fn is_compat_confirmation_method(method: &str) -> bool {
    matches!(
        method,
        "confirmation_response"
            | "confirmation.respond"
            | "confirmation/response"
            | "confirmation/respond"
    )
}

type WsSender = std::sync::Arc<tokio::sync::Mutex<futures::stream::SplitSink<WebSocket, WsMessage>>>;

fn ws_write_timeout() -> StdDuration {
    std::env::var("NANOBOT_WS_WRITE_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(StdDuration::from_millis)
        .unwrap_or_else(|| StdDuration::from_millis(1500))
}

async fn send_ws_text_timed(ws_tx: &WsSender, payload: String) -> std::result::Result<(), String> {
    let started = std::time::Instant::now();
    let timeout = ws_write_timeout();
    #[cfg(test)]
    if std::env::var("NANOBOT_TEST_FORCE_WS_SEND_TIMEOUT")
        .ok()
        .as_deref()
        == Some("1")
    {
        crate::metrics::GLOBAL_METRICS.record_duration(
            "gateway_ws_send_wait_seconds",
            started.elapsed(),
            false,
        );
        crate::metrics::GLOBAL_METRICS.increment_counter("gateway_ws_send_timeouts_total", 1);
        crate::metrics::GLOBAL_METRICS.increment_counter("slow_client_disconnects_total", 1);
        return Err("forced ws send timeout (test)".to_string());
    }

    let send_result = tokio::time::timeout(timeout, async {
        ws_tx.lock().await.send(WsMessage::Text(payload)).await
    })
    .await;

    match send_result {
        Ok(Ok(())) => {
            crate::metrics::GLOBAL_METRICS.record_duration(
                "gateway_ws_send_wait_seconds",
                started.elapsed(),
                true,
            );
            Ok(())
        }
        Ok(Err(e)) => {
            crate::metrics::GLOBAL_METRICS.record_duration(
                "gateway_ws_send_wait_seconds",
                started.elapsed(),
                false,
            );
            Err(e.to_string())
        }
        Err(_) => {
            crate::metrics::GLOBAL_METRICS.record_duration(
                "gateway_ws_send_wait_seconds",
                started.elapsed(),
                false,
            );
            crate::metrics::GLOBAL_METRICS.increment_counter("gateway_ws_send_timeouts_total", 1);
            crate::metrics::GLOBAL_METRICS.increment_counter("slow_client_disconnects_total", 1);
            Err(format!("ws send timed out after {}ms", timeout.as_millis()))
        }
    }
}

#[derive(Clone)]
pub struct GatewayConfig {
    pub port: u16,
    pub bind_host: String,
}

#[derive(Clone)]
pub struct Gateway {
    config: GatewayConfig,
    dm_scope: crate::config::DmScope,
    gateway_session_secret: std::sync::Arc<Vec<u8>>,
    agent_tx: mpsc::Sender<AgentMessage>,
    confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    pending_questions: crate::distributed::PendingQuestionStoreRef,
    correlation_store: crate::distributed::SessionCorrelationStoreRef,
    #[cfg(test)]
    panic_send_on_toolresult: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoutePolicy {
    Public,
    Protected,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RoutePolicyEntry {
    method: &'static str,
    path: &'static str,
    policy: RoutePolicy,
}

fn register_route(
    router: Router,
    registry: &mut Vec<RoutePolicyEntry>,
    method: &'static str,
    path: &'static str,
    policy: RoutePolicy,
    handler: MethodRouter,
) -> Router {
    registry.push(RoutePolicyEntry {
        method,
        path,
        policy,
    });
    router.route(path, handler)
}

fn register_gateway_routes(
    router: Router,
    registry: &mut Vec<RoutePolicyEntry>,
    gateway_state: Arc<Gateway>,
) -> Router {
    let router = register_route(
        router,
        registry,
        "GET",
        "/health",
        RoutePolicy::Public,
        get(health_check),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/metrics",
        RoutePolicy::Internal,
        get(metrics_handler),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/api/bootstrap",
        RoutePolicy::Public,
        get(api_bootstrap),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/api/channels/status",
        RoutePolicy::Public,
        get(channels_status),
    );
    let router = register_route(
        router,
        registry,
        "GET|PATCH",
        "/api/channels/config",
        RoutePolicy::Protected,
        get(get_channels_config).patch(patch_channels_config),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/channels/:id/verify",
        RoutePolicy::Protected,
        post(verify_channel),
    );
    let router = register_route(
        router,
        registry,
        "GET|PATCH",
        "/api/settings",
        RoutePolicy::Protected,
        get(get_settings).patch(patch_settings),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/settings/auth/google/connect",
        RoutePolicy::Protected,
        post(auth_google_connect),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/settings/auth/google/complete",
        RoutePolicy::Protected,
        post(auth_google_complete),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/settings/security/profile",
        RoutePolicy::Protected,
        post(set_security_profile),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/api/settings/doctor",
        RoutePolicy::Protected,
        get(settings_doctor),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/api/skills",
        RoutePolicy::Public,
        get(list_skills),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/skills/install",
        RoutePolicy::Protected,
        post(install_skill),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/api/skills/:id/schema",
        RoutePolicy::Public,
        get(skill_schema),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/skills/:id/config",
        RoutePolicy::Protected,
        post(update_skill_config),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/api/skills/:id/test",
        RoutePolicy::Public,
        post(test_skill),
    );
    let router = register_route(
        router,
        registry,
        "GET",
        "/ws",
        RoutePolicy::Public,
        get(ws_handler).with_state(gateway_state.clone()),
    );
    let router = register_route(
        router,
        registry,
        "POST",
        "/webhooks/teams",
        RoutePolicy::Protected,
        axum::routing::post(teams_webhook).with_state(gateway_state.clone()),
    );
    register_route(
        router,
        registry,
        "POST",
        "/webhooks/google_chat",
        RoutePolicy::Protected,
        axum::routing::post(google_chat_webhook).with_state(gateway_state.clone()),
    )
}

#[cfg(test)]
fn gateway_route_policies() -> Vec<RoutePolicyEntry> {
    vec![
        RoutePolicyEntry { method: "GET", path: "/health", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "GET", path: "/metrics", policy: RoutePolicy::Internal },
        RoutePolicyEntry { method: "GET", path: "/api/bootstrap", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "GET", path: "/api/channels/status", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "GET|PATCH", path: "/api/channels/config", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "POST", path: "/api/channels/:id/verify", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "GET|PATCH", path: "/api/settings", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "POST", path: "/api/settings/auth/google/connect", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "POST", path: "/api/settings/auth/google/complete", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "POST", path: "/api/settings/security/profile", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "GET", path: "/api/settings/doctor", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "GET", path: "/api/skills", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "POST", path: "/api/skills/install", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "GET", path: "/api/skills/:id/schema", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "POST", path: "/api/skills/:id/config", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "POST", path: "/api/skills/:id/test", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "GET", path: "/ws", policy: RoutePolicy::Public },
        RoutePolicyEntry { method: "POST", path: "/webhooks/teams", policy: RoutePolicy::Protected },
        RoutePolicyEntry { method: "POST", path: "/webhooks/google_chat", policy: RoutePolicy::Protected },
    ]
}

impl Gateway {
    pub fn new(
        config: GatewayConfig,
        agent_tx: mpsc::Sender<AgentMessage>,
        confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    ) -> Self {
        Self::new_with_pending_store(
            config,
            agent_tx,
            confirmation_service,
            crate::distributed::build_pending_question_store(),
            crate::distributed::build_session_correlation_store(),
        )
    }

    pub fn new_with_pending_store(
        config: GatewayConfig,
        agent_tx: mpsc::Sender<AgentMessage>,
        confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
        pending_questions: crate::distributed::PendingQuestionStoreRef,
        correlation_store: crate::distributed::SessionCorrelationStoreRef,
    ) -> Self {
        let dm_scope = Config::load()
            .map(|c| c.session.dm_scope)
            .unwrap_or_default();
        let gateway_session_secret = std::env::var("NANOBOT_GATEWAY_SESSION_SECRET")
            .map(|s| s.into_bytes())
            .unwrap_or_else(|_| {
                crate::security::get_or_create_session_secrets()
                    .map(|s| s.gateway_session_secret.into_bytes())
                    .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string().into_bytes())
            });

        Self {
            config,
            dm_scope,
            gateway_session_secret: std::sync::Arc::new(gateway_session_secret),
            agent_tx,
            confirmation_service,
            pending_questions,
            correlation_store,
            #[cfg(test)]
            panic_send_on_toolresult: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn build_router(&self) -> Router {
        let mut registry = Vec::new();
        let gateway_state = Arc::new(self.clone());
        register_gateway_routes(
            Router::new(),
            &mut registry,
            gateway_state,
        )
    }

    pub async fn start(&self) -> Result<()> {
        crate::security::enforce_runtime_security_baseline()?;
        crate::distributed::enforce_distributed_backend_support()?;
        crate::distributed::enforce_scaling_mode_support()?;
        crate::distributed::enforce_multi_replica_runtime_readiness().await?;

        let distributed_backend = crate::distributed::selected_distributed_store_backend();
        let scaling_mode = crate::distributed::selected_scaling_mode();
        let effective_scaling_mode = crate::distributed::effective_scaling_mode();
        let sticky_signal_header = crate::distributed::sticky_signal_header();
        tracing::info!(
            backend = ?distributed_backend,
            "Distributed store backend selection"
        );
        tracing::info!(
            requested_scaling_mode = ?scaling_mode,
            effective_scaling_mode = ?effective_scaling_mode,
            sticky_signal_header = sticky_signal_header.as_deref().unwrap_or("<unset>"),
            "Scaling mode selection"
        );

        tracing::warn!(
            "Gateway runtime is single-instance scoped: session state and pending questions are in-memory"
        );
        tracing::warn!(
            "Horizontal scaling is not guaranteed for session continuity without sticky routing and external shared state"
        );

        let app = self.build_router();

        let bind_addr = format!("{}:{}", self.config.bind_host, self.config.port);
        println!("🚀 Gateway listening on {}", bind_addr);

        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::{body::Body, http::Request};
    use futures::{SinkExt, StreamExt};
    use serde_json::Value;
    use std::env;
    use std::sync::Mutex;
    use std::time::Duration;
    use tower::util::ServiceExt;
    use tokio_tungstenite::MaybeTlsStream;
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    static TEST_ENV_LOCK: once_cell::sync::Lazy<Mutex<()>> =
        once_cell::sync::Lazy::new(|| Mutex::new(()));
    static WS_TEST_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
        once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

    type TestWs = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

    async fn recv_json(ws: &mut TestWs) -> Value {
        let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("json frame timeout")
            .expect("json frame missing")
            .expect("json websocket frame ok");
        let text = match frame {
            Message::Text(t) => t,
            other => panic!("expected text frame, got {other:?}"),
        };
        serde_json::from_str(&text).expect("valid json frame")
    }

    fn prometheus_counter_value(metric_name: &str) -> f64 {
        let dump = crate::metrics::GLOBAL_METRICS.export_prometheus();
        let prefix = format!("{} ", metric_name);
        dump.lines()
            .find_map(|line| {
                line.strip_prefix(&prefix)
                    .and_then(|v| v.trim().parse::<f64>().ok())
            })
            .unwrap_or(0.0)
    }

    fn spawn_test_agent() -> mpsc::Sender<AgentMessage> {
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentMessage>(8);
        tokio::spawn(async move {
            while let Some(msg) = agent_rx.recv().await {
                if msg.content.contains("question-test") {
                    let _ = msg
                        .response_tx
                        .send(StreamChunk::ToolResult(
                            r#"{"type":"question","header":"Mode","question":"Pick one","options":["A","B"],"multiple":false}"#
                                .to_string(),
                        ))
                        .await;
                } else {
                    if msg.content.contains("slow-test") {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    let _ = msg
                        .response_tx
                        .send(StreamChunk::TextDelta("compat-ok".to_string()))
                        .await;
                    let _ = msg
                        .response_tx
                        .send(StreamChunk::Done {
                            request_id: msg.request_id.clone(),
                            kind: TerminalKind::SuccessDone,
                        })
                        .await;
                }
            }
        });
        agent_tx
    }

    async fn spawn_ws_server(gateway: Arc<Gateway>) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .with_state(gateway.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (addr, server)
    }

    async fn connect_test_socket_with_token(addr: std::net::SocketAddr) -> (TestWs, String) {
        let url = format!("ws://{}/ws", addr);
        let (mut ws, _) = connect_async(url).await.expect("connect websocket");

        let init_json = recv_json(&mut ws).await;
        assert_eq!(
            init_json.get("type").and_then(|v| v.as_str()),
            Some("session_init")
        );
        let token = init_json
            .get("token")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        assert!(!token.is_empty(), "session_init must include token");
        (ws, token)
    }

    async fn connect_test_socket(addr: std::net::SocketAddr) -> TestWs {
        let (ws, _token) = connect_test_socket_with_token(addr).await;
        ws
    }

    async fn open_test_socket() -> (TestWs, tokio::task::JoinHandle<()>) {
        let (ws, server, _gateway) = open_test_socket_with_gateway().await;
        (ws, server)
    }

    async fn open_test_socket_with_gateway() -> (TestWs, tokio::task::JoinHandle<()>, Arc<Gateway>) {
        let agent_tx = spawn_test_agent();
        open_test_socket_with_agent_tx(agent_tx).await
    }

    async fn open_test_socket_with_agent_tx(
        agent_tx: mpsc::Sender<AgentMessage>,
    ) -> (TestWs, tokio::task::JoinHandle<()>, Arc<Gateway>) {

        let gateway = Arc::new(Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            agent_tx,
            Arc::new(tokio::sync::Mutex::new(
                crate::tools::ConfirmationService::new(),
            )),
        ));
        let (addr, server) = spawn_ws_server(gateway.clone()).await;
        let ws = connect_test_socket(addr).await;

        (ws, server, gateway)
    }

    fn reset_settings_auth_cache() {
        if let Ok(mut cache) = SETTINGS_AUTH_CACHE.write() {
            cache.loaded_at = None;
            cache.candidates.clear();
        }
    }

    async fn compat_connect_and_get_token(ws: &mut TestWs, req_id: &str) -> String {
        let connect_req = serde_json::json!({
            "type": "req",
            "id": req_id,
            "method": "connect",
            "params": {}
        });
        ws.send(Message::Text(connect_req.to_string()))
            .await
            .expect("send connect req");

        let connect_json = recv_json(ws).await;
        assert_eq!(
            connect_json.get("type").and_then(|v| v.as_str()),
            Some("res")
        );
        assert_eq!(
            connect_json.get("id").and_then(|v| v.as_str()),
            Some(req_id)
        );
        assert_eq!(connect_json.get("ok").and_then(|v| v.as_bool()), Some(true));

        let token = connect_json
            .get("payload")
            .and_then(|v| v.get("token"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        assert!(!token.is_empty());
        token
    }

    #[test]
    fn test_session_token_roundtrip() {
        let secret = b"test-secret";
        let session_id = "session-1";
        let token = encode_session_token(secret, session_id);
        assert!(validate_session_token(secret, &token, session_id));
        assert!(!validate_session_token(secret, &token, "other"));
    }

    #[test]
    fn test_compat_extract_token_plain_and_nested() {
        let plain = serde_json::json!({ "token": " abc123 " });
        assert_eq!(compat_extract_token(&plain), "abc123");

        let nested = serde_json::json!({ "auth": { "token": " nested-token " } });
        assert_eq!(compat_extract_token(&nested), "nested-token");

        let missing = serde_json::json!({ "auth": {} });
        assert!(compat_extract_token(&missing).is_empty());
    }

    #[test]
    fn test_request_id_validation_rules() {
        assert!(is_valid_ws_request_id("req-1"));
        assert!(is_valid_ws_request_id("a:b_c.d-9"));
        assert!(!is_valid_ws_request_id(""));
        assert!(!is_valid_ws_request_id("-bad-start"));
        assert!(!is_valid_ws_request_id("contains space"));
        assert!(!is_valid_ws_request_id(&"a".repeat(MAX_WS_REQUEST_ID_LEN + 1)));
    }

    #[test]
    fn test_compat_method_aliases() {
        assert!(is_compat_send_method("agent"));
        assert!(is_compat_send_method("agent.send"));
        assert!(is_compat_send_method("agent/send/message"));
        assert!(!is_compat_send_method("health"));

        assert!(is_compat_confirmation_method("confirmation_response"));
        assert!(is_compat_confirmation_method("confirmation.respond"));
        assert!(is_compat_confirmation_method("confirmation/respond"));
        assert!(!is_compat_confirmation_method("confirmation"));
    }

    #[tokio::test]
    async fn test_ws_req_res_event_compat_flow() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let (mut ws, server) = open_test_socket().await;
        let token = compat_connect_and_get_token(&mut ws, "req-1").await;

        let send_req = serde_json::json!({
            "type": "req",
            "id": "req-2",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "hello from compat test"
            }
        });
        ws.send(Message::Text(send_req.to_string()))
            .await
            .expect("send agent req");

        let mut got_accept_res = false;
        let mut got_delta = false;
        let mut got_done = false;

        for _ in 0..8 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("compat frame timeout")
                .expect("compat frame missing")
                .expect("compat ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };

            let json: serde_json::Value = serde_json::from_str(&text).expect("valid json frame");
            let frame_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if frame_type == "res" && json.get("id").and_then(|v| v.as_str()) == Some("req-2") {
                assert_eq!(json.get("ok").and_then(|v| v.as_bool()), Some(true));
                got_accept_res = true;
            }

            if frame_type == "event" {
                match json.get("event").and_then(|v| v.as_str()) {
                    Some("agent.delta") => {
                        got_delta = true;
                        let delta = json
                            .get("payload")
                            .and_then(|v| v.get("delta"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        assert_eq!(delta, "compat-ok");
                    }
                    Some("agent.done") => {
                        got_done = true;
                        assert_eq!(
                            json.get("payload")
                                .and_then(|v| v.get("status"))
                                .and_then(|v| v.as_str()),
                            Some("success_done")
                        );
                        assert!(
                            json.get("payload")
                                .and_then(|v| v.get("request_id"))
                                .and_then(|v| v.as_str())
                                .is_some(),
                            "agent.done payload missing request_id"
                        );
                    }
                    _ => {}
                }
            }

            if got_accept_res && got_delta && got_done {
                break;
            }
        }

        assert!(got_accept_res, "missing req-2 acceptance response");
        assert!(got_delta, "missing agent.delta event");
        assert!(got_done, "missing agent.done event");

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_rejects_concurrent_inflight_request_per_session() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_MAX_INFLIGHT_PER_SESSION", "1");
        }

        let (mut ws, server) = open_test_socket().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-concurrent").await;

        let req_a = serde_json::json!({
            "type": "req",
            "id": "req-a",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "slow-test first"
            }
        });
        ws.send(Message::Text(req_a.to_string()))
            .await
            .expect("send req-a");

        let req_b = serde_json::json!({
            "type": "req",
            "id": "req-b",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "second should reject"
            }
        });
        ws.send(Message::Text(req_b.to_string()))
            .await
            .expect("send req-b");

        let mut req_b_rejected = false;
        let mut req_b_done_error = false;
        let mut req_a_done = false;

        for _ in 0..16 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("compat frame timeout")
                .expect("compat frame missing")
                .expect("compat ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("valid json frame");

            if json.get("type").and_then(|v| v.as_str()) == Some("res")
                && json.get("id").and_then(|v| v.as_str()) == Some("req-b")
            {
                req_b_rejected = json.get("ok").and_then(|v| v.as_bool()) == Some(false)
                    && json
                        .get("error")
                        .and_then(|v| v.get("code"))
                        .and_then(|v| v.as_str())
                        == Some("concurrent_request_rejected");
            }

            if json.get("type").and_then(|v| v.as_str()) == Some("event")
                && json.get("event").and_then(|v| v.as_str()) == Some("agent.done")
            {
                let req_id = json
                    .get("payload")
                    .and_then(|v| v.get("request_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let status = json
                    .get("payload")
                    .and_then(|v| v.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if req_id == "req-b" && status == "error_done" {
                    let reason = json
                        .get("payload")
                        .and_then(|v| v.get("reason"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if reason == "concurrent_request_rejected" {
                        req_b_done_error = true;
                    }
                }
                if req_id == "req-a" && status == "success_done" {
                    req_a_done = true;
                }
            }

            if req_b_rejected && req_b_done_error && req_a_done {
                break;
            }
        }

        unsafe {
            env::remove_var("NANOBOT_MAX_INFLIGHT_PER_SESSION");
        }

        assert!(req_b_rejected, "req-b should be rejected by in-flight guard");
        assert!(
            req_b_done_error,
            "req-b should receive error_done terminal with concurrent_request_rejected"
        );
        assert!(req_a_done, "req-a should complete successfully");

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_rejects_duplicate_inflight_request_id_without_terminal() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_MAX_INFLIGHT_PER_SESSION", "4");
        }

        let (mut ws, server) = open_test_socket().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-dup").await;

        let req_a = serde_json::json!({
            "type": "req",
            "id": "dup-req",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "slow-test first"
            }
        });
        ws.send(Message::Text(req_a.to_string()))
            .await
            .expect("send first dup request");

        let req_b = serde_json::json!({
            "type": "req",
            "id": "dup-req",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "slow-test second"
            }
        });
        ws.send(Message::Text(req_b.to_string()))
            .await
            .expect("send second dup request");

        let mut got_duplicate_res = false;
        let mut duplicate_done_count = 0usize;
        let mut success_done_count = 0usize;

        for _ in 0..16 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("compat frame timeout")
                .expect("compat frame missing")
                .expect("compat ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("valid json frame");

            if json.get("type").and_then(|v| v.as_str()) == Some("res")
                && json.get("id") == Some(&serde_json::Value::String("dup-req".to_string()))
                && json.get("ok").and_then(|v| v.as_bool()) == Some(false)
                && json
                    .get("error")
                    .and_then(|v| v.get("code"))
                    .and_then(|v| v.as_str())
                    == Some("duplicate_inflight_request")
            {
                got_duplicate_res = true;
            }

            if json.get("type").and_then(|v| v.as_str()) == Some("event")
                && json.get("event").and_then(|v| v.as_str()) == Some("agent.done")
            {
                let req_id = json
                    .get("payload")
                    .and_then(|v| v.get("request_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let status = json
                    .get("payload")
                    .and_then(|v| v.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if req_id == "dup-req" {
                    if status == "error_done" {
                        duplicate_done_count += 1;
                    } else if status == "success_done" {
                        success_done_count += 1;
                    }
                }
            }

            if got_duplicate_res && success_done_count == 1 {
                break;
            }
        }

        unsafe {
            env::remove_var("NANOBOT_MAX_INFLIGHT_PER_SESSION");
        }

        assert!(got_duplicate_res, "duplicate request should be rejected");
        assert_eq!(
            duplicate_done_count, 0,
            "duplicate in-flight request must not emit error_done terminal"
        );
        assert_eq!(
            success_done_count, 1,
            "only admitted request should produce success_done terminal"
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_releases_inflight_slot_on_terminal() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_MAX_INFLIGHT_PER_SESSION", "1");
        }

        let (mut ws, server) = open_test_socket().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-slot").await;

        let req_a = serde_json::json!({
            "type": "req",
            "id": "req-slot-a",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "hello A"
            }
        });
        ws.send(Message::Text(req_a.to_string()))
            .await
            .expect("send req-a");

        let mut a_done = false;
        for _ in 0..10 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("frame timeout")
                .expect("frame missing")
                .expect("ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("json frame");
            if json.get("type").and_then(|v| v.as_str()) == Some("event")
                && json.get("event").and_then(|v| v.as_str()) == Some("agent.done")
                && json
                    .get("payload")
                    .and_then(|v| v.get("request_id"))
                    .and_then(|v| v.as_str())
                    == Some("req-slot-a")
            {
                a_done = true;
                break;
            }
        }
        assert!(a_done, "first request should complete");

        let req_b = serde_json::json!({
            "type": "req",
            "id": "req-slot-b",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "hello B"
            }
        });
        ws.send(Message::Text(req_b.to_string()))
            .await
            .expect("send req-b");

        let mut b_accepted = false;
        for _ in 0..10 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("frame timeout")
                .expect("frame missing")
                .expect("ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("json frame");
            if json.get("type").and_then(|v| v.as_str()) == Some("res")
                && json.get("id").and_then(|v| v.as_str()) == Some("req-slot-b")
            {
                b_accepted = json.get("ok").and_then(|v| v.as_bool()) == Some(true);
                break;
            }
        }
        assert!(b_accepted, "second request should be accepted after terminal release");

        unsafe {
            env::remove_var("NANOBOT_MAX_INFLIGHT_PER_SESSION");
        }

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_enqueue_failure_emits_terminal_error_done_compat() {
        let _ws_guard = WS_TEST_LOCK.lock().await;

        let (closed_tx, closed_rx) = mpsc::channel::<AgentMessage>(1);
        drop(closed_rx);

        let (mut ws, server, _gateway) = open_test_socket_with_agent_tx(closed_tx).await;
        let token = compat_connect_and_get_token(&mut ws, "connect-enqueue-fail").await;

        let req = serde_json::json!({
            "type": "req",
            "id": "req-enqueue-fail",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "should fail enqueue"
            }
        });
        ws.send(Message::Text(req.to_string()))
            .await
            .expect("send compat request");

        let mut got_res_error = false;
        let mut got_done_error = false;

        for _ in 0..8 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("frame timeout")
                .expect("missing frame")
                .expect("ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("json frame");

            if json.get("type").and_then(|v| v.as_str()) == Some("res")
                && json.get("id").and_then(|v| v.as_str()) == Some("req-enqueue-fail")
            {
                got_res_error = json.get("ok").and_then(|v| v.as_bool()) == Some(false)
                    && json
                        .get("error")
                        .and_then(|v| v.get("code"))
                        .and_then(|v| v.as_str())
                        == Some("agent_unavailable");
            }

            if json.get("type").and_then(|v| v.as_str()) == Some("event")
                && json.get("event").and_then(|v| v.as_str()) == Some("agent.done")
            {
                let req_id = json
                    .get("payload")
                    .and_then(|v| v.get("request_id"))
                    .and_then(|v| v.as_str());
                let status = json
                    .get("payload")
                    .and_then(|v| v.get("status"))
                    .and_then(|v| v.as_str());
                let reason = json
                    .get("payload")
                    .and_then(|v| v.get("reason"))
                    .and_then(|v| v.as_str());
                got_done_error = req_id == Some("req-enqueue-fail")
                    && status == Some("error_done")
                    && reason == Some("agent_unavailable");
            }

            if got_res_error && got_done_error {
                break;
            }
        }

        assert!(got_res_error, "missing compat res error for enqueue failure");
        assert!(
            got_done_error,
            "missing compat terminal error_done for enqueue failure"
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_enqueue_failure_emits_terminal_error_done_non_compat() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_TEST_BYPASS_ONBOARDING", "1");
        }

        let (closed_tx, closed_rx) = mpsc::channel::<AgentMessage>(1);
        drop(closed_rx);

        let gateway = Arc::new(Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            closed_tx,
            Arc::new(tokio::sync::Mutex::new(crate::tools::ConfirmationService::new())),
        ));
        let (addr, server) = spawn_ws_server(gateway).await;
        let (mut ws, token) = connect_test_socket_with_token(addr).await;

        let req = serde_json::json!({
            "message": "should fail enqueue",
            "token": token
        });
        ws.send(Message::Text(req.to_string()))
            .await
            .expect("send non-compat request");

        let mut got_done_error = false;
        for _ in 0..8 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("frame timeout")
                .expect("missing frame")
                .expect("ws frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("json frame");
            if json.get("type").and_then(|v| v.as_str()) == Some("done") {
                got_done_error = json.get("status").and_then(|v| v.as_str())
                    == Some("error_done")
                    && json.get("reason").and_then(|v| v.as_str())
                        == Some("agent_unavailable")
                    && json
                        .get("request_id")
                        .and_then(|v| v.as_str())
                        .is_some();
                if got_done_error {
                    break;
                }
            }
        }

        unsafe {
            env::remove_var("NANOBOT_TEST_BYPASS_ONBOARDING");
        }

        assert!(
            got_done_error,
            "missing non-compat terminal error_done for enqueue failure"
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_negative_and_confirmation_aliases() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let (mut ws, server) = open_test_socket().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-1").await;

        let missing_token_req = serde_json::json!({
            "type": "req",
            "id": "req-missing",
            "method": "health",
            "params": {}
        });
        ws.send(Message::Text(missing_token_req.to_string()))
            .await
            .expect("send missing token req");
        let missing_token_res = recv_json(&mut ws).await;
        assert_eq!(
            missing_token_res.get("type").and_then(|v| v.as_str()),
            Some("res")
        );
        assert_eq!(
            missing_token_res.get("id").and_then(|v| v.as_str()),
            Some("req-missing")
        );
        assert_eq!(
            missing_token_res.get("ok").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            missing_token_res
                .get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("missing_token")
        );

        let invalid_token_req = serde_json::json!({
            "type": "req",
            "id": "req-invalid",
            "method": "health",
            "params": { "token": "definitely-invalid" }
        });
        ws.send(Message::Text(invalid_token_req.to_string()))
            .await
            .expect("send invalid token req");
        let invalid_token_res = recv_json(&mut ws).await;
        assert_eq!(
            invalid_token_res.get("type").and_then(|v| v.as_str()),
            Some("res")
        );
        assert_eq!(
            invalid_token_res.get("id").and_then(|v| v.as_str()),
            Some("req-invalid")
        );
        assert_eq!(
            invalid_token_res.get("ok").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            invalid_token_res
                .get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("invalid_token")
        );

        let unknown_req = serde_json::json!({
            "type": "req",
            "id": "req-unknown",
            "method": "not-a-real-method",
            "params": { "token": token.clone() }
        });
        ws.send(Message::Text(unknown_req.to_string()))
            .await
            .expect("send unknown method req");
        let unknown_res = recv_json(&mut ws).await;
        assert_eq!(
            unknown_res.get("type").and_then(|v| v.as_str()),
            Some("res")
        );
        assert_eq!(
            unknown_res.get("id").and_then(|v| v.as_str()),
            Some("req-unknown")
        );
        assert_eq!(unknown_res.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            unknown_res
                .get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("unknown_method")
        );

        let confirmation_req = serde_json::json!({
            "type": "req",
            "id": "req-confirm",
            "method": "confirmation/respond",
            "params": {
                "token": token,
                "id": "confirm-1",
                "allowed": true
            }
        });
        ws.send(Message::Text(confirmation_req.to_string()))
            .await
            .expect("send confirmation alias req");
        let confirmation_res = recv_json(&mut ws).await;
        assert_eq!(
            confirmation_res.get("type").and_then(|v| v.as_str()),
            Some("res")
        );
        assert_eq!(
            confirmation_res.get("id").and_then(|v| v.as_str()),
            Some("req-confirm")
        );
        assert_eq!(
            confirmation_res.get("ok").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            confirmation_res
                .get("payload")
                .and_then(|v| v.get("accepted"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_rejects_non_string_request_id() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let (mut ws, server) = open_test_socket().await;

        let req = serde_json::json!({
            "type": "req",
            "id": 123,
            "method": "connect",
            "params": {}
        });
        ws.send(Message::Text(req.to_string()))
            .await
            .expect("send req");

        let res = recv_json(&mut ws).await;
        assert_eq!(res.get("type").and_then(|v| v.as_str()), Some("res"));
        assert_eq!(res.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            res.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("invalid_request_id")
        );

        let next = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
        assert!(next.is_err(), "invalid request id should not emit terminal events");

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_rejects_oversized_request_id() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let (mut ws, server) = open_test_socket().await;

        let req = serde_json::json!({
            "type": "req",
            "id": "a".repeat(MAX_WS_REQUEST_ID_LEN + 1),
            "method": "connect",
            "params": {}
        });
        ws.send(Message::Text(req.to_string()))
            .await
            .expect("send req");

        let res = recv_json(&mut ws).await;
        assert_eq!(res.get("type").and_then(|v| v.as_str()), Some("res"));
        assert_eq!(res.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            res.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("invalid_request_id")
        );

        let next = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
        assert!(next.is_err(), "invalid request id should not emit terminal events");

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_compat_rejects_invalid_charset_request_id() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let (mut ws, server) = open_test_socket().await;

        let req = serde_json::json!({
            "type": "req",
            "id": "bad/id",
            "method": "connect",
            "params": {}
        });
        ws.send(Message::Text(req.to_string()))
            .await
            .expect("send req");

        let res = recv_json(&mut ws).await;
        assert_eq!(res.get("type").and_then(|v| v.as_str()), Some("res"));
        assert_eq!(res.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            res.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("invalid_request_id")
        );

        let next = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
        assert!(next.is_err(), "invalid request id should not emit terminal events");

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_non_compat_rejects_invalid_request_id() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_TEST_BYPASS_ONBOARDING", "1");
        }

        let agent_tx = spawn_test_agent();
        let gateway = Arc::new(Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            agent_tx,
            Arc::new(tokio::sync::Mutex::new(crate::tools::ConfirmationService::new())),
        ));
        let (addr, server) = spawn_ws_server(gateway).await;
        let (mut ws, token) = connect_test_socket_with_token(addr).await;
        let req = serde_json::json!({
            "message": "hello",
            "token": token,
            "request_id": 7,
        });
        ws.send(Message::Text(req.to_string()))
            .await
            .expect("send req");

        let res = recv_json(&mut ws).await;
        assert_eq!(res.get("type").and_then(|v| v.as_str()), Some("error"));
        assert_eq!(res.get("error").and_then(|v| v.as_str()), Some("invalid_request_id"));

        let next = tokio::time::timeout(Duration::from_millis(200), ws.next()).await;
        assert!(next.is_err(), "invalid request id should not emit done events");

        unsafe {
            env::remove_var("NANOBOT_TEST_BYPASS_ONBOARDING");
        }

        server.abort();
    }

    fn sign_webhook(secret: &str, timestamp: i64, nonce: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("valid key");
        let payload = format!("{}.{}.", timestamp, nonce);
        mac.update(payload.as_bytes());
        mac.update(body);
        to_hex(&mac.finalize().into_bytes())
    }

    #[tokio::test]
    async fn test_webhook_signature_validation_and_replay() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");

        let token_key = "NANOBOT_TEST_WEBHOOK_TOKEN";
        let sign_key = "NANOBOT_TEST_WEBHOOK_SIGN";
        unsafe {
            env::remove_var(token_key);
            env::set_var(sign_key, "test-signing-secret-32-characters-min!1234");
        }

        let body = br#"{"text":"hello"}"#;
        let ts = chrono::Utc::now().timestamp();
        let nonce = "nonce-1";
        let sig = sign_webhook(
            "test-signing-secret-32-characters-min!1234",
            ts,
            nonce,
            body,
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-nanobot-timestamp", ts.to_string().parse().unwrap());
        headers.insert("x-nanobot-nonce", nonce.parse().unwrap());
        headers.insert("x-nanobot-signature", sig.parse().unwrap());

        assert!(
            verify_webhook_request("teams", &headers, body, token_key, sign_key)
                .await
                .is_ok()
        );

        let replay = verify_webhook_request("teams", &headers, body, token_key, sign_key).await;
        assert!(matches!(replay, Err("replayed webhook nonce")));
    }

    #[tokio::test]
    async fn test_webhook_rejects_body_tamper_and_bad_timestamp() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");

        let token_key = "NANOBOT_TEST_WEBHOOK_TOKEN2";
        let sign_key = "NANOBOT_TEST_WEBHOOK_SIGN2";
        unsafe {
            env::remove_var(token_key);
            env::set_var(sign_key, "another-signing-secret-32-characters-min!5678");
        }

        let original = br#"{"text":"hello"}"#;
        let tampered = br#"{"text":"hello!"}"#;
        let ts = chrono::Utc::now().timestamp();
        let nonce = "nonce-2";
        let sig = sign_webhook(
            "another-signing-secret-32-characters-min!5678",
            ts,
            nonce,
            original,
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-nanobot-timestamp", ts.to_string().parse().unwrap());
        headers.insert("x-nanobot-nonce", nonce.parse().unwrap());
        headers.insert("x-nanobot-signature", sig.parse().unwrap());

        let tamper_result =
            verify_webhook_request("google_chat", &headers, tampered, token_key, sign_key).await;
        assert!(matches!(tamper_result, Err("invalid webhook signature")));

        let old_ts = ts - 601;
        let old_sig = sign_webhook(
            "another-signing-secret-32-characters-min!5678",
            old_ts,
            "nonce-3",
            original,
        );
        let mut old_headers = HeaderMap::new();
        old_headers.insert("x-nanobot-timestamp", old_ts.to_string().parse().unwrap());
        old_headers.insert("x-nanobot-nonce", "nonce-3".parse().unwrap());
        old_headers.insert("x-nanobot-signature", old_sig.parse().unwrap());
        let old_result =
            verify_webhook_request("google_chat", &old_headers, original, token_key, sign_key)
                .await;
        assert!(matches!(old_result, Err("webhook timestamp too old")));
    }

    #[tokio::test]
    async fn test_webhook_nonce_store_failure_fails_closed() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");

        let token_key = "NANOBOT_TEST_WEBHOOK_TOKEN3";
        let sign_key = "NANOBOT_TEST_WEBHOOK_SIGN3";
        let tmpdir = tempfile::tempdir().expect("temp dir");

        unsafe {
            env::remove_var(token_key);
            env::set_var(sign_key, "failure-test-signing-secret-32-characters!!");
            env::set_var("NANOBOT_WEBHOOK_NONCE_DB_PATH", tmpdir.path());
        }

        let body = br#"{"text":"hello"}"#;
        let ts = chrono::Utc::now().timestamp();
        let nonce = "nonce-failure";
        let sig = sign_webhook(
            "failure-test-signing-secret-32-characters!!",
            ts,
            nonce,
            body,
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-nanobot-timestamp", ts.to_string().parse().unwrap());
        headers.insert("x-nanobot-nonce", nonce.parse().unwrap());
        headers.insert("x-nanobot-signature", sig.parse().unwrap());

        let result = verify_webhook_request("teams", &headers, body, token_key, sign_key).await;
        assert!(
            result.is_err(),
            "webhook should fail closed when nonce store fails"
        );

        unsafe {
            env::remove_var("NANOBOT_WEBHOOK_NONCE_DB_PATH");
        }
    }

    #[tokio::test]
    async fn test_auth_surface_settings_requires_auth() {
        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );

        let app = gateway.build_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/channels/config")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_settings_auth_token_semantics() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_ADMIN_TOKEN", "test-admin-token-123");
            env::remove_var("NANOBOT_MASTER_PASSWORD");
            env::remove_var("NANOBOT_WEB_PASSWORD");
        }
        reset_settings_auth_cache();

        let missing = HeaderMap::new();
        assert!(!is_settings_authorized(&missing));

        let mut wrong = HeaderMap::new();
        wrong.insert(header::AUTHORIZATION, "Bearer wrong-token".parse().unwrap());
        assert!(!is_settings_authorized(&wrong));

        let mut correct = HeaderMap::new();
        correct.insert(
            header::AUTHORIZATION,
            "Bearer test-admin-token-123".parse().unwrap(),
        );
        assert!(is_settings_authorized(&correct));

        unsafe {
            env::remove_var("NANOBOT_ADMIN_TOKEN");
        }
        reset_settings_auth_cache();
    }

    #[tokio::test]
    async fn test_ws_disconnect_cleans_pending_questions() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let (mut ws, server, gateway) = open_test_socket_with_gateway().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-q").await;

        let send_req = serde_json::json!({
            "type": "req",
            "id": "req-q",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "question-test"
            }
        });
        ws.send(Message::Text(send_req.to_string()))
            .await
            .expect("send question req");

        let mut got_question = false;
        for _ in 0..8 {
            let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
                .await
                .expect("question frame timeout")
                .expect("question frame missing")
                .expect("question frame ok");
            let text = match frame {
                Message::Text(t) => t,
                _ => continue,
            };
            let json: serde_json::Value = serde_json::from_str(&text).expect("valid json");
            if json.get("type").and_then(|v| v.as_str()) == Some("event")
                && json.get("event").and_then(|v| v.as_str()) == Some("agent.question")
            {
                got_question = true;
                break;
            }
        }
        assert!(got_question, "expected question event before disconnect");

        ws.close(None).await.expect("close websocket");

        let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if gateway.pending_questions.is_empty().await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(cleanup.is_ok(), "pending questions were not cleaned after disconnect");

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_send_loop_panic_terminates_session_and_cleans_pending() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let panic_metric_before = prometheus_counter_value("gateway_ws_send_task_panics_total");
        let (mut ws, server, gateway) = open_test_socket_with_gateway().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-panic").await;
        gateway
            .panic_send_on_toolresult
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let send_req = serde_json::json!({
            "type": "req",
            "id": "req-panic",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "question-test"
            }
        });
        ws.send(Message::Text(send_req.to_string()))
            .await
            .expect("send panic trigger req");

        for _ in 0..4 {
            let _ = tokio::time::timeout(Duration::from_millis(150), ws.next()).await;
        }

        let probe_req = serde_json::json!({
            "type": "req",
            "id": "probe-after-panic",
            "method": "health",
            "params": { "token": token }
        });

        let recv_stopped = match ws.send(Message::Text(probe_req.to_string())).await {
            Err(_) => true,
            Ok(_) => {
                let probe_result = tokio::time::timeout(Duration::from_millis(600), ws.next()).await;
                match probe_result {
                    Err(_) => true,
                    Ok(None) => true,
                    Ok(Some(Ok(Message::Close(_)))) => true,
                    Ok(Some(Err(_))) => true,
                    Ok(Some(Ok(Message::Text(text)))) => {
                        let parsed: serde_json::Value =
                            serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
                        let is_probe_res = parsed.get("type").and_then(|v| v.as_str()) == Some("res")
                            && parsed.get("id").and_then(|v| v.as_str()) == Some("probe-after-panic");
                        assert!(
                            !is_probe_res,
                            "recv loop should not process requests after send-loop panic"
                        );
                        true
                    }
                    Ok(Some(Ok(_))) => true,
                }
            }
        };
        assert!(recv_stopped, "recv side remained active after send-loop panic");

        let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if gateway.pending_questions.is_empty().await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            cleanup.is_ok(),
            "pending questions were not cleaned after send-loop panic"
        );

        let panic_metric_after = prometheus_counter_value("gateway_ws_send_task_panics_total");
        assert!(
            panic_metric_after >= panic_metric_before + 1.0,
            "expected panic metric increment, before={panic_metric_before}, after={panic_metric_after}"
        );

        gateway
            .panic_send_on_toolresult
            .store(false, std::sync::atomic::Ordering::Relaxed);

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_send_timeout_terminates_session_and_counts_slow_client_disconnect() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let metric_before = prometheus_counter_value("slow_client_disconnects_total");

        let (mut ws, server, _gateway) = open_test_socket_with_gateway().await;
        let token = compat_connect_and_get_token(&mut ws, "connect-timeout").await;
        unsafe {
            env::set_var("NANOBOT_TEST_FORCE_WS_SEND_TIMEOUT", "1");
        }

        let send_req = serde_json::json!({
            "type": "req",
            "id": "req-timeout",
            "method": "agent/send/message",
            "params": {
                "token": token,
                "message": "hello-timeout"
            }
        });
        ws.send(Message::Text(send_req.to_string()))
            .await
            .expect("send timeout trigger req");

        for _ in 0..4 {
            let _ = tokio::time::timeout(Duration::from_millis(150), ws.next()).await;
        }

        let probe_req = serde_json::json!({
            "type": "req",
            "id": "probe-after-timeout",
            "method": "health",
            "params": { "token": token }
        });

        let recv_stopped = match ws.send(Message::Text(probe_req.to_string())).await {
            Err(_) => true,
            Ok(_) => {
                let probe_result = tokio::time::timeout(Duration::from_millis(600), ws.next()).await;
                match probe_result {
                    Err(_) => true,
                    Ok(None) => true,
                    Ok(Some(Ok(Message::Close(_)))) => true,
                    Ok(Some(Err(_))) => true,
                    Ok(Some(Ok(Message::Text(text)))) => {
                        let parsed: serde_json::Value =
                            serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
                        let is_probe_res = parsed.get("type").and_then(|v| v.as_str()) == Some("res")
                            && parsed.get("id").and_then(|v| v.as_str()) == Some("probe-after-timeout");
                        assert!(
                            !is_probe_res,
                            "recv loop should not process requests after send timeout"
                        );
                        true
                    }
                    Ok(Some(Ok(_))) => true,
                }
            }
        };

        unsafe {
            env::remove_var("NANOBOT_TEST_FORCE_WS_SEND_TIMEOUT");
        }

        assert!(recv_stopped, "recv side remained active after send timeout");

        let metric_after = prometheus_counter_value("slow_client_disconnects_total");
        assert!(
            metric_after >= metric_before + 1.0,
            "expected slow_client_disconnects_total to increment (before={}, after={})",
            metric_before,
            metric_after
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_ws_pending_questions_soak_100_cycles() {
        let _ws_guard = WS_TEST_LOCK.lock().await;
        let agent_tx = spawn_test_agent();
        let gateway = Arc::new(Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            agent_tx,
            Arc::new(tokio::sync::Mutex::new(
                crate::tools::ConfirmationService::new(),
            )),
        ));
        let (addr, server) = spawn_ws_server(gateway.clone()).await;

        for i in 0..100 {
            let mut ws = connect_test_socket(addr).await;
            let token = compat_connect_and_get_token(&mut ws, &format!("connect-soak-{i}"))
                .await;
            let send_req = serde_json::json!({
                "type": "req",
                "id": format!("req-soak-{i}"),
                "method": "agent/send/message",
                "params": {
                    "token": token,
                    "message": "question-test"
                }
            });
            ws.send(Message::Text(send_req.to_string()))
                .await
                .expect("send soak req");

            let mut got_question = false;
            for _ in 0..8 {
                let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
                    .await
                    .expect("soak frame timeout")
                    .expect("soak frame missing")
                    .expect("soak frame ok");
                let text = match frame {
                    Message::Text(t) => t,
                    _ => continue,
                };
                let json: serde_json::Value =
                    serde_json::from_str(&text).expect("valid soak json");
                if json.get("type").and_then(|v| v.as_str()) == Some("event")
                    && json.get("event").and_then(|v| v.as_str()) == Some("agent.question")
                {
                    got_question = true;
                    break;
                }
            }
            assert!(got_question, "expected question event in cycle {i}");

            ws.close(None).await.expect("close soak websocket");

            let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if gateway.pending_questions.is_empty().await {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(cleanup.is_ok(), "pending cleanup failed in cycle {i}");
            assert!(
                gateway.pending_questions.is_empty().await,
                "pending map leaked entries in cycle {i}"
            );
        }

        server.abort();
    }

    #[tokio::test]
    async fn test_settings_auth_routes_in_production_modes() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let tmp = tempfile::NamedTempFile::new().expect("temp config");
        std::fs::write(tmp.path(), "default_provider = \"openai\"\n[providers]\n")
            .expect("write temp config");

        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_ADMIN_TOKEN", "prod-admin-token-123");
            env::set_var("NANOBOT_CONFIG_PATH", tmp.path());
            env::remove_var("NANOBOT_MASTER_PASSWORD");
            env::remove_var("NANOBOT_WEB_PASSWORD");
        }
        reset_settings_auth_cache();

        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );

        let app = gateway.build_router();
        let missing = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let wrong = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("GET")
                    .header(header::AUTHORIZATION, "Bearer wrong")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        let correct = app
            .oneshot(
                Request::builder()
                    .uri("/api/settings")
                    .method("GET")
                    .header(
                        header::AUTHORIZATION,
                        "Bearer prod-admin-token-123",
                    )
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(correct.status(), StatusCode::OK);

        unsafe {
            env::remove_var("NANOBOT_ENV");
            env::remove_var("NANOBOT_ADMIN_TOKEN");
            env::remove_var("NANOBOT_CONFIG_PATH");
        }
        reset_settings_auth_cache();
    }

    #[tokio::test]
    async fn test_auth_surface_health_is_public() {
        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );

        let app = gateway.build_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sticky_missing_violation_degrades_health_in_strict_multi_replica() {
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_SCALING_STRICT", "1");
            env::set_var("NANOBOT_REPLICA_COUNT", "2");
            env::set_var("NANOBOT_STICKY_VIOLATION_GRACE_MS", "5000");
        }
        reset_sticky_violation_state();
        crate::metrics::GLOBAL_METRICS.reset();

        let before = prometheus_counter_value("distributed_sticky_signal_missing_total");
        record_sticky_signal_missing();

        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );
        let app = gateway.build_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            prometheus_counter_value("distributed_sticky_signal_missing_total") >= before + 1.0,
            "sticky missing counter should increment"
        );

        unsafe {
            env::remove_var("NANOBOT_SCALING_STRICT");
            env::remove_var("NANOBOT_REPLICA_COUNT");
            env::remove_var("NANOBOT_STICKY_VIOLATION_GRACE_MS");
        }
        reset_sticky_violation_state();
    }

    #[tokio::test]
    async fn sticky_health_recovers_after_grace_window() {
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_SCALING_STRICT", "1");
            env::set_var("NANOBOT_REPLICA_COUNT", "2");
            env::set_var("NANOBOT_STICKY_VIOLATION_GRACE_MS", "500");
        }
        reset_sticky_violation_state();
        record_sticky_signal_missing();

        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );
        let app = gateway.build_router();

        let degraded = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(degraded.status(), StatusCode::SERVICE_UNAVAILABLE);

        tokio::time::sleep(Duration::from_millis(700)).await;

        let recovered = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(recovered.status(), StatusCode::OK);

        unsafe {
            env::remove_var("NANOBOT_SCALING_STRICT");
            env::remove_var("NANOBOT_REPLICA_COUNT");
            env::remove_var("NANOBOT_STICKY_VIOLATION_GRACE_MS");
        }
        reset_sticky_violation_state();
    }

    #[tokio::test]
    async fn sticky_conflict_violation_degrades_health() {
        let _env_guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_SCALING_STRICT", "1");
            env::set_var("NANOBOT_REPLICA_COUNT", "2");
            env::set_var("NANOBOT_STICKY_VIOLATION_GRACE_MS", "5000");
        }
        reset_sticky_violation_state();
        crate::metrics::GLOBAL_METRICS.reset();

        let before = prometheus_counter_value("distributed_sticky_signal_conflict_total");
        record_sticky_signal_conflict();

        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );
        let app = gateway.build_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            prometheus_counter_value("distributed_sticky_signal_conflict_total") >= before + 1.0,
            "sticky conflict counter should increment"
        );

        unsafe {
            env::remove_var("NANOBOT_SCALING_STRICT");
            env::remove_var("NANOBOT_REPLICA_COUNT");
            env::remove_var("NANOBOT_STICKY_VIOLATION_GRACE_MS");
        }
        reset_sticky_violation_state();
    }

    #[tokio::test]
    async fn test_auth_surface_protected_routes_not_public() {
        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );

        let protected = [
            ("GET", "/api/channels/config"),
            ("GET", "/api/settings"),
            ("GET", "/api/settings/doctor"),
            ("POST", "/api/channels/discord/verify"),
        ];

        for (method, route) in protected {
            let app = gateway.build_router();
            let request = Request::builder()
                .uri(route)
                .method(method)
                .body(Body::empty())
                .expect("request");
            let response = app.oneshot(request).await.expect("router response");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "route {} {} should require authorization",
                method,
                route
            );
        }
    }

    #[tokio::test]
    async fn test_metrics_requires_auth_in_production() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::remove_var("NANOBOT_METRICS_PUBLIC");
        }

        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        let gateway = Gateway::new(
            GatewayConfig {
                port: 0,
                bind_host: "127.0.0.1".to_string(),
            },
            tx,
            confirmation_service,
        );

        let app = gateway.build_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("router response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_all_gateway_routes_have_policy_tags() {
        let routes = gateway_route_policies();
        assert!(!routes.is_empty(), "route policy registry should not be empty");

        let mut seen = std::collections::HashSet::new();
        for route in &routes {
            let key = format!("{} {}", route.method, route.path);
            assert!(seen.insert(key), "duplicate route policy entry detected");
        }

        assert!(routes.iter().any(|r| r.path == "/health" && r.policy == RoutePolicy::Public));
        assert!(routes.iter().any(|r| r.path == "/metrics" && r.policy == RoutePolicy::Internal));
        assert!(
            routes
                .iter()
                .any(|r| r.path == "/api/settings" && r.policy == RoutePolicy::Protected)
        );
    }
}

async fn health_check() -> impl IntoResponse {
    if sticky_health_degraded_now() {
        return (StatusCode::SERVICE_UNAVAILABLE, "DEGRADED_STICKY");
    }
    (StatusCode::OK, "OK")
}

fn metrics_is_public() -> bool {
    std::env::var("NANOBOT_METRICS_PUBLIC")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

async fn metrics_handler(headers: HeaderMap) -> impl IntoResponse {
    let env = std::env::var("NANOBOT_ENV")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "development".to_string());
    let production = env == "production" || env == "prod";

    if production && !metrics_is_public() && !is_settings_authorized(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "application/json")],
            json!({"error": "metrics auth required"}).to_string(),
        );
    }

    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        crate::metrics::GLOBAL_METRICS.export_prometheus(),
    )
}

async fn api_bootstrap(headers: HeaderMap) -> impl IntoResponse {
    let cfg = load_config_async().await.ok();

    let channels = json!({
        "web": true,
        "telegram": std::env::var("TELEGRAM_BOT_TOKEN").or_else(|_| std::env::var("NANOBOT_TELEGRAM_TOKEN")).ok().filter(|v| !v.trim().is_empty()).is_some() || cfg.as_ref().and_then(|c| c.providers.telegram.as_ref()).is_some(),
        "slack": std::env::var("SLACK_BOT_TOKEN").ok().filter(|v| !v.trim().is_empty()).is_some() || cfg.as_ref().and_then(|c| c.providers.slack.as_ref()).is_some(),
        "discord": std::env::var("DISCORD_TOKEN").ok().filter(|v| !v.trim().is_empty()).is_some() || cfg.as_ref().and_then(|c| c.providers.discord.as_ref()).is_some(),
        "teams": std::env::var("TEAMS_WEBHOOK_URL").ok().filter(|v| !v.trim().is_empty()).is_some() || cfg.as_ref().and_then(|c| c.providers.teams.as_ref()).is_some(),
        "google_chat": std::env::var("GOOGLE_CHAT_WEBHOOK_URL").ok().filter(|v| !v.trim().is_empty()).is_some() || cfg.as_ref().and_then(|c| c.providers.google_chat.as_ref()).is_some(),
    });

    let settings = if let Some(c) = cfg.as_ref() {
        let bind_host = std::env::var("NANOBOT_GATEWAY_BIND")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        json!({
            "interaction_policy": format!("{:?}", c.interaction_policy),
            "gateway": { "port": 18789, "bind_host": bind_host },
            "providers": {
                "openai_connected": c.providers.openai.as_ref().and_then(|p| p.api_key.clone()).map(|k| !k.trim().is_empty()).unwrap_or(false),
                "openrouter_connected": c.providers.openrouter.as_ref().and_then(|p| p.api_key.clone()).map(|k| !k.trim().is_empty()).unwrap_or(false),
                "antigravity_connected": c.providers.antigravity.as_ref().and_then(|p| p.api_key.clone()).map(|k| !k.trim().is_empty()).unwrap_or(false),
                "google_oauth_configured": c.providers.google.as_ref().and_then(|g| g.oauth_client_id.clone()).is_some() && c.providers.google.as_ref().and_then(|g| g.oauth_client_secret.clone()).is_some(),
            }
        })
    } else {
        json!({"error": "config_unavailable"})
    };

    let skills = {
        let workspace = crate::workspace::resolve_workspace_dir();
        {
            let mut loader = crate::skills::SkillLoader::new(workspace);
            if loader.scan().is_ok() {
                let cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
                let mut items: Vec<serde_json::Value> = loader
                    .skills()
                    .values()
                    .map(|s| {
                        json!({
                            "name": s.name,
                            "description": s.description,
                            "enabled": cfg.is_enabled(&s.name),
                            "backend": s.backend,
                            "runtime_override": cfg.runtime_override(&s.name),
                            "has_schema": s.config_schema.is_some(),
                        })
                    })
                    .collect();
                items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                json!({"items": items})
            } else {
                json!({"items": [], "error": "skill_scan_failed"})
            }
        }
    };

    let doctor = json!({
        "gh": command_exists_async("gh").await,
        "deno": command_exists_async("deno").await,
        "node": command_exists_async("node").await,
        "openclaw_auth_writable": openclaw_auth_writable_async().await,
    });

    let payload = BootstrapPayload {
        version: env!("CARGO_PKG_VERSION").to_string(),
        gateway_port: 18789,
        settings,
        channels,
        skills,
        doctor,
    };

    let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload_bytes.hash(&mut hasher);
    let etag = format!("W/\"{:x}\"", hasher.finish());

    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if if_none_match == etag {
        let mut resp = StatusCode::NOT_MODIFIED.into_response();
        if let Ok(val) = etag.parse() {
            resp.headers_mut().insert(header::ETAG, val);
        }
        return resp;
    }

    let mut resp = Json(payload).into_response();
    if let Ok(etag_val) = etag.parse() {
        resp.headers_mut().insert(header::ETAG, etag_val);
    }
    if let Ok(cache_val) = "no-cache".parse() {
        resp.headers_mut().insert(header::CACHE_CONTROL, cache_val);
    }
    resp
}

async fn channels_status() -> (StatusCode, Json<serde_json::Value>) {
    let cfg = load_config_async().await.ok();

    let telegram = std::env::var("TELEGRAM_BOT_TOKEN")
        .or_else(|_| std::env::var("NANOBOT_TELEGRAM_TOKEN"))
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
        || cfg
            .as_ref()
            .and_then(|c| c.providers.telegram.as_ref())
            .is_some();

    let slack = std::env::var("SLACK_BOT_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
        || cfg
            .as_ref()
            .and_then(|c| c.providers.slack.as_ref())
            .is_some();

    let discord = std::env::var("DISCORD_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
        || cfg
            .as_ref()
            .and_then(|c| c.providers.discord.as_ref())
            .is_some();

    let teams = std::env::var("TEAMS_WEBHOOK_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
        || cfg
            .as_ref()
            .and_then(|c| c.providers.teams.as_ref())
            .is_some();

    let google_chat = std::env::var("GOOGLE_CHAT_WEBHOOK_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
        || cfg
            .as_ref()
            .and_then(|c| c.providers.google_chat.as_ref())
            .is_some();

    (
        StatusCode::OK,
        Json(json!({
            "gateway_port": 18789,
            "channels": {
                "web": true,
                "telegram": telegram,
                "slack": slack,
                "discord": discord,
                "teams": teams,
                "google_chat": google_chat
            }
        })),
    )
}

#[derive(Debug, serde::Deserialize)]
struct PatchSettingsRequest {
    interaction_policy: Option<String>,
    teams_webhook_url: Option<String>,
    google_chat_webhook_url: Option<String>,
    openai_api_key: Option<String>,
    openrouter_api_key: Option<String>,
    antigravity_api_key: Option<String>,
    antigravity_base_url: Option<String>,
    google_api_key: Option<String>,
    google_oauth_client_id: Option<String>,
    google_oauth_client_secret: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SecurityProfileRequest {
    level: String,
}

#[derive(Debug, serde::Deserialize)]
struct SkillConfigUpdateRequest {
    enabled: Option<bool>,
    runtime: Option<String>,
    credentials: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, serde::Deserialize)]
struct SkillInstallRequest {
    skill: String,
    repo: Option<String>,
    auto_enable: Option<bool>,
    runtime: Option<String>,
    credentials: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, serde::Deserialize)]
struct GoogleAuthCompleteRequest {
    redirect_url: String,
}

#[derive(Debug, serde::Deserialize)]
struct PatchChannelsConfigRequest {
    telegram_token: Option<String>,
    telegram_allowed_users: Option<Vec<i64>>,
    slack_bot_token: Option<String>,
    slack_app_token: Option<String>,
    discord_token: Option<String>,
    discord_app_id: Option<String>,
    teams_webhook_url: Option<String>,
    google_chat_webhook_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct BootstrapPayload {
    version: String,
    gateway_port: u16,
    settings: serde_json::Value,
    channels: serde_json::Value,
    skills: serde_json::Value,
    doctor: serde_json::Value,
}

fn unauthorized_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "Unauthorized",
            "hint": "Provide Authorization: Bearer <admin-token-or-primary-password>"
        })),
    )
}

fn load_settings_auth_candidates() -> Vec<String> {
    let expected_admin = std::env::var("NANOBOT_ADMIN_TOKEN")
        .ok()
        .or_else(|| crate::security::read_admin_token().ok().flatten());
    let expected_primary = crate::security::read_primary_password();

    let mut candidates = Vec::new();
    if let Some(v) = expected_admin.filter(|s| !s.trim().is_empty()) {
        candidates.push(v);
    }
    if let Some(v) = expected_primary.filter(|s| !s.trim().is_empty()) {
        candidates.push(v);
    }
    candidates
}

fn settings_auth_candidates() -> Vec<String> {
    let now = Instant::now();
    if let Ok(cache) = SETTINGS_AUTH_CACHE.read()
        && let Some(loaded_at) = cache.loaded_at
        && now.saturating_duration_since(loaded_at) < SETTINGS_AUTH_CACHE_TTL
    {
        return cache.candidates.clone();
    }

    let loaded = load_settings_auth_candidates();
    if let Ok(mut cache) = SETTINGS_AUTH_CACHE.write() {
        cache.loaded_at = Some(now);
        cache.candidates = loaded.clone();
    }
    loaded
}

fn matches_any_secret_constant_time(candidate: &str, candidates: &[String]) -> bool {
    let mut matched = false;
    for secret in candidates {
        matched |= crate::security::secure_eq(candidate, secret);
    }
    matched
}

fn is_settings_authorized(headers: &HeaderMap) -> bool {
    let candidates = settings_auth_candidates();

    if candidates.is_empty() {
        return false;
    }

    let bearer_ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.strip_prefix("Bearer ")
                .map(|token| matches_any_secret_constant_time(token.trim(), &candidates))
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if bearer_ok {
        return true;
    }

    headers
        .get("x-admin-token")
        .and_then(|v| v.to_str().ok())
        .map(|v| matches_any_secret_constant_time(v.trim(), &candidates))
        .unwrap_or(false)
}

async fn get_settings(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let cfg = load_config_async().await;
    match cfg {
        Ok(c) => {
            let bind_host = std::env::var("NANOBOT_GATEWAY_BIND")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "127.0.0.1".to_string());

            (
                StatusCode::OK,
                Json(json!({
                    "interaction_policy": format!("{:?}", c.interaction_policy),
                    "gateway": {
                        "port": 18789,
                        "bind_host": bind_host,
                    },
                    "providers": {
                        "openai_connected": c.providers.openai.as_ref().and_then(|p| p.api_key.clone()).map(|k| !k.trim().is_empty()).unwrap_or(false),
                        "openrouter_connected": c.providers.openrouter.as_ref().and_then(|p| p.api_key.clone()).map(|k| !k.trim().is_empty()).unwrap_or(false),
                        "antigravity_connected": c.providers.antigravity.as_ref().and_then(|p| p.api_key.clone()).map(|k| !k.trim().is_empty()).unwrap_or(false),
                        "google_oauth_configured": c.providers.google.as_ref().and_then(|g| g.oauth_client_id.clone()).is_some() && c.providers.google.as_ref().and_then(|g| g.oauth_client_secret.clone()).is_some(),
                    }
                    ,
                    "channels": {
                        "telegram_configured": c.providers.telegram.as_ref().is_some(),
                        "slack_configured": c.providers.slack.as_ref().is_some(),
                        "discord_configured": c.providers.discord.as_ref().is_some(),
                        "teams_configured": c.providers.teams.as_ref().is_some(),
                        "google_chat_configured": c.providers.google_chat.as_ref().is_some(),
                    }
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to load settings: {}", e)})),
        ),
    }
}

async fn patch_settings(
    headers: HeaderMap,
    Json(req): Json<PatchSettingsRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let had_interaction_policy = req.interaction_policy.is_some();
    let had_teams_webhook = req.teams_webhook_url.is_some();
    let had_google_chat_webhook = req.google_chat_webhook_url.is_some();
    let had_provider_keys = req.openai_api_key.is_some()
        || req.openrouter_api_key.is_some()
        || req.antigravity_api_key.is_some()
        || req.antigravity_base_url.is_some()
        || req.google_api_key.is_some()
        || req.google_oauth_client_id.is_some()
        || req.google_oauth_client_secret.is_some();

    let mut cfg = match load_config_async().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load config: {}", e)})),
            );
        }
    };

    if let Some(policy) = req.interaction_policy {
        cfg.interaction_policy = match policy.trim().to_ascii_lowercase().as_str() {
            "interactive" | "ask_me" | "ask" => crate::config::InteractionPolicy::Interactive,
            "headlessdeny" | "safe" => crate::config::InteractionPolicy::HeadlessDeny,
            "headlessallowlog" | "autonomous" => crate::config::InteractionPolicy::HeadlessAllowLog,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid interaction_policy"})),
                );
            }
        };
    }

    if let Some(url) = req.teams_webhook_url {
        let val = url.trim().to_string();
        if !val.is_empty() {
            cfg.providers.teams = Some(crate::config::TeamsConfig { webhook_url: val });
        }
    }

    if let Some(url) = req.google_chat_webhook_url {
        let val = url.trim().to_string();
        if !val.is_empty() {
            cfg.providers.google_chat = Some(crate::config::GoogleChatConfig { webhook_url: val });
        }
    }

    if let Some(v) = req.openai_api_key {
        let p = cfg
            .providers
            .openai
            .get_or_insert(crate::config::OpenAIConfig {
                api_key: None,
                api_keys: None,
            });
        p.api_key = Some(v.trim().to_string());
    }
    if let Some(v) = req.openrouter_api_key {
        let p = cfg
            .providers
            .openrouter
            .get_or_insert(crate::config::OpenRouterConfig {
                api_key: None,
                api_keys: None,
            });
        p.api_key = Some(v.trim().to_string());
    }
    if let Some(v) = req.antigravity_api_key {
        let p = cfg
            .providers
            .antigravity
            .get_or_insert(crate::config::AntigravityConfig {
                api_key: None,
                api_keys: None,
                base_url: None,
                fallback_base_urls: None,
            });
        p.api_key = Some(v.trim().to_string());
    }
    if let Some(v) = req.antigravity_base_url {
        let p = cfg
            .providers
            .antigravity
            .get_or_insert(crate::config::AntigravityConfig {
                api_key: None,
                api_keys: None,
                base_url: None,
                fallback_base_urls: None,
            });
        p.base_url = Some(v.trim().to_string());
    }
    if let Some(v) = req.google_api_key {
        let p = cfg
            .providers
            .google
            .get_or_insert(crate::config::GoogleConfig {
                api_key: None,
                api_keys: None,
                oauth_client_id: None,
                oauth_client_secret: None,
            });
        p.api_key = Some(v.trim().to_string());
    }
    if let Some(v) = req.google_oauth_client_id {
        let p = cfg
            .providers
            .google
            .get_or_insert(crate::config::GoogleConfig {
                api_key: None,
                api_keys: None,
                oauth_client_id: None,
                oauth_client_secret: None,
            });
        p.oauth_client_id = Some(v.trim().to_string());
    }
    if let Some(v) = req.google_oauth_client_secret {
        let p = cfg
            .providers
            .google
            .get_or_insert(crate::config::GoogleConfig {
                api_key: None,
                api_keys: None,
                oauth_client_id: None,
                oauth_client_secret: None,
            });
        p.oauth_client_secret = Some(v.trim().to_string());
    }

    if let Err(e) = cfg.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save settings: {}", e)})),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "applied": {
                "interaction_policy": had_interaction_policy,
                "teams_webhook_url": had_teams_webhook,
                "google_chat_webhook_url": had_google_chat_webhook,
                "provider_keys": had_provider_keys,
            }
        })),
    )
}

async fn set_security_profile(
    headers: HeaderMap,
    Json(req): Json<SecurityProfileRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let mut cfg = match load_config_async().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load config: {}", e)})),
            );
        }
    };

    let level = req.level.trim().to_ascii_lowercase();
    let (policy, label) = match level.as_str() {
        "safe" => (crate::config::InteractionPolicy::HeadlessDeny, "safe"),
        "ask" | "ask_me" | "default" => (crate::config::InteractionPolicy::Interactive, "ask_me"),
        "autonomous" => (
            crate::config::InteractionPolicy::HeadlessAllowLog,
            "autonomous",
        ),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "level must be one of: safe, ask_me, autonomous"})),
            );
        }
    };

    cfg.interaction_policy = policy;
    if let Err(e) = cfg.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save profile: {}", e)})),
        );
    }

    (StatusCode::OK, Json(json!({"ok": true, "level": label})))
}

async fn settings_doctor(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let checks = json!({
        "gh": command_exists_async("gh").await,
        "deno": command_exists_async("deno").await,
        "node": command_exists_async("node").await,
        "primary_password_configured": crate::security::read_primary_password().is_some(),
        "openclaw_auth_writable": openclaw_auth_writable_async().await,
    });
    (StatusCode::OK, Json(json!({"checks": checks})))
}

async fn auth_google_connect(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let mut flow = crate::oauth::OAuthFlow::new("google");
    match flow.get_auth_url() {
        Ok(url) => (StatusCode::OK, Json(json!({"auth_url": url}))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Failed to build Google auth URL: {}", e)})),
        ),
    }
}

async fn auth_google_complete(
    headers: HeaderMap,
    Json(req): Json<GoogleAuthCompleteRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let mut flow = crate::oauth::OAuthFlow::new("google");
    match flow.complete_flow(&req.redirect_url).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok": true, "provider": "google"})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Google auth failed: {}", e)})),
        ),
    }
}

async fn get_channels_config(headers: HeaderMap) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let cfg = match load_config_async().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load config: {}", e)})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "telegram": {
                "configured": cfg.providers.telegram.is_some(),
                "allowed_users": cfg.providers.telegram.as_ref().and_then(|t| t.allowed_users.clone()).unwrap_or_default(),
            },
            "slack": {
                "configured": cfg.providers.slack.is_some(),
            },
            "discord": {
                "configured": cfg.providers.discord.is_some(),
            },
            "teams": {
                "configured": cfg.providers.teams.is_some(),
            },
            "google_chat": {
                "configured": cfg.providers.google_chat.is_some(),
            }
        })),
    )
}

async fn patch_channels_config(
    headers: HeaderMap,
    Json(req): Json<PatchChannelsConfigRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let mut cfg = match load_config_async().await {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load config: {}", e)})),
            );
        }
    };

    if let Some(token) = req.telegram_token {
        let tg = cfg
            .providers
            .telegram
            .get_or_insert(crate::config::TelegramConfig {
                bot_token: String::new(),
                allowed_users: None,
            });
        tg.bot_token = token.trim().to_string();
    }
    if let Some(allowed) = req.telegram_allowed_users {
        let tg = cfg
            .providers
            .telegram
            .get_or_insert(crate::config::TelegramConfig {
                bot_token: String::new(),
                allowed_users: None,
            });
        tg.allowed_users = Some(allowed);
    }

    if let Some(bot) = req.slack_bot_token {
        let slack = cfg
            .providers
            .slack
            .get_or_insert(crate::config::SlackConfig {
                bot_token: String::new(),
                app_token: None,
            });
        slack.bot_token = bot.trim().to_string();
    }
    if let Some(app) = req.slack_app_token {
        let slack = cfg
            .providers
            .slack
            .get_or_insert(crate::config::SlackConfig {
                bot_token: String::new(),
                app_token: None,
            });
        slack.app_token = Some(app.trim().to_string());
    }

    if let Some(token) = req.discord_token {
        let discord = cfg
            .providers
            .discord
            .get_or_insert(crate::config::DiscordConfig {
                token: String::new(),
                app_id: String::new(),
            });
        discord.token = token.trim().to_string();
    }
    if let Some(app_id) = req.discord_app_id {
        let discord = cfg
            .providers
            .discord
            .get_or_insert(crate::config::DiscordConfig {
                token: String::new(),
                app_id: String::new(),
            });
        discord.app_id = app_id.trim().to_string();
    }

    if let Some(url) = req.teams_webhook_url {
        cfg.providers.teams = Some(crate::config::TeamsConfig {
            webhook_url: url.trim().to_string(),
        });
    }
    if let Some(url) = req.google_chat_webhook_url {
        cfg.providers.google_chat = Some(crate::config::GoogleChatConfig {
            webhook_url: url.trim().to_string(),
        });
    }

    if let Err(e) = cfg.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save config: {}", e)})),
        );
    }

    (StatusCode::OK, Json(json!({"ok": true})))
}

async fn verify_channel(
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let channel = id.trim().to_ascii_lowercase();
    let cfg = load_config_async().await.ok();

    let response = match channel.as_str() {
        "web" => json!({
            "channel": "web",
            "ok": true,
            "message": "Web channel is built-in and available.",
        }),
        "telegram" => {
            let token = cfg
                .as_ref()
                .and_then(|c| c.providers.telegram.as_ref())
                .map(|t| t.bot_token.clone())
                .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok())
                .or_else(|| std::env::var("NANOBOT_TELEGRAM_TOKEN").ok())
                .unwrap_or_default();
            json!({
                "channel": "telegram",
                "ok": !token.trim().is_empty(),
                "message": if token.trim().is_empty() { "Telegram token missing" } else { "Telegram token configured" },
            })
        }
        "slack" => {
            let token = cfg
                .as_ref()
                .and_then(|c| c.providers.slack.as_ref())
                .map(|s| s.bot_token.clone())
                .or_else(|| std::env::var("SLACK_BOT_TOKEN").ok())
                .unwrap_or_default();
            json!({
                "channel": "slack",
                "ok": !token.trim().is_empty(),
                "message": if token.trim().is_empty() { "Slack bot token missing" } else { "Slack bot token configured" },
            })
        }
        "discord" => {
            let token = cfg
                .as_ref()
                .and_then(|c| c.providers.discord.as_ref())
                .map(|d| d.token.clone())
                .or_else(|| std::env::var("DISCORD_TOKEN").ok())
                .unwrap_or_default();
            let app_id = cfg
                .as_ref()
                .and_then(|c| c.providers.discord.as_ref())
                .map(|d| d.app_id.clone())
                .or_else(|| std::env::var("DISCORD_APP_ID").ok())
                .unwrap_or_default();
            json!({
                "channel": "discord",
                "ok": !token.trim().is_empty() && !app_id.trim().is_empty(),
                "message": if token.trim().is_empty() || app_id.trim().is_empty() { "Discord token or app_id missing" } else { "Discord credentials configured" },
            })
        }
        "teams" => {
            let url = cfg
                .as_ref()
                .and_then(|c| c.providers.teams.as_ref())
                .map(|t| t.webhook_url.clone())
                .or_else(|| std::env::var("TEAMS_WEBHOOK_URL").ok())
                .unwrap_or_default();
            let valid = url.starts_with("https://");
            json!({
                "channel": "teams",
                "ok": valid,
                "message": if valid { "Teams webhook configured" } else { "Teams webhook missing or invalid (must start with https://)" },
            })
        }
        "google_chat" | "google-chat" => {
            let url = cfg
                .as_ref()
                .and_then(|c| c.providers.google_chat.as_ref())
                .map(|t| t.webhook_url.clone())
                .or_else(|| std::env::var("GOOGLE_CHAT_WEBHOOK_URL").ok())
                .unwrap_or_default();
            let valid = url.starts_with("https://");
            json!({
                "channel": "google_chat",
                "ok": valid,
                "message": if valid { "Google Chat webhook configured" } else { "Google Chat webhook missing or invalid (must start with https://)" },
            })
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Unsupported channel",
                    "supported": ["web", "telegram", "slack", "discord", "teams", "google_chat"]
                })),
            );
        }
    };

    (StatusCode::OK, Json(response))
}

async fn list_skills() -> (StatusCode, Json<serde_json::Value>) {
    let workspace = crate::workspace::resolve_workspace_dir();

    let mut loader = crate::skills::SkillLoader::new(workspace);
    if let Err(e) = loader.scan() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to scan skills: {}", e)})),
        );
    }

    let cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    let mut items: Vec<serde_json::Value> = loader
        .skills()
        .values()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "enabled": cfg.is_enabled(&s.name),
                "backend": s.backend,
                "runtime_override": cfg.runtime_override(&s.name),
            })
        })
        .collect();

    items.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    (StatusCode::OK, Json(json!({"skills": items})))
}

#[derive(Debug, serde::Deserialize)]
struct GitHubDirEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    download_url: Option<String>,
    url: String,
}

async fn download_skill_tree_gateway(
    client: &reqwest::Client,
    repo: &str,
    skill_name: &str,
    destination: &std::path::Path,
) -> Result<()> {
    let root = format!("skills/{}", skill_name);
    let mut stack = vec![format!(
        "https://api.github.com/repos/{}/contents/{}",
        repo, root
    )];

    while let Some(api_url) = stack.pop() {
        let resp = client
            .get(&api_url)
            .header("User-Agent", "nanobot-gateway-skills")
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Failed to fetch skill tree from {} (status {})",
                api_url,
                resp.status()
            );
        }

        let entries: Vec<GitHubDirEntry> = resp.json().await?;
        for entry in entries {
            match entry.entry_type.as_str() {
                "dir" => stack.push(entry.url),
                "file" => {
                    let download_url = entry.download_url.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("Missing download_url for {}", entry.path)
                    })?;

                    let prefix = format!("skills/{}/", skill_name);
                    let relative = normalize_skill_relative_path(&entry.path, &prefix, &entry.name)?;
                    let output = destination.join(relative);
                    if let Some(parent) = output.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }

                    let bytes = client
                        .get(download_url)
                        .header("User-Agent", "nanobot-gateway-skills")
                        .send()
                        .await?
                        .bytes()
                        .await?;
                    tokio::fs::write(output, &bytes).await?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn normalize_skill_relative_path(
    path: &str,
    prefix: &str,
    fallback: &str,
) -> Result<std::path::PathBuf> {
    use std::path::{Component, Path, PathBuf};

    let raw = path.strip_prefix(prefix).unwrap_or(fallback);
    let rel = Path::new(raw);

    if rel.is_absolute() {
        anyhow::bail!("absolute path rejected in skill tree: {}", raw);
    }

    for component in rel.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("unsafe path rejected in skill tree: {}", raw)
            }
        }
    }

    let out = PathBuf::from(rel);
    if out.as_os_str().is_empty() {
        anyhow::bail!("empty path rejected in skill tree");
    }

    Ok(out)
}

async fn maybe_bootstrap_skill_dependencies_gateway(skill_dir: &std::path::Path) -> Vec<String> {
    let mut notes = Vec::new();

    if skill_dir.join("package.json").exists() {
        if !command_exists_async("npm").await {
            notes.push("package.json found but npm is not installed".to_string());
        } else {
            let dir = skill_dir.to_path_buf();
            let status = crate::blocking::process_output_in_dir(
                "npm".to_string(),
                vec!["install".to_string(), "--omit=dev".to_string()],
                std::time::Duration::from_secs(300),
                Some(dir.clone()),
            )
            .await
            .and_then(|output| {
                if output.status.success() {
                    Ok("ok".to_string())
                } else {
                    Err(anyhow::anyhow!(
                        "npm install failed in {}: {}",
                        dir.display(),
                        String::from_utf8_lossy(&output.stderr)
                    ))
                }
            });

            match status {
                Ok(_) => notes.push("npm dependencies installed".to_string()),
                Err(e) => notes.push(format!("npm install failed: {}", e)),
            }
        }
    }

    notes
}

async fn install_skill(
    headers: HeaderMap,
    Json(req): Json<SkillInstallRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let skill_name = req.skill.trim().to_ascii_lowercase();
    if skill_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "skill name cannot be empty"})),
        );
    }

    let repo = req
        .repo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("openclaw/openclaw")
        .to_string();
    let auto_enable = req.auto_enable.unwrap_or(true);
    let runtime_override = req.runtime.clone().map(|r| r.trim().to_ascii_lowercase());

    let skills_root = crate::workspace::resolve_skills_dir();
    if let Err(e) = crate::blocking::fs("gateway_install_skill_create_root", {
        let skills_root = skills_root.clone();
        move || {
            std::fs::create_dir_all(&skills_root)?;
            Ok(())
        }
    })
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to create skills dir: {}", e)})),
        );
    }

    let skill_dir = skills_root.join(&skill_name);
    if skill_dir.exists() {
        let _ = crate::blocking::fs("gateway_install_skill_remove_existing", {
            let skill_dir = skill_dir.clone();
            move || {
                std::fs::remove_dir_all(&skill_dir)?;
                Ok(())
            }
        })
        .await;
    }
    if let Err(e) = crate::blocking::fs("gateway_install_skill_create_dir", {
        let skill_dir = skill_dir.clone();
        move || {
            std::fs::create_dir_all(&skill_dir)?;
            Ok(())
        }
    })
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to prepare skill dir: {}", e)})),
        );
    }

    let client = reqwest::Client::new();
    if let Err(e) = download_skill_tree_gateway(&client, &repo, &skill_name, &skill_dir).await {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": e.to_string(), "repo": repo, "skill": skill_name})),
        );
    }

    let skill_md_path = skill_dir.join("SKILL.md");
    let local_skill_md = match crate::blocking::fs("gateway_install_skill_read_markdown", {
        let skill_md_path = skill_md_path.clone();
        move || {
            let v = std::fs::read_to_string(&skill_md_path)?;
            Ok(v)
        }
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Downloaded skill is missing SKILL.md: {}", e)})),
            );
        }
    };

    let parsed = match crate::skills::metadata::SkillMetadata::from_markdown(
        std::path::PathBuf::from(format!("/skills/{}/SKILL.md", skill_name)),
        &local_skill_md,
    ) {
        Ok(meta) => meta,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid SKILL.md: {}", e)})),
            );
        }
    };

    let bootstrap_notes = maybe_bootstrap_skill_dependencies_gateway(&skill_dir).await;

    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    if auto_enable {
        cfg.enable_skill(&skill_name);
    }
    if let Some(runtime) = runtime_override.as_deref()
        && matches!(runtime, "deno" | "node" | "native" | "mcp")
    {
        cfg.set_runtime_override(&skill_name, runtime);
    }
    if let Some(credentials) = req.credentials {
        for (k, v) in credentials {
            cfg.set_credential(&skill_name, &k, v);
        }
    }
    if let Err(e) = cfg.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Installed but failed to persist skill config: {}", e)})),
        );
    }

    let mut required = crate::skills::config::known_required_credentials(&skill_name);
    required.extend(crate::skills::config::required_credentials_from_schema(
        parsed.config_schema.as_deref(),
    ));
    required.sort();
    required.dedup();
    let missing_credentials = required
        .into_iter()
        .filter(|k| cfg.get_credential(&skill_name, k).is_none())
        .collect::<Vec<_>>();

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "skill": skill_name,
            "repo": repo,
            "path": skill_dir,
            "auto_enabled": auto_enable,
            "runtime": runtime_override,
            "backend": parsed.backend,
            "tools": parsed.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            "bootstrap": bootstrap_notes,
            "missing_credentials": missing_credentials,
        })),
    )
}

async fn skill_schema(AxumPath(id): AxumPath<String>) -> (StatusCode, Json<serde_json::Value>) {
    let skill_id = id.trim().to_ascii_lowercase();
    let workspace = crate::workspace::resolve_workspace_dir();

    let mut loader = crate::skills::SkillLoader::new(workspace);
    let _ = loader.scan();

    let schema_from_skill = loader
        .get_skill(&skill_id)
        .and_then(|s| s.config_schema.clone())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());

    let known = match skill_id.as_str() {
        "spotify" => vec![
            json!({"key":"client_id","type":"string","secret":false,"required":true,"label":"Spotify Client ID"}),
            json!({"key":"client_secret","type":"string","secret":true,"required":true,"label":"Spotify Client Secret"}),
        ],
        "weather" => vec![
            json!({"key":"api_key","type":"string","secret":true,"required":true,"label":"OpenWeather API Key"}),
        ],
        "notion" => vec![
            json!({"key":"api_key","type":"string","secret":true,"required":true,"label":"Notion API Key"}),
        ],
        _ => Vec::new(),
    };

    let schema = schema_from_skill.unwrap_or_else(|| {
        json!({
            "type": "object",
            "properties": known,
        })
    });

    (
        StatusCode::OK,
        Json(json!({
            "skill": skill_id,
            "schema": schema
        })),
    )
}

async fn update_skill_config(
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<SkillConfigUpdateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !is_settings_authorized(&headers) {
        return unauthorized_response();
    }

    let skill_id = id.trim().to_ascii_lowercase();
    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();

    if let Some(enabled) = req.enabled {
        if enabled {
            cfg.enable_skill(&skill_id);
        } else {
            cfg.disable_skill(&skill_id);
        }
    }

    if let Some(runtime) = req.runtime {
        cfg.set_runtime_override(&skill_id, &runtime);
    }

    if let Some(creds) = req.credentials {
        for (k, v) in creds {
            cfg.set_credential(&skill_id, &k, v);
        }
    }

    if let Err(e) = cfg.save() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save skill config: {}", e)})),
        );
    }

    (StatusCode::OK, Json(json!({"ok": true, "skill": skill_id})))
}

async fn test_skill(AxumPath(id): AxumPath<String>) -> (StatusCode, Json<serde_json::Value>) {
    let skill_id = id.trim().to_ascii_lowercase();
    let runtime_dir = crate::workspace::resolve_skills_dir().join(&skill_id);

    let skill_md = runtime_dir.join("SKILL.md");
    if !skill_md.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "skill not installed"})),
        );
    }

    let content = match crate::blocking::fs("gateway_test_skill_read_markdown", {
        let skill_md = skill_md.clone();
        move || {
            let content = std::fs::read_to_string(&skill_md)?;
            Ok(content)
        }
    })
    .await
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("read failure: {}", e)})),
            );
        }
    };

    let meta = match crate::skills::metadata::SkillMetadata::from_markdown(skill_md, &content) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid SKILL.md: {}", e)})),
            );
        }
    };

    let backend = meta.backend.to_ascii_lowercase();
    let runtime_ok = match backend.as_str() {
        "native" => {
            if let Some(c) = meta.native_command.as_deref() {
                command_exists_async(c).await
            } else {
                false
            }
        }
        "deno" => command_exists_async("deno").await,
        "node" => command_exists_async("node").await,
        "mcp" => true,
        _ => false,
    };

    (
        StatusCode::OK,
        Json(json!({
            "skill": skill_id,
            "backend": backend,
            "runtime_ok": runtime_ok,
            "installed": true,
        })),
    )
}

// WebSocket Handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(gateway): State<Arc<Gateway>>,
) -> impl IntoResponse {
    if let Some(sticky_header) = crate::distributed::sticky_signal_header() {
        let values = headers.get_all(sticky_header.as_str());
        let value_count = values.iter().count();
        if value_count > 1 {
            record_sticky_signal_conflict();
            tracing::warn!(
                sticky_signal_header = %sticky_header,
                "multiple sticky signal header values observed on websocket upgrade"
            );
        }

        let sticky_value = header_string(&headers, sticky_header.as_str());
        if sticky_value.is_none() {
            record_sticky_signal_missing();
            tracing::warn!(
                sticky_signal_header = %sticky_header,
                "sticky signal header missing on websocket upgrade"
            );

            if strict_multi_replica_sticky_mode() {
                crate::metrics::GLOBAL_METRICS
                    .increment_counter("sticky_violation_fatal_total", 1);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "missing_sticky_signal",
                        "message": format!("Missing required sticky signal header '{}'", sticky_header)
                    })),
                )
                    .into_response();
            }
        }
    }

    ws.on_upgrade(|socket| handle_socket(socket, gateway)).into_response()
}

async fn teams_webhook(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let started_at = Instant::now();
    let ingress_at = Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();
    incr_counter("webhook_requests_total", &[("channel", "teams"), ("status", "attempt")]);

    if let Err(reason) = verify_webhook_request(
        "teams",
        &headers,
        &body,
        "NANOBOT_TEAMS_WEBHOOK_SECRET",
        "NANOBOT_TEAMS_WEBHOOK_SIGNING_SECRET",
    )
    .await
    {
        tracing::warn!("Denied Teams webhook: {}", reason);
        incr_counter(
            "webhook_auth_fail_total",
            &[("channel", "teams"), ("reason", reason)],
        );
        incr_counter(
            "webhook_requests_total",
            &[("channel", "teams"), ("status", "unauthorized")],
        );
        record_duration(
            "webhook_request_duration_seconds",
            &[("channel", "teams")],
            started_at,
            false,
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": reason})),
        );
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            incr_counter(
                "webhook_requests_total",
                &[("channel", "teams"), ("status", "bad_request")],
            );
            record_duration(
                "webhook_request_duration_seconds",
                &[("channel", "teams")],
                started_at,
                false,
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid json payload"})),
            );
        }
    };

    let mut text = payload["text"].as_str().unwrap_or("").to_string();
    let user_id = payload["user_id"].as_str().unwrap_or("unknown").to_string();
    let channel_id = payload["channel_id"]
        .as_str()
        .unwrap_or("teams")
        .to_string();

    if text.is_empty() {
        incr_counter(
            "webhook_requests_total",
            &[("channel", "teams"), ("status", "bad_request")],
        );
        record_duration(
            "webhook_request_duration_seconds",
            &[("channel", "teams")],
            started_at,
            false,
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing text"})),
        );
    }

    match crate::gateway::onboarding::process_onboarding_message("teams", &user_id, &text).await {
        Ok(crate::gateway::onboarding::OnboardingOutcome::ReplyOnly(reply)) => {
            return (StatusCode::OK, Json(json!({"text": reply})));
        }
        Ok(crate::gateway::onboarding::OnboardingOutcome::NotNeeded) => {}
        Err(e) => {
            return (
                StatusCode::OK,
                Json(json!({"text": format!("Setup error: {}", e)})),
            );
        }
    }

    let session_id = build_session_id("teams", &channel_id, &user_id, gateway.dm_scope, true);

    if let Some(pending) = gateway.pending_questions.get(&session_id).await {
        match crate::tools::question::normalize_question_answer(&pending, &text) {
            Ok(normalized) => {
                text = normalized;
                gateway.pending_questions.remove(&session_id).await;
            }
            Err(err_msg) => {
                let prompt = crate::tools::question::format_question_prompt(&pending);
                return (
                    StatusCode::OK,
                    Json(json!({"text": format!("{}\n{}", err_msg, prompt)})),
                );
            }
        }
    }

    let (response_tx, mut response_rx) = mpsc::channel(100);
    let msg = AgentMessage {
        session_id: session_id.clone(),
        tenant_id: session_id.clone(),
        request_id: request_id.clone(),
        content: text,
        response_tx,
        ingress_at,
    };

    if gateway.agent_tx.send(msg).await.is_err() {
        incr_counter(
            "webhook_requests_total",
            &[("channel", "teams"), ("status", "agent_unavailable")],
        );
        record_duration(
            "webhook_request_duration_seconds",
            &[("channel", "teams")],
            started_at,
            false,
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "agent unavailable"})),
        );
    }

    let mut full_response = String::new();
    while let Some(chunk) = response_rx.recv().await {
        match chunk {
            StreamChunk::TextDelta(delta) => full_response.push_str(&delta),
            StreamChunk::ToolResult(result) => {
                if let Some(payload) = crate::tools::question::parse_question_payload(&result) {
                    let prompt = crate::tools::question::format_question_prompt(&payload);
                    gateway
                        .pending_questions
                        .insert(session_id.clone(), payload)
                        .await;
                    if !full_response.is_empty() {
                        full_response.push_str("\n\n");
                    }
                    full_response.push_str(&prompt);
                }
            }
            StreamChunk::Done { .. } => break,
            _ => {}
        }
    }

    incr_counter(
        "webhook_requests_total",
        &[("channel", "teams"), ("status", "ok")],
    );
    record_duration(
        "webhook_request_duration_seconds",
        &[("channel", "teams")],
        started_at,
        true,
    );
    (StatusCode::OK, Json(json!({"text": full_response})))
}

async fn google_chat_webhook(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let started_at = Instant::now();
    let ingress_at = Instant::now();
    let request_id = uuid::Uuid::new_v4().to_string();
    incr_counter(
        "webhook_requests_total",
        &[("channel", "google_chat"), ("status", "attempt")],
    );

    if let Err(reason) = verify_webhook_request(
        "google_chat",
        &headers,
        &body,
        "NANOBOT_GOOGLE_CHAT_WEBHOOK_SECRET",
        "NANOBOT_GOOGLE_CHAT_WEBHOOK_SIGNING_SECRET",
    )
    .await
    {
        tracing::warn!("Denied Google Chat webhook: {}", reason);
        incr_counter(
            "webhook_auth_fail_total",
            &[("channel", "google_chat"), ("reason", reason)],
        );
        incr_counter(
            "webhook_requests_total",
            &[("channel", "google_chat"), ("status", "unauthorized")],
        );
        record_duration(
            "webhook_request_duration_seconds",
            &[("channel", "google_chat")],
            started_at,
            false,
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": reason})),
        );
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            incr_counter(
                "webhook_requests_total",
                &[("channel", "google_chat"), ("status", "bad_request")],
            );
            record_duration(
                "webhook_request_duration_seconds",
                &[("channel", "google_chat")],
                started_at,
                false,
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid json payload"})),
            );
        }
    };

    let mut text = payload
        .get("message")
        .and_then(|m| m.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let user_id = payload
        .get("message")
        .and_then(|m| m.get("sender"))
        .and_then(|s| s.get("name"))
        .and_then(|t| t.as_str())
        .unwrap_or("users/unknown")
        .to_string();
    let channel_id = payload
        .get("space")
        .and_then(|s| s.get("name"))
        .and_then(|t| t.as_str())
        .unwrap_or("spaces/unknown")
        .to_string();
    let is_dm = payload
        .get("space")
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        .map(|t| t.eq_ignore_ascii_case("DM"))
        .unwrap_or(false);

    if text.is_empty() {
        incr_counter(
            "webhook_requests_total",
            &[("channel", "google_chat"), ("status", "bad_request")],
        );
        record_duration(
            "webhook_request_duration_seconds",
            &[("channel", "google_chat")],
            started_at,
            false,
        );
        return (StatusCode::BAD_REQUEST, Json(json!({"text": ""})));
    }

    match crate::gateway::onboarding::process_onboarding_message("google_chat", &user_id, &text)
        .await
    {
        Ok(crate::gateway::onboarding::OnboardingOutcome::ReplyOnly(reply)) => {
            return (StatusCode::OK, Json(json!({"text": reply})));
        }
        Ok(crate::gateway::onboarding::OnboardingOutcome::NotNeeded) => {}
        Err(e) => {
            return (
                StatusCode::OK,
                Json(json!({"text": format!("Setup error: {}", e)})),
            );
        }
    }

    let session_id = build_session_id(
        "google_chat",
        &channel_id,
        &user_id,
        gateway.dm_scope,
        is_dm,
    );

    if let Some(pending) = gateway.pending_questions.get(&session_id).await {
        match crate::tools::question::normalize_question_answer(&pending, &text) {
            Ok(normalized) => {
                text = normalized;
                gateway.pending_questions.remove(&session_id).await;
            }
            Err(err_msg) => {
                let prompt = crate::tools::question::format_question_prompt(&pending);
                return (
                    StatusCode::OK,
                    Json(json!({"text": format!("{}\n{}", err_msg, prompt)})),
                );
            }
        }
    }

    let (response_tx, mut response_rx) = mpsc::channel(100);
    let msg = AgentMessage {
        session_id: session_id.clone(),
        tenant_id: session_id.clone(),
        request_id: request_id.clone(),
        content: text,
        response_tx,
        ingress_at,
    };

    if gateway.agent_tx.send(msg).await.is_err() {
        incr_counter(
            "webhook_requests_total",
            &[("channel", "google_chat"), ("status", "agent_unavailable")],
        );
        record_duration(
            "webhook_request_duration_seconds",
            &[("channel", "google_chat")],
            started_at,
            false,
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"text": "Agent unavailable"})),
        );
    }

    let mut full_response = String::new();
    while let Some(chunk) = response_rx.recv().await {
        match chunk {
            StreamChunk::TextDelta(delta) => full_response.push_str(&delta),
            StreamChunk::ToolResult(result) => {
                if let Some(payload) = crate::tools::question::parse_question_payload(&result) {
                    let prompt = crate::tools::question::format_question_prompt(&payload);
                    gateway
                        .pending_questions
                        .insert(session_id.clone(), payload)
                        .await;
                    if !full_response.is_empty() {
                        full_response.push_str("\n\n");
                    }
                    full_response.push_str(&prompt);
                }
            }
            StreamChunk::Done { .. } => break,
            _ => {}
        }
    }

    incr_counter(
        "webhook_requests_total",
        &[("channel", "google_chat"), ("status", "ok")],
    );
    record_duration(
        "webhook_request_duration_seconds",
        &[("channel", "google_chat")],
        started_at,
        true,
    );
    (StatusCode::OK, Json(json!({"text": full_response})))
}

async fn handle_socket(socket: WebSocket, gateway: Arc<Gateway>) {
    let (ws_tx, mut ws_rx) = socket.split();
    let ws_tx = std::sync::Arc::new(tokio::sync::Mutex::new(ws_tx));
    let compat_mode = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let event_seq = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    // Do not consume/drop any initial client frame here.
    // Session init is server-driven and must not discard the first payload.
    let session_id = uuid::Uuid::new_v4().to_string();
    tracing::info!("New session: {}", session_id);

    let secret = gateway.gateway_session_secret.clone();
    let token = encode_session_token(secret.as_ref(), &session_id);

    // Send session_id back to client
    let response = json!({"type": "session_init", "session_id": session_id, "token": token});
    if let Err(err) = send_ws_text_timed(&ws_tx, response.to_string()).await {
        tracing::warn!(session_id = %session_id, error = %err, "Failed to send session_init");
        return;
    }

    let span = tracing::info_span!("websocket_session", session_id = %session_id);
    let _enter = span.enter();

    let require_token = require_ws_token_per_message();
    let mut warned_missing_token = false;

    tracing::info!("WebSocket session established");

    let (confirm_req_tx, mut confirm_req_rx) =
        mpsc::channel::<crate::tools::gateway_confirmation::GatewayConfirmationEvent>(10);
    let (confirm_resp_tx, confirm_resp_rx) =
        mpsc::channel::<crate::tools::gateway_confirmation::GatewayConfirmationEvent>(10);
    let confirm_channel = format!("web:{}", session_id);

    {
        let mut service = gateway.confirmation_service.lock().await;
        service.register_adapter(Box::new(
            crate::tools::gateway_confirmation::GatewayConfirmationAdapter::new(
                confirm_req_tx,
                confirm_resp_rx,
                confirm_channel,
            ),
        ));
    }

    // Create channel for agent responses
    let (response_tx, mut response_rx) = mpsc::channel(100);
    let (send_task_done_tx, mut send_task_done_rx) = oneshot::channel::<()>();
    let correlation_store = gateway.correlation_store.clone();
    let max_inflight_per_session = ws_max_inflight_per_session();

    // Spawn task to forward agent responses to WebSocket
    let ws_tx_clone = ws_tx.clone();
    let pending_questions = gateway.pending_questions.clone();
    let session_id_for_send = session_id.clone();
    let correlation_store_for_send = correlation_store.clone();
    let compat_mode_send = compat_mode.clone();
    let event_seq_send = event_seq.clone();
    #[cfg(test)]
    let panic_send_on_toolresult = gateway.panic_send_on_toolresult.clone();
    let send_task = tokio::spawn(async move {
        let send_loop_outcome = std::panic::AssertUnwindSafe(async {
            loop {
            tokio::select! {
                Some(chunk) = response_rx.recv() => {
                    match chunk {
                        StreamChunk::TextDelta(text) => {
                            let msg = if compat_mode_send.load(std::sync::atomic::Ordering::Relaxed) {
                                let seq = event_seq_send.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                json!({
                                    "type": "event",
                                    "event": "agent.delta",
                                    "seq": seq,
                                    "payload": { "delta": text }
                                })
                            } else {
                                json!({
                                    "type": "text_delta",
                                    "delta": text
                                })
                            };
                            if let Err(e) = send_ws_text_timed(&ws_tx_clone, msg.to_string()).await {
                                tracing::warn!("WS send error: {}", e);
                                break;
                            }
                        }
                        StreamChunk::Done { request_id, kind } => {
                            let (status, reason) = match &kind {
                                TerminalKind::SuccessDone => ("success_done", None),
                                TerminalKind::ErrorDone { reason, .. } => {
                                    ("error_done", Some(reason.clone()))
                                }
                                TerminalKind::CancelledDone { reason } => {
                                    ("cancelled_done", Some(reason.clone()))
                                }
                            };

                            if let Some(started) = correlation_store_for_send
                                .complete_request(&session_id_for_send, &request_id)
                                .await
                            {
                                crate::intelligent_router::INTELLIGENT_ROUTER
                                    .record_outcome(
                                        &session_id_for_send,
                                        status == "success_done",
                                        started,
                                    );
                            }
                            let msg = if compat_mode_send.load(std::sync::atomic::Ordering::Relaxed) {
                                let seq = event_seq_send.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                json!({"type": "event", "event": "agent.done", "seq": seq, "payload": {"request_id": request_id, "status": status, "reason": reason}})
                            } else {
                                json!({ "type": "done", "request_id": request_id, "status": status, "reason": reason })
                            };
                            if let Err(e) = send_ws_text_timed(&ws_tx_clone, msg.to_string()).await {
                                tracing::warn!("WS send error: {}", e);
                                break;
                            }
                        }
                        StreamChunk::ToolResult(result) => {
                            if let Some(payload) = crate::tools::question::parse_question_payload(&result) {
                                let prompt = crate::tools::question::format_question_prompt(&payload);
                                {
                                    pending_questions
                                        .insert(session_id_for_send.clone(), payload)
                                        .await;
                                }
                                #[cfg(test)]
                                if panic_send_on_toolresult
                                    .swap(false, std::sync::atomic::Ordering::Relaxed)
                                {
                                    panic!("test panic in ws send loop");
                                }
                                let msg = if compat_mode_send.load(std::sync::atomic::Ordering::Relaxed) {
                                    let seq = event_seq_send.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                    json!({
                                        "type": "event",
                                        "event": "agent.question",
                                        "seq": seq,
                                        "payload": { "prompt": prompt }
                                    })
                                } else {
                                    json!({
                                        "type": "question",
                                        "prompt": prompt,
                                    })
                                };
                                if let Err(e) = send_ws_text_timed(&ws_tx_clone, msg.to_string()).await {
                                    tracing::warn!("WS send error: {}", e);
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some(event) = confirm_req_rx.recv() => {
                    let serialized = if compat_mode_send.load(std::sync::atomic::Ordering::Relaxed) {
                        let seq = event_seq_send.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        serde_json::to_string(&json!({
                            "type": "event",
                            "event": "confirmation.request",
                            "seq": seq,
                            "payload": event,
                        }))
                    } else {
                        serde_json::to_string(&event)
                    };

                    if let Ok(text) = serialized {
                        if let Err(e) = send_ws_text_timed(&ws_tx_clone, text).await {
                            tracing::warn!("WS send error: {}", e);
                            break;
                        }
                    }
                }
                else => break,
            }
            }
        })
        .catch_unwind()
        .await;

        if send_loop_outcome.is_err() {
            crate::metrics::GLOBAL_METRICS.increment_counter("gateway_ws_send_task_panics_total", 1);
            tracing::error!(session_id = %session_id_for_send, "WebSocket send loop task panicked");
        }

        let _ = send_task_done_tx.send(());
    });

    // Handle incoming messages
    loop {
        let result = tokio::select! {
            _ = &mut send_task_done_rx => {
                tracing::warn!(session_id = %session_id, "WebSocket send loop ended; closing session");
                break;
            }
            result = ws_rx.next() => {
                match result {
                    Some(result) => result,
                    None => break,
                }
            }
        };

        match result {
            Ok(msg) => {
                if let WsMessage::Text(text) = msg {
                    let ws_received_at = std::time::Instant::now();
                    tracing::debug!(
                        session_id = %session_id,
                        bytes = text.len(),
                        "Received websocket text frame"
                    );
                    // Parse as JSON or assumes raw text in MVP?
                    // Let's assume raw text for "chat" for now, or JSON object.
                    // Basic protocol: {"message": "hello"}

                    let parsed_json = serde_json::from_str::<serde_json::Value>(&text).ok();

                    // Protocol compatibility mode: req/res/event envelope.
                    if let Some(req) = parsed_json.as_ref()
                        && req.get("type").and_then(|v| v.as_str()) == Some("req")
                    {
                        compat_mode.store(true, std::sync::atomic::Ordering::Relaxed);

                        let req_id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
                        let request_id = match parse_compat_request_id(req) {
                            Ok(id) => id,
                            Err(code) => {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": false,
                                    "error": { "code": code, "message": "Invalid request id" }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                        };
                        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
                        let params = req
                            .get("params")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));

                        let extracted_token = compat_extract_token(&params);

                        if method != "connect" {
                            if extracted_token.is_empty() {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": false,
                                    "error": { "code": "missing_token", "message": "Token required" }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }

                            if !validate_session_token(secret.as_ref(), &extracted_token, &session_id) {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": false,
                                    "error": { "code": "invalid_token", "message": "Invalid token" }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                        }

                        match method {
                            "connect" => {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": true,
                                    "payload": {
                                        "session_id": session_id,
                                        "token": encode_session_token(secret.as_ref(), &session_id),
                                        "protocol": "nanobot-gateway-v1"
                                    }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                            "health" | "ping" => {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": true,
                                    "payload": { "status": "ok" }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                            "status" => {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": true,
                                    "payload": { "session_id": session_id }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                            "refresh_token" => {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": true,
                                    "payload": { "token": encode_session_token(secret.as_ref(), &session_id) }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                            m if is_compat_confirmation_method(m) => {
                                let id = params
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let allowed = params
                                    .get("allowed")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if !id.is_empty() {
                                    let _ = confirm_resp_tx
                                        .send(crate::tools::gateway_confirmation::GatewayConfirmationEvent::Response {
                                            id,
                                            allowed,
                                            remember: false,
                                        })
                                        .await;
                                }
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": true,
                                    "payload": { "accepted": true }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                            m if is_compat_send_method(m) => {
                                let content = params
                                    .get("message")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| params.get("text").and_then(|v| v.as_str()))
                                    .unwrap_or("")
                                    .to_string();

                                if content.trim().is_empty() {
                                    let res = json!({
                                        "type": "res",
                                        "id": req_id,
                                        "ok": false,
                                        "error": { "code": "invalid_params", "message": "Missing params.message" }
                                    });
                                    if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                        tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                        break;
                                    }
                                    continue;
                                }

                                // NEW: Intelligent routing
                                let incoming_msg = crate::intelligent_router::IncomingMessage {
                                    id: request_id.clone(),
                                    content: content.clone(),
                                    user_id: session_id.clone(),
                                    channel: "web".to_string(),
                                    timestamp: std::time::Instant::now(),
                                };

                                let routing_result = crate::intelligent_router::INTELLIGENT_ROUTER
                                    .route(incoming_msg)
                                    .await;

                                match routing_result {
                                    crate::intelligent_router::RoutingResult::Throttled {
                                        retry_after,
                                    } => {
                                        let res = json!({
                                            "type": "res",
                                            "id": req_id,
                                            "ok": false,
                                            "error": {
                                                "code": "rate_limited",
                                                "message": format!("Rate limited. Retry after {:?}", retry_after)
                                            }
                                        });
                                        if let Err(err) =
                                            send_ws_text_timed(&ws_tx, res.to_string()).await
                                        {
                                            tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                            break;
                                        }
                                        continue;
                                    }
                                    crate::intelligent_router::RoutingResult::Command {
                                        command,
                                    } => {
                                        let payload = crate::intelligent_router::INTELLIGENT_ROUTER
                                            .command_response(&command, &session_id)
                                            .await;
                                        let res = json!({
                                            "type": "res",
                                            "id": req_id,
                                            "ok": true,
                                            "payload": {
                                                "accepted": true,
                                                "command_response": payload,
                                            }
                                        });
                                        if let Err(err) =
                                            send_ws_text_timed(&ws_tx, res.to_string()).await
                                        {
                                            tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                            break;
                                        }
                                        continue;
                                    }
                                    _ => {} // Continue with normal processing
                                }

                                let admission = correlation_store
                                    .try_register_inflight(
                                        &session_id,
                                        &request_id,
                                        max_inflight_per_session,
                                    )
                                    .await;
                                if !matches!(admission, crate::distributed::InflightAdmission::Admitted) {
                                    crate::metrics::GLOBAL_METRICS.increment_counter(
                                        "gateway_ws_concurrent_request_rejections_total",
                                        1,
                                    );
                                    let (code, message) = match admission {
                                        crate::distributed::InflightAdmission::Duplicate => (
                                            "duplicate_inflight_request",
                                            "Duplicate in-flight request id for this session",
                                        ),
                                        crate::distributed::InflightAdmission::BackendError => (
                                            "inflight_tracking_unavailable",
                                            "In-flight request tracking unavailable",
                                        ),
                                        _ => (
                                            "concurrent_request_rejected",
                                            "Too many in-flight requests for this session",
                                        ),
                                    };
                                    let res = json!({
                                        "type": "res",
                                        "id": req_id,
                                        "ok": false,
                                        "error": {
                                            "code": code,
                                            "message": message
                                        }
                                    });
                                    if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                        tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                        break;
                                    }
                                    if !matches!(admission, crate::distributed::InflightAdmission::Duplicate) {
                                        let seq = event_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                        let terminal = json!({
                                            "type": "event",
                                            "event": "agent.done",
                                            "seq": seq,
                                            "payload": {
                                                "request_id": request_id,
                                                "status": "error_done",
                                                "reason": code
                                            }
                                        });
                                        if let Err(err) = send_ws_text_timed(&ws_tx, terminal.to_string()).await {
                                            tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                            break;
                                        }
                                    }
                                    continue;
                                }

                                let msg = AgentMessage {
                                    session_id: session_id.clone(),
                                    tenant_id: format!("web:{}", session_id),
                                    request_id: request_id.clone(),
                                    content,
                                    response_tx: response_tx.clone(),
                                    ingress_at: ws_received_at,
                                };
                                correlation_store
                                    .mark_started(
                                        &session_id,
                                        &request_id,
                                        std::time::Instant::now(),
                                    )
                                    .await;
                                let recv_to_send_started = std::time::Instant::now();
                                let send_res = gateway.agent_tx.send(msg).await;
                                crate::metrics::GLOBAL_METRICS.record_duration(
                                    "gateway_ws_recv_to_agent_send_seconds",
                                    ws_received_at.elapsed(),
                                    send_res.is_ok(),
                                );
                                crate::metrics::GLOBAL_METRICS.record_duration(
                                    "gateway_ws_agent_send_wait_seconds",
                                    recv_to_send_started.elapsed(),
                                    send_res.is_ok(),
                                );
                                if let Err(e) = send_res {
                                    if let Some(started) = correlation_store
                                        .remove_request(&session_id, &request_id)
                                        .await
                                    {
                                        crate::intelligent_router::INTELLIGENT_ROUTER
                                            .record_outcome(&session_id, false, started);
                                    }
                                    let res = json!({
                                        "type": "res",
                                        "id": req_id,
                                        "ok": false,
                                        "error": { "code": "agent_unavailable", "message": e.to_string() }
                                    });
                                    if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await
                                    {
                                        tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                        break;
                                    }
                                    let seq = event_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                                    let terminal = json!({
                                        "type": "event",
                                            "event": "agent.done",
                                            "seq": seq,
                                            "payload": {
                                            "request_id": request_id,
                                            "status": "error_done",
                                            "reason": "agent_unavailable"
                                        }
                                    });
                                    if let Err(err) =
                                        send_ws_text_timed(&ws_tx, terminal.to_string()).await
                                    {
                                        tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                        break;
                                    }
                                } else {
                                    let res = json!({
                                        "type": "res",
                                        "id": req_id,
                                        "ok": true,
                                        "payload": { "accepted": true }
                                    });
                                    if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await
                                    {
                                        tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                        break;
                                    }
                                }
                            }
                            _ => {
                                let res = json!({
                                    "type": "res",
                                    "id": req_id,
                                    "ok": false,
                                    "error": { "code": "unknown_method", "message": format!("Unknown method: {}", method) }
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, res.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                        }

                        continue;
                    }

                    if let Some(json) = parsed_json.as_ref() {
                        if json["type"] == "refresh_token" {
                            let new_token = encode_session_token(secret.as_ref(), &session_id);
                            let msg = json!({"type": "session_refresh", "token": new_token});
                            if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                break;
                            }
                            continue;
                        }

                        let token = json["token"].as_str().unwrap_or("").trim();
                        if token.is_empty() {
                            if require_token {
                                let msg = json!({"type": "error", "error": "missing_token", "action": "refresh_token"});
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                            if !warned_missing_token {
                                warned_missing_token = true;
                                tracing::warn!(
                                    "Legacy WS client without per-message token detected for session {}",
                                    session_id
                                );
                            }
                        } else if !validate_session_token(secret.as_ref(), token, &session_id) {
                            let msg = json!({"type": "error", "error": "invalid_token", "action": "refresh_token"});
                            if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                break;
                            }
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

                    let (mut content, request_id) = if let Some(json) = parsed_json {
                        let parsed_request_id = match parse_non_compat_request_id(&json) {
                            Ok(parsed) => parsed,
                            Err(code) => {
                                let msg = json!({"type": "error", "error": code});
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                        };
                        (
                            json["message"].as_str().unwrap_or("").to_string(),
                            parsed_request_id,
                        )
                    } else {
                        let msg = json!({"type": "error", "error": "invalid_payload"});
                        if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                            tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                            break;
                        }
                        continue;
                    };

                    if let Some(pending) = gateway.pending_questions.get(&session_id).await {
                        match crate::tools::question::normalize_question_answer(&pending, &content)
                        {
                            Ok(normalized) => {
                                content = normalized;
                                gateway.pending_questions.remove(&session_id).await;
                            }
                            Err(err_msg) => {
                                let prompt =
                                    crate::tools::question::format_question_prompt(&pending);
                                let msg =
                                    json!({"type": "question", "error": err_msg, "prompt": prompt});
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                        }
                    }

                    if !onboarding_bypass_for_tests() {
                        match crate::gateway::onboarding::process_onboarding_message(
                            "web",
                            &session_id,
                            &content,
                        )
                        .await
                        {
                            Ok(crate::gateway::onboarding::OnboardingOutcome::ReplyOnly(reply)) => {
                                let msg = json!({"type": "text_delta", "delta": reply});
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                let done_request_id = request_id
                                    .clone()
                                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                                let done = json!({
                                    "type": "done",
                                    "request_id": done_request_id,
                                    "status": "success_done",
                                    "reason": serde_json::Value::Null
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, done.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                            Ok(crate::gateway::onboarding::OnboardingOutcome::NotNeeded) => {}
                            Err(e) => {
                                let msg =
                                    json!({"type": "error", "error": format!("Setup error: {}", e)});
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                        }
                    }

                    if !content.is_empty() {
                        let incoming_msg = crate::intelligent_router::IncomingMessage {
                            id: uuid::Uuid::new_v4().to_string(),
                            content: content.clone(),
                            user_id: session_id.clone(),
                            channel: "web".to_string(),
                            timestamp: std::time::Instant::now(),
                        };

                        let routing_result = crate::intelligent_router::INTELLIGENT_ROUTER
                            .route(incoming_msg)
                            .await;

                        match routing_result {
                            crate::intelligent_router::RoutingResult::Throttled { retry_after } => {
                                let msg = json!({
                                    "type": "error",
                                    "error": format!("Rate limited. Retry after {:?}", retry_after)
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                            crate::intelligent_router::RoutingResult::Command { command } => {
                                let payload = crate::intelligent_router::INTELLIGENT_ROUTER
                                    .command_response(&command, &session_id)
                                    .await;
                                let msg = json!({
                                    "type": "text_delta",
                                    "delta": payload.to_string()
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                let done_request_id = request_id
                                    .clone()
                                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                                let done = json!({
                                    "type": "done",
                                    "request_id": done_request_id,
                                    "status": "success_done",
                                    "reason": serde_json::Value::Null
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, done.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                                continue;
                            }
                            _ => {}
                        }

                        let request_id = request_id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

                        let admission = correlation_store
                            .try_register_inflight(
                                &session_id,
                                &request_id,
                                max_inflight_per_session,
                            )
                            .await;
                        if !matches!(admission, crate::distributed::InflightAdmission::Admitted) {
                            crate::metrics::GLOBAL_METRICS.increment_counter(
                                "gateway_ws_concurrent_request_rejections_total",
                                1,
                            );
                            let reason = match admission {
                                crate::distributed::InflightAdmission::Duplicate => {
                                    "duplicate_inflight_request"
                                }
                                crate::distributed::InflightAdmission::BackendError => {
                                    "inflight_tracking_unavailable"
                                }
                                _ => "concurrent_request_rejected",
                            };
                            if !matches!(admission, crate::distributed::InflightAdmission::Duplicate) {
                                let terminal = json!({
                                    "type": "done",
                                    "request_id": request_id,
                                    "status": "error_done",
                                    "reason": reason
                                });
                                if let Err(err) = send_ws_text_timed(&ws_tx, terminal.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            } else {
                                let msg = json!({"type": "error", "error": reason});
                                if let Err(err) = send_ws_text_timed(&ws_tx, msg.to_string()).await {
                                    tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                                    break;
                                }
                            }
                            continue;
                        }

                        let msg = AgentMessage {
                            session_id: session_id.clone(),
                            tenant_id: format!("web:{}", session_id),
                            request_id: request_id.clone(),
                            content,
                            response_tx: response_tx.clone(),
                            ingress_at: ws_received_at,
                        };
                        correlation_store
                            .mark_started(&session_id, &request_id, std::time::Instant::now())
                            .await;
                        let recv_to_send_started = std::time::Instant::now();
                        let send_res = gateway.agent_tx.send(msg).await;
                        crate::metrics::GLOBAL_METRICS.record_duration(
                            "gateway_ws_recv_to_agent_send_seconds",
                            ws_received_at.elapsed(),
                            send_res.is_ok(),
                        );
                        crate::metrics::GLOBAL_METRICS.record_duration(
                            "gateway_ws_agent_send_wait_seconds",
                            recv_to_send_started.elapsed(),
                            send_res.is_ok(),
                        );
                        if let Err(e) = send_res {
                            if let Some(started) = correlation_store
                                .remove_request(&session_id, &request_id)
                                .await
                            {
                                crate::intelligent_router::INTELLIGENT_ROUTER.record_outcome(
                                    &session_id,
                                    false,
                                    started,
                                );
                            }
                            tracing::warn!("Failed to send to agent: {}", e);
                            let terminal = json!({
                                "type": "done",
                                "request_id": request_id,
                                "status": "error_done",
                                "reason": "agent_unavailable"
                            });
                            if let Err(err) = send_ws_text_timed(&ws_tx, terminal.to_string()).await {
                                tracing::warn!(session_id = %session_id, error = %err, "WS send error");
                            }
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("WS receive error: {}", e);
                break;
            }
        }
    }

    {
        gateway.pending_questions.remove(&session_id).await;
    }
    correlation_store.clear_session(&session_id).await;

    if !send_task.is_finished() {
        send_task.abort();
    }
    match send_task.await {
        Ok(_) => {}
        Err(join_err) if join_err.is_cancelled() => {
            tracing::debug!(session_id = %session_id, "WebSocket send loop task cancelled");
        }
        Err(join_err) if join_err.is_panic() => {
            crate::metrics::GLOBAL_METRICS.increment_counter("gateway_ws_send_task_join_panics_total", 1);
            tracing::error!(session_id = %session_id, "WebSocket send task join panicked");
        }
        Err(join_err) => {
            tracing::warn!(session_id = %session_id, error = %join_err, "WebSocket send loop task failed");
        }
    }
    println!("WebSocket disconnected: {}", session_id);
}
