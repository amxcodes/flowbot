use async_trait::async_trait;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::HashMap;
#[cfg(all(test, feature = "distributed-redis"))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributedStoreBackend {
    InMemory,
    Redis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionMode {
    Local,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalingMode {
    Sticky,
    Stateless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLimiterBackend {
    Local,
    Redis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLimiterFailureMode {
    Open,
    Closed,
}

fn parse_distributed_store_backend(raw: &str) -> Option<DistributedStoreBackend> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "in-memory" | "in_memory" | "memory" | "local" | "default" => {
            Some(DistributedStoreBackend::InMemory)
        }
        "redis" => Some(DistributedStoreBackend::Redis),
        _ => None,
    }
}

fn parse_admission_mode(raw: &str) -> Option<AdmissionMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" | "default" => Some(AdmissionMode::Local),
        "global" => Some(AdmissionMode::Global),
        _ => None,
    }
}

fn parse_scaling_mode(raw: &str) -> Option<ScalingMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "sticky" | "default" => Some(ScalingMode::Sticky),
        "stateless" => Some(ScalingMode::Stateless),
        _ => None,
    }
}

fn parse_provider_limiter_backend(raw: &str) -> Option<ProviderLimiterBackend> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" | "default" => Some(ProviderLimiterBackend::Local),
        "redis" => Some(ProviderLimiterBackend::Redis),
        _ => None,
    }
}

fn parse_provider_limiter_failure_mode(raw: &str) -> Option<ProviderLimiterFailureMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "open" | "allow" | "fail-open" => Some(ProviderLimiterFailureMode::Open),
        "closed" | "deny" | "fail-closed" => Some(ProviderLimiterFailureMode::Closed),
        _ => None,
    }
}

pub fn selected_distributed_store_backend() -> DistributedStoreBackend {
    std::env::var("NANOBOT_DISTRIBUTED_STORE_BACKEND")
        .ok()
        .as_deref()
        .and_then(parse_distributed_store_backend)
        .unwrap_or(DistributedStoreBackend::InMemory)
}

pub fn selected_admission_mode() -> AdmissionMode {
    std::env::var("NANOBOT_ADMISSION_MODE")
        .ok()
        .as_deref()
        .and_then(parse_admission_mode)
        .unwrap_or(AdmissionMode::Local)
}

pub fn selected_scaling_mode() -> ScalingMode {
    std::env::var("NANOBOT_SCALING_MODE")
        .ok()
        .as_deref()
        .and_then(parse_scaling_mode)
        .unwrap_or(ScalingMode::Sticky)
}

fn provider_limiter_qps_limit() -> u64 {
    std::env::var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}

#[cfg(feature = "distributed-redis")]
fn ci_env_enabled() -> bool {
    std::env::var("CI")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(feature = "distributed-redis")]
fn redis_test_ttl_override_secs() -> Option<usize> {
    if !(cfg!(test) || ci_env_enabled()) {
        return None;
    }
    std::env::var("NANOBOT_TEST_REDIS_STORE_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
}

#[cfg(feature = "distributed-redis")]
fn effective_redis_store_ttl_secs(default_secs: usize) -> usize {
    redis_test_ttl_override_secs().unwrap_or(default_secs)
}

fn provider_limiter_redis_timeout_ms() -> (u64, bool) {
    match std::env::var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS") {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(ms) if ms > 0 => (ms, true),
            _ => (150, false),
        },
        Err(_) => (150, false),
    }
}

#[cfg(feature = "distributed-redis")]
fn provider_limiter_redis_timeout() -> std::time::Duration {
    let (millis, _) = provider_limiter_redis_timeout_ms();
    std::time::Duration::from_millis(millis)
}

pub fn provider_limiter_enabled() -> bool {
    provider_limiter_qps_limit() > 0
}

fn selected_provider_limiter_backend() -> ProviderLimiterBackend {
    std::env::var("NANOBOT_PROVIDER_LIMITER_BACKEND")
        .ok()
        .as_deref()
        .and_then(parse_provider_limiter_backend)
        .unwrap_or(ProviderLimiterBackend::Local)
}

fn selected_provider_limiter_failure_mode() -> ProviderLimiterFailureMode {
    std::env::var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE")
        .ok()
        .as_deref()
        .and_then(parse_provider_limiter_failure_mode)
        .unwrap_or(ProviderLimiterFailureMode::Open)
}

fn distributed_store_strict_mode() -> bool {
    std::env::var("NANOBOT_DISTRIBUTED_STORE_STRICT")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn admission_mode_strict() -> bool {
    std::env::var("NANOBOT_ADMISSION_STRICT")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn scaling_mode_strict() -> bool {
    std::env::var("NANOBOT_SCALING_STRICT")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn runtime_env_is_production() -> bool {
    std::env::var("NANOBOT_ENV")
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "production" || v == "prod"
        })
        .unwrap_or(false)
}

fn configured_replica_count() -> usize {
    std::env::var("NANOBOT_REPLICA_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1)
}

fn configured_replica_count_raw() -> Option<usize> {
    std::env::var("NANOBOT_REPLICA_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
}

fn redis_url_configured() -> bool {
    std::env::var("NANOBOT_REDIS_URL")
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(feature = "distributed-redis")]
fn redis_startup_timeout_ms() -> (u64, bool) {
    match std::env::var("NANOBOT_REDIS_STARTUP_TIMEOUT_MS") {
        Ok(raw) => match raw.parse::<u64>() {
            Ok(ms) if ms > 0 => (ms, true),
            _ => (300, false),
        },
        Err(_) => (300, false),
    }
}

#[cfg(feature = "distributed-redis")]
fn redis_startup_retry_count() -> u32 {
    std::env::var("NANOBOT_REDIS_STARTUP_RETRY_COUNT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| v.min(1))
        .unwrap_or(0)
}

fn format_multi_replica_prereq_error(replicas: usize, checks: &[(String, bool)]) -> String {
    let mut out = vec![format!(
        "FATAL: multi-replica configuration invalid (replicas={})",
        replicas
    )];
    out.push("Requirements: ".to_string());
    for (label, ok) in checks {
        out.push(format!("[{}] {}", if *ok { "PASS" } else { "FAIL" }, label));
    }
    out.push("Fix: set sticky scaling + sticky header + redis distributed store + redis limiter (closed) + NANOBOT_REDIS_URL".to_string());
    out.join("\n")
}

#[cfg(feature = "distributed-redis")]
fn classify_redis_startup_error(err: &redis::RedisError) -> &'static str {
    use redis::ErrorKind;
    match err.kind() {
        ErrorKind::AuthenticationFailed => "AUTH failure",
        ErrorKind::IoError => "DNS/TCP connect failed",
        _ => "Redis command failed",
    }
}

#[cfg(feature = "distributed-redis")]
async fn check_redis_reachability(timeout_ms: u64) -> anyhow::Result<()> {
    let redis_url = std::env::var("NANOBOT_REDIS_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("NANOBOT_REDIS_URL missing"))?;

    let client = redis::Client::open(redis_url)
        .map_err(|err| anyhow::anyhow!("Redis URL parse failed: {}", err))?;

    let timeout = std::time::Duration::from_millis(timeout_ms);
    let mut conn = match tokio::time::timeout(timeout, client.get_multiplexed_async_connection()).await {
        Ok(Ok(c)) => c,
        Ok(Err(err)) => {
            return Err(anyhow::anyhow!(
                "Redis connect failed: {} ({})",
                err,
                classify_redis_startup_error(&err)
            ));
        }
        Err(_) => {
            return Err(anyhow::anyhow!(
                "Redis connect timeout after {}ms",
                timeout_ms
            ));
        }
    };

    let ping: redis::RedisResult<String> =
        tokio::time::timeout(timeout, redis::cmd("PING").query_async(&mut conn))
            .await
            .map_err(|_| anyhow::anyhow!("Redis PING timeout after {}ms", timeout_ms))?;
    let pong = ping.map_err(|err| {
        anyhow::anyhow!(
            "Redis PING failed: {} ({})",
            err,
            classify_redis_startup_error(&err)
        )
    })?;
    if pong != "PONG" {
        return Err(anyhow::anyhow!("Redis PING unexpected response: {}", pong));
    }
    Ok(())
}

pub async fn enforce_multi_replica_runtime_readiness() -> anyhow::Result<()> {
    let replicas = configured_replica_count();
    if replicas <= 1 {
        return Ok(());
    }

    #[cfg(not(feature = "distributed-redis"))]
    {
        return Err(anyhow::anyhow!(
            "FATAL: multi-replica requires distributed-redis feature; binary built without it"
        ));
    }

    #[cfg(feature = "distributed-redis")]
    {
        let (timeout_ms, explicit_timeout) = redis_startup_timeout_ms();
        if !explicit_timeout {
            tracing::warn!(
                timeout_ms = timeout_ms,
                "multi-replica startup using default redis reachability timeout"
            );
        }

        let retries = redis_startup_retry_count();
        for attempt in 0..=retries {
            match check_redis_reachability(timeout_ms).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if attempt == retries {
                        return Err(anyhow::anyhow!(
                            "FATAL: multi-replica startup redis reachability check failed (attempts={} timeout_ms={}): {}",
                            retries + 1,
                            timeout_ms,
                            err
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn terminal_dedupe_fail_closed() -> bool {
    configured_replica_count() > 1
}

fn provider_limiter_identity(provider: &str) -> String {
    let env_key = format!(
        "NANOBOT_PROVIDER_LIMITER_IDENTITY_{}",
        provider.to_ascii_uppercase().replace('-', "_")
    );
    std::env::var(&env_key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("NANOBOT_PROVIDER_LIMITER_IDENTITY")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "default".to_string())
}

static LOCAL_PROVIDER_LIMITER_STATE: Lazy<Mutex<HashMap<String, (i64, u64)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(all(test, feature = "distributed-redis"))]
static PROVIDER_LIMITER_TEST_CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(all(test, feature = "distributed-redis"))]
fn provider_limiter_test_sleep_config() -> (u64, Option<u64>) {
    let sleep_ms = std::env::var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let every_n = std::env::var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0);
    (sleep_ms, every_n)
}

#[cfg(all(test, feature = "distributed-redis"))]
async fn maybe_inject_provider_limiter_test_sleep() {
    let (sleep_ms, every_n) = provider_limiter_test_sleep_config();
    if sleep_ms == 0 {
        return;
    }

    let call_idx = PROVIDER_LIMITER_TEST_CALL_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    let should_sleep = match every_n {
        Some(n) => call_idx % n == 0,
        None => true,
    };
    if should_sleep {
        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
    }
}

#[cfg(all(test, feature = "distributed-redis"))]
pub(crate) fn reset_provider_limiter_test_state() {
    PROVIDER_LIMITER_TEST_CALL_COUNTER.store(0, Ordering::Relaxed);
}

pub async fn allow_provider_request(provider: &str) -> bool {
    if !provider_limiter_enabled() {
        return true;
    }

    match selected_provider_limiter_backend() {
        ProviderLimiterBackend::Local => allow_provider_request_local(provider).await,
        ProviderLimiterBackend::Redis => {
            #[cfg(feature = "distributed-redis")]
            {
                allow_provider_request_redis(provider).await
            }
            #[cfg(not(feature = "distributed-redis"))]
            {
                crate::metrics::GLOBAL_METRICS.increment_counter(
                    &format!(
                        "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                        provider
                    ),
                    1,
                );
                match selected_provider_limiter_failure_mode() {
                    ProviderLimiterFailureMode::Open => true,
                    ProviderLimiterFailureMode::Closed => false,
                }
            }
        }
    }
}

async fn allow_provider_request_local(provider: &str) -> bool {
    let limit = provider_limiter_qps_limit();
    if limit == 0 {
        return true;
    }

    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let identity = provider_limiter_identity(provider);
    let key = format!("{}:{}", provider, identity);

    let mut state = LOCAL_PROVIDER_LIMITER_STATE.lock().await;
    let entry = state.entry(key).or_insert((now_sec, 0));
    if entry.0 != now_sec {
        entry.0 = now_sec;
        entry.1 = 0;
    }
    entry.1 = entry.1.saturating_add(1);
    let allowed = entry.1 <= limit;

    crate::metrics::GLOBAL_METRICS.increment_counter(
        &format!(
            "provider_global_limiter_checks_total{{provider={},result={}}}",
            provider,
            if allowed { "allow" } else { "deny" }
        ),
        1,
    );

    allowed
}

#[cfg(feature = "distributed-redis")]
async fn allow_provider_request_redis(provider: &str) -> bool {
    const PROVIDER_LIMITER_LUA: &str = r#"
local current = redis.call('INCR', KEYS[1])
if current == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
if current <= tonumber(ARGV[2]) then
  return 1
end
return 0
"#;

    let limit = provider_limiter_qps_limit();
    if limit == 0 {
        return true;
    }

    let redis_url = match std::env::var("NANOBOT_REDIS_URL") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            crate::metrics::GLOBAL_METRICS.increment_counter(
                &format!(
                    "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                    provider
                ),
                1,
            );
            return match selected_provider_limiter_failure_mode() {
                ProviderLimiterFailureMode::Open => true,
                ProviderLimiterFailureMode::Closed => false,
            };
        }
    };

    let key_prefix = std::env::var("NANOBOT_REDIS_KEY_PREFIX")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "nanobot".to_string());

    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let identity = provider_limiter_identity(provider);
    let key = format!(
        "{}:provider_limiter:{}:{}:{}",
        key_prefix, provider, identity, now_sec
    );

    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, "provider limiter redis client init failed");
            crate::metrics::GLOBAL_METRICS.increment_counter(
                &format!(
                    "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                    provider
                ),
                1,
            );
            return match selected_provider_limiter_failure_mode() {
                ProviderLimiterFailureMode::Open => true,
                ProviderLimiterFailureMode::Closed => false,
            };
        }
    };

    let timeout = provider_limiter_redis_timeout();

    let mut conn = match tokio::time::timeout(timeout, client.get_multiplexed_async_connection()).await
    {
        Ok(Ok(c)) => c,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "provider limiter redis connect failed");
            crate::metrics::GLOBAL_METRICS.increment_counter(
                &format!(
                    "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                    provider
                ),
                1,
            );
            return match selected_provider_limiter_failure_mode() {
                ProviderLimiterFailureMode::Open => true,
                ProviderLimiterFailureMode::Closed => false,
            };
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = timeout.as_millis() as u64,
                "provider limiter redis connect timed out"
            );
            crate::metrics::GLOBAL_METRICS.increment_counter(
                &format!(
                    "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                    provider
                ),
                1,
            );
            return match selected_provider_limiter_failure_mode() {
                ProviderLimiterFailureMode::Open => true,
                ProviderLimiterFailureMode::Closed => false,
            };
        }
    };

    let script = redis::Script::new(PROVIDER_LIMITER_LUA);
    let allowed = tokio::time::timeout(timeout, async {
        #[cfg(test)]
        {
            maybe_inject_provider_limiter_test_sleep().await;
        }
        script
            .key(&key)
            .arg(2000_i64)
            .arg(limit as i64)
            .invoke_async::<i64>(&mut conn)
            .await
    })
    .await;
    let allowed = match allowed {
        Ok(Ok(v)) => v == 1,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, key = %key, "provider limiter redis script failed");
            crate::metrics::GLOBAL_METRICS.increment_counter(
                &format!(
                    "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                    provider
                ),
                1,
            );
            return match selected_provider_limiter_failure_mode() {
                ProviderLimiterFailureMode::Open => true,
                ProviderLimiterFailureMode::Closed => false,
            };
        }
        Err(_) => {
            tracing::warn!(
                key = %key,
                timeout_ms = timeout.as_millis() as u64,
                "provider limiter redis script timed out"
            );
            crate::metrics::GLOBAL_METRICS.increment_counter(
                &format!(
                    "provider_global_limiter_errors_total{{provider={},backend=redis}}",
                    provider
                ),
                1,
            );
            return match selected_provider_limiter_failure_mode() {
                ProviderLimiterFailureMode::Open => true,
                ProviderLimiterFailureMode::Closed => false,
            };
        }
    };
    crate::metrics::GLOBAL_METRICS.increment_counter(
        &format!(
            "provider_global_limiter_checks_total{{provider={},result={}}}",
            provider,
            if allowed { "allow" } else { "deny" }
        ),
        1,
    );
    allowed
}

pub fn sticky_signal_header() -> Option<String> {
    std::env::var("NANOBOT_STICKY_SIGNAL_HEADER")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn effective_scaling_mode() -> ScalingMode {
    let selected = selected_scaling_mode();
    if selected == ScalingMode::Stateless {
        ScalingMode::Sticky
    } else {
        selected
    }
}

pub fn enforce_scaling_mode_support() -> anyhow::Result<()> {
    let selected = selected_scaling_mode();
    if selected == ScalingMode::Stateless && scaling_mode_strict() {
        return Err(anyhow::anyhow!(
            "NANOBOT_SCALING_MODE=stateless is selected but stateless bus-based streaming is not implemented yet (strict mode enabled)"
        ));
    }

    let effective = effective_scaling_mode();

    if effective == ScalingMode::Sticky
        && scaling_mode_strict()
        && configured_replica_count_raw().is_none()
    {
        return Err(anyhow::anyhow!(
            "Sticky scaling strict mode requires NANOBOT_REPLICA_COUNT to be explicitly configured"
        ));
    }

    let replicas = configured_replica_count();

    if replicas > 1 {
        let backend = selected_provider_limiter_backend();
        let failure_mode = selected_provider_limiter_failure_mode();
        let checks = vec![
            (
                format!(
                    "NANOBOT_SCALING_MODE=sticky (got={:?})",
                    selected
                ),
                selected == ScalingMode::Sticky,
            ),
            (
                format!(
                    "NANOBOT_STICKY_SIGNAL_HEADER present (present={})",
                    sticky_signal_header().is_some()
                ),
                sticky_signal_header().is_some(),
            ),
            (
                format!(
                    "global provider limiter enabled (NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT>0, enabled={})",
                    provider_limiter_enabled()
                ),
                provider_limiter_enabled(),
            ),
            (
                format!(
                    "provider limiter backend=redis (got={:?})",
                    backend
                ),
                backend == ProviderLimiterBackend::Redis,
            ),
            (
                format!(
                    "provider limiter failure_mode=closed (got={:?})",
                    failure_mode
                ),
                failure_mode == ProviderLimiterFailureMode::Closed,
            ),
            (
                format!(
                    "distributed store backend=redis (got={:?})",
                    selected_distributed_store_backend()
                ),
                selected_distributed_store_backend() == DistributedStoreBackend::Redis,
            ),
            (
                format!(
                    "distributed-redis feature compiled (compiled={})",
                    cfg!(feature = "distributed-redis")
                ),
                cfg!(feature = "distributed-redis"),
            ),
            (
                format!("NANOBOT_REDIS_URL present (present={})", redis_url_configured()),
                redis_url_configured(),
            ),
        ];

        if checks.iter().any(|(_, ok)| !ok) {
            return Err(anyhow::anyhow!(format_multi_replica_prereq_error(
                replicas, &checks
            )));
        }
    }

    if effective == ScalingMode::Sticky && scaling_mode_strict() && replicas > 1 {
        if !provider_limiter_enabled() {
            return Err(anyhow::anyhow!(
                "Sticky scaling mode with NANOBOT_REPLICA_COUNT>1 requires global provider limiter configuration in strict mode"
            ));
        }

        let backend = selected_provider_limiter_backend();
        let failure_mode = selected_provider_limiter_failure_mode();
        if backend != ProviderLimiterBackend::Redis {
            return Err(anyhow::anyhow!(
                "Sticky scaling strict mode with NANOBOT_REPLICA_COUNT>1 requires NANOBOT_PROVIDER_LIMITER_BACKEND=redis"
            ));
        }
        if failure_mode != ProviderLimiterFailureMode::Closed {
            return Err(anyhow::anyhow!(
                "Sticky scaling strict mode with NANOBOT_REPLICA_COUNT>1 requires NANOBOT_PROVIDER_LIMITER_FAILURE_MODE=closed"
            ));
        }

        let (timeout_ms, explicit) = provider_limiter_redis_timeout_ms();
        if explicit {
            tracing::info!(
                timeout_ms = timeout_ms,
                "Sticky strict multi-replica mode using explicit provider limiter redis timeout"
            );
        } else {
            tracing::warn!(
                timeout_ms = timeout_ms,
                "Sticky strict multi-replica mode using default provider limiter redis timeout; set NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS explicitly if needed"
            );
            crate::metrics::GLOBAL_METRICS.increment_counter(
                "distributed_scaling_mode_guard_warnings_total{reason=provider_limiter_redis_timeout_defaulted}",
                1,
            );
        }
    }

    if selected == ScalingMode::Stateless {
        tracing::warn!(
            "NANOBOT_SCALING_MODE=stateless requested but cross-node stream bus is not implemented; falling back to sticky mode"
        );
        crate::metrics::GLOBAL_METRICS.increment_counter(
            "distributed_scaling_mode_fallback_total{reason=stateless_not_implemented}",
            1,
        );
    }

    if effective == ScalingMode::Sticky && runtime_env_is_production() {
        if sticky_signal_header().is_none() {
            if scaling_mode_strict() {
                return Err(anyhow::anyhow!(
                    "Sticky scaling mode in production requires NANOBOT_STICKY_SIGNAL_HEADER to be configured when strict mode is enabled"
                ));
            }
            tracing::warn!(
                "Sticky scaling mode active in production but NANOBOT_STICKY_SIGNAL_HEADER is not configured"
            );
            crate::metrics::GLOBAL_METRICS.increment_counter(
                "distributed_scaling_mode_guard_warnings_total{reason=sticky_signal_header_missing}",
                1,
            );
        }

        if replicas > 1 && !provider_limiter_enabled() {
            tracing::warn!(
                replicas = replicas,
                "Sticky scaling mode with multiple replicas detected but global provider limiter is disabled"
            );
            crate::metrics::GLOBAL_METRICS.increment_counter(
                "distributed_scaling_mode_guard_warnings_total{reason=global_provider_limiter_missing}",
                1,
            );
        }
    }

    Ok(())
}

pub fn enforce_admission_mode_support() -> anyhow::Result<()> {
    let mode = selected_admission_mode();
    if mode == AdmissionMode::Global {
        if admission_mode_strict() {
            return Err(anyhow::anyhow!(
                "NANOBOT_ADMISSION_MODE=global is selected but global/distributed admission is not implemented yet (strict mode enabled)"
            ));
        }
        tracing::warn!(
            "NANOBOT_ADMISSION_MODE=global requested but not implemented; falling back to local admission mode"
        );
        crate::metrics::GLOBAL_METRICS.increment_counter(
            "distributed_admission_mode_fallback_total{reason=global_not_implemented}",
            1,
        );
    }
    Ok(())
}

pub fn enforce_distributed_backend_support() -> anyhow::Result<()> {
    let backend = selected_distributed_store_backend();
    if backend == DistributedStoreBackend::Redis
        && distributed_store_strict_mode()
        && !cfg!(feature = "distributed-redis")
    {
        return Err(anyhow::anyhow!(
            "NANOBOT_DISTRIBUTED_STORE_BACKEND=redis is selected but binary was built without 'distributed-redis' feature (strict mode enabled)"
        ));
    }
    Ok(())
}

pub fn build_pending_question_store() -> PendingQuestionStoreRef {
    match selected_distributed_store_backend() {
        DistributedStoreBackend::InMemory => Arc::new(InMemoryPendingQuestionStore::new()),
        DistributedStoreBackend::Redis => {
            #[cfg(feature = "distributed-redis")]
            {
                match RedisPendingQuestionStore::from_env() {
                    Ok(store) => Arc::new(store),
                    Err(err) => {
                        if distributed_store_strict_mode() {
                            panic!(
                                "strict distributed mode: failed to initialize redis pending-question store: {}",
                                err
                            );
                        }
                        tracing::warn!(
                            error = %err,
                            "Redis pending-question store init failed; using in-memory store fallback"
                        );
                        crate::metrics::GLOBAL_METRICS.increment_counter(
                            "distributed_store_backend_fallback_total{backend=redis}",
                            1,
                        );
                        Arc::new(InMemoryPendingQuestionStore::new())
                    }
                }
            }
            #[cfg(not(feature = "distributed-redis"))]
            {
                tracing::warn!(
                    "Redis pending-question store requested but 'distributed-redis' feature is disabled; using in-memory store fallback"
                );
                crate::metrics::GLOBAL_METRICS
                    .increment_counter("distributed_store_backend_fallback_total{backend=redis}", 1);
                Arc::new(InMemoryPendingQuestionStore::new())
            }
        }
    }
}

pub fn build_session_correlation_store() -> SessionCorrelationStoreRef {
    match selected_distributed_store_backend() {
        DistributedStoreBackend::InMemory => Arc::new(InMemorySessionCorrelationStore::new()),
        DistributedStoreBackend::Redis => {
            #[cfg(feature = "distributed-redis")]
            {
                match RedisSessionCorrelationStore::from_env() {
                    Ok(store) => Arc::new(store),
                    Err(err) => {
                        if distributed_store_strict_mode() {
                            panic!(
                                "strict distributed mode: failed to initialize redis session-correlation store: {}",
                                err
                            );
                        }
                        tracing::warn!(
                            error = %err,
                            "Redis session-correlation store init failed; using in-memory store fallback"
                        );
                        crate::metrics::GLOBAL_METRICS.increment_counter(
                            "distributed_store_backend_fallback_total{backend=redis}",
                            1,
                        );
                        Arc::new(InMemorySessionCorrelationStore::new())
                    }
                }
            }
            #[cfg(not(feature = "distributed-redis"))]
            {
                tracing::warn!(
                    "Redis session-correlation store requested but 'distributed-redis' feature is disabled; using in-memory store fallback"
                );
                crate::metrics::GLOBAL_METRICS
                    .increment_counter("distributed_store_backend_fallback_total{backend=redis}", 1);
                Arc::new(InMemorySessionCorrelationStore::new())
            }
        }
    }
}

pub fn build_terminal_dedupe_store() -> TerminalDedupeStoreRef {
    match selected_distributed_store_backend() {
        DistributedStoreBackend::InMemory => Arc::new(InMemoryTerminalDedupeStore::new()),
        DistributedStoreBackend::Redis => {
            #[cfg(feature = "distributed-redis")]
            {
                match RedisTerminalDedupeStore::from_env() {
                    Ok(store) => Arc::new(store),
                    Err(err) => {
                        if distributed_store_strict_mode() {
                            panic!(
                                "strict distributed mode: failed to initialize redis terminal-dedupe store: {}",
                                err
                            );
                        }
                        tracing::warn!(
                            error = %err,
                            "Redis terminal-dedupe store init failed; using in-memory fallback"
                        );
                        crate::metrics::GLOBAL_METRICS.increment_counter(
                            "distributed_store_backend_fallback_total{backend=redis}",
                            1,
                        );
                        Arc::new(InMemoryTerminalDedupeStore::new())
                    }
                }
            }
            #[cfg(not(feature = "distributed-redis"))]
            {
                tracing::warn!(
                    "Redis terminal-dedupe store requested but 'distributed-redis' feature is disabled; using in-memory fallback"
                );
                crate::metrics::GLOBAL_METRICS
                    .increment_counter("distributed_store_backend_fallback_total{backend=redis}", 1);
                Arc::new(InMemoryTerminalDedupeStore::new())
            }
        }
    }
}

#[cfg(feature = "distributed-redis")]
#[derive(Clone)]
pub struct RedisSessionCorrelationStore {
    client: redis::Client,
    key_prefix: String,
}

#[cfg(feature = "distributed-redis")]
impl RedisSessionCorrelationStore {
    pub fn from_env() -> anyhow::Result<Self> {
        let redis_url = std::env::var("NANOBOT_REDIS_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("NANOBOT_REDIS_URL is required for redis backend"))?;
        let key_prefix = std::env::var("NANOBOT_REDIS_KEY_PREFIX")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "nanobot".to_string());
        Ok(Self {
            client: redis::Client::open(redis_url)?,
            key_prefix,
        })
    }

    fn inflight_key(&self, session_id: &str) -> String {
        format!("{}:corr:{}:inflight", self.key_prefix, session_id)
    }

    fn started_key(&self, session_id: &str) -> String {
        format!("{}:corr:{}:started_ms", self.key_prefix, session_id)
    }

    fn now_epoch_ms() -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        now.as_millis() as i64
    }

    fn elapsed_from_epoch_ms(started_ms: i64) -> Option<std::time::Duration> {
        let now_ms = Self::now_epoch_ms();
        if now_ms < started_ms {
            return None;
        }
        Some(std::time::Duration::from_millis((now_ms - started_ms) as u64))
    }
}

#[cfg(feature = "distributed-redis")]
#[async_trait]
impl SessionCorrelationStore for RedisSessionCorrelationStore {
    async fn inflight_count(&self, session_id: &str) -> usize {
        use redis::AsyncCommands;
        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis correlation inflight_count failed to connect");
                return 0;
            }
        };
        let key = self.inflight_key(session_id);
        match conn.scard::<_, usize>(&key).await {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(error = %err, key = %key, "redis correlation inflight_count failed");
                0
            }
        }
    }

    async fn register_inflight(&self, session_id: &str, request_id: &str) {
        use redis::AsyncCommands;
        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis correlation register_inflight failed to connect");
                return;
            }
        };
        let key = self.inflight_key(session_id);
        let res: redis::RedisResult<usize> = conn.sadd(&key, request_id).await;
        if let Err(err) = res {
            tracing::warn!(error = %err, key = %key, "redis correlation register_inflight failed");
            return;
        }
        let ttl = effective_redis_store_ttl_secs(3600);
        let _ = conn.expire::<_, bool>(&key, ttl as i64).await;
    }

    async fn try_register_inflight(
        &self,
        session_id: &str,
        request_id: &str,
        cap: usize,
    ) -> InflightAdmission {
        if cap == 0 {
            return InflightAdmission::OverLimit;
        }

        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "redis correlation try_register_inflight failed to connect"
                );
                return InflightAdmission::BackendError;
            }
        };

        let key = self.inflight_key(session_id);
        let ttl = effective_redis_store_ttl_secs(3600) as i64;
        let script = redis::Script::new(
            r#"
local key = KEYS[1]
local request_id = ARGV[1]
local cap = tonumber(ARGV[2])
local ttl = tonumber(ARGV[3])

if redis.call('SISMEMBER', key, request_id) == 1 then
  return 2
end

local current = redis.call('SCARD', key)
if current >= cap then
  return 1
end

redis.call('SADD', key, request_id)
redis.call('EXPIRE', key, ttl)
return 0
"#,
        );

        let result: redis::RedisResult<i64> = script
            .key(&key)
            .arg(request_id)
            .arg(cap as i64)
            .arg(ttl)
            .invoke_async(&mut conn)
            .await;

        match result {
            Ok(0) => InflightAdmission::Admitted,
            Ok(1) => InflightAdmission::OverLimit,
            Ok(2) => InflightAdmission::Duplicate,
            Ok(_) => InflightAdmission::BackendError,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    key = %key,
                    "redis correlation try_register_inflight script failed"
                );
                InflightAdmission::BackendError
            }
        }
    }

    async fn mark_started(&self, session_id: &str, request_id: &str, _started: std::time::Instant) {
        use redis::AsyncCommands;
        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis correlation mark_started failed to connect");
                return;
            }
        };
        let key = self.started_key(session_id);
        let ms = Self::now_epoch_ms();
        let res: redis::RedisResult<usize> = conn.hset(&key, request_id, ms).await;
        if let Err(err) = res {
            tracing::warn!(error = %err, key = %key, "redis correlation mark_started failed");
            return;
        }
        let ttl = effective_redis_store_ttl_secs(3600);
        let _ = conn.expire::<_, bool>(&key, ttl as i64).await;
    }

    async fn complete_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<std::time::Duration> {
        use redis::AsyncCommands;
        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis correlation complete_request failed to connect");
                return None;
            }
        };

        let inflight_key = self.inflight_key(session_id);
        let started_key = self.started_key(session_id);

        let _ = conn.srem::<_, _, usize>(&inflight_key, request_id).await;
        let started_ms: Option<i64> = match conn.hget(&started_key, request_id).await {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(error = %err, key = %started_key, "redis correlation hget failed");
                None
            }
        };
        let _ = conn.hdel::<_, _, usize>(&started_key, request_id).await;

        let inflight_len: redis::RedisResult<usize> = conn.scard(&inflight_key).await;
        if matches!(inflight_len, Ok(0)) {
            let _ = conn.del::<_, usize>(&inflight_key).await;
        }
        let started_len: redis::RedisResult<usize> = conn.hlen(&started_key).await;
        if matches!(started_len, Ok(0)) {
            let _ = conn.del::<_, usize>(&started_key).await;
        }

        started_ms.and_then(Self::elapsed_from_epoch_ms)
    }

    async fn remove_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<std::time::Duration> {
        self.complete_request(session_id, request_id).await
    }

    async fn clear_session(&self, session_id: &str) {
        use redis::AsyncCommands;
        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis correlation clear_session failed to connect");
                return;
            }
        };
        let inflight_key = self.inflight_key(session_id);
        let started_key = self.started_key(session_id);
        let _ = conn.del::<_, usize>(&inflight_key).await;
        let _ = conn.del::<_, usize>(&started_key).await;
    }
}

#[cfg(feature = "distributed-redis")]
#[derive(Clone)]
pub struct RedisTerminalDedupeStore {
    client: redis::Client,
    key_prefix: String,
    ttl_secs: usize,
}

#[cfg(feature = "distributed-redis")]
impl RedisTerminalDedupeStore {
    pub fn from_env() -> anyhow::Result<Self> {
        let redis_url = std::env::var("NANOBOT_REDIS_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("NANOBOT_REDIS_URL is required for redis backend"))?;
        let key_prefix = std::env::var("NANOBOT_REDIS_KEY_PREFIX")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "nanobot".to_string());
        let ttl_secs = std::env::var("NANOBOT_TERMINAL_DEDUPE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(86_400);
        let ttl_secs = effective_redis_store_ttl_secs(ttl_secs);
        Ok(Self {
            client: redis::Client::open(redis_url)?,
            key_prefix,
            ttl_secs,
        })
    }

    fn key(&self, session_id: &str, request_id: &str) -> String {
        format!(
            "{}:terminal_dedupe:{}:{}",
            self.key_prefix, session_id, request_id
        )
    }
}

#[cfg(feature = "distributed-redis")]
#[async_trait]
impl TerminalDedupeStore for RedisTerminalDedupeStore {
    async fn try_mark_terminal(&self, session_id: &str, request_id: &str) -> bool {
        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                crate::metrics::GLOBAL_METRICS
                    .increment_counter("terminal_dedupe_store_errors_total{backend=redis}", 1);
                if terminal_dedupe_fail_closed() {
                    tracing::error!(
                        error = %err,
                        "redis terminal dedupe failed to connect; blocking terminal emit in multi-replica mode"
                    );
                    return false;
                }
                tracing::warn!(
                    error = %err,
                    "redis terminal dedupe failed to connect; allowing terminal emit in single-replica mode"
                );
                return true;
            }
        };

        let key = self.key(session_id, request_id);
        let result: redis::RedisResult<Option<String>> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(self.ttl_secs)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(err) => {
                crate::metrics::GLOBAL_METRICS
                    .increment_counter("terminal_dedupe_store_errors_total{backend=redis}", 1);
                if terminal_dedupe_fail_closed() {
                    tracing::error!(
                        error = %err,
                        key = %key,
                        "redis terminal dedupe set failed; blocking terminal emit in multi-replica mode"
                    );
                    return false;
                }
                tracing::warn!(
                    error = %err,
                    key = %key,
                    "redis terminal dedupe set failed; allowing terminal emit in single-replica mode"
                );
                true
            }
        }
    }
}

#[cfg(feature = "distributed-redis")]
#[derive(Clone)]
pub struct RedisPendingQuestionStore {
    client: redis::Client,
    key_prefix: String,
    index_key: String,
    ttl_secs: usize,
}

#[cfg(feature = "distributed-redis")]
impl RedisPendingQuestionStore {
    pub fn from_env() -> anyhow::Result<Self> {
        let redis_url = std::env::var("NANOBOT_REDIS_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("NANOBOT_REDIS_URL is required for redis backend"))?;

        let key_prefix = std::env::var("NANOBOT_REDIS_KEY_PREFIX")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "nanobot".to_string());
        let ttl_secs = std::env::var("NANOBOT_PENDING_QUESTION_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(3600);
        let ttl_secs = effective_redis_store_ttl_secs(ttl_secs);

        let client = redis::Client::open(redis_url)?;
        let index_key = format!("{}:pending_questions:index", key_prefix);

        Ok(Self {
            client,
            key_prefix,
            index_key,
            ttl_secs,
        })
    }

    fn item_key(&self, session_id: &str) -> String {
        format!("{}:pending_questions:{}", self.key_prefix, session_id)
    }
}

#[cfg(all(feature = "distributed-redis", test))]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RedisDebugCounts {
    pub pending_index_count: usize,
    pub correlation_key_count: usize,
    pub terminal_dedupe_key_count: usize,
}

#[cfg(all(feature = "distributed-redis", test))]
async fn redis_scan_count(
    conn: &mut redis::aio::MultiplexedConnection,
    pattern: &str,
) -> redis::RedisResult<usize> {
    let mut cursor: u64 = 0;
    let mut total: usize = 0;
    loop {
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(256)
            .query_async(conn)
            .await?;
        total += keys.len();
        if next == 0 {
            break;
        }
        cursor = next;
    }
    Ok(total)
}

#[cfg(all(feature = "distributed-redis", test))]
pub(crate) async fn redis_debug_counts_for_prefix(
    redis_url: &str,
    key_prefix: &str,
) -> anyhow::Result<RedisDebugCounts> {
    use redis::AsyncCommands;

    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let pending_index_key = format!("{}:pending_questions:index", key_prefix);
    let pending_index_count: usize = conn.scard(&pending_index_key).await.unwrap_or(0);
    let correlation_key_count =
        redis_scan_count(&mut conn, &format!("{}:corr:*", key_prefix)).await.unwrap_or(0);
    let terminal_dedupe_key_count =
        redis_scan_count(&mut conn, &format!("{}:terminal_dedupe:*", key_prefix))
            .await
            .unwrap_or(0);
    Ok(RedisDebugCounts {
        pending_index_count,
        correlation_key_count,
        terminal_dedupe_key_count,
    })
}

#[cfg(feature = "distributed-redis")]
#[async_trait]
impl PendingQuestionStore for RedisPendingQuestionStore {
    async fn get(
        &self,
        session_id: &str,
    ) -> Option<crate::tools::question::QuestionPayload> {
        use redis::AsyncCommands;

        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis pending-question get failed to connect");
                return None;
            }
        };

        let key = self.item_key(session_id);
        let raw: Option<String> = match conn.get(&key).await {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(error = %err, key = %key, "redis pending-question get failed");
                return None;
            }
        };

        raw.and_then(|v| match serde_json::from_str::<crate::tools::question::QuestionPayload>(&v) {
            Ok(payload) => Some(payload),
            Err(err) => {
                tracing::warn!(error = %err, key = %key, "redis pending-question decode failed");
                None
            }
        })
    }

    async fn insert(&self, session_id: String, payload: crate::tools::question::QuestionPayload) {
        use redis::AsyncCommands;

        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis pending-question insert failed to connect");
                return;
            }
        };

        let key = self.item_key(&session_id);
        let serialized = match serde_json::to_string(&payload) {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(error = %err, "redis pending-question serialize failed");
                return;
            }
        };

        let set_res: redis::RedisResult<()> = conn.set_ex(&key, serialized, self.ttl_secs as u64).await;
        if let Err(err) = set_res {
            tracing::warn!(error = %err, key = %key, "redis pending-question set failed");
            return;
        }

        let sadd_res: redis::RedisResult<()> = conn.sadd(&self.index_key, &session_id).await;
        if let Err(err) = sadd_res {
            tracing::warn!(error = %err, key = %self.index_key, "redis pending-question index add failed");
        }
    }

    async fn remove(&self, session_id: &str) {
        use redis::AsyncCommands;

        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis pending-question remove failed to connect");
                return;
            }
        };

        let key = self.item_key(session_id);
        let del_res: redis::RedisResult<usize> = conn.del(&key).await;
        if let Err(err) = del_res {
            tracing::warn!(error = %err, key = %key, "redis pending-question delete failed");
        }
        let srem_res: redis::RedisResult<usize> = conn.srem(&self.index_key, session_id).await;
        if let Err(err) = srem_res {
            tracing::warn!(error = %err, key = %self.index_key, "redis pending-question index remove failed");
        }
    }

    async fn is_empty(&self) -> bool {
        use redis::AsyncCommands;

        let mut conn = match self.client.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(error = %err, "redis pending-question is_empty failed to connect");
                return false;
            }
        };

        let len: redis::RedisResult<usize> = conn.scard(&self.index_key).await;
        match len {
            Ok(v) => v == 0,
            Err(err) => {
                tracing::warn!(error = %err, key = %self.index_key, "redis pending-question index count failed");
                false
            }
        }
    }
}

/// Abstraction seam for session-scoped pending question state.
///
/// Current default implementation is in-memory; this trait exists to enable
/// distributed backends (e.g. Redis/Postgres) without rewriting gateway logic.
#[async_trait]
pub trait PendingQuestionStore: Send + Sync {
    async fn get(
        &self,
        session_id: &str,
    ) -> Option<crate::tools::question::QuestionPayload>;

    async fn insert(&self, session_id: String, payload: crate::tools::question::QuestionPayload);

    async fn remove(&self, session_id: &str);

    async fn is_empty(&self) -> bool;
}

pub type PendingQuestionStoreRef = Arc<dyn PendingQuestionStore>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflightAdmission {
    Admitted,
    OverLimit,
    Duplicate,
    BackendError,
}

#[async_trait]
pub trait SessionCorrelationStore: Send + Sync {
    async fn inflight_count(&self, session_id: &str) -> usize;

    async fn try_register_inflight(
        &self,
        session_id: &str,
        request_id: &str,
        cap: usize,
    ) -> InflightAdmission;

    async fn register_inflight(&self, session_id: &str, request_id: &str);

    async fn mark_started(&self, session_id: &str, request_id: &str, started: std::time::Instant);

    async fn complete_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<std::time::Duration>;

    async fn remove_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<std::time::Duration>;

    async fn clear_session(&self, session_id: &str);
}

pub type SessionCorrelationStoreRef = Arc<dyn SessionCorrelationStore>;

#[derive(Default)]
pub struct InMemorySessionCorrelationStore {
    inflight: Mutex<HashMap<String, std::collections::HashSet<String>>>,
    starts: Mutex<HashMap<String, HashMap<String, std::time::Instant>>>,
}

impl InMemorySessionCorrelationStore {
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
            starts: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl SessionCorrelationStore for InMemorySessionCorrelationStore {
    async fn inflight_count(&self, session_id: &str) -> usize {
        let inflight = self.inflight.lock().await;
        inflight
            .get(session_id)
            .map(std::collections::HashSet::len)
            .unwrap_or(0)
    }

    async fn register_inflight(&self, session_id: &str, request_id: &str) {
        let mut inflight = self.inflight.lock().await;
        inflight
            .entry(session_id.to_string())
            .or_insert_with(std::collections::HashSet::new)
            .insert(request_id.to_string());
    }

    async fn try_register_inflight(
        &self,
        session_id: &str,
        request_id: &str,
        cap: usize,
    ) -> InflightAdmission {
        let mut inflight = self.inflight.lock().await;
        let set = inflight
            .entry(session_id.to_string())
            .or_insert_with(std::collections::HashSet::new);

        if set.contains(request_id) {
            return InflightAdmission::Duplicate;
        }
        if set.len() >= cap {
            return InflightAdmission::OverLimit;
        }
        set.insert(request_id.to_string());
        InflightAdmission::Admitted
    }

    async fn mark_started(&self, session_id: &str, request_id: &str, started: std::time::Instant) {
        let mut starts = self.starts.lock().await;
        starts
            .entry(session_id.to_string())
            .or_insert_with(HashMap::new)
            .insert(request_id.to_string(), started);
    }

    async fn complete_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<std::time::Duration> {
        {
            let mut inflight = self.inflight.lock().await;
            if let Some(set) = inflight.get_mut(session_id) {
                set.remove(request_id);
                if set.is_empty() {
                    inflight.remove(session_id);
                }
            }
        }

        let mut starts = self.starts.lock().await;
        if let Some(map) = starts.get_mut(session_id) {
            let out = map.remove(request_id);
            if map.is_empty() {
                starts.remove(session_id);
            }
            return out.map(|started| started.elapsed());
        }
        None
    }

    async fn remove_request(
        &self,
        session_id: &str,
        request_id: &str,
    ) -> Option<std::time::Duration> {
        self.complete_request(session_id, request_id).await
    }

    async fn clear_session(&self, session_id: &str) {
        self.inflight.lock().await.remove(session_id);
        self.starts.lock().await.remove(session_id);
    }
}

#[async_trait]
pub trait TerminalDedupeStore: Send + Sync {
    async fn try_mark_terminal(&self, session_id: &str, request_id: &str) -> bool;
}

pub type TerminalDedupeStoreRef = Arc<dyn TerminalDedupeStore>;

#[derive(Default)]
pub struct InMemoryTerminalDedupeStore {
    seen: DashMap<String, ()>,
}

impl InMemoryTerminalDedupeStore {
    pub fn new() -> Self {
        Self {
            seen: DashMap::new(),
        }
    }
}

#[async_trait]
impl TerminalDedupeStore for InMemoryTerminalDedupeStore {
    async fn try_mark_terminal(&self, session_id: &str, request_id: &str) -> bool {
        let key = format!("{}:{}", session_id, request_id);
        self.seen.insert(key, ()).is_none()
    }
}

static TERMINAL_DEDUPE_STORE: Lazy<TerminalDedupeStoreRef> = Lazy::new(build_terminal_dedupe_store);

pub fn terminal_dedupe_store() -> TerminalDedupeStoreRef {
    TERMINAL_DEDUPE_STORE.clone()
}

#[derive(Default)]
pub struct InMemoryPendingQuestionStore {
    inner: Mutex<HashMap<String, crate::tools::question::QuestionPayload>>,
}

impl InMemoryPendingQuestionStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl PendingQuestionStore for InMemoryPendingQuestionStore {
    async fn get(
        &self,
        session_id: &str,
    ) -> Option<crate::tools::question::QuestionPayload> {
        let map = self.inner.lock().await;
        map.get(session_id).cloned()
    }

    async fn insert(&self, session_id: String, payload: crate::tools::question::QuestionPayload) {
        let mut map = self.inner.lock().await;
        map.insert(session_id, payload);
    }

    async fn remove(&self, session_id: &str) {
        let mut map = self.inner.lock().await;
        map.remove(session_id);
    }

    async fn is_empty(&self) -> bool {
        let map = self.inner.lock().await;
        map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(feature = "distributed-redis")]
    fn ci_env_enabled() -> bool {
        std::env::var("CI")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    }

    #[cfg(feature = "distributed-redis")]
    async fn redis_test_url() -> Option<String> {
        let Some(url) = std::env::var("NANOBOT_REDIS_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for redis soak tests");
            }
            return None;
        };

        let client = match redis::Client::open(url.clone()) {
            Ok(c) => c,
            Err(err) => {
                if ci_env_enabled() {
                    panic!("CI requires valid NANOBOT_REDIS_URL for redis soak tests: {}", err);
                }
                eprintln!("skipping redis soak test: invalid NANOBOT_REDIS_URL: {}", err);
                return None;
            }
        };

        let mut conn = match tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.get_multiplexed_async_connection(),
        )
        .await
        {
            Ok(Ok(c)) => c,
            Ok(Err(err)) => {
                if ci_env_enabled() {
                    panic!("CI requires reachable NANOBOT_REDIS_URL for redis soak tests: {}", err);
                }
                eprintln!("skipping redis soak test: connect failed: {}", err);
                return None;
            }
            Err(_) => {
                if ci_env_enabled() {
                    panic!("CI requires reachable NANOBOT_REDIS_URL for redis soak tests: connect timed out");
                }
                eprintln!("skipping redis soak test: connect timed out");
                return None;
            }
        };

        let ping: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        match ping {
            Ok(_) => Some(url),
            Err(err) => {
                if ci_env_enabled() {
                    panic!("CI requires reachable NANOBOT_REDIS_URL for redis soak tests: {}", err);
                }
                eprintln!("skipping redis soak test: ping failed: {}", err);
                None
            }
        }
    }

    #[test]
    fn parse_distributed_backend_aliases() {
        assert_eq!(
            parse_distributed_store_backend("in_memory"),
            Some(DistributedStoreBackend::InMemory)
        );
        assert_eq!(
            parse_distributed_store_backend("in-memory"),
            Some(DistributedStoreBackend::InMemory)
        );
        assert_eq!(
            parse_distributed_store_backend("redis"),
            Some(DistributedStoreBackend::Redis)
        );
        assert_eq!(parse_distributed_store_backend("unknown"), None);
    }

    #[test]
    fn parse_admission_mode_aliases() {
        assert_eq!(parse_admission_mode("local"), Some(AdmissionMode::Local));
        assert_eq!(parse_admission_mode("global"), Some(AdmissionMode::Global));
        assert_eq!(parse_admission_mode("unknown"), None);
    }

    #[test]
    fn parse_scaling_mode_aliases() {
        assert_eq!(parse_scaling_mode("sticky"), Some(ScalingMode::Sticky));
        assert_eq!(parse_scaling_mode("stateless"), Some(ScalingMode::Stateless));
        assert_eq!(parse_scaling_mode("unknown"), None);
    }

    #[test]
    fn parse_provider_limiter_modes() {
        assert_eq!(
            parse_provider_limiter_backend("local"),
            Some(ProviderLimiterBackend::Local)
        );
        assert_eq!(
            parse_provider_limiter_backend("redis"),
            Some(ProviderLimiterBackend::Redis)
        );
        assert_eq!(
            parse_provider_limiter_failure_mode("open"),
            Some(ProviderLimiterFailureMode::Open)
        );
        assert_eq!(
            parse_provider_limiter_failure_mode("closed"),
            Some(ProviderLimiterFailureMode::Closed)
        );
    }

    #[test]
    fn selected_distributed_backend_defaults_to_in_memory() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
        }
        assert_eq!(
            selected_distributed_store_backend(),
            DistributedStoreBackend::InMemory
        );
    }

    #[test]
    fn selected_distributed_backend_reads_env_value() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
        }
        assert_eq!(
            selected_distributed_store_backend(),
            DistributedStoreBackend::Redis
        );
        unsafe {
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
        }
    }

    #[test]
    fn selected_admission_mode_defaults_to_local() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_ADMISSION_MODE");
        }
        assert_eq!(selected_admission_mode(), AdmissionMode::Local);
    }

    #[test]
    fn selected_admission_mode_reads_env_value() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ADMISSION_MODE", "global");
        }
        assert_eq!(selected_admission_mode(), AdmissionMode::Global);
        unsafe {
            std::env::remove_var("NANOBOT_ADMISSION_MODE");
        }
    }

    #[test]
    fn selected_scaling_mode_defaults_to_sticky() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
        }
        assert_eq!(selected_scaling_mode(), ScalingMode::Sticky);
    }

    #[test]
    fn selected_scaling_mode_reads_env_value() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_SCALING_MODE", "stateless");
        }
        assert_eq!(selected_scaling_mode(), ScalingMode::Stateless);
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
        }
    }

    #[test]
    fn strict_mode_rejects_unimplemented_global_admission() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ADMISSION_MODE", "global");
            std::env::set_var("NANOBOT_ADMISSION_STRICT", "1");
        }
        let result = enforce_admission_mode_support();
        assert!(
            result.is_err(),
            "strict mode should reject unimplemented global admission mode"
        );
        unsafe {
            std::env::remove_var("NANOBOT_ADMISSION_MODE");
            std::env::remove_var("NANOBOT_ADMISSION_STRICT");
        }
    }

    #[test]
    fn strict_mode_rejects_unimplemented_stateless_scaling() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_SCALING_MODE", "stateless");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
        }
        let result = enforce_scaling_mode_support();
        assert!(
            result.is_err(),
            "strict mode should reject unimplemented stateless scaling"
        );
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
        }
    }

    #[test]
    fn strict_mode_rejects_missing_sticky_header_in_production() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
        }
        let result = enforce_scaling_mode_support();
        assert!(
            result.is_err(),
            "strict sticky mode in production should require sticky header configuration"
        );
        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
        }
    }

    #[test]
    fn strict_mode_rejects_missing_replica_count_in_production() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
        let result = enforce_scaling_mode_support();
        assert!(
            result.is_err(),
            "strict sticky mode in production should require explicit replica count"
        );
        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
        }
    }

    #[test]
    fn strict_mode_rejects_missing_replica_count_outside_production() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
        let result = enforce_scaling_mode_support();
        assert!(
            result.is_err(),
            "strict sticky mode should require explicit replica count in all environments"
        );
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
        }
    }

    #[test]
    fn sticky_mode_with_header_passes_in_production() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "1");
        }
        let result = enforce_scaling_mode_support();
        assert!(result.is_ok(), "sticky mode should pass when sticky header is configured");
        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[test]
    fn strict_mode_rejects_multi_replica_without_global_limiter() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
        }

        let result = enforce_scaling_mode_support();
        assert!(
            result.is_err(),
            "strict sticky mode with multi-replica should require global provider limiter"
        );

        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[test]
    fn strict_mode_rejects_multi_replica_without_global_limiter_outside_production() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
        }

        let result = enforce_scaling_mode_support();
        assert!(
            result.is_err(),
            "strict sticky mode with multi-replica should require global provider limiter in all environments"
        );

        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[test]
    fn strict_mode_allows_multi_replica_with_global_limiter() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "10");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::set_var("NANOBOT_REDIS_URL", "redis://127.0.0.1:6379/");
        }

        let result = enforce_scaling_mode_support();
        if cfg!(feature = "distributed-redis") {
            assert!(
                result.is_ok(),
                "strict sticky mode should pass with global provider limiter configured"
            );
        } else {
            assert!(
                result.is_err(),
                "multi-replica sticky must fail when distributed-redis feature is absent"
            );
        }

        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_REDIS_URL");
        }
    }

    #[test]
    fn strict_mode_rejects_multi_replica_with_local_limiter_backend() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "10");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "local");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
        }

        let result = enforce_scaling_mode_support();
        assert!(result.is_err(), "strict mode should reject local limiter backend for multi-replica sticky");

        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
        }
    }

    #[test]
    fn strict_mode_rejects_multi_replica_with_open_failure_mode() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_ENV", "production");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_SCALING_STRICT", "1");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "10");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "open");
        }

        let result = enforce_scaling_mode_support();
        assert!(result.is_err(), "strict mode should reject open limiter failure mode for multi-replica sticky");

        unsafe {
            std::env::remove_var("NANOBOT_ENV");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
        }
    }

    #[test]
    fn multi_replica_rejects_missing_redis_url_without_strict_mode() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "10");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::remove_var("NANOBOT_REDIS_URL");
        }

        let result = enforce_scaling_mode_support();
        assert!(result.is_err(), "multi-replica should require redis url without strict mode");

        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
        }
    }

    #[test]
    fn multi_replica_rejects_missing_sticky_header_without_strict_mode() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "10");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::set_var("NANOBOT_REDIS_URL", "redis://127.0.0.1:6379/");
        }

        let result = enforce_scaling_mode_support();
        assert!(result.is_err(), "multi-replica should require sticky header without strict mode");

        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_REDIS_URL");
        }
    }

    #[test]
    fn multi_replica_rejects_open_failure_mode_without_strict_mode() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_STICKY_SIGNAL_HEADER", "x-session-affinity");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "10");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "open");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::set_var("NANOBOT_REDIS_URL", "redis://127.0.0.1:6379/");
        }

        let result = enforce_scaling_mode_support();
        assert!(result.is_err(), "multi-replica should reject open limiter failure mode without strict mode");

        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_REDIS_URL");
        }
    }

    #[test]
    fn single_replica_allows_missing_redis_url() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::set_var("NANOBOT_SCALING_MODE", "sticky");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "1");
            std::env::remove_var("NANOBOT_REDIS_URL");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
        }

        let result = enforce_scaling_mode_support();
        assert!(result.is_ok(), "single-replica should allow missing redis url");

        unsafe {
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[test]
    fn multi_replica_aggregates_all_missing_prereqs() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_SCALING_STRICT");
            std::env::remove_var("NANOBOT_SCALING_MODE");
            std::env::remove_var("NANOBOT_STICKY_SIGNAL_HEADER");
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_REDIS_URL");
        }

        let err = enforce_scaling_mode_support().expect_err("multi-replica should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("FATAL: multi-replica configuration invalid"),
            "should include fatal header"
        );
        assert!(
            msg.contains("[FAIL] NANOBOT_STICKY_SIGNAL_HEADER present"),
            "should include sticky header failure"
        );
        assert!(
            msg.contains("[FAIL] global provider limiter enabled"),
            "should include limiter failure"
        );
        assert!(
            msg.contains("[FAIL] NANOBOT_REDIS_URL present"),
            "should include redis url failure"
        );
        assert!(
            msg.contains("distributed-redis feature compiled"),
            "should include feature/binary requirement"
        );

        unsafe {
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[cfg(not(feature = "distributed-redis"))]
    #[tokio::test]
    async fn multi_replica_runtime_readiness_requires_distributed_redis_feature() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
        }

        let err = enforce_multi_replica_runtime_readiness()
            .await
            .expect_err("multi-replica should fail without distributed-redis feature");
        assert!(
            err.to_string()
                .contains("multi-replica requires distributed-redis feature"),
            "missing explicit feature-required startup error"
        );

        unsafe {
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn multi_replica_runtime_readiness_fails_when_redis_unconfigured() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
            std::env::remove_var("NANOBOT_REDIS_URL");
            std::env::set_var("NANOBOT_REDIS_STARTUP_TIMEOUT_MS", "50");
        }

        let err = enforce_multi_replica_runtime_readiness()
            .await
            .expect_err("multi-replica should fail when redis is unconfigured");
        assert!(
            err.to_string().contains("redis reachability check failed"),
            "missing reachability failure message"
        );

        unsafe {
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
            std::env::remove_var("NANOBOT_REDIS_STARTUP_TIMEOUT_MS");
        }
    }

    #[test]
    fn terminal_dedupe_fail_closed_disabled_single_replica() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
        assert!(
            !terminal_dedupe_fail_closed(),
            "single replica should keep dedupe fail-open behavior"
        );
    }

    #[test]
    fn terminal_dedupe_fail_closed_enabled_multi_replica() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_REPLICA_COUNT", "2");
        }
        assert!(
            terminal_dedupe_fail_closed(),
            "multi replica should force dedupe fail-closed behavior"
        );
        unsafe {
            std::env::remove_var("NANOBOT_REPLICA_COUNT");
        }
    }

    #[cfg(not(feature = "distributed-redis"))]
    #[test]
    fn strict_mode_rejects_unimplemented_redis_backend() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_STRICT", "1");
        }

        let result = enforce_distributed_backend_support();
        assert!(result.is_err(), "strict mode should reject unimplemented redis backend");

        unsafe {
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_STRICT");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[test]
    fn strict_mode_allows_redis_backend_when_feature_enabled() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_STRICT", "1");
        }

        let result = enforce_distributed_backend_support();
        assert!(result.is_ok(), "strict mode should allow redis backend when feature is enabled");

        unsafe {
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_STRICT");
        }
    }

    #[tokio::test]
    async fn in_memory_pending_question_store_roundtrip() {
        let store = InMemoryPendingQuestionStore::new();
        let payload = crate::tools::question::QuestionPayload {
            header: "Mode".to_string(),
            question: "Choose one".to_string(),
            options: vec!["A".to_string(), "B".to_string()],
            multiple: false,
        };

        assert!(store.is_empty().await);
        store.insert("s1".to_string(), payload.clone()).await;
        assert!(!store.is_empty().await);

        let got = store.get("s1").await.expect("payload should exist");
        assert_eq!(got.header, payload.header);
        assert_eq!(got.question, payload.question);

        store.remove("s1").await;
        assert!(store.get("s1").await.is_none());
        assert!(store.is_empty().await);
    }

    #[tokio::test]
    async fn in_memory_session_correlation_store_roundtrip() {
        let store = InMemorySessionCorrelationStore::new();
        assert_eq!(store.inflight_count("s1").await, 0);

        store.register_inflight("s1", "r1").await;
        store.mark_started("s1", "r1", std::time::Instant::now()).await;
        assert_eq!(store.inflight_count("s1").await, 1);

        let started = store.complete_request("s1", "r1").await;
        assert!(started.is_some());
        assert_eq!(store.inflight_count("s1").await, 0);

        store.register_inflight("s1", "r2").await;
        store.mark_started("s1", "r2", std::time::Instant::now()).await;
        store.clear_session("s1").await;
        assert_eq!(store.inflight_count("s1").await, 0);
    }

    #[tokio::test]
    async fn in_memory_try_register_inflight_enforces_cap_under_contention() {
        let store = Arc::new(InMemorySessionCorrelationStore::new());
        let mut tasks = Vec::new();
        for i in 0..32usize {
            let s = store.clone();
            tasks.push(tokio::spawn(async move {
                s.try_register_inflight("s-cap", &format!("r-{i}"), 1).await
            }));
        }

        let mut admitted = 0usize;
        let mut over_limit = 0usize;
        let mut duplicate = 0usize;
        let mut backend = 0usize;
        for t in tasks {
            match t.await.expect("join") {
                InflightAdmission::Admitted => admitted += 1,
                InflightAdmission::OverLimit => over_limit += 1,
                InflightAdmission::Duplicate => duplicate += 1,
                InflightAdmission::BackendError => backend += 1,
            }
        }

        assert_eq!(admitted, 1, "exactly one request should be admitted");
        assert_eq!(over_limit, 31, "remaining requests should be over-limit");
        assert_eq!(duplicate, 0, "unique ids should not be marked duplicate");
        assert_eq!(backend, 0, "in-memory backend should not error");
        assert_eq!(store.inflight_count("s-cap").await, 1);
    }

    #[tokio::test]
    async fn in_memory_try_register_inflight_rejects_duplicate_request_id() {
        let store = Arc::new(InMemorySessionCorrelationStore::new());
        let mut tasks = Vec::new();
        for _ in 0..2 {
            let s = store.clone();
            tasks.push(tokio::spawn(async move {
                s.try_register_inflight("s-dup", "same-request", 8).await
            }));
        }

        let mut admitted = 0usize;
        let mut duplicate = 0usize;
        for t in tasks {
            match t.await.expect("join") {
                InflightAdmission::Admitted => admitted += 1,
                InflightAdmission::Duplicate => duplicate += 1,
                other => panic!("unexpected admission outcome: {other:?}"),
            }
        }

        assert_eq!(admitted, 1, "first duplicate contender should be admitted");
        assert_eq!(duplicate, 1, "second duplicate contender should be rejected");
    }

    #[tokio::test]
    async fn in_memory_terminal_dedupe_store_marks_once() {
        let store = InMemoryTerminalDedupeStore::new();
        assert!(store.try_mark_terminal("s1", "r1").await);
        assert!(!store.try_mark_terminal("s1", "r1").await);
    }

    #[tokio::test]
    async fn local_provider_limiter_denies_after_limit() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "1");
            std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "local");
        }

        {
            let mut map = LOCAL_PROVIDER_LIMITER_STATE.lock().await;
            map.clear();
        }

        let first = allow_provider_request("openai").await;
        let second = allow_provider_request("openai").await;
        assert!(first, "first request should be allowed");
        assert!(!second, "second request in same second should be denied");

        unsafe {
            std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    #[ignore = "nightly soak"]
    async fn redis_store_soak_stability_nightly() {
        use redis::AsyncCommands;

        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for redis soak tests");
            }
            eprintln!("skipping redis soak test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        let duration_secs = std::env::var("NANOBOT_REDIS_SOAK_DURATION_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(30);
        let session_pool = std::env::var("NANOBOT_REDIS_SOAK_SESSION_POOL")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(32);
        let unique_prefix = format!(
            "nanobot-soak-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let terminal_dedupe_ttl_secs = 5usize;

        unsafe {
            std::env::set_var("NANOBOT_REDIS_URL", redis_url);
            std::env::set_var("NANOBOT_REDIS_KEY_PREFIX", &unique_prefix);
            std::env::set_var("NANOBOT_PENDING_QUESTION_TTL_SECS", "5");
            std::env::set_var(
                "NANOBOT_TERMINAL_DEDUPE_TTL_SECS",
                terminal_dedupe_ttl_secs.to_string(),
            );
        }

        let pending = RedisPendingQuestionStore::from_env().expect("redis pending store init");
        let correlation =
            RedisSessionCorrelationStore::from_env().expect("redis correlation store init");
        let dedupe = RedisTerminalDedupeStore::from_env().expect("redis terminal dedupe init");

        let sampling_client = redis::Client::open(
            std::env::var("NANOBOT_REDIS_URL").expect("redis url should be present"),
        )
        .expect("redis sampling client should initialize");
        let mut sampling_conn = sampling_client
            .get_multiplexed_async_connection()
            .await
            .expect("redis should connect for soak sampling");
        let pending_index_key = format!("{}:pending_questions:index", unique_prefix);
        let corr_pattern = format!("{}:corr:*", unique_prefix);
        let dedupe_pattern = format!("{}:terminal_dedupe:*", unique_prefix);

        let mut pending_index_peak: usize = 0;
        let mut correlation_key_peak: usize = 0;
        let mut terminal_dedupe_key_peak: usize = 0;

        let start_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let started = std::time::Instant::now();
        let loop_deadline = started + std::time::Duration::from_secs(duration_secs);
        let mut iterations: u64 = 0;

        while std::time::Instant::now() < loop_deadline {
            let session_id = format!("session-{}", iterations % session_pool);
            let request_id = format!("request-{}", iterations);

            let payload = crate::tools::question::QuestionPayload {
                header: "Soak".to_string(),
                question: "Continue?".to_string(),
                options: vec!["yes".to_string()],
                multiple: false,
            };
            pending.insert(session_id.clone(), payload).await;
            if iterations % 2 == 0 {
                pending.remove(&session_id).await;
            }

            correlation.register_inflight(&session_id, &request_id).await;
            correlation
                .mark_started(&session_id, &request_id, std::time::Instant::now())
                .await;
            let _ = correlation.complete_request(&session_id, &request_id).await;

            let _ = dedupe.try_mark_terminal(&session_id, &request_id).await;

            iterations += 1;
            if iterations % 250 == 0 {
                let pending_index_sample: usize = sampling_conn
                    .scard(&pending_index_key)
                    .await
                    .expect("pending index sample should succeed");
                pending_index_peak = pending_index_peak.max(pending_index_sample);

                let corr_sample: Vec<String> = redis::cmd("KEYS")
                    .arg(&corr_pattern)
                    .query_async(&mut sampling_conn)
                    .await
                    .expect("correlation key sample should succeed");
                correlation_key_peak = correlation_key_peak.max(corr_sample.len());

                let dedupe_sample: Vec<String> = redis::cmd("KEYS")
                    .arg(&dedupe_pattern)
                    .query_async(&mut sampling_conn)
                    .await
                    .expect("terminal dedupe key sample should succeed");
                terminal_dedupe_key_peak = terminal_dedupe_key_peak.max(dedupe_sample.len());
            }
            if iterations % 200 == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        }

        for idx in 0..session_pool {
            let sid = format!("session-{}", idx);
            pending.remove(&sid).await;
            correlation.clear_session(&sid).await;
        }

        tokio::time::sleep(std::time::Duration::from_secs(terminal_dedupe_ttl_secs as u64 + 2)).await;

        let client = redis::Client::open(
            std::env::var("NANOBOT_REDIS_URL").expect("redis url should be present"),
        )
        .expect("redis client should initialize");
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .expect("redis should connect for verification");

        let pending_items_pattern = format!("{}:pending_questions:*", unique_prefix);
        let pending_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&pending_items_pattern)
            .query_async(&mut conn)
            .await
            .expect("pending keys query should succeed");

        let corr_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&corr_pattern)
            .query_async(&mut conn)
            .await
            .expect("correlation keys query should succeed");

        let dedupe_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&dedupe_pattern)
            .query_async(&mut conn)
            .await
            .expect("terminal dedupe keys query should succeed");

        let pending_index_count: usize = conn
            .scard(&pending_index_key)
            .await
            .expect("pending index scard should succeed");

        pending_index_peak = pending_index_peak.max(pending_index_count);
        correlation_key_peak = correlation_key_peak.max(corr_keys.len());
        terminal_dedupe_key_peak = terminal_dedupe_key_peak.max(dedupe_keys.len());

        assert!(iterations > 0, "soak run should perform at least one iteration");
        assert!(
            pending_keys.len() <= 1,
            "pending-question keys should be cleaned up (index key may remain)"
        );
        assert_eq!(
            pending_index_count, 0,
            "pending-question index should be empty after soak cleanup"
        );
        assert!(corr_keys.is_empty(), "correlation keys should be empty after cleanup");
        assert!(
            dedupe_keys.is_empty(),
            "terminal dedupe keys should expire after ttl window"
        );

        let elapsed = started.elapsed();
        let end_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let ops_per_sec = if elapsed.as_secs_f64() > 0.0 {
            iterations as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        let summary = serde_json::json!({
            "schema": 1,
            "scenario": "redis_store_soak_stability",
            "status": "pass",
            "start_epoch_ms": start_epoch_ms,
            "end_epoch_ms": end_epoch_ms,
            "duration_secs": duration_secs,
            "elapsed_ms": elapsed.as_millis() as u64,
            "iterations": iterations,
            "session_pool": session_pool,
            "ops_per_sec": ops_per_sec,
            "pending_key_count": pending_keys.len(),
            "pending_index_count": pending_index_count,
            "pending_index_peak": pending_index_peak,
            "correlation_key_count": corr_keys.len(),
            "correlation_key_peak": correlation_key_peak,
            "terminal_dedupe_key_count": dedupe_keys.len(),
            "terminal_dedupe_key_peak": terminal_dedupe_key_peak,
            "terminal_dedupe_ttl_secs": terminal_dedupe_ttl_secs,
        });
        eprintln!("SOAK_SUMMARY {}", summary);
        eprintln!(
            "SOAK_VERDICT schema=1 ok=1 reasons=[] peaks={{\"pending_index_peak\":{},\"correlation_key_peak\":{},\"terminal_dedupe_key_peak\":{}}}",
            pending_index_peak,
            correlation_key_peak,
            terminal_dedupe_key_peak,
        );

        let cleanup_pattern = format!("{}:*", unique_prefix);
        let cleanup_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&cleanup_pattern)
            .query_async(&mut conn)
            .await
            .expect("cleanup key query should succeed");
        if !cleanup_keys.is_empty() {
            let _: () = redis::cmd("DEL")
                .arg(cleanup_keys)
                .query_async(&mut conn)
                .await
                .expect("cleanup delete should succeed");
        }

        unsafe {
            std::env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            std::env::remove_var("NANOBOT_PENDING_QUESTION_TTL_SECS");
            std::env::remove_var("NANOBOT_TERMINAL_DEDUPE_TTL_SECS");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    #[ignore = "ci smoke"]
    async fn redis_ttl_drain_smoke_ci() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for redis ttl drain smoke test");
            }
            eprintln!("skipping redis ttl drain smoke test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        let unique_prefix = format!(
            "ttl_smoke_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        unsafe {
            std::env::set_var("NANOBOT_REDIS_URL", &redis_url);
            std::env::set_var("NANOBOT_REDIS_KEY_PREFIX", &unique_prefix);
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
            std::env::set_var("NANOBOT_DISTRIBUTED_STORE_STRICT", "1");
            std::env::set_var("NANOBOT_TEST_REDIS_STORE_TTL_SECS", "3");
        }

        let pending = RedisPendingQuestionStore::from_env().expect("redis pending store init");
        let correlation =
            RedisSessionCorrelationStore::from_env().expect("redis correlation store init");
        let dedupe = RedisTerminalDedupeStore::from_env().expect("redis dedupe store init");

        for i in 0..160usize {
            let sid = format!("smoke-session-{}", i % 24);
            let rid = format!("smoke-request-{}", i);
            pending
                .insert(
                    sid.clone(),
                    crate::tools::question::QuestionPayload {
                        header: "Smoke".to_string(),
                        question: "Proceed?".to_string(),
                        options: vec!["yes".to_string()],
                        multiple: false,
                    },
                )
                .await;
            correlation.register_inflight(&sid, &rid).await;
            correlation
                .mark_started(&sid, &rid, std::time::Instant::now())
                .await;
            let _ = dedupe.try_mark_terminal(&sid, &rid).await;
        }

        let before = redis_debug_counts_for_prefix(&redis_url, &unique_prefix)
            .await
            .expect("debug counts before drain should work");
        assert!(
            before.pending_index_count > 0,
            "pending index count should be > 0 before ttl drain"
        );
        assert!(
            before.correlation_key_count > 0,
            "correlation key count should be > 0 before ttl drain"
        );
        assert!(
            before.terminal_dedupe_key_count > 0,
            "terminal dedupe key count should be > 0 before ttl drain"
        );

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);
        let final_counts = loop {
            let counts = redis_debug_counts_for_prefix(&redis_url, &unique_prefix)
                .await
                .expect("debug counts during drain should work");
            if counts.pending_index_count == 0
                && counts.correlation_key_count == 0
                && counts.terminal_dedupe_key_count == 0
            {
                break counts;
            }
            if start.elapsed() >= timeout {
                break counts;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        };

        assert_eq!(
            final_counts.pending_index_count, 0,
            "pending index should drain to zero"
        );
        assert_eq!(
            final_counts.correlation_key_count, 0,
            "correlation keys should drain to zero"
        );
        assert_eq!(
            final_counts.terminal_dedupe_key_count, 0,
            "terminal dedupe keys should drain to zero"
        );

        unsafe {
            std::env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
            std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_STRICT");
            std::env::remove_var("NANOBOT_TEST_REDIS_STORE_TTL_SECS");
        }
    }
}
