use crate::antigravity::AntigravityClient;
use crate::context::ContextTree;
use crate::events::AgentEvent;
use anyhow::Result;
use futures::StreamExt;
use rig::OneOrMany;
use rig::client::CompletionClient;
use rig::completion::message::{AssistantContent, Text, UserContent};
use rig::completion::{CompletionModel, CompletionRequest, Document, Message};
use rig::streaming::StreamedAssistantContent;
use serde_json::{Value, json};
use tokio::sync::{Notify, Semaphore, mpsc};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub mod personality;
pub mod supervisor;
use std::path::PathBuf;

static PERSISTENCE_BLOCKING_SEMAPHORE: once_cell::sync::Lazy<Semaphore> =
    once_cell::sync::Lazy::new(|| {
        Semaphore::new(persistence_blocking_limit())
    });

static LLM_TASK_SEMAPHORE: once_cell::sync::Lazy<Semaphore> = once_cell::sync::Lazy::new(|| {
    Semaphore::new(llm_task_concurrency_limit())
});

static LLM_PERMIT_NOTIFY: once_cell::sync::Lazy<Notify> = once_cell::sync::Lazy::new(Notify::new);
static LLM_ADMISSION_QUEUE_NOTIFY: once_cell::sync::Lazy<Notify> =
    once_cell::sync::Lazy::new(Notify::new);
static LLM_ADMISSION_NEXT_ID: once_cell::sync::Lazy<AtomicU64> =
    once_cell::sync::Lazy::new(|| AtomicU64::new(1));
static LLM_ADMISSION_QUEUE: once_cell::sync::Lazy<tokio::sync::Mutex<std::collections::VecDeque<u64>>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::VecDeque::new()));
static ADMISSION_GLOBAL_FALLBACK_WARNED: once_cell::sync::Lazy<AtomicBool> =
    once_cell::sync::Lazy::new(|| AtomicBool::new(false));

static ADAPTIVE_LLM_PERMIT_LIMIT: once_cell::sync::Lazy<AtomicUsize> =
    once_cell::sync::Lazy::new(|| AtomicUsize::new(llm_task_concurrency_limit()));
static ADAPTIVE_LAST_UNHEALTHY_MS: once_cell::sync::Lazy<AtomicU64> =
    once_cell::sync::Lazy::new(|| AtomicU64::new(0));
static ADAPTIVE_LAST_STEP_UP_MS: once_cell::sync::Lazy<AtomicU64> =
    once_cell::sync::Lazy::new(|| AtomicU64::new(0));
static ADAPTIVE_PROVIDER_UNHEALTHY: once_cell::sync::Lazy<AtomicBool> =
    once_cell::sync::Lazy::new(|| AtomicBool::new(false));
static ADAPTIVE_RECOVERY_TASK_STARTED: once_cell::sync::Lazy<AtomicBool> =
    once_cell::sync::Lazy::new(|| AtomicBool::new(false));

#[cfg(test)]
static TEST_FORCE_SOFT_LIMIT_DROP_ON_ACQUIRE: once_cell::sync::Lazy<AtomicBool> =
    once_cell::sync::Lazy::new(|| AtomicBool::new(false));

fn selected_provider_from_env() -> Option<String> {
    std::env::var("NANOBOT_PROVIDER")
        .ok()
        .or_else(|| std::env::var("NANOBOT_DEFAULT_PROVIDER").ok())
        .map(|v| v.trim().to_ascii_lowercase())
}

fn selected_model_from_env() -> Option<String> {
    std::env::var("NANOBOT_MODEL")
        .ok()
        .or_else(|| std::env::var("NANOBOT_DEFAULT_MODEL").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn persistence_blocking_limit() -> usize {
    std::env::var("NANOBOT_PERSISTENCE_BLOCKING_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(32)
}

fn llm_task_concurrency_limit() -> usize {
    std::env::var("NANOBOT_LLM_CONCURRENCY_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or_else(|| {
            if selected_provider_from_env().as_deref() == Some("antigravity") {
                8
            } else {
                32
            }
        })
}

fn llm_upstream_connect_timeout() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_UPSTREAM_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(10000))
}

fn llm_stream_chunk_timeout() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_STREAM_CHUNK_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(10000))
}

fn llm_stream_total_timeout() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_STREAM_TOTAL_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(60000))
}

fn llm_unhealthy_ttl_auth() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_UNHEALTHY_TTL_AUTH_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(300000))
}

fn llm_unhealthy_ttl_rate_limit() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_UNHEALTHY_TTL_RATELIMIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(15000))
}

fn llm_unhealthy_ttl_timeout() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_UNHEALTHY_TTL_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(30000))
}

fn increment_llm_rejected(reason: &str) {
    crate::metrics::GLOBAL_METRICS.increment_counter(
        &format!("llm_rejected_total{{reason={}}}", reason),
        1,
    );
}

fn llm_queue_wait_timeout() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_QUEUE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| {
            if selected_provider_from_env().as_deref() == Some("antigravity") {
                std::time::Duration::from_millis(1000)
            } else {
                std::time::Duration::from_millis(5000)
            }
        })
}

fn llm_admission_queue_max() -> usize {
    std::env::var("NANOBOT_LLM_ADMISSION_QUEUE_MAX")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or_else(|| llm_task_concurrency_limit().saturating_mul(8).max(8))
}

fn adaptive_permits_enabled() -> bool {
    std::env::var("NANOBOT_LLM_ADAPTIVE_PERMITS")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or_else(|| selected_provider_from_env().as_deref() == Some("antigravity"))
}

fn adaptive_permit_floor() -> usize {
    std::env::var("NANOBOT_LLM_ADAPTIVE_MIN_PERMITS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or_else(|| {
            if selected_provider_from_env().as_deref() == Some("antigravity") {
                8
            } else {
                4
            }
        })
}

fn adaptive_permit_cooldown() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_ADAPTIVE_COOLDOWN_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(60000))
}

fn now_epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn current_adaptive_llm_limit() -> usize {
    let hard_limit = llm_task_concurrency_limit();
    if !adaptive_permits_enabled() {
        return hard_limit;
    }
    let floor = adaptive_permit_floor().min(hard_limit);
    ADAPTIVE_LLM_PERMIT_LIMIT
        .load(Ordering::Relaxed)
        .clamp(floor, hard_limit)
}

fn adaptive_recovery_tick() {
    if !adaptive_permits_enabled() {
        return;
    }
    if ADAPTIVE_PROVIDER_UNHEALTHY.load(Ordering::Relaxed) {
        return;
    }
    let hard_limit = llm_task_concurrency_limit();
    let mut current = ADAPTIVE_LLM_PERMIT_LIMIT.load(Ordering::Relaxed);
    if current >= hard_limit {
        return;
    }

    let now_ms = now_epoch_ms();
    let cooldown_ms = adaptive_permit_cooldown().as_millis() as u64;
    let last_unhealthy = ADAPTIVE_LAST_UNHEALTHY_MS.load(Ordering::Relaxed);
    let last_step_up = ADAPTIVE_LAST_STEP_UP_MS.load(Ordering::Relaxed);

    if now_ms.saturating_sub(last_unhealthy) < cooldown_ms
        || now_ms.saturating_sub(last_step_up) < cooldown_ms
    {
        return;
    }

    current += 1;
    if current > hard_limit {
        current = hard_limit;
    }
    ADAPTIVE_LLM_PERMIT_LIMIT.store(current, Ordering::Relaxed);
    ADAPTIVE_LAST_STEP_UP_MS.store(now_ms, Ordering::Relaxed);
    crate::metrics::GLOBAL_METRICS.increment_counter("llm_adaptive_step_up_total", 1);
    crate::metrics::GLOBAL_METRICS.increment_counter("llm_adaptive_recoveries_total", 1);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_adaptive_concurrency_limit", current as f64);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_permits_current", current as f64);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_permits_target", hard_limit as f64);
}

fn adaptive_step_down_on_unhealthy() {
    if !adaptive_permits_enabled() {
        return;
    }
    let now_ms = now_epoch_ms();
    ADAPTIVE_LAST_UNHEALTHY_MS.store(now_ms, Ordering::Relaxed);
    ADAPTIVE_LAST_STEP_UP_MS.store(now_ms, Ordering::Relaxed);

    let floor = adaptive_permit_floor().min(llm_task_concurrency_limit());
    let current = ADAPTIVE_LLM_PERMIT_LIMIT.load(Ordering::Relaxed);
    let halved = (current / 2).max(floor);
    if halved < current {
        ADAPTIVE_LLM_PERMIT_LIMIT.store(halved, Ordering::Relaxed);
        crate::metrics::GLOBAL_METRICS.increment_counter("llm_adaptive_step_down_total", 1);
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_adaptive_concurrency_limit", halved as f64);
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_permits_current", halved as f64);
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_permits_target", llm_task_concurrency_limit() as f64);
        tracing::warn!(
            current_limit = current,
            new_limit = halved,
            floor = floor,
            "Adaptive LLM permit step-down applied"
        );
    }
}

fn spawn_adaptive_permit_recovery_task() {
    if !adaptive_permits_enabled() {
        return;
    }

    if ADAPTIVE_RECOVERY_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(adaptive_permit_cooldown());
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            adaptive_recovery_tick();
        }
    });
}

fn llm_queue_wait_epsilon() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_QUEUE_WAIT_EPSILON_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(100))
}

fn llm_queue_poll_interval() -> std::time::Duration {
    std::env::var("NANOBOT_LLM_QUEUE_POLL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(10))
}

fn debug_reject_timings_enabled() -> bool {
    std::env::var("NANOBOT_DEBUG_REJECT_TIMINGS")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn debug_reject_sample_pct() -> u64 {
    std::env::var("NANOBOT_DEBUG_REJECT_SAMPLE_PCT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(1, 100))
        .unwrap_or(1)
}

fn should_sample_reject_timing(request_id: &str) -> bool {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    request_id.hash(&mut hasher);
    let bucket = hasher.finish() % 100;
    bucket < debug_reject_sample_pct()
}

fn stream_flush_token_threshold() -> usize {
    std::env::var("NANOBOT_STREAM_FLUSH_TOKENS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(128)
}

fn stream_flush_interval() -> std::time::Duration {
    std::env::var("NANOBOT_STREAM_FLUSH_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| std::time::Duration::from_millis(750))
}

fn stream_flush_max_buffer_tokens() -> usize {
    std::env::var("NANOBOT_STREAM_MAX_BUFFER_TOKENS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1024)
}

fn update_llm_inflight_metric() {
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "llm_tasks_inflight",
        (llm_task_concurrency_limit().saturating_sub(LLM_TASK_SEMAPHORE.available_permits())) as f64,
    );
}

struct LlmTaskPermitGuard<'a> {
    _permit: tokio::sync::SemaphorePermit<'a>,
}

impl Drop for LlmTaskPermitGuard<'_> {
    fn drop(&mut self) {
        update_llm_inflight_metric();
        LLM_PERMIT_NOTIFY.notify_waiters();
    }
}

struct LlmServiceTimer {
    started: std::time::Instant,
    success: std::cell::Cell<bool>,
}

impl LlmServiceTimer {
    fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            success: std::cell::Cell::new(false),
        }
    }

    fn mark_success(&self) {
        self.success.set(true);
    }
}

impl Drop for LlmServiceTimer {
    fn drop(&mut self) {
        crate::metrics::GLOBAL_METRICS.record_duration(
            "llm_service_time_seconds",
            self.started.elapsed(),
            self.success.get(),
        );
    }
}

enum LlmPermitAcquireOutcome<'a> {
    Acquired(tokio::sync::SemaphorePermit<'a>, std::time::Duration),
    Closed,
    BudgetExceeded { waited: std::time::Duration },
    QueueOverCapacity { waited: std::time::Duration },
}

#[derive(Debug, Clone)]
enum ProviderHealthState {
    Healthy,
    Unhealthy {
        reason: &'static str,
        until: std::time::Instant,
    },
}

fn classify_provider_failure(err: &str) -> Option<(&'static str, std::time::Duration)> {
    let lower = err.to_ascii_lowercase();

    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("invalid key")
        || lower.contains("invalid token")
    {
        return Some(("provider_auth", llm_unhealthy_ttl_auth()));
    }

    if lower.contains("429") || lower.contains("quota") || lower.contains("rate limit") {
        return Some(("provider_rate_limited", llm_unhealthy_ttl_rate_limit()));
    }

    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("deadline")
        || lower.contains("connection reset")
    {
        return Some(("provider_timeout", llm_unhealthy_ttl_timeout()));
    }

    None
}

fn bench_verbose_upstream_enabled() -> bool {
    llm_bench_mode_enabled()
        && std::env::var("NANOBOT_BENCH_VERBOSE_UPSTREAM")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
}

fn extract_http_status_code(err: &str) -> Option<u16> {
    let bytes = err.as_bytes();
    for window in bytes.windows(3) {
        if window.iter().all(|b| b.is_ascii_digit()) {
            let code = ((window[0] - b'0') as u16) * 100
                + ((window[1] - b'0') as u16) * 10
                + ((window[2] - b'0') as u16);
            if (100..=599).contains(&code) {
                return Some(code);
            }
        }
    }
    None
}

fn classify_upstream_error_class(err: &str) -> &'static str {
    let lower = err.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") || lower.contains("deadline") {
        return "timeout";
    }
    if lower.contains("dns")
        || lower.contains("failed to lookup address")
        || lower.contains("name or service not known")
        || lower.contains("no such host")
    {
        return "dns";
    }
    if lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("connect")
    {
        return "connect_error";
    }
    if let Some(code) = extract_http_status_code(err) {
        return match code {
            401 => "401",
            403 => "403",
            429 => "429",
            500..=599 => "5xx",
            _ => "http_error",
        };
    }
    "unknown"
}

fn bench_provider_name_hint(default_provider: &str) -> String {
    selected_provider_from_env().unwrap_or_else(|| default_provider.to_ascii_lowercase())
}

fn bench_model_hint(provider: &str) -> &'static str {
    match provider {
        "antigravity" => "gemini-2.0-flash-exp",
        "openai" => "gpt-4o",
        "openrouter" => "google/gemini-2.0-flash-001",
        "google" => "gemini-2.0-flash",
        _ => "unknown",
    }
}

fn bench_upstream_suffix(default_provider: &str, err: &str, upstream_elapsed: std::time::Duration) -> String {
    let provider = bench_provider_name_hint(default_provider);
    let model = bench_model_hint(&provider);
    let class = classify_upstream_error_class(err);
    let status = extract_http_status_code(err)
        .map(|c| c.to_string())
        .unwrap_or_else(|| "none".to_string());
    format!(
        " [bench upstream_class={} provider={} model={} upstream_ms={} status={}]",
        class,
        provider,
        model,
        upstream_elapsed.as_millis(),
        status
    )
}

#[cfg(test)]
async fn acquire_llm_permit_with_timeout(
    timeout_duration: std::time::Duration,
) -> LlmPermitAcquireOutcome<'static> {
    let deadline = std::time::Instant::now() + timeout_duration;
    acquire_llm_permit_with_deadline(deadline, llm_queue_wait_epsilon()).await
}

fn record_queue_wait_over_budget(stage: &str, overshoot: std::time::Duration) {
    crate::metrics::GLOBAL_METRICS.increment_counter("llm_queue_wait_over_budget_total", 1);
    crate::metrics::GLOBAL_METRICS.increment_counter(
        &format!("llm_timeout_stage_total{{stage={}}}", stage),
        1,
    );
    crate::metrics::GLOBAL_METRICS.record_duration(
        "llm_queue_wait_overshoot_seconds",
        overshoot,
        true,
    );
    crate::metrics::GLOBAL_METRICS.record_duration(
        &format!("llm_queue_wait_overshoot_seconds{{stage={}}}", stage),
        overshoot,
        true,
    );
    record_duration_buckets("llm_queue_wait_overshoot_seconds", overshoot);
    record_duration_buckets(
        &format!("llm_queue_wait_overshoot_seconds{{stage={}}}", stage),
        overshoot,
    );
}

async fn admission_enqueue() -> Option<u64> {
    let id = LLM_ADMISSION_NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut q = LLM_ADMISSION_QUEUE.lock().await;
    let max_depth = llm_admission_queue_max();
    if q.len() >= max_depth {
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_admission_queue_depth", q.len() as f64);
        crate::metrics::GLOBAL_METRICS.increment_counter("llm_admission_drop_total", 1);
        increment_llm_rejected("queue_over_capacity");
        return None;
    }
    q.push_back(id);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_admission_queue_depth", q.len() as f64);
    Some(id)
}

async fn admission_release(id: u64) {
    let mut q = LLM_ADMISSION_QUEUE.lock().await;
    if let Some(pos) = q.iter().position(|queued| *queued == id) {
        q.remove(pos);
    }
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_admission_queue_depth", q.len() as f64);
    drop(q);
    LLM_ADMISSION_QUEUE_NOTIFY.notify_waiters();
}

async fn admission_is_turn(id: u64) -> bool {
    let q = LLM_ADMISSION_QUEUE.lock().await;
    q.front().copied() == Some(id)
}

async fn acquire_llm_permit_with_deadline_local(
    deadline: std::time::Instant,
    epsilon: std::time::Duration,
) -> LlmPermitAcquireOutcome<'static> {
    let now = std::time::Instant::now();
    if now >= deadline {
        let overshoot = now.saturating_duration_since(deadline);
        increment_llm_rejected("semaphore_timeout");
        record_queue_wait_over_budget("pre_acquire_deadline", overshoot);
        return LlmPermitAcquireOutcome::BudgetExceeded {
            waited: std::time::Duration::ZERO,
        };
    }

    let timeout_duration = deadline.saturating_duration_since(now);
    crate::metrics::GLOBAL_METRICS.set_gauge(
        "llm_timeout_remaining_ms_last",
        timeout_duration.as_millis() as f64,
    );
    record_duration_buckets("llm_timeout_remaining_seconds", timeout_duration);

    let admission_id = match admission_enqueue().await {
        Some(id) => id,
        None => {
            return LlmPermitAcquireOutcome::QueueOverCapacity {
                waited: std::time::Duration::ZERO,
            };
        }
    };
    let poll_interval = llm_queue_poll_interval();
    let wait_started = std::time::Instant::now();
    let hard_limit = llm_task_concurrency_limit();

    loop {
        let loop_now = std::time::Instant::now();
        if loop_now >= deadline {
            let waited = wait_started.elapsed();
            record_duration_buckets("llm_queue_wait_seconds", waited);
            increment_llm_rejected("semaphore_timeout");
            let overshoot = loop_now.saturating_duration_since(deadline);
            record_queue_wait_over_budget("deadline_expired", overshoot);
            admission_release(admission_id).await;
            return LlmPermitAcquireOutcome::BudgetExceeded { waited };
        }

        if !admission_is_turn(admission_id).await {
            let remaining = deadline.saturating_duration_since(loop_now);
            let sleep_for = std::cmp::min(poll_interval, remaining);
            tokio::select! {
                _ = LLM_ADMISSION_QUEUE_NOTIFY.notified() => {},
                _ = tokio::time::sleep(sleep_for) => {},
            }
            continue;
        }

        let soft_limit = current_adaptive_llm_limit().min(hard_limit);
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_adaptive_concurrency_limit", soft_limit as f64);
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_permits_current", soft_limit as f64);
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_permits_target", hard_limit as f64);
        let inflight = hard_limit.saturating_sub(LLM_TASK_SEMAPHORE.available_permits());

        if inflight >= soft_limit {
            let remaining = deadline.saturating_duration_since(loop_now);
            crate::metrics::GLOBAL_METRICS
                .set_gauge("llm_timeout_remaining_ms_last", remaining.as_millis() as f64);
            record_duration_buckets("llm_timeout_remaining_seconds", remaining);

            let sleep_for = std::cmp::min(poll_interval, remaining);
            tokio::select! {
                _ = LLM_PERMIT_NOTIFY.notified() => {},
                _ = tokio::time::sleep(sleep_for) => {},
            }
            continue;
        }

        #[cfg(test)]
        if TEST_FORCE_SOFT_LIMIT_DROP_ON_ACQUIRE.load(Ordering::Relaxed) {
            let forced_floor = adaptive_permit_floor().min(hard_limit);
            ADAPTIVE_LLM_PERMIT_LIMIT.store(forced_floor, Ordering::Relaxed);
        }

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let acquire_result = tokio::time::timeout(remaining, LLM_TASK_SEMAPHORE.acquire()).await;
        match acquire_result {
            Ok(Ok(permit)) => {
                let post_soft_limit = current_adaptive_llm_limit().min(hard_limit);
                let inflight_after_acquire =
                    hard_limit.saturating_sub(LLM_TASK_SEMAPHORE.available_permits());
                if inflight_after_acquire > post_soft_limit {
                    drop(permit);
                    LLM_PERMIT_NOTIFY.notify_waiters();
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    let sleep_for = std::cmp::min(poll_interval, remaining);
                    tokio::select! {
                        _ = LLM_PERMIT_NOTIFY.notified() => {},
                        _ = tokio::time::sleep(sleep_for) => {},
                    }
                    continue;
                }
                let waited = wait_started.elapsed();
                record_duration_buckets("llm_queue_wait_seconds", waited);
                let after_acquire = std::time::Instant::now();
                let overshoot = after_acquire.saturating_duration_since(deadline);
                if overshoot > epsilon {
                    drop(permit);
                    LLM_PERMIT_NOTIFY.notify_waiters();
                    increment_llm_rejected("semaphore_timeout");
                    record_queue_wait_over_budget("post_acquire_over_budget", overshoot);
                    admission_release(admission_id).await;
                    return LlmPermitAcquireOutcome::BudgetExceeded { waited };
                }
                admission_release(admission_id).await;
                return LlmPermitAcquireOutcome::Acquired(permit, waited);
            }
            Ok(Err(_)) => {
                admission_release(admission_id).await;
                return LlmPermitAcquireOutcome::Closed;
            }
            Err(_) => {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                crate::metrics::GLOBAL_METRICS
                    .set_gauge("llm_timeout_remaining_ms_last", remaining.as_millis() as f64);
                record_duration_buckets("llm_timeout_remaining_seconds", remaining);
                let waited = wait_started.elapsed();
                record_duration_buckets("llm_queue_wait_seconds", waited);
                increment_llm_rejected("semaphore_timeout");
                record_queue_wait_over_budget("semaphore_acquire_timeout", std::time::Duration::ZERO);
                admission_release(admission_id).await;
                return LlmPermitAcquireOutcome::BudgetExceeded { waited };
            }
        }
    }
}

async fn acquire_llm_permit_with_deadline(
    deadline: std::time::Instant,
    epsilon: std::time::Duration,
) -> LlmPermitAcquireOutcome<'static> {
    match crate::distributed::selected_admission_mode() {
        crate::distributed::AdmissionMode::Local => {
            acquire_llm_permit_with_deadline_local(deadline, epsilon).await
        }
        crate::distributed::AdmissionMode::Global => {
            if !ADMISSION_GLOBAL_FALLBACK_WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "Admission mode 'global' requested but not implemented; using local admission backend"
                );
            }
            crate::metrics::GLOBAL_METRICS.increment_counter(
                "admission_backend_fallback_total{reason=global_to_local}",
                1,
            );
            acquire_llm_permit_with_deadline_local(deadline, epsilon).await
        }
    }
}

#[cfg(test)]
fn read_counter_from_metrics(metric_name: &str) -> f64 {
    crate::metrics::GLOBAL_METRICS
        .export_prometheus()
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(metric_name) {
                trimmed
                    .split_whitespace()
                    .last()
                    .and_then(|v| v.parse::<f64>().ok())
            } else {
                None
            }
        })
        .unwrap_or(0.0)
}

// Define message types for internal communication
#[derive(Debug)]
pub enum StreamChunk {
    TextDelta(String),
    Thinking(String),
    ToolCall(String),
    ToolResult(String),
    Done {
        request_id: String,
        kind: TerminalKind,
    },
}

#[derive(Debug, Clone)]
pub enum TerminalKind {
    SuccessDone,
    ErrorDone { code: String, reason: String },
    CancelledDone { reason: String },
}

#[derive(Debug)]
pub struct AgentMessage {
    pub session_id: String,
    pub tenant_id: String, // Added for Multi-tenancy
    pub request_id: String,
    pub content: String,
    pub response_tx: mpsc::Sender<StreamChunk>,
    pub ingress_at: std::time::Instant,
}

use crate::config;
// AntigravityClient kept for initialization
use futures::stream::Stream;
use rig::providers::openai;
use std::pin::Pin;

static ACTIVE_STREAMING_HANDLERS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_STREAMING_HANDLERS_PEAK: AtomicUsize = AtomicUsize::new(0);
static LLM_IN_SERVICE: AtomicUsize = AtomicUsize::new(0);
static LLM_IN_SERVICE_PEAK: AtomicUsize = AtomicUsize::new(0);
static WS_SEND_INFLIGHT: AtomicUsize = AtomicUsize::new(0);
static WS_SEND_INFLIGHT_PEAK: AtomicUsize = AtomicUsize::new(0);

fn update_streaming_handler_metrics(current: usize) {
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_active_handlers", current as f64);
}

fn update_streaming_handler_peak(candidate: usize) {
    let mut observed = ACTIVE_STREAMING_HANDLERS_PEAK.load(Ordering::Relaxed);
    while candidate > observed {
        match ACTIVE_STREAMING_HANDLERS_PEAK.compare_exchange_weak(
            observed,
            candidate,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(v) => observed = v,
        }
    }
    let peak = ACTIVE_STREAMING_HANDLERS_PEAK.load(Ordering::Relaxed);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_active_handlers_peak", peak as f64);
}

fn update_llm_in_service_metrics(current: usize) {
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_in_service_current", current as f64);
}

fn update_llm_in_service_peak(candidate: usize) {
    let mut observed = LLM_IN_SERVICE_PEAK.load(Ordering::Relaxed);
    while candidate > observed {
        match LLM_IN_SERVICE_PEAK.compare_exchange_weak(
            observed,
            candidate,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(v) => observed = v,
        }
    }
    let peak = LLM_IN_SERVICE_PEAK.load(Ordering::Relaxed);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_in_service_peak", peak as f64);
}

fn update_ws_send_inflight_metrics(current: usize) {
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_ws_send_inflight", current as f64);
}

fn update_ws_send_inflight_peak(candidate: usize) {
    let mut observed = WS_SEND_INFLIGHT_PEAK.load(Ordering::Relaxed);
    while candidate > observed {
        match WS_SEND_INFLIGHT_PEAK.compare_exchange_weak(
            observed,
            candidate,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(v) => observed = v,
        }
    }
    let peak = WS_SEND_INFLIGHT_PEAK.load(Ordering::Relaxed);
    crate::metrics::GLOBAL_METRICS.set_gauge("llm_ws_send_inflight_peak", peak as f64);
}

fn record_duration_buckets(metric: &str, elapsed: std::time::Duration) {
    let secs = elapsed.as_secs_f64();
    let buckets = [0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];
    for le in buckets {
        if secs <= le {
            crate::metrics::GLOBAL_METRICS
                .increment_counter(&format!("{}_bucket{{le={}}}", metric, le), 1);
        }
    }
}

async fn send_stream_chunk_timed(
    tx: &mpsc::Sender<StreamChunk>,
    chunk: StreamChunk,
) -> Result<(), mpsc::error::SendError<StreamChunk>> {
    let current = WS_SEND_INFLIGHT.fetch_add(1, Ordering::Relaxed) + 1;
    update_ws_send_inflight_metrics(current);
    update_ws_send_inflight_peak(current);

    let started = std::time::Instant::now();
    let res = tx.send(chunk).await;
    let elapsed = started.elapsed();

    let after = WS_SEND_INFLIGHT
        .fetch_sub(1, Ordering::Relaxed)
        .saturating_sub(1);
    update_ws_send_inflight_metrics(after);

    crate::metrics::GLOBAL_METRICS.record_duration(
        "ws_send_wait_seconds",
        elapsed,
        res.is_ok(),
    );
    record_duration_buckets("ws_send_wait_seconds", elapsed);

    res
}

async fn emit_terminal(
    response_tx: &mpsc::Sender<StreamChunk>,
    session_id: &str,
    request_id: &str,
    kind: TerminalKind,
) {
    if !crate::distributed::terminal_dedupe_store()
        .try_mark_terminal(session_id, request_id)
        .await
    {
        crate::metrics::GLOBAL_METRICS.increment_counter("llm_terminal_duplicate_total", 1);
        tracing::error!(
            session_id = %session_id,
            request_id = %request_id,
            "Terminal invariant violation: duplicate terminal emission attempt"
        );
        return;
    }

    let _ = send_stream_chunk_timed(
        response_tx,
        StreamChunk::Done {
            request_id: request_id.to_string(),
            kind,
        },
    )
    .await;
}

async fn emit_error_and_done(
    response_tx: &mpsc::Sender<StreamChunk>,
    session_id: &str,
    request_id: &str,
    code: &str,
    err: &str,
) {
    let _ = send_stream_chunk_timed(
        response_tx,
        StreamChunk::TextDelta(format!("Error: {}", err)),
    )
    .await;
    emit_terminal(
        response_tx,
        session_id,
        request_id,
        TerminalKind::ErrorDone {
            code: code.to_string(),
            reason: err.to_string(),
        },
    )
    .await;
}

struct ActiveStreamingHandlerGuard;

impl ActiveStreamingHandlerGuard {
    fn new() -> Self {
        let current = ACTIVE_STREAMING_HANDLERS.fetch_add(1, Ordering::Relaxed) + 1;
        update_streaming_handler_metrics(current);
        update_streaming_handler_peak(current);
        Self
    }
}

impl Drop for ActiveStreamingHandlerGuard {
    fn drop(&mut self) {
        let current = ACTIVE_STREAMING_HANDLERS
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        update_streaming_handler_metrics(current);
    }
}

struct LlmInServiceGuard;

impl LlmInServiceGuard {
    fn new() -> Self {
        let current = LLM_IN_SERVICE.fetch_add(1, Ordering::Relaxed) + 1;
        update_llm_in_service_metrics(current);
        update_llm_in_service_peak(current);
        Self
    }
}

impl Drop for LlmInServiceGuard {
    fn drop(&mut self) {
        let current = LLM_IN_SERVICE
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        update_llm_in_service_metrics(current);
    }
}

#[derive(Debug, Clone)]
pub struct MockProviderConfig {
    chunk_count: usize,
    chunk_delay: std::time::Duration,
    chunk_text: String,
    chunk_script: Option<Vec<MockChunkSpec>>,
    chunk_script_sequence: Option<Vec<Vec<MockChunkSpec>>>,
    stream_call_index: std::sync::Arc<AtomicUsize>,
}

#[derive(Debug, Clone)]
enum MockChunkSpec {
    Text(String),
    Tool(String),
    Error(String),
    End,
}

fn parse_mock_chunk_script(raw: &str) -> Option<Vec<MockChunkSpec>> {
    let mut script = Vec::new();
    for token in raw.split(',') {
        let item = token.trim();
        if item.is_empty() {
            continue;
        }
        if let Some(rest) = item.strip_prefix("text:") {
            script.push(MockChunkSpec::Text(rest.trim().to_string()));
            continue;
        }
        if let Some(rest) = item.strip_prefix("tool:") {
            script.push(MockChunkSpec::Tool(rest.trim().to_string()));
            continue;
        }
        if let Some(rest) = item.strip_prefix("error:") {
            script.push(MockChunkSpec::Error(rest.trim().to_string()));
            continue;
        }
        if item.eq_ignore_ascii_case("end") {
            script.push(MockChunkSpec::End);
            continue;
        }
        return None;
    }

    if script.is_empty() {
        None
    } else {
        Some(script)
    }
}

fn parse_mock_chunk_script_sequence(raw: &str) -> Option<Vec<Vec<MockChunkSpec>>> {
    let mut sequence = Vec::new();
    for segment in raw.split("||") {
        let script = parse_mock_chunk_script(segment.trim())?;
        sequence.push(script);
    }
    if sequence.is_empty() {
        None
    } else {
        Some(sequence)
    }
}

fn mock_provider_supports_tool_calls() -> bool {
    std::env::var("NANOBOT_MOCK_SUPPORTS_TOOL_CALLS")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn mock_provider_enabled(config: &config::Config) -> bool {
    std::env::var("NANOBOT_MOCK_PROVIDER")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
        || config.default_provider.eq_ignore_ascii_case("mock")
}

fn mock_provider_config() -> MockProviderConfig {
    let chunk_count = std::env::var("NANOBOT_MOCK_CHUNKS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(5);
    let service_ms = std::env::var("NANOBOT_MOCK_SERVICE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(500);
    let chunk_text = std::env::var("NANOBOT_MOCK_CHUNK_TEXT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "mock-response".to_string());
    let chunk_script = std::env::var("NANOBOT_MOCK_CHUNK_SCRIPT")
        .ok()
        .and_then(|v| parse_mock_chunk_script(&v));
    let chunk_script_sequence = std::env::var("NANOBOT_MOCK_CHUNK_SCRIPT_SEQUENCE")
        .ok()
        .and_then(|v| parse_mock_chunk_script_sequence(&v));

    MockProviderConfig {
        chunk_count,
        chunk_delay: std::time::Duration::from_millis((service_ms / chunk_count as u64).max(1)),
        chunk_text,
        chunk_script,
        chunk_script_sequence,
        stream_call_index: std::sync::Arc::new(AtomicUsize::new(0)),
    }
}

fn llm_bench_mode_enabled() -> bool {
    std::env::var("NANOBOT_LLM_BENCH_MODE")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn llm_bench_disable_persistence() -> bool {
    std::env::var("NANOBOT_LLM_BENCH_NO_PERSISTENCE")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or_else(llm_bench_mode_enabled)
}

pub enum AgentProvider {
    Antigravity(crate::antigravity::AntigravityCompletionModel),
    Google(crate::google::GoogleCompletionModel),
    OpenAI(openai::CompletionModel),
    Meta(crate::llm::meta_provider::MetaCompletionModel),
    Mock(MockProviderConfig),
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderCapabilities {
    pub supports_streaming: bool,
    pub supports_tool_calls: bool,
}

#[derive(Debug, Clone)]
pub enum ProviderChunk {
    TextDelta(String),
    ToolCall { name: String, arguments: Value },
    Error(String),
    End,
}

fn parse_prefixed_tool_call_payload(raw: &str) -> Result<(String, Value), String> {
    let parsed: Value =
        serde_json::from_str(raw.trim()).map_err(|e| format!("invalid tool payload json: {e}"))?;

    let function = parsed
        .get("function")
        .cloned()
        .unwrap_or_else(|| parsed.clone());
    let name = function
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "tool payload missing function.name".to_string())?
        .to_string();
    let arguments = function
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err("tool payload function.arguments must be an object".to_string());
    }

    Ok((name, arguments))
}

fn declared_provider_capabilities(provider_name: &str) -> Option<ProviderCapabilities> {
    match provider_name {
        "antigravity" => Some(ProviderCapabilities {
            supports_streaming: true,
            supports_tool_calls: true,
        }),
        "google" => Some(ProviderCapabilities {
            supports_streaming: true,
            supports_tool_calls: false,
        }),
        "openai" => Some(ProviderCapabilities {
            supports_streaming: true,
            supports_tool_calls: true,
        }),
        "meta" => Some(ProviderCapabilities {
            supports_streaming: true,
            supports_tool_calls: false,
        }),
        "mock" => Some(ProviderCapabilities {
            supports_streaming: true,
            supports_tool_calls: mock_provider_supports_tool_calls(),
        }),
        _ => None,
    }
}

fn classify_stream_integrity_error(
    stream_error: Option<&str>,
    saw_text_chunk: bool,
    tool_call_count: usize,
    provider_caps: ProviderCapabilities,
    provider_name: &str,
) -> Option<(String, String)> {
    if let Some(err) = stream_error {
        return Some(("stream_error".to_string(), err.to_string()));
    }

    if !saw_text_chunk && tool_call_count == 0 {
        return Some((
            "empty_stream_no_content".to_string(),
            "provider stream ended without text or tool calls".to_string(),
        ));
    }

    if !provider_caps.supports_tool_calls && tool_call_count > 0 {
        return Some((
            "provider_tool_calls_unsupported".to_string(),
            format!(
                "provider '{}' emitted tool calls but declares tool-call support disabled",
                provider_name
            ),
        ));
    }

    None
}

impl AgentProvider {
    pub fn provider_name(&self) -> &'static str {
        match self {
            AgentProvider::Antigravity(_) => "antigravity",
            AgentProvider::Google(_) => "google",
            AgentProvider::OpenAI(_) => "openai",
            AgentProvider::Meta(_) => "meta",
            AgentProvider::Mock(_) => "mock",
        }
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        declared_provider_capabilities(self.provider_name()).unwrap_or(ProviderCapabilities {
            supports_streaming: false,
            supports_tool_calls: false,
        })
    }

    pub async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<
                        Item = Result<ProviderChunk, rig::completion::CompletionError>,
                    > + Send,
            >,
        >,
        rig::completion::CompletionError,
    > {
        match self {
            AgentProvider::Antigravity(m) => {
                let stream = m.stream(request).await?;
                // Map AntigravityStreamingResponse to String
                let mapped = stream.map(|res| {
                    res.map(|content| match content {
                        StreamedAssistantContent::Text(t) => {
                            if let Some(raw_tool_call) = t.text.strip_prefix("__TOOL_CALL__") {
                                match parse_prefixed_tool_call_payload(raw_tool_call) {
                                    Ok((name, arguments)) => {
                                        ProviderChunk::ToolCall { name, arguments }
                                    }
                                    Err(err) => ProviderChunk::Error(format!(
                                        "antigravity_prefixed_tool_parse_failed: {}",
                                        err
                                    )),
                                }
                            } else {
                                ProviderChunk::TextDelta(t.text)
                            }
                        }
                        StreamedAssistantContent::ToolCall(t) => ProviderChunk::ToolCall {
                            name: t.function.name,
                            arguments: t.function.arguments,
                        },
                        StreamedAssistantContent::Final(_) => ProviderChunk::End,
                        _ => ProviderChunk::End,
                    })
                });
                Ok(Box::pin(mapped))
            }
            AgentProvider::Google(m) => {
                let stream = m.stream(request).await?;
                let mapped = stream.map(|res| {
                    res.map(|content| match content {
                        StreamedAssistantContent::Text(t) => ProviderChunk::TextDelta(t.text),
                        StreamedAssistantContent::ToolCall(t) => ProviderChunk::Error(format!(
                            "provider_tool_calls_unsupported: provider=google tool={}",
                            t.function.name
                        )),
                        StreamedAssistantContent::Final(_) => ProviderChunk::End,
                        _ => ProviderChunk::End,
                    })
                });
                Ok(Box::pin(mapped))
            }
            AgentProvider::OpenAI(m) => {
                let stream = m.stream(request).await?;
                let mapped = stream.map(|res| {
                    res.map(|content| match content {
                        StreamedAssistantContent::Text(t) => ProviderChunk::TextDelta(t.text),
                        StreamedAssistantContent::ToolCall(t) => ProviderChunk::ToolCall {
                            name: t.function.name,
                            arguments: t.function.arguments,
                        },
                        StreamedAssistantContent::Final(_) => ProviderChunk::End,
                        _ => ProviderChunk::End,
                    })
                });
                Ok(Box::pin(mapped))
            }
            AgentProvider::Meta(m) => {
                let stream = m.stream(request).await?;
                let mapped = stream.map(|res| {
                    res.map(|content| match content {
                        StreamedAssistantContent::Text(t) => ProviderChunk::TextDelta(t.text),
                        StreamedAssistantContent::ToolCall(t) => ProviderChunk::Error(format!(
                            "provider_tool_calls_unsupported: provider=meta tool={}",
                            t.function.name
                        )),
                        StreamedAssistantContent::Final(_) => ProviderChunk::End,
                        _ => ProviderChunk::End,
                    })
                });
                Ok(Box::pin(mapped))
            }
            AgentProvider::Mock(cfg) => {
                let cfg = cfg.clone();
                if let Some(sequence) = cfg.chunk_script_sequence.clone() {
                    let call_idx = cfg.stream_call_index.fetch_add(1, Ordering::Relaxed);
                    let selected = sequence
                        .get(call_idx)
                        .cloned()
                        .or_else(|| sequence.last().cloned())
                        .unwrap_or_default();
                    let stream = futures::stream::unfold(0usize, move |idx| {
                        let script = selected.clone();
                        async move {
                            if idx >= script.len() {
                                return None;
                            }
                            let chunk = match &script[idx] {
                                MockChunkSpec::Text(text) => ProviderChunk::TextDelta(text.clone()),
                                MockChunkSpec::Tool(name) => ProviderChunk::ToolCall {
                                    name: name.clone(),
                                    arguments: json!({}),
                                },
                                MockChunkSpec::Error(msg) => ProviderChunk::Error(msg.clone()),
                                MockChunkSpec::End => ProviderChunk::End,
                            };
                            Some((Ok(chunk), idx + 1))
                        }
                    });
                    return Ok(Box::pin(stream));
                }
                if let Some(script) = cfg.chunk_script.clone() {
                    let stream = futures::stream::unfold(0usize, move |idx| {
                        let script = script.clone();
                        async move {
                            if idx >= script.len() {
                                return None;
                            }
                            let chunk = match &script[idx] {
                                MockChunkSpec::Text(text) => ProviderChunk::TextDelta(text.clone()),
                                MockChunkSpec::Tool(name) => ProviderChunk::ToolCall {
                                    name: name.clone(),
                                    arguments: json!({}),
                                },
                                MockChunkSpec::Error(msg) => ProviderChunk::Error(msg.clone()),
                                MockChunkSpec::End => ProviderChunk::End,
                            };
                            Some((Ok(chunk), idx + 1))
                        }
                    });
                    return Ok(Box::pin(stream));
                }

                let stream = futures::stream::unfold(0usize, move |idx| {
                    let cfg = cfg.clone();
                    async move {
                        if idx >= cfg.chunk_count {
                            return None;
                        }
                        tokio::time::sleep(cfg.chunk_delay).await;
                        let text = format!("{}-{}", cfg.chunk_text, idx + 1);
                        Some((Ok(ProviderChunk::TextDelta(text)), idx + 1))
                    }
                });
                Ok(Box::pin(stream))
            }
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    static ENCODER: once_cell::sync::Lazy<Option<tiktoken_rs::CoreBPE>> =
        once_cell::sync::Lazy::new(|| tiktoken_rs::cl100k_base().ok());

    if let Some(encoder) = ENCODER.as_ref() {
        return encoder.encode_with_special_tokens(text).len();
    }

    text.len() / 4
}

fn estimate_message_tokens(msg: &Message) -> usize {
    match msg {
        Message::User { content } => {
            content
                .iter()
                .map(|c| match c {
                    UserContent::Text(t) => estimate_tokens(&t.text),
                    _ => 0, // Ignore images for simple estimation
                })
                .sum()
        }
        Message::Assistant { content, .. } => content
            .iter()
            .map(|c| match c {
                AssistantContent::Text(t) => estimate_tokens(&t.text),
                _ => 0,
            })
            .sum(),
    }
}

fn prune_tool_outputs(chat_history: &mut [Message], max_chars: usize) {
    let head = max_chars / 2;
    let tail = max_chars.saturating_sub(head);

    for msg in chat_history.iter_mut() {
        if let Message::User { content } = msg {
            for part in content.iter_mut() {
                if let UserContent::Text(text) = part
                    && text.text.starts_with("Tool '")
                    && text.text.contains("Output:")
                    && text.text.len() > max_chars
                {
                    let head_part = &text.text[..head.min(text.text.len())];
                    let tail_part = &text.text[text.text.len().saturating_sub(tail)..];
                    text.text = format!(
                        "{}\n... [tool output truncated] ...\n{}",
                        head_part, tail_part
                    );
                }
            }
        }
    }
}

type LastInteractionHandle =
    std::sync::Arc<tokio::sync::Mutex<Option<(String, mpsc::Sender<StreamChunk>)>>>;

struct AutoToolArgs {
    session_id: Option<String>,
    message: Option<String>,
}

fn detect_auto_tool(input: &str, current_session: &str) -> Option<(String, AutoToolArgs)> {
    let trimmed = input.trim();
    let lower = trimmed.to_lowercase();

    if lower == "/list sessions" || lower == "/sessions list" {
        return Some((
            "sessions_list".to_string(),
            AutoToolArgs {
                session_id: None,
                message: None,
            },
        ));
    }

    if lower.starts_with("/session status") || lower.starts_with("/status session") {
        let session_id = trimmed.split_whitespace().last().map(|s| s.to_string());
        return Some((
            "session_status".to_string(),
            AutoToolArgs {
                session_id,
                message: None,
            },
        ));
    }

    if lower.starts_with("/session history") || lower.starts_with("/history session") {
        let session_id = trimmed.split_whitespace().last().map(|s| s.to_string());
        return Some((
            "sessions_history".to_string(),
            AutoToolArgs {
                session_id,
                message: None,
            },
        ));
    }

    if lower.starts_with("/send session ") || lower.starts_with("/sessions send ") {
        let rest = trimmed.splitn(4, ' ').skip(2).collect::<Vec<_>>();
        if rest.len() >= 2 {
            return Some((
                "sessions_send".to_string(),
                AutoToolArgs {
                    session_id: Some(rest[0].to_string()),
                    message: Some(rest[1].to_string()),
                },
            ));
        }
    }

    if lower.starts_with("/message ") {
        let msg = trimmed.strip_prefix("/message ").map(|s| s.to_string());
        return Some((
            "message".to_string(),
            AutoToolArgs {
                session_id: Some(current_session.to_string()),
                message: msg,
            },
        ));
    }

    None
}

pub struct AgentLoop {
    provider: std::sync::Arc<tokio::sync::RwLock<AgentProvider>>,
    provider_health: std::sync::Arc<tokio::sync::RwLock<ProviderHealthState>>,
    config: config::Config,
    #[allow(dead_code)]
    key_indices: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, usize>>>,
    context_tree: std::sync::Arc<ContextTree>,
    skill_loader: Option<std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>>,
    cron_scheduler: crate::cron::CronScheduler,
    agent_manager: std::sync::Arc<crate::gateway::agent_manager::AgentManager>,
    memory_manager: std::sync::Arc<crate::memory::MemoryManager>,
    #[allow(dead_code)]
    mcp_manager: Option<std::sync::Arc<crate::mcp::McpManager>>,
    #[allow(dead_code)]
    workspace_watcher: Option<crate::memory::WorkspaceWatcher>,
    personality: Option<personality::PersonalityContext>,
    system_prompt_override: Option<String>,
    agent_event_rx: Option<tokio::sync::mpsc::Receiver<AgentEvent>>,
    last_interaction: LastInteractionHandle,
    session_senders: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, mpsc::Sender<StreamChunk>>>,
    >,
    permission_manager: std::sync::Arc<tokio::sync::Mutex<crate::tools::PermissionManager>>,
    tool_policy: std::sync::Arc<crate::tools::ToolPolicy>,
    confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    resource_monitor: std::sync::Arc<crate::system::resources::ResourceMonitor>,
    persistence: std::sync::Arc<crate::persistence::PersistenceManager>,
    #[cfg(feature = "browser")]
    browser_client: Option<crate::browser::BrowserClient>,
    active_tasks: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>,
    >,
}

impl AgentLoop {
    fn provider_has_credentials(config: &config::Config, provider: &str) -> bool {
        match provider {
            "openai" => config
                .providers
                .openai
                .as_ref()
                .and_then(|c| {
                    c.api_keys
                        .as_ref()
                        .filter(|keys| !keys.is_empty())
                        .map(|_| true)
                        .or_else(|| c.api_key.as_ref().map(|k| !k.is_empty()))
                })
                .unwrap_or_else(|| std::env::var("OPENAI_API_KEY").is_ok()),
            "openrouter" => config
                .providers
                .openrouter
                .as_ref()
                .and_then(|c| {
                    c.api_keys
                        .as_ref()
                        .filter(|keys| !keys.is_empty())
                        .map(|_| true)
                        .or_else(|| c.api_key.as_ref().map(|k| !k.is_empty()))
                })
                .unwrap_or(false),
            "google" => config
                .providers
                .google
                .as_ref()
                .and_then(|c| {
                    c.api_keys
                        .as_ref()
                        .filter(|keys| !keys.is_empty())
                        .map(|_| true)
                        .or_else(|| c.api_key.as_ref().map(|k| !k.is_empty()))
                })
                .unwrap_or(false),
            "mock" => true,
            "antigravity" => true,
            _ => false,
        }
    }

    #[allow(dead_code)]
    fn provider_key_count(config: &config::Config, provider: &str) -> usize {
        match provider {
            "openai" => {
                if let Some(c) = config.providers.openai.as_ref() {
                    if let Some(keys) = c.api_keys.as_ref()
                        && !keys.is_empty()
                    {
                        return keys.len();
                    }
                    if c.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false) {
                        return 1;
                    }
                }
                if std::env::var("OPENAI_API_KEY").is_ok() {
                    1
                } else {
                    0
                }
            }
            "openrouter" => {
                if let Some(c) = config.providers.openrouter.as_ref() {
                    if let Some(keys) = c.api_keys.as_ref()
                        && !keys.is_empty()
                    {
                        return keys.len();
                    }
                    if c.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false) {
                        return 1;
                    }
                }
                0
            }
            "google" => {
                if let Some(c) = config.providers.google.as_ref() {
                    if let Some(keys) = c.api_keys.as_ref()
                        && !keys.is_empty()
                    {
                        return keys.len();
                    }
                    if c.api_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false) {
                        return 1;
                    }
                }
                0
            }
            "mock" => 1,
            "antigravity" => 1,
            _ => 0,
        }
    }

    fn normal_provider_chain(config: &config::Config) -> Vec<String> {
        let default = config.default_provider.to_lowercase();
        if default == "antigravity" {
            return vec!["antigravity".to_string()];
        }

        let mut chain = Vec::new();
        if Self::provider_has_credentials(config, &default) {
            chain.push(default.clone());
        }

        for candidate in ["openai", "openrouter", "google"] {
            if candidate != default && Self::provider_has_credentials(config, candidate) {
                chain.push(candidate.to_string());
            }
        }

        if chain.is_empty() {
            chain.push(default);
        }

        chain
    }

    #[allow(dead_code)]
    fn normal_retry_budget(config: &config::Config) -> u32 {
        let chain = Self::normal_provider_chain(config);
        let total_slots: usize = chain
            .iter()
            .map(|p| Self::provider_key_count(config, p).max(1))
            .sum();
        let rotations = total_slots.saturating_sub(1).max(3);
        rotations as u32
    }

    /// Create a provider instance based on config and key indices
    pub async fn create_provider(
        config: &config::Config,
        indices: &std::collections::HashMap<String, usize>,
    ) -> Result<AgentProvider> {
        let model_override = selected_model_from_env();

        if mock_provider_enabled(config) {
            return Ok(AgentProvider::Mock(mock_provider_config()));
        }

        // Priority 1: Check for LLM failover config
        if let Some(llm_config) = config.llm.clone() {
            tracing::info!(
                "Using MetaProvider with failover chain: {:?}",
                llm_config.failover_chain
            );

            let meta_client =
                crate::llm::meta_provider::MetaClient::new(llm_config.clone()).await?;
            let model_name = model_override
                .clone()
                .unwrap_or_else(|| "gemini-2.0-flash-001".to_string());

            return Ok(AgentProvider::Meta(
                crate::llm::meta_provider::MetaCompletionModel::make(&meta_client, model_name),
            ));
        }

        // Priority 2: Fall back to normal provider path with key/provider rotation
        let chain = Self::normal_provider_chain(config);
        let provider_idx = *indices.get("__provider_index").unwrap_or(&0);
        let provider_name = chain
            .get(provider_idx % chain.len())
            .cloned()
            .unwrap_or_else(|| config.default_provider.clone());
        let index = *indices.get(&provider_name).unwrap_or(&0);

        match provider_name.as_str() {
            "antigravity" => {
                let _ag_config = config.providers.antigravity.as_ref();

                // Resolution logic:
                // Antigravity now strictly uses OAuth via TokenManager (handled inside client)
                // We REMOVE the unsafe env var injection for API keys here.

                let client = AntigravityClient::from_env().await?;
                let model_name = model_override.as_deref().unwrap_or("gemini-2.0-flash-exp");
                Ok(AgentProvider::Antigravity(
                    client.completion_model(model_name),
                ))
            }
            "google" => {
                let google_config = config
                    .providers
                    .google
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Google provider not configured"))?;

                let api_key = if let Some(keys) = &google_config.api_keys {
                    if !keys.is_empty() {
                        keys[index % keys.len()].clone()
                    } else {
                        google_config.api_key.clone().unwrap_or_default()
                    }
                } else {
                    google_config.api_key.clone().unwrap_or_default()
                };

                if api_key.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Google API key missing. Configure 'api_key' or 'api_keys' in [providers.google]"
                    ));
                }

                let client = reqwest::Client::new();
                let model_name = model_override.as_deref().unwrap_or("gemini-2.0-flash");
                let mut model = crate::google::GoogleCompletionModel::make(&client, model_name);
                model.api_key = api_key;

                Ok(AgentProvider::Google(model))
            }
            "openrouter" => {
                let or_config = config
                    .providers
                    .openrouter
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenRouter not configured"))?;

                let api_key = if let Some(keys) = &or_config.api_keys {
                    if !keys.is_empty() {
                        keys[index % keys.len()].clone()
                    } else {
                        or_config.api_key.clone().unwrap_or_default()
                    }
                } else {
                    or_config.api_key.clone().unwrap_or_default()
                };

                if api_key.is_empty() {
                    return Err(anyhow::anyhow!(
                        "OpenRouter API key missing. Configure 'api_key' or 'api_keys' in config.toml"
                    ));
                }

                let client = openai::Client::new(&api_key)?;
                let model_name = model_override
                    .as_deref()
                    .unwrap_or("google/gemini-2.0-flash-001");
                Ok(AgentProvider::OpenAI(
                    client
                        .completions_api()
                        .completion_model(model_name),
                ))
            }
            _ => {
                let oa_config = config.providers.openai.as_ref();
                let api_key = if let Some(c) = oa_config {
                    if let Some(keys) = &c.api_keys {
                        if !keys.is_empty() {
                            Some(keys[index % keys.len()].clone())
                        } else {
                            c.api_key.clone()
                        }
                    } else {
                        c.api_key.clone()
                    }
                } else {
                    None
                }
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| anyhow::anyhow!("OpenAI API Key missing"))?;

                let client = openai::Client::new(&api_key)?;
                let model_name = model_override.as_deref().unwrap_or("gpt-4o");
                Ok(AgentProvider::OpenAI(
                    client.completions_api().completion_model(model_name),
                ))
            }
        }
    }

    /// Rotate the current provider's API key
    #[allow(dead_code)]
    async fn rotate_provider(&self) -> Result<()> {
        let mut key_indices = self.key_indices.lock().await;
        if self.config.llm.is_some() {
            let entry = key_indices.entry("meta".to_string()).or_insert(0);
            *entry += 1;
            tracing::info!("Rotating auth key for provider 'meta' (index: {})", entry);
        } else {
            let chain = Self::normal_provider_chain(&self.config);
            let current_provider_idx =
                *key_indices.get("__provider_index").unwrap_or(&0) % chain.len();
            let current_provider = chain[current_provider_idx].clone();
            let key_count = Self::provider_key_count(&self.config, &current_provider).max(1);

            let entry = key_indices.entry(current_provider.clone()).or_insert(0);
            *entry += 1;

            if *entry >= key_count {
                *entry = 0;
                if chain.len() > 1 {
                    let next_idx = (current_provider_idx + 1) % chain.len();
                    key_indices.insert("__provider_index".to_string(), next_idx);
                    tracing::warn!(
                        "Provider '{}' keys exhausted. Failing over to provider '{}'",
                        current_provider,
                        chain[next_idx]
                    );
                } else {
                    tracing::warn!(
                        "Provider '{}' keys exhausted but no fallback provider configured",
                        current_provider
                    );
                }
            } else {
                tracing::info!(
                    "Rotating auth key for provider '{}' (index: {}/{})",
                    current_provider,
                    *entry,
                    key_count
                );
            }
        }

        // Re-create provider
        let new_provider = Self::create_provider(&self.config, &key_indices).await?;

        let mut provider_guard = self.provider.write().await;
        *provider_guard = new_provider;

        Ok(())
    }

    async fn mark_provider_unhealthy(&self, reason: &'static str, ttl: std::time::Duration) {
        ADAPTIVE_PROVIDER_UNHEALTHY.store(true, Ordering::Relaxed);
        adaptive_step_down_on_unhealthy();
        let until = std::time::Instant::now() + ttl;
        let mut guard = self.provider_health.write().await;
        *guard = ProviderHealthState::Unhealthy { reason, until };
        increment_llm_rejected("provider_unhealthy");
        crate::metrics::GLOBAL_METRICS.set_gauge("llm_provider_unhealthy", 1.0);
        tracing::warn!(reason = reason, ttl_ms = ttl.as_millis(), "LLM provider marked unhealthy");
    }

    async fn mark_provider_healthy(&self) {
        let mut guard = self.provider_health.write().await;
        if !matches!(*guard, ProviderHealthState::Healthy) {
            *guard = ProviderHealthState::Healthy;
            ADAPTIVE_PROVIDER_UNHEALTHY.store(false, Ordering::Relaxed);
            crate::metrics::GLOBAL_METRICS.set_gauge("llm_provider_unhealthy", 0.0);
            tracing::info!("LLM provider recovered and marked healthy");
        }
    }

    async fn provider_unhealthy_reason(&self) -> Option<String> {
        let now = std::time::Instant::now();
        let mut guard = self.provider_health.write().await;
        match &*guard {
            ProviderHealthState::Healthy => None,
            ProviderHealthState::Unhealthy { reason, until } => {
                if *until > now {
                    Some(format!(
                        "{} (retry in {}ms)",
                        reason,
                        until.saturating_duration_since(now).as_millis()
                    ))
                } else {
                    *guard = ProviderHealthState::Healthy;
                    ADAPTIVE_PROVIDER_UNHEALTHY.store(false, Ordering::Relaxed);
                    crate::metrics::GLOBAL_METRICS.set_gauge("llm_provider_unhealthy", 0.0);
                    None
                }
            }
        }
    }

    pub async fn new() -> Result<Self> {
        let config = config::Config::load()?;
        crate::distributed::enforce_admission_mode_support()?;
        tracing::info!(
            admission_mode = ?crate::distributed::selected_admission_mode(),
            "Admission mode selection"
        );

        ADAPTIVE_LLM_PERMIT_LIMIT.store(llm_task_concurrency_limit(), Ordering::Relaxed);
        ADAPTIVE_PROVIDER_UNHEALTHY.store(false, Ordering::Relaxed);
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_adaptive_concurrency_limit",
            current_adaptive_llm_limit() as f64,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_permits_current",
            current_adaptive_llm_limit() as f64,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_permits_target",
            llm_task_concurrency_limit() as f64,
        );
        spawn_adaptive_permit_recovery_task();

        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_config_concurrency_limit",
            llm_task_concurrency_limit() as f64,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_config_queue_wait_timeout_ms",
            llm_queue_wait_timeout().as_millis() as f64,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_config_mock_provider_enabled",
            if mock_provider_enabled(&config) { 1.0 } else { 0.0 },
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_config_bench_mode_enabled",
            if llm_bench_mode_enabled() { 1.0 } else { 0.0 },
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "llm_config_bench_no_persistence",
            if llm_bench_disable_persistence() {
                1.0
            } else {
                0.0
            },
        );

        #[cfg(not(feature = "browser"))]
        if config.browser.is_some() {
            tracing::warn!("Browser config present but binary built without 'browser' feature");
        }

        // Initial provider creation
        let indices_map = std::collections::HashMap::new();
        let provider = Self::create_provider(&config, &indices_map).await?;
        let provider = std::sync::Arc::new(tokio::sync::RwLock::new(provider));
        let key_indices = std::sync::Arc::new(tokio::sync::Mutex::new(indices_map));

        let db_path = PathBuf::from(".").join(".nanobot").join("context_tree.db");

        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let context_tree = std::sync::Arc::new(ContextTree::new(
            db_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid db path"))?,
        )?);

        // Create channel for agent events (cron + subagent updates)
        let (agent_event_tx, agent_event_rx) = tokio::sync::mpsc::channel(100);

        // Initialize Cron Scheduler
        let cron_scheduler =
            crate::cron::CronScheduler::new(db_path.clone(), agent_event_tx.clone()).await?;
        cron_scheduler.start().await?;

        // Initialize Agent Manager
        let agent_manager = std::sync::Arc::new(crate::gateway::agent_manager::AgentManager::new());
        agent_manager.load_registry().await?; // Restore persistent state
        agent_manager.set_event_sender(agent_event_tx.clone()).await;
        let recovered = agent_manager.recover_sessions().await?;
        if recovered > 0 {
            tracing::info!("Recovered {} running subagent session(s)", recovered);
        }
        agent_manager.start_cleanup_task();

        // Initialize Memory Manager
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
        let memory_db_path = PathBuf::from(&home)
            .join(".nanobot")
            .join("memory")
            .join("index.json");

        // Create provider for memory (OpenAI or Local)
        let mem_provider = if let Ok(openai_key) = std::env::var("OPENAI_API_KEY") {
            crate::memory::EmbeddingProvider::openai(openai_key)
        } else {
            crate::memory::EmbeddingProvider::local()?
        };

        let memory_manager = std::sync::Arc::new(crate::memory::MemoryManager::new(
            memory_db_path,
            mem_provider,
        )?);
        let _ = memory_manager.load_index(); // Best effort load

        // Initialize Workspace Watcher
        let workspace_dir = crate::workspace::resolve_workspace_dir();

        // Check local override or use default
        let watch_path = if PathBuf::from(".").join("Cargo.toml").exists() {
            PathBuf::from(".")
                .canonicalize()
                .unwrap_or(PathBuf::from("."))
        } else {
            workspace_dir.clone()
        };

        let workspace_watcher = match crate::memory::WorkspaceWatcher::new(
            watch_path.clone(),
            memory_manager.clone(),
            Some("system".to_string()),
        ) {
            Ok(w) => {
                println!("👀 File watcher active on {:?}", watch_path);
                Some(w)
            }
            Err(e) => {
                tracing::warn!("Failed to start file watcher: {}", e);
                None
            }
        };

        // Initialize Skills Loader
        let skills_path = workspace_dir.join("skills");
        let skill_loader = if skills_path.exists() {
            let mut loader = crate::skills::SkillLoader::new(workspace_dir.clone());
            match loader.scan() {
                Ok(_) => {
                    let skill_count = loader.skills().len();
                    if skill_count > 0 {
                        tracing::info!("Loaded {} skills from {:?}", skill_count, skills_path);
                    }
                    Some(std::sync::Arc::new(tokio::sync::Mutex::new(loader)))
                }
                Err(e) => {
                    tracing::warn!("Failed to load skills: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let personality = if workspace_dir.exists() {
            match personality::PersonalityContext::load(&workspace_dir).await {
                Ok(p) => {
                    tracing::info!("Loaded personality: {} {}", p.agent_emoji(), p.agent_name());
                    Some(p)
                }
                Err(e) => {
                    tracing::warn!("Could not load personality: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize Permission Manager with workspace scope
        let workspace_root = std::env::current_dir()?;
        let security_profile = crate::tools::permissions::SecurityProfile::standard(workspace_root);
        let permission_manager = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::tools::PermissionManager::new(security_profile),
        ));

        let tool_policy = std::sync::Arc::new(match config.interaction_policy {
            crate::config::InteractionPolicy::Interactive => {
                crate::tools::ToolPolicy::ask_me_default()
            }
            crate::config::InteractionPolicy::HeadlessDeny => {
                crate::tools::ToolPolicy::headless_deny_default()
            }
            crate::config::InteractionPolicy::HeadlessAllowLog => {
                crate::tools::ToolPolicy::permissive()
            }
        });

        let confirmation_service = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));

        {
            let mut service = confirmation_service.lock().await;
            service.register_adapter(Box::new(
                crate::tools::cli_confirmation::CliConfirmationAdapter::new(),
            ));
        }

        // Initialize Resource Monitor
        let resource_monitor =
            std::sync::Arc::new(crate::system::resources::ResourceMonitor::new());
        resource_monitor.start_monitoring().await;

        // Initialize MCP Manager
        let mcp_manager = if let Some(mcp_config) = config.mcp.as_ref() {
            if mcp_config.enabled && !mcp_config.servers.is_empty() {
                let manager = std::sync::Arc::new(crate::mcp::McpManager::new());

                for server_config in &mcp_config.servers {
                    match manager.add_server(server_config.clone()).await {
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(
                                "Failed to connect to MCP server '{}': {}",
                                server_config.name,
                                e
                            );
                        }
                    }
                }

                let tool_count = manager.tool_count().await;
                if tool_count > 0 {
                    tracing::info!(
                        "MCP loaded {} tools from {} servers",
                        tool_count,
                        manager.server_count().await
                    );
                }

                // Start health check loop
                manager.start_health_check();

                Some(manager)
            } else {
                None
            }
        } else {
            None
        };

        // Initialize Browser Client
        #[cfg(feature = "browser")]
        let browser_client = if let Some(browser_config) = config.browser.clone() {
            Some(crate::browser::BrowserClient::new(browser_config))
        } else {
            None
        };

        let persistence_db_path = db_path.clone();
        let persistence = std::sync::Arc::new(crate::persistence::PersistenceManager::new(
            persistence_db_path,
        ));
        persistence.init()?;

        if let Err(e) = Self::sync_heartbeat_schedule(&cron_scheduler, &workspace_dir).await {
            tracing::warn!("Failed to sync HEARTBEAT.md schedule: {}", e);
        }

        Ok(Self {
            provider,
            provider_health: std::sync::Arc::new(tokio::sync::RwLock::new(ProviderHealthState::Healthy)),
            config,
            key_indices,
            context_tree,
            skill_loader,
            cron_scheduler,
            agent_manager,
            memory_manager,
            mcp_manager,
            workspace_watcher,
            personality,
            system_prompt_override: None,
            agent_event_rx: Some(agent_event_rx),
            last_interaction: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            session_senders: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            permission_manager,
            tool_policy,
            confirmation_service,
            resource_monitor,
            persistence,
            #[cfg(feature = "browser")]
            browser_client,
            active_tasks: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        })
    }

    pub fn confirmation_service(
        &self,
    ) -> std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>> {
        self.confirmation_service.clone()
    }

    pub fn set_system_prompt_override(&mut self, prompt: Option<String>) {
        self.system_prompt_override = prompt;
    }

    pub fn set_tool_policy(&mut self, policy: crate::tools::ToolPolicy) {
        self.tool_policy = std::sync::Arc::new(policy);
    }

    async fn sync_heartbeat_schedule(
        cron_scheduler: &crate::cron::CronScheduler,
        workspace_dir: &std::path::Path,
    ) -> Result<()> {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let candidates = vec![
            current_dir.join("HEARTBEAT.md"),
            workspace_dir.join("HEARTBEAT.md"),
        ];

        let jobs = cron_scheduler.list_jobs(true)?;
        let heartbeat_jobs: Vec<String> = jobs
            .into_iter()
            .filter(|j| {
                j.name
                    .as_deref()
                    .map(|n| n.starts_with("heartbeat:"))
                    .unwrap_or(false)
            })
            .map(|j| j.id)
            .collect();

        for job_id in heartbeat_jobs {
            let _ = cron_scheduler.remove_job(&job_id);
        }

        let Some((path, spec)) = crate::heartbeat::load_first(&candidates)? else {
            return Ok(());
        };

        let heartbeat_prompt = spec.system_prompt();

        let mut cron_job = crate::cron::CronJob::new(
            Some(format!("heartbeat:{}", path.display())),
            crate::cron::Schedule::Cron {
                expr: spec.schedule,
                tz: spec.timezone,
            },
            crate::cron::Payload::SystemEvent {
                text: heartbeat_prompt,
            },
            crate::cron::SessionTarget::Main,
        );
        cron_job.wake_mode = crate::cron::WakeMode::NextHeartbeat;

        cron_scheduler.add_job(cron_job).await?;
        tracing::info!("Loaded HEARTBEAT.md schedule from {}", path.display());
        Ok(())
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<AgentMessage>) {
        // Take ownership of agent_event_rx
        let agent_event_rx = self.agent_event_rx.take();

        // Wrap self in Arc to share with the cron handler task
        let agent = std::sync::Arc::new(self);

        // Spawn agent event handler task
        if let Some(mut event_rx) = agent_event_rx {
            let agent_inner = agent.clone();
            let event_task = tokio::spawn(async move {
                println!("🕐 Agent event handler started");
                while let Some(event) = event_rx.recv().await {
                    match event {
                        AgentEvent::SystemEvent { job_id, text } => {
                            let source = job_id.clone().unwrap_or_else(|| "unknown".to_string());
                            println!("📅 [AgentEvent] SystemEvent from {}: {}", source, text);

                            // Try to inject into last active session
                            let interaction = {
                                let last = agent_inner.last_interaction.lock().await;
                                last.clone()
                            };

                            if let Some((session_id, response_tx)) = interaction {
                                println!("💉 Injecting SystemEvent into session {}", session_id);
                                let msg = AgentMessage {
                                    session_id,
                                    tenant_id: "default".to_string(), // Scheduler runs as system/default
                                    request_id: format!("system:{}", uuid::Uuid::new_v4()),
                                    content: format!("(System Event) {}", text),
                                    response_tx,
                                    ingress_at: std::time::Instant::now(),
                                };
                                agent_inner.process_streaming(msg).await;
                            } else {
                                println!(
                                    "⚠️ No active interaction found for SystemEvent injection."
                                );
                            }
                        }
                        AgentEvent::AgentTurn {
                            job_id,
                            message,
                            model,
                            ..
                        } => {
                            let source = job_id.clone().unwrap_or_else(|| "unknown".to_string());
                            println!(
                                "📅 [AgentEvent] AgentTurn from {}: {} (model: {:?})",
                                source, message, model
                            );

                            // Spawn task to execute isolated agent
                            let agent_mgr = agent_inner.agent_manager.clone();
                            let last_int = agent_inner.last_interaction.clone();
                            let job_id_clone =
                                job_id.clone().unwrap_or_else(|| "unknown".to_string());
                            let message_clone = message.clone();

                            tokio::spawn(async move {
                                println!(
                                    "🔄 [Cron] Executing isolated agent for job {}...",
                                    job_id_clone
                                );

                                // Create a minimal CronJob for isolated execution
                                // In a full implementation, we'd fetch the job from DB to get isolation config
                                // For now, we use a simple inline job
                                let job = crate::cron::CronJob {
                                    id: job_id_clone.clone(),
                                    name: Some(format!("Agent Turn: {}", job_id_clone)),
                                    enabled: true,
                                    schedule: crate::cron::Schedule::At { at_ms: 0 },
                                    payload: crate::cron::Payload::AgentTurn {
                                        message: message_clone.clone(),
                                        model: model.clone(),
                                        thinking: None,
                                        timeout_seconds: Some(120),
                                    },
                                    session_target: crate::cron::SessionTarget::Main,
                                    wake_mode: crate::cron::WakeMode::default(),
                                    isolation: None, // Defaults to inline isolated execution for ad-hoc cron turns
                                    delete_after_run: false,
                                    created_at: 0,
                                };

                                // Execute isolated agent turn
                                match crate::cron::isolated_agent::run_isolated_agent_turn(
                                    &job,
                                    &agent_mgr,
                                    message_clone.clone(),
                                )
                                .await
                                {
                                    Ok(result) => {
                                        println!(
                                            "✅ [Cron] {} Isolated agent completed: {:?}",
                                            job_id_clone, result.status
                                        );

                                        // Post-to-main feedback
                                        if let Some(output) = result.output_text {
                                            let feedback_msg =
                                                format!("[Cron Job: {}]: {}", job_id_clone, output);

                                            // Try to inject into last active session
                                            let interaction = {
                                                let last = last_int.lock().await;
                                                last.clone()
                                            };

                                            if let Some((session_id, response_tx)) = interaction {
                                                println!(
                                                    "💉 [Cron] Injecting result into session {}",
                                                    session_id
                                                );
                                                let _ = response_tx
                                                    .send(crate::agent::StreamChunk::TextDelta(
                                                        format!("\n\n{}\n\n", feedback_msg),
                                                    ))
                                                    .await;
                                            } else {
                                                println!(
                                                    "⚠️ [Cron] No active session for feedback injection"
                                                );
                                            }
                                        }

                                        if let Some(error) = result.error {
                                            tracing::error!("[Cron] Agent turn error: {}", error);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "[Cron] Failed to execute isolated agent: {}",
                                            e
                                        );
                                    }
                                }
                            });
                        }
                        AgentEvent::SessionMessage { session_id, text } => {
                            let tx = {
                                let senders = agent_inner.session_senders.lock().await;
                                senders.get(&session_id).cloned()
                            };

                            if let Some(response_tx) = tx {
                                let _ = response_tx
                                    .send(crate::agent::StreamChunk::TextDelta(format!(
                                        "\n\n{}\n\n",
                                        text
                                    )))
                                    .await;
                                if let Err(e) =
                                    agent_inner.save_message(&session_id, "assistant", &text).await
                                {
                                    tracing::warn!("Failed to persist injected message: {}", e);
                                }
                            } else {
                                println!(
                                    "⚠️ No active session sender found for SessionMessage injection."
                                );
                            }
                        }
                    }
                }
                println!("🕐 Agent event handler stopped");
            });
            tokio::spawn(async move {
                match event_task.await {
                    Ok(_) => {}
                    Err(join_err) if join_err.is_cancelled() => {
                        tracing::debug!("Agent event handler task cancelled");
                    }
                    Err(join_err) if join_err.is_panic() => {
                        crate::metrics::GLOBAL_METRICS
                            .increment_counter("agent_event_task_panics_total", 1);
                        tracing::error!("Agent event handler task panicked");
                    }
                    Err(join_err) => {
                        tracing::warn!(error = %join_err, "Agent event handler task failed");
                    }
                }
            });
        }

        // Main agent message loop
        while let Some(msg) = rx.recv().await {
            let session_id = msg.session_id.clone();
            let agent_clone = agent.clone();

            // Cancel existing task for this session if any regarding interruptibility
            {
                let mut tasks = agent.active_tasks.lock().await;
                if let Some(handle) = tasks.remove(&session_id) {
                    tracing::info!("⚠️ Interrupting active task for session {}", session_id);
                    handle.abort();
                }
            }

            // Spawn new task
            let session_id_clone = session_id.clone();
            let task = tokio::spawn(async move {
                let session_id = session_id_clone;
                // Update last interaction tracking
                {
                    let mut last = agent_clone.last_interaction.lock().await;
                    *last = Some((msg.session_id.clone(), msg.response_tx.clone()));
                }

                // Track active session sender for targeted injections
                {
                    let mut senders = agent_clone.session_senders.lock().await;
                    senders.insert(msg.session_id.clone(), msg.response_tx.clone());
                }

                agent_clone.process_streaming(msg).await;

                // Cleanup task from map upon completion
                let mut tasks = agent_clone.active_tasks.lock().await;
                tasks.remove(&session_id);

                let mut senders = agent_clone.session_senders.lock().await;
                senders.remove(&session_id);
            });

            let task_abort_handle = task.abort_handle();
            let task_session_id = session_id.clone();
            let task_agent = agent.clone();
            tokio::spawn(async move {
                let join_result = task.await;

                {
                    let mut tasks = task_agent.active_tasks.lock().await;
                    tasks.remove(&task_session_id);
                }

                {
                    let mut senders = task_agent.session_senders.lock().await;
                    senders.remove(&task_session_id);
                }

                match join_result {
                    Ok(_) => {}
                    Err(join_err) if join_err.is_cancelled() => {
                        tracing::debug!(session_id = %task_session_id, "Agent session task cancelled");
                    }
                    Err(join_err) if join_err.is_panic() => {
                        crate::metrics::GLOBAL_METRICS
                            .increment_counter("agent_session_task_panics_total", 1);
                        tracing::error!(session_id = %task_session_id, "Agent session task panicked");
                    }
                    Err(join_err) => {
                        tracing::warn!(session_id = %task_session_id, error = %join_err, "Agent session task failed");
                    }
                }
            });

            // Store handle
            let mut tasks = agent.active_tasks.lock().await;
            tasks.insert(session_id, task_abort_handle);
        }
    }

    // Agent turn loop - Process one message with streaming
    #[tracing::instrument(skip(self, msg), fields(session_id = %msg.session_id, request_id = %msg.request_id, tenant_id = %msg.tenant_id))]
    async fn process_streaming(&self, msg: AgentMessage) {
        let _active_handler_guard = ActiveStreamingHandlerGuard::new();
        let handler_started_at = std::time::Instant::now();
        let dispatch_delay = msg.ingress_at.elapsed();
        crate::metrics::GLOBAL_METRICS.record_duration(
            "llm_dispatch_delay_seconds",
            dispatch_delay,
            true,
        );
        record_duration_buckets("llm_dispatch_delay_seconds", dispatch_delay);

        if let Some(reason) = self.provider_unhealthy_reason().await {
            increment_llm_rejected("provider_unhealthy");
            let _ = send_stream_chunk_timed(
                &msg.response_tx,
                StreamChunk::TextDelta(format!(
                    "Server busy: LLM provider unavailable ({})",
                    reason
                )),
            )
            .await;
            emit_terminal(
                &msg.response_tx,
                &msg.session_id,
                &msg.request_id,
                TerminalKind::ErrorDone {
                    code: "provider_unhealthy".to_string(),
                    reason,
                },
            )
            .await;
            return;
        }

        let queue_deadline = msg.ingress_at + llm_queue_wait_timeout();
        let llm_permit = match acquire_llm_permit_with_deadline(queue_deadline, llm_queue_wait_epsilon()).await {
            LlmPermitAcquireOutcome::Acquired(permit, waited) => {
                crate::metrics::GLOBAL_METRICS.record_duration(
                    "llm_task_semaphore_wait_seconds",
                    waited,
                    true,
                );
                permit
            }
            LlmPermitAcquireOutcome::Closed => {
                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::TextDelta("Error: LLM request queue is unavailable".to_string()),
                )
                .await;
                emit_terminal(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    TerminalKind::ErrorDone {
                        code: "queue_closed".to_string(),
                        reason: "LLM request queue is unavailable".to_string(),
                    },
                )
                .await;
                return;
            }
            LlmPermitAcquireOutcome::BudgetExceeded { waited } => {
                let reject_decision_at = std::time::Instant::now();
                crate::metrics::GLOBAL_METRICS.record_duration(
                    "llm_task_semaphore_wait_seconds",
                    waited,
                    false,
                );
                crate::metrics::GLOBAL_METRICS.record_duration(
                    "llm_handler_to_reject_decision_seconds",
                    reject_decision_at.saturating_duration_since(handler_started_at),
                    true,
                );
                record_duration_buckets(
                    "llm_handler_to_reject_decision_seconds",
                    reject_decision_at.saturating_duration_since(handler_started_at),
                );
                let reject_emit_started = std::time::Instant::now();
                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::TextDelta(
                        "Server busy: too many concurrent LLM requests (429). Please retry shortly."
                            .to_string(),
                    ),
                )
                .await;
                let reject_emit_delay = reject_emit_started.elapsed();
                crate::metrics::GLOBAL_METRICS.record_duration(
                    "llm_reject_emit_delay_seconds",
                    reject_emit_delay,
                    true,
                );
                record_duration_buckets("llm_reject_emit_delay_seconds", reject_emit_delay);
                emit_terminal(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    TerminalKind::ErrorDone {
                        code: "admission_timeout".to_string(),
                        reason: "too many concurrent LLM requests".to_string(),
                    },
                )
                .await;
                let reject_sent_at = std::time::Instant::now();

                if debug_reject_timings_enabled() && should_sample_reject_timing(&msg.request_id) {
                    let payload = json!({
                        "event": "reject_timing",
                        "reason": "semaphore_timeout",
                        "request_id": msg.request_id,
                        "recv_to_handler_start_ms": handler_started_at
                            .saturating_duration_since(msg.ingress_at)
                            .as_millis(),
                        "handler_start_to_deadline_expired_ms": reject_decision_at
                            .saturating_duration_since(handler_started_at)
                            .as_millis(),
                        "deadline_expired_to_reject_emit_ms": reject_emit_started
                            .saturating_duration_since(reject_decision_at)
                            .as_millis(),
                        "reject_emit_to_ws_send_complete_ms": reject_sent_at
                            .saturating_duration_since(reject_emit_started)
                            .as_millis(),
                        "total_recv_to_ws_send_ms": reject_sent_at
                            .saturating_duration_since(msg.ingress_at)
                            .as_millis(),
                    });
                    tracing::info!("{}", payload);
                }
                return;
            }
            LlmPermitAcquireOutcome::QueueOverCapacity { waited } => {
                crate::metrics::GLOBAL_METRICS.record_duration(
                    "llm_task_semaphore_wait_seconds",
                    waited,
                    false,
                );
                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::TextDelta(
                        "Server busy: admission queue is over capacity (503). Please retry shortly."
                            .to_string(),
                    ),
                )
                .await;
                emit_terminal(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    TerminalKind::ErrorDone {
                        code: "queue_over_capacity".to_string(),
                        reason: "admission queue is over capacity".to_string(),
                    },
                )
                .await;
                return;
            }
        };

        update_llm_inflight_metric();
        let _llm_permit_guard = LlmTaskPermitGuard {
            _permit: llm_permit,
        };
        let _llm_in_service_guard = LlmInServiceGuard::new();
        let llm_service_timer = LlmServiceTimer::new();

        if let Some((tool_name, args)) = detect_auto_tool(&msg.content, &msg.session_id) {
            let mut call = serde_json::Map::new();
            call.insert("tool".to_string(), serde_json::Value::String(tool_name));
            if let Some(message) = args.message {
                call.insert("message".to_string(), serde_json::Value::String(message));
            }
            if let Some(session_id) = args.session_id {
                call.insert(
                    "session_id".to_string(),
                    serde_json::Value::String(session_id),
                );
            }
            let tool_input = serde_json::Value::Object(call).to_string();

            let result = crate::tools::executor::execute_tool(
                &tool_input,
                crate::tools::executor::ExecuteToolContext {
                    cron_scheduler: Some(&self.cron_scheduler),
                    agent_manager: Some(&self.agent_manager),
                    memory_manager: Some(&self.memory_manager),
                    persistence: Some(&self.persistence),
                    permission_manager: Some(&*self.permission_manager),
                    tool_policy: Some(&self.tool_policy),
                    confirmation_service: Some(&*self.confirmation_service),
                    skill_loader: self.skill_loader.as_ref(),
                    #[cfg(feature = "browser")]
                    browser_client: self.browser_client.as_ref(),
                    tenant_id: Some(&msg.tenant_id),
                    mcp_manager: self.mcp_manager.as_ref(),
                },
            )
            .await;

            let response_text = match result {
                Ok(output) => output,
                Err(e) => format!("Tool error: {}", e),
            };

            let _ =
                send_stream_chunk_timed(&msg.response_tx, StreamChunk::TextDelta(response_text.clone()))
                    .await;
            emit_terminal(
                &msg.response_tx,
                &msg.session_id,
                &msg.request_id,
                TerminalKind::SuccessDone,
            )
            .await;
            llm_service_timer.mark_success();

            if let Err(e) = self
                .save_message_for_request(
                    &msg.session_id,
                    "assistant",
                    &msg.request_id,
                    &response_text,
                )
                .await
            {
                tracing::warn!("Failed to save auto-tool response: {}", e);
            }

            return;
        }
        // Get adaptive configuration based on current resources
        let adaptive_config = self.resource_monitor.get_adaptive_config();
        let bench_mode = llm_bench_mode_enabled();
        let skip_persistence = llm_bench_disable_persistence();
        let resource_level = self.resource_monitor.get_resource_level();

        // Warn user if resources are constrained
        if resource_level != crate::system::resources::ResourceLevel::High {
            let level_str = match resource_level {
                crate::system::resources::ResourceLevel::Low => "LOW (Throttled)",
                crate::system::resources::ResourceLevel::Medium => "MEDIUM (Limited)",
                crate::system::resources::ResourceLevel::High => unreachable!(),
            };
            let _ = send_stream_chunk_timed(
                &msg.response_tx,
                StreamChunk::TextDelta(format!(
                    "⚠️ Resource Mode: {} | Context: {} msgs | RAG: {} docs | Tokens: {}\n\n",
                    level_str,
                    adaptive_config.context_history_limit,
                    adaptive_config.rag_doc_count,
                    adaptive_config.max_tokens
                )),
            )
            .await;
        }

        if !skip_persistence
            && let Err(e) = self
                .save_message_for_request(&msg.session_id, "user", &msg.request_id, &msg.content)
                .await
        {
            if e.to_string().contains("request_id_content_mismatch") {
                crate::metrics::GLOBAL_METRICS.increment_counter(
                    "persistence_request_id_mismatch_total",
                    1,
                );
            }
            tracing::warn!("Failed to save user message: {}", e);
        }

        let mut chat_history = self.get_conversation_history(&msg.session_id);

        // Apply adaptive context history limit
        // Apply adaptive context history limit (Count-based)
        if chat_history.len() > adaptive_config.context_history_limit {
            let skip = chat_history.len() - adaptive_config.context_history_limit;
            chat_history = chat_history.into_iter().skip(skip).collect();
        }

        prune_tool_outputs(&mut chat_history, 4000);

        let token_limit = self.config.context_token_limit.max(1000);
        let compaction_threshold = ((token_limit as f64) * 0.8) as usize;
        let total_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();

        if total_tokens > compaction_threshold {
            tracing::warn!(
                "⚠️ Context guard triggered (~{} > {} [80% of {}])",
                total_tokens,
                compaction_threshold,
                token_limit
            );

            let min_keep_messages = 8usize;
            let mut total = total_tokens;
            let mut dropped: Vec<Message> = Vec::new();

            while total > compaction_threshold && chat_history.len() > min_keep_messages {
                let msg = chat_history.remove(0);
                total = total.saturating_sub(estimate_message_tokens(&msg));
                dropped.push(msg);
            }

            if !dropped.is_empty() {
                match self.summarize_messages(&dropped).await {
                    Ok(summary) => {
                        if let Err(e) = Self::append_compaction_summary_markdown(
                            &msg.session_id,
                            dropped.len(),
                            &summary,
                        ) {
                            tracing::warn!("Failed to append compaction summary: {}", e);
                        }

                        chat_history.insert(
                            0,
                            Message::User {
                                content: OneOrMany::one(UserContent::Text(Text {
                                    text: format!("(Context Summary)\n{}", summary),
                                })),
                            },
                        );

                        let keep_recent = chat_history.len().max(min_keep_messages);
                        match self.context_tree.prune_session(
                            &msg.session_id,
                            keep_recent,
                            keep_recent,
                        ) {
                            Ok(removed) if removed > 0 => {
                                tracing::info!(
                                    "Context compaction removed {} persisted messages for {}",
                                    removed,
                                    msg.session_id
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    "Context compaction could not prune persisted history: {}",
                                    e
                                );
                            }
                        }

                        tracing::info!("✅ Compressed {} messages into summary", dropped.len());
                    }
                    Err(e) => {
                        tracing::error!("Failed to summarize compacted history: {}", e);
                    }
                }
            }
        }

        chat_history.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: msg.content.clone(),
            })),
        });

        let runtime_personality = if self.system_prompt_override.is_none() {
            if let Some(ref p) = self.personality {
                Some(p.clone())
            } else {
                let workspace_dir = crate::workspace::resolve_workspace_dir();
                if workspace_dir.exists() {
                    personality::PersonalityContext::load(&workspace_dir)
                        .await
                        .ok()
                } else {
                    None
                }
            }
        } else {
            None
        };

        let max_loops = 5;
        let mut loop_count = 0;

        loop {
            loop_count += 1;
            if loop_count > max_loops {
                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::TextDelta("\n[System: Max tool loops reached]".to_string()),
                )
                .await;
                emit_terminal(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    TerminalKind::CancelledDone {
                        reason: "max_tool_loops_reached".to_string(),
                    },
                )
                .await;
                break;
            }

            let system_msg = if let Some(ref override_prompt) = self.system_prompt_override {
                format!(
                    "{}\n\n# Available Tools\n{}",
                    override_prompt,
                    crate::tools::executor::get_tool_descriptions()
                )
            } else if let Some(ref personality) = runtime_personality {
                format!(
                    "{}\n\n# Available Tools\n{}",
                    personality.to_preamble(),
                    crate::tools::executor::get_tool_descriptions()
                )
            } else {
                format!(
                    "You are Flowbot, a helpful AI assistant.\n\n# Available Tools\n{}",
                    crate::tools::executor::get_tool_descriptions()
                )
            };

            // Auto-RAG: Retrieve relevant context from Memory (with adaptive limit)
            let context_docs = if bench_mode {
                Vec::new()
            } else {
                match self
                    .memory_manager
                    .search(
                        &msg.content,
                        adaptive_config.rag_doc_count,
                        Some(&msg.tenant_id),
                    )
                    .await
                {
                    Ok(results) => {
                        if !results.is_empty() {
                            tracing::info!("📚 RAG: Found {} relevant documents", results.len());
                        }
                        results
                            .into_iter()
                            .map(|(_score, entry)| Document {
                                id: entry.id,
                                text: entry.content,
                                additional_props: entry.metadata,
                            })
                            .collect()
                    }
                    Err(e) => {
                        tracing::error!("RAG Search failed: {}", e);
                        vec![]
                    }
                }
            };

            let request = CompletionRequest {
                chat_history: OneOrMany::many(chat_history.clone())
                    .expect("Chat history should convert to OneOrMany::Many"),
                preamble: Some(system_msg),
                max_tokens: Some(adaptive_config.max_tokens), // ADAPTIVE
                temperature: Some(0.7),
                tools: vec![],
                tool_choice: None,
                documents: context_docs,
                additional_params: Some(json!({})),
            };

            let selected_provider_for_limit = {
                let provider_guard = self.provider.read().await;
                provider_guard.provider_name().to_string()
            };
            let provider_allowed =
                crate::distributed::allow_provider_request(&selected_provider_for_limit).await;
            if !provider_allowed {
                emit_error_and_done(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    "provider_rate_limited_global",
                    &format!(
                        "global provider rate limit reached for '{}'",
                        selected_provider_for_limit
                    ),
                )
                .await;
                break;
            }

            let stream = loop {
                let provider_guard = self.provider.read().await;
                let caps = provider_guard.capabilities();
                let provider_name = provider_guard.provider_name().to_string();
                let upstream_started = std::time::Instant::now();
                match tokio::time::timeout(
                    llm_upstream_connect_timeout(),
                    provider_guard.stream(request.clone()),
                )
                .await
                {
                    Ok(Ok(s)) => {
                        drop(provider_guard);
                        self.mark_provider_healthy().await;
                        break Ok((s, caps, provider_name));
                    }
                    Ok(Err(e)) => {
                        drop(provider_guard);
                        let err_str = e.to_string();
                        let upstream_elapsed = upstream_started.elapsed();
                        if bench_verbose_upstream_enabled() {
                            let provider = bench_provider_name_hint(&self.config.default_provider);
                            let model = bench_model_hint(&provider);
                            let class = classify_upstream_error_class(&err_str);
                            let status = extract_http_status_code(&err_str)
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "none".to_string());
                            tracing::warn!(
                                upstream_class = class,
                                provider = %provider,
                                model = model,
                                upstream_ms = upstream_elapsed.as_millis() as u64,
                                http_status = %status,
                                "Bench upstream failure"
                            );
                        }
                        let verbose_suffix = if bench_verbose_upstream_enabled() {
                            bench_upstream_suffix(&self.config.default_provider, &err_str, upstream_elapsed)
                        } else {
                            String::new()
                        };
                        if let Some((reason, ttl)) = classify_provider_failure(&err_str) {
                            self.mark_provider_unhealthy(reason, ttl).await;
                            break Err(anyhow::anyhow!(
                                "Provider unavailable ({}) - failing fast: {}{}",
                                reason,
                                err_str,
                                verbose_suffix
                            ));
                        }
                        break Err(anyhow::anyhow!("{}{}", err_str, verbose_suffix));
                    }
                    Err(_) => {
                        drop(provider_guard);
                        self.mark_provider_unhealthy("provider_timeout", llm_unhealthy_ttl_timeout())
                            .await;
                        let timeout_msg = format!(
                            "Provider connect timeout after {:?}",
                            llm_upstream_connect_timeout()
                        );
                        if bench_verbose_upstream_enabled() {
                            let provider = bench_provider_name_hint(&self.config.default_provider);
                            let model = bench_model_hint(&provider);
                            tracing::warn!(
                                upstream_class = "timeout",
                                provider = %provider,
                                model = model,
                                upstream_ms = upstream_started.elapsed().as_millis() as u64,
                                http_status = "none",
                                "Bench upstream failure"
                            );
                        }
                        let verbose_suffix = if bench_verbose_upstream_enabled() {
                            bench_upstream_suffix(
                                &self.config.default_provider,
                                &timeout_msg,
                                upstream_started.elapsed(),
                            )
                        } else {
                            String::new()
                        };
                        break Err(anyhow::anyhow!(
                            "{}{}",
                            timeout_msg,
                            verbose_suffix
                        ));
                    }
                }
            };

            let (mut stream, provider_caps, active_provider_name) = match stream {
                Ok(v) => v,
                Err(e) => {
                    emit_error_and_done(
                        &msg.response_tx,
                        &msg.session_id,
                        &msg.request_id,
                        "provider_setup_failed",
                        &e.to_string(),
                    )
                    .await;
                    break;
                }
            };

            if !provider_caps.supports_streaming {
                emit_error_and_done(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    "provider_streaming_unsupported",
                    &format!(
                        "provider '{}' does not support streaming for this runtime",
                        active_provider_name
                    ),
                )
                .await;
                break;
            }

            let mut tool_calls: Vec<(String, Value)> = Vec::new();
            let mut current_text = String::new();
            let mut saw_text_chunk = false;
            let mut stream_error: Option<String> = None;
            let mut stream_message_id: Option<i64> = None;
            let mut stream_persistence_ok = true;
            let mut persist_buffer = String::new();
            let mut persist_buffer_tokens = 0usize;
            let flush_tokens = stream_flush_token_threshold();
            let flush_interval = stream_flush_interval();
            let max_buffer_tokens = stream_flush_max_buffer_tokens();
            let mut last_flush_at = std::time::Instant::now();
            let mut thinking = false;

            let stream_started = std::time::Instant::now();
            let mut stream_timed_out = false;
            loop {
                let total_elapsed = stream_started.elapsed();
                if total_elapsed >= llm_stream_total_timeout() {
                    stream_timed_out = true;
                    break;
                }

                let remaining_total = llm_stream_total_timeout().saturating_sub(total_elapsed);
                let per_chunk_timeout = std::cmp::min(llm_stream_chunk_timeout(), remaining_total);

                let next_chunk = match tokio::time::timeout(per_chunk_timeout, stream.next()).await {
                    Ok(v) => v,
                    Err(_) => {
                        stream_timed_out = true;
                        break;
                    }
                };

                let Some(chunk_res) = next_chunk else {
                    break;
                };

                match chunk_res {
                    Ok(chunk) => {
                        match chunk {
                            ProviderChunk::TextDelta(content) => {
                                current_text.push_str(&content);
                                if !content.is_empty() {
                                    saw_text_chunk = true;
                                }

                                if stream_persistence_ok && !skip_persistence {
                                    persist_buffer.push_str(&content);
                                    persist_buffer_tokens =
                                        persist_buffer_tokens.saturating_add(estimate_tokens(&content));
                                    let should_flush = persist_buffer_tokens >= flush_tokens
                                        || persist_buffer_tokens >= max_buffer_tokens
                                        || last_flush_at.elapsed() >= flush_interval;
                                    if should_flush {
                                        match self
                                            .flush_stream_assistant_buffer(
                                                &msg.session_id,
                                                &msg.request_id,
                                                &mut stream_message_id,
                                                &persist_buffer,
                                            )
                                            .await
                                        {
                                            Ok(()) => {
                                                persist_buffer.clear();
                                                persist_buffer_tokens = 0;
                                                last_flush_at = std::time::Instant::now();
                                            }
                                            Err(e) => {
                                                stream_persistence_ok = false;
                                                tracing::warn!(
                                                    "Failed to flush assistant stream buffer: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }

                                // Simple parser for <think> blocks
                                // Note: detailed split-tag handling omitted for brevity, assumes tags arrive mostly intact
                                let mut remaining = content.as_str();

                                while !remaining.is_empty() {
                                    if !thinking {
                                        if let Some(start_idx) = remaining.find("<think>") {
                                            if start_idx > 0 {
                                                let pre = &remaining[0..start_idx];
                                                let _ = send_stream_chunk_timed(
                                                    &msg.response_tx,
                                                    StreamChunk::TextDelta(pre.to_string()),
                                                )
                                                .await;
                                            }
                                            thinking = true;
                                            remaining = &remaining[start_idx + 7..];
                                        } else {
                                            // No start tag, normal text
                                            let _ = send_stream_chunk_timed(
                                                &msg.response_tx,
                                                StreamChunk::TextDelta(remaining.to_string()),
                                            )
                                            .await;
                                            break;
                                        }
                                    } else if let Some(end_idx) = remaining.find("</think>") {
                                        let think_content = &remaining[0..end_idx];
                                        let _ = send_stream_chunk_timed(
                                            &msg.response_tx,
                                            StreamChunk::Thinking(think_content.to_string()),
                                        )
                                        .await;
                                        thinking = false;
                                        remaining = &remaining[end_idx + 8..];
                                    } else {
                                        // No end tag, all thinking
                                        let _ = send_stream_chunk_timed(
                                            &msg.response_tx,
                                            StreamChunk::Thinking(remaining.to_string()),
                                        )
                                        .await;
                                        break;
                                    }
                                }
                            }
                            ProviderChunk::ToolCall { name, arguments } => {
                                tool_calls.push((name, arguments));
                            }
                            ProviderChunk::Error(err) => {
                                stream_error = Some(err);
                                break;
                            }
                            ProviderChunk::End => {}
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Stream error: {}", e);
                        stream_error = Some(e.to_string());
                        break;
                    }
                }
            }

            if stream_timed_out {
                self.mark_provider_unhealthy("provider_timeout", llm_unhealthy_ttl_timeout())
                    .await;
                let timeout_text = if bench_verbose_upstream_enabled() {
                    format!(
                        "Server busy: upstream LLM timeout (503). Please retry shortly.{}",
                        bench_upstream_suffix(
                            &self.config.default_provider,
                            "stream chunk timeout",
                            llm_stream_total_timeout(),
                        )
                    )
                } else {
                    "Server busy: upstream LLM timeout (503). Please retry shortly.".to_string()
                };
                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::TextDelta(timeout_text),
                )
                .await;
                emit_terminal(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    TerminalKind::ErrorDone {
                        code: "stream_timeout".to_string(),
                        reason: "upstream stream timeout".to_string(),
                    },
                )
                .await;
                break;
            }

            if let Some((code, reason)) = classify_stream_integrity_error(
                stream_error.as_deref(),
                saw_text_chunk,
                tool_calls.len(),
                provider_caps,
                &active_provider_name,
            ) {
                emit_error_and_done(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    &code,
                    &reason,
                )
                .await;
                break;
            }

            if !skip_persistence && stream_persistence_ok && !persist_buffer.is_empty() {
                match self
                    .flush_stream_assistant_buffer(
                        &msg.session_id,
                        &msg.request_id,
                        &mut stream_message_id,
                        &persist_buffer,
                    )
                    .await
                {
                    Ok(()) => {
                        persist_buffer.clear();
                    }
                    Err(e) => {
                        tracing::warn!("Failed final assistant stream flush: {}", e);
                    }
                }
            }

            // Save assistant response including tool calls
            if !current_text.is_empty() && !skip_persistence {
                if let Err(e) = self
                    .save_message_for_request(
                        &msg.session_id,
                        "assistant",
                        &msg.request_id,
                        &current_text,
                    )
                    .await
                {
                    tracing::warn!("Failed to save assistant message: {}", e);
                }
                chat_history.push(Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(Text {
                        text: current_text.clone(),
                    })),
                });
            } else if !current_text.is_empty() {
                chat_history.push(Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(Text {
                        text: current_text.clone(),
                    })),
                });
            }

            if tool_calls.is_empty() {
                let completion_tokens = estimate_tokens(&current_text) as u64;
                self.log_llm_usage_event(&msg.session_id, total_tokens as u64, completion_tokens);
                emit_terminal(
                    &msg.response_tx,
                    &msg.session_id,
                    &msg.request_id,
                    TerminalKind::SuccessDone,
                )
                .await;
                llm_service_timer.mark_success();
                break;
            }

            // Execute Tools
            for (tool_name, tool_arguments) in tool_calls {
                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::ToolCall(tool_name.clone()),
                )
                .await;

                // Parse args to check valid JSON, but we need to merge with tool name for executor
                let mut args_value = tool_arguments;

                // Add "tool" field to args for legacy executor compatibility
                if let Some(obj) = args_value.as_object_mut() {
                    obj.insert("tool".to_string(), json!(tool_name));
                }

                let tool_input_str = serde_json::to_string(&args_value).unwrap_or_default();

                let result_str = match tool_name.as_str() {
                    "cron" => {
                        match crate::tools::cron::execute_cron_tool(
                            &self.cron_scheduler,
                            &args_value,
                        )
                        .await
                        {
                            Ok(res) => res,
                            Err(e) => format!("Error: {}", e),
                        }
                    }
                    _ => {
                        // Fallback to general executor
                        match crate::tools::executor::execute_tool(
                            &tool_input_str,
                            crate::tools::executor::ExecuteToolContext {
                                cron_scheduler: Some(&self.cron_scheduler),
                                agent_manager: Some(&self.agent_manager),
                                memory_manager: Some(&self.memory_manager),
                                persistence: Some(&self.persistence),
                                permission_manager: Some(&*self.permission_manager),
                                tool_policy: Some(&self.tool_policy),
                                confirmation_service: Some(&*self.confirmation_service),
                                skill_loader: self.skill_loader.as_ref(),
                                #[cfg(feature = "browser")]
                                browser_client: self.browser_client.as_ref(),
                                tenant_id: Some(&msg.tenant_id),
                                mcp_manager: self.mcp_manager.as_ref(),
                            },
                        )
                        .await
                        {
                            Ok(res) => res,
                            Err(e) => format!("Error: {}", e),
                        }
                    }
                };

                let _ = send_stream_chunk_timed(
                    &msg.response_tx,
                    StreamChunk::ToolResult(result_str.clone()),
                )
                .await;

                // Add tool result to history for next loop
                // Note: Rig implementation specific, usually need specific ToolResult message
                // For now, we append user message with Tool Result as a simple pattern if Rig types are restrictive,
                // or use proper ToolResult content if available.
                // Let's use User message with "Tool Output" for broad compatibility.
                chat_history.push(Message::User {
                    content: OneOrMany::one(UserContent::Text(Text {
                        text: format!("Tool '{}' Output: {}", tool_name, result_str),
                    })),
                });
            }
            // Loop automatically continues with updated history
        }
    }

    async fn flush_stream_assistant_buffer(
        &self,
        session_id: &str,
        request_id: &str,
        message_id: &mut Option<i64>,
        content_chunk: &str,
    ) -> Result<()> {
        if content_chunk.is_empty() {
            return Ok(());
        }

        if message_id.is_none() {
            *message_id = Some(
                self.start_stream_message(session_id, "assistant", request_id)
                    .await?,
            );
        }

        if let Some(id) = *message_id {
            self.append_stream_message_content(id, content_chunk).await?;
        }

        Ok(())
    }

    async fn start_stream_message(&self, session_id: &str, role: &str, request_id: &str) -> Result<i64> {
        let wait_started = std::time::Instant::now();
        let permit = PERSISTENCE_BLOCKING_SEMAPHORE
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("persistence semaphore closed"))?;
        crate::metrics::GLOBAL_METRICS.record_duration(
            "blocking_semaphore_wait_seconds{pool=persistence}",
            wait_started.elapsed(),
            true,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        let persistence = self.persistence.clone();
        let session_id_owned = session_id.to_string();
        let role_owned = role.to_string();
        let request_id_owned = request_id.to_string();

        let id = tokio::task::spawn_blocking(move || {
            persistence.start_message_for_request(&session_id_owned, &role_owned, &request_id_owned)
        })
            .await
            .map_err(|e| anyhow::anyhow!("persistence task join error: {}", e))??;

        drop(permit);
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        Ok(id)
    }

    async fn append_stream_message_content(&self, message_id: i64, chunk: &str) -> Result<()> {
        let wait_started = std::time::Instant::now();
        let permit = PERSISTENCE_BLOCKING_SEMAPHORE
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("persistence semaphore closed"))?;
        crate::metrics::GLOBAL_METRICS.record_duration(
            "blocking_semaphore_wait_seconds{pool=persistence}",
            wait_started.elapsed(),
            true,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        let persistence = self.persistence.clone();
        let chunk_owned = chunk.to_string();

        tokio::task::spawn_blocking(move || persistence.append_message_content(message_id, &chunk_owned))
            .await
            .map_err(|e| anyhow::anyhow!("persistence task join error: {}", e))??;

        drop(permit);
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        Ok(())
    }

    async fn save_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        let wait_started = std::time::Instant::now();
        let permit = PERSISTENCE_BLOCKING_SEMAPHORE
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("persistence semaphore closed"))?;
        crate::metrics::GLOBAL_METRICS.record_duration(
            "blocking_semaphore_wait_seconds{pool=persistence}",
            wait_started.elapsed(),
            true,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        let context_tree = self.context_tree.clone();
        let session_id_owned = session_id.to_string();
        let role_owned = role.to_string();
        let content_owned = content.to_string();

        tokio::task::spawn_blocking(move || -> Result<()> {
            context_tree.with_transaction(|tx| {
                let parent_id =
                    crate::context::tree::ContextTree::get_active_leaf_tx(tx, &session_id_owned)?;
                crate::context::tree::ContextTree::add_message_in_tx(
                    tx,
                    &session_id_owned,
                    &role_owned,
                    &content_owned,
                    parent_id,
                    None,
                )?;
                crate::persistence::PersistenceManager::save_message_tx(
                    tx,
                    &session_id_owned,
                    &role_owned,
                    &content_owned,
                )?;
                Ok(())
            })?;

            const MAX_NODES: usize = 2000;
            const KEEP_RECENT: usize = 1500;
            if let Ok(count) = context_tree.count_session_nodes(&session_id_owned)
                && count > MAX_NODES
                && let Ok(removed) =
                    context_tree.prune_session(&session_id_owned, MAX_NODES, KEEP_RECENT)
            {
                tracing::info!(
                    "Context prune removed {} nodes for session {}",
                    removed,
                    session_id_owned
                );
            }

            if let Err(e) = Self::append_daily_memory_markdown(
                &session_id_owned,
                &role_owned,
                &content_owned,
            ) {
                tracing::warn!("Failed to append daily memory log: {}", e);
            }

            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("persistence task join error: {}", e))??;

        drop(permit);
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        Ok(())
    }

    async fn save_message_for_request(
        &self,
        session_id: &str,
        role: &str,
        request_id: &str,
        content: &str,
    ) -> Result<()> {
        let wait_started = std::time::Instant::now();
        let permit = PERSISTENCE_BLOCKING_SEMAPHORE
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("persistence semaphore closed"))?;
        crate::metrics::GLOBAL_METRICS.record_duration(
            "blocking_semaphore_wait_seconds{pool=persistence}",
            wait_started.elapsed(),
            true,
        );
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        let context_tree = self.context_tree.clone();
        let session_id_owned = session_id.to_string();
        let role_owned = role.to_string();
        let request_id_owned = request_id.to_string();
        let content_owned = content.to_string();

        let committed_context = tokio::task::spawn_blocking(move || -> Result<bool> {
            context_tree.with_transaction(|tx| {
                let should_commit_context = crate::persistence::PersistenceManager::save_message_tx_for_request(
                    tx,
                    &session_id_owned,
                    &role_owned,
                    &request_id_owned,
                    &content_owned,
                )?;

                if should_commit_context {
                    let parent_id =
                        crate::context::tree::ContextTree::get_active_leaf_tx(tx, &session_id_owned)?;
                    crate::context::tree::ContextTree::add_message_in_tx(
                        tx,
                        &session_id_owned,
                        &role_owned,
                        &content_owned,
                        parent_id,
                        None,
                    )?;
                }

                Ok(should_commit_context)
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("persistence task join error: {}", e))??;

        if committed_context {
            const MAX_NODES: usize = 2000;
            const KEEP_RECENT: usize = 1500;
            if let Ok(count) = self.context_tree.count_session_nodes(session_id)
                && count > MAX_NODES
                && let Ok(removed) =
                    self.context_tree.prune_session(session_id, MAX_NODES, KEEP_RECENT)
            {
                tracing::info!(
                    "Context prune removed {} nodes for session {}",
                    removed,
                    session_id
                );
            }

            if let Err(e) = Self::append_daily_memory_markdown(session_id, role, content) {
                tracing::warn!("Failed to append daily memory log: {}", e);
            }
        }

        drop(permit);
        crate::metrics::GLOBAL_METRICS.set_gauge(
            "blocking_tasks_inflight{pool=persistence}",
            (persistence_blocking_limit()
                .saturating_sub(PERSISTENCE_BLOCKING_SEMAPHORE.available_permits())) as f64,
        );

        Ok(())
    }

    fn append_daily_memory_markdown(session_id: &str, role: &str, content: &str) -> Result<()> {
        use std::io::Write;

        let workspace_dir = crate::workspace::resolve_workspace_dir();
        let memory_dir = workspace_dir.join("memory");
        std::fs::create_dir_all(&memory_dir)?;

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let log_path = memory_dir.join(format!("{}.md", date));

        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        let normalized = content.replace("\r\n", "\n");
        let rendered = normalized
            .lines()
            .map(|l| format!("> {}", l))
            .collect::<Vec<_>>()
            .join("\n");

        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        writeln!(f, "\n### {} [{}] ({})\n{}", ts, role, session_id, rendered)?;
        Ok(())
    }

    fn append_compaction_summary_markdown(
        session_id: &str,
        compacted_messages: usize,
        summary: &str,
    ) -> Result<()> {
        use std::io::Write;

        let workspace_dir = crate::workspace::resolve_workspace_dir();
        let memory_dir = workspace_dir.join("memory");
        std::fs::create_dir_all(&memory_dir)?;

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let log_path = memory_dir.join(format!("{}.md", date));
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();

        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;

        writeln!(
            f,
            "\n### {} [context-compaction] ({})\n- compacted_messages: {}\n\n{}",
            ts, session_id, compacted_messages, summary
        )?;

        Ok(())
    }

    fn log_llm_usage_event(&self, session_id: &str, prompt_tokens: u64, completion_tokens: u64) {
        tracing::info!(
            target: "llm_usage",
            session_id = session_id,
            provider = %self.config.default_provider,
            prompt_tokens,
            completion_tokens,
            total_tokens = prompt_tokens.saturating_add(completion_tokens),
            "llm usage"
        );

        if let Some(path) = self.config.audit_log_path.as_ref() {
            let logger = crate::system::audit::AuditLogger::new(std::path::PathBuf::from(path));
            logger.log_usage(
                session_id,
                &self.config.default_provider,
                "unknown",
                prompt_tokens,
                completion_tokens,
            );
        }
    }

    // Helper to summarize a list of messages
    async fn summarize_messages(&self, msgs: &[Message]) -> Result<String> {
        let text_content: String = msgs
            .iter()
            .map(|m| match m {
                Message::User { content } => content
                    .iter()
                    .map(|c| match c {
                        UserContent::Text(t) => format!("User: {}\n", t.text),
                        _ => "User: [Media]\n".to_string(),
                    })
                    .collect::<String>(),
                Message::Assistant { content, .. } => content
                    .iter()
                    .map(|c| match c {
                        AssistantContent::Text(t) => format!("Assistant: {}\n", t.text),
                        _ => "Assistant: [Media]\n".to_string(),
                    })
                    .collect::<String>(),
            })
            .collect();

        if text_content.is_empty() {
            return Ok("No history.".to_string());
        }

        let prompt = format!(
            "Summarize the following conversation history, retaining key facts, user preferences, and important context, while removing conversational filler:\n\n{}",
            text_content
        );

        let request = CompletionRequest {
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::one(UserContent::text(prompt)),
            }),
            preamble: Some("You are a helpful summarizer.".to_string()),
            max_tokens: Some(500),
            temperature: Some(0.3),
            tools: vec![],
            tool_choice: None,
            documents: vec![],
            additional_params: None,
        };

        // We use the same provider
        let provider_guard = self.provider.read().await;
        let stream = provider_guard.stream(request).await?;

        let mut summary = String::new();
        let mut s = stream;
        while let Some(chunk_res) = s.next().await {
            if let Ok(chunk) = chunk_res
                && let ProviderChunk::TextDelta(t) = chunk
            {
                summary.push_str(&t);
            }
        }

        Ok(summary)
    }

fn get_conversation_history(&self, session_id: &str) -> Vec<Message> {
        // Get active leaf and reconstruct trace
        match self.context_tree.get_active_leaf(session_id) {
            Ok(Some(leaf_id)) => {
                // Get trace from root to leaf
                match self.context_tree.get_trace(&leaf_id) {
                    Ok(nodes) => {
                        // Convert ContextNode to rig::Message
                        nodes
                            .iter()
                            .map(|node| match node.role.as_str() {
                                "user" => Message::User {
                                    content: OneOrMany::one(UserContent::Text(Text {
                                        text: node.content.clone(),
                                    })),
                                },
                                "assistant" => Message::Assistant {
                                    id: None,
                                    content: OneOrMany::one(AssistantContent::Text(Text {
                                        text: node.content.clone(),
                                    })),
                                },
                                _ => Message::User {
                                    content: OneOrMany::one(UserContent::Text(Text {
                                        text: node.content.clone(),
                                    })),
                                },
                            })
                            .collect()
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load conversation trace: {}", e);
                        Vec::new()
                    }
                }
            }
            Ok(None) => {
                // No history for this session yet
                Vec::new()
            }
            Err(e) => {
                tracing::warn!("Failed to get active leaf: {}", e);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static TEST_ENV_LOCK: once_cell::sync::Lazy<Mutex<()>> =
        once_cell::sync::Lazy::new(|| Mutex::new(()));

    fn env_truthy(key: &str) -> bool {
        env::var(key)
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn llm_timeout_increments_rejected_counter() {
        let before = read_counter_from_metrics("llm_rejected_total{reason=semaphore_timeout}");

        let permits_to_take = LLM_TASK_SEMAPHORE.available_permits() as u32;
        let _guard = if permits_to_take > 0 {
            Some(
                LLM_TASK_SEMAPHORE
                    .acquire_many(permits_to_take)
                    .await
                    .expect("failed to acquire semaphore permits"),
            )
        } else {
            None
        };

        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(10)),
        )
        .await
        .expect("acquire_llm_permit should return quickly when saturated");

        assert!(matches!(
            outcome,
            LlmPermitAcquireOutcome::BudgetExceeded { .. }
        ));

        let after = read_counter_from_metrics("llm_rejected_total{reason=semaphore_timeout}");
        assert!(after >= before + 1.0, "expected rejected counter to increase");
    }

    #[tokio::test]
    async fn real_provider_openai_stream_smoke() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");

        let required = env_truthy("NANOBOT_REAL_PROVIDER_SMOKE_REQUIRED");
        let api_key = env::var("OPENAI_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let Some(api_key) = api_key else {
            if required {
                panic!(
                    "NANOBOT_REAL_PROVIDER_SMOKE_REQUIRED=1 but OPENAI_API_KEY is missing"
                );
            }
            eprintln!("skipping real provider smoke: OPENAI_API_KEY is not set");
            return;
        };

        let mut root = std::env::temp_dir();
        root.push(format!(
            "nanobot-real-provider-smoke-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, "default_provider = \"openai\"\n[providers]\n")
            .expect("write config");

        unsafe {
            env::set_var("NANOBOT_CONFIG_PATH", &config_path);
            env::set_var("OPENAI_API_KEY", api_key);
            env::remove_var("NANOBOT_MOCK_PROVIDER");
        }

        let config = crate::config::Config::load().expect("load config");
        let provider = AgentLoop::create_provider(&config, &std::collections::HashMap::new())
            .await
            .expect("create openai provider");

        let request = CompletionRequest {
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::one(UserContent::text(
                    "Reply with one short sentence about Rust safety.",
                )),
            }),
            preamble: Some("You are concise.".to_string()),
            max_tokens: Some(64),
            temperature: Some(0.0),
            tools: vec![],
            tool_choice: None,
            documents: vec![],
            additional_params: None,
        };

        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            provider.stream(request),
        )
        .await
        .expect("provider stream startup timeout")
        .expect("provider stream should initialize");

        let mut text = String::new();
        let mut got_end = false;
        let mut s = stream;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(20), s.next()).await {
                Ok(Some(chunk)) => {
                    let chunk = chunk.expect("provider chunk should be ok");
                    match chunk {
                        ProviderChunk::TextDelta(delta) => text.push_str(&delta),
                        ProviderChunk::End => {
                            got_end = true;
                            break;
                        }
                        ProviderChunk::Error(err) => panic!("provider stream error: {err}"),
                        ProviderChunk::ToolCall { .. } => {
                            panic!("unexpected tool call in basic openai smoke")
                        }
                    }
                }
                Ok(None) => {
                    got_end = true;
                    break;
                }
                Err(_) => break,
            }
        }

        assert!(got_end, "real provider smoke must reach terminal stream end");
        assert!(
            !text.trim().is_empty(),
            "real provider smoke must produce non-empty assistant output"
        );

        unsafe {
            env::remove_var("NANOBOT_CONFIG_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn real_provider_antigravity_stream_smoke() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");

        let required = env_truthy("NANOBOT_REAL_PROVIDER_SMOKE_REQUIRED");
        let mut root = std::env::temp_dir();
        root.push(format!(
            "nanobot-real-antigravity-smoke-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, "default_provider = \"antigravity\"\n[providers]\n")
            .expect("write config");

        unsafe {
            env::set_var("NANOBOT_CONFIG_PATH", &config_path);
            env::remove_var("NANOBOT_MOCK_PROVIDER");
        }

        let config = crate::config::Config::load().expect("load config");
        let provider = match AgentLoop::create_provider(&config, &std::collections::HashMap::new()).await {
            Ok(p) => p,
            Err(err) => {
                if required {
                    panic!(
                        "NANOBOT_REAL_PROVIDER_SMOKE_REQUIRED=1 but antigravity provider init failed: {}",
                        err
                    );
                }
                eprintln!(
                    "skipping antigravity provider smoke: antigravity provider not configured: {}",
                    err
                );
                unsafe {
                    env::remove_var("NANOBOT_CONFIG_PATH");
                }
                let _ = std::fs::remove_dir_all(root);
                return;
            }
        };

        let request = CompletionRequest {
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::one(UserContent::text(
                    "Reply with one short sentence about Rust safety.",
                )),
            }),
            preamble: Some("You are concise.".to_string()),
            max_tokens: Some(64),
            temperature: Some(0.0),
            tools: vec![],
            tool_choice: None,
            documents: vec![],
            additional_params: None,
        };

        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(25),
            provider.stream(request),
        )
        .await
        .expect("provider stream startup timeout")
        .expect("provider stream should initialize");

        let mut text = String::new();
        let mut got_end = false;
        let mut s = stream;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(25), s.next()).await {
                Ok(Some(chunk)) => {
                    let chunk = chunk.expect("provider chunk should be ok");
                    match chunk {
                        ProviderChunk::TextDelta(delta) => text.push_str(&delta),
                        ProviderChunk::End => {
                            got_end = true;
                            break;
                        }
                        ProviderChunk::Error(err) => panic!("provider stream error: {err}"),
                        ProviderChunk::ToolCall { .. } => {
                            panic!("unexpected tool call in basic antigravity smoke")
                        }
                    }
                }
                Ok(None) => {
                    got_end = true;
                    break;
                }
                Err(_) => break,
            }
        }

        assert!(got_end, "antigravity smoke must reach terminal stream end");
        assert!(
            !text.trim().is_empty(),
            "antigravity smoke must produce non-empty assistant output"
        );

        unsafe {
            env::remove_var("NANOBOT_CONFIG_PATH");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn classify_provider_failure_covers_core_modes() {
        let auth = classify_provider_failure("401 Unauthorized: invalid api key")
            .expect("auth failure should be classified");
        assert_eq!(auth.0, "provider_auth");

        let rate = classify_provider_failure("429 rate limit exceeded")
            .expect("rate limit failure should be classified");
        assert_eq!(rate.0, "provider_rate_limited");

        let timeout = classify_provider_failure("request timed out")
            .expect("timeout failure should be classified");
        assert_eq!(timeout.0, "provider_timeout");
    }

    #[tokio::test]
    async fn adaptive_recovery_does_not_step_up_while_unhealthy() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_LLM_ADAPTIVE_PERMITS", "true");
            env::set_var("NANOBOT_LLM_CONCURRENCY_LIMIT", "6");
            env::set_var("NANOBOT_LLM_ADAPTIVE_MIN_PERMITS", "1");
            env::set_var("NANOBOT_LLM_ADAPTIVE_COOLDOWN_MS", "1");
        }

        ADAPTIVE_LLM_PERMIT_LIMIT.store(2, Ordering::Relaxed);
        ADAPTIVE_LAST_UNHEALTHY_MS.store(0, Ordering::Relaxed);
        ADAPTIVE_LAST_STEP_UP_MS.store(0, Ordering::Relaxed);
        ADAPTIVE_PROVIDER_UNHEALTHY.store(true, Ordering::Relaxed);

        adaptive_recovery_tick();
        assert_eq!(ADAPTIVE_LLM_PERMIT_LIMIT.load(Ordering::Relaxed), 2);

        ADAPTIVE_PROVIDER_UNHEALTHY.store(false, Ordering::Relaxed);
        ADAPTIVE_LAST_UNHEALTHY_MS.store(0, Ordering::Relaxed);
        ADAPTIVE_LAST_STEP_UP_MS.store(0, Ordering::Relaxed);
        adaptive_recovery_tick();
        assert_eq!(ADAPTIVE_LLM_PERMIT_LIMIT.load(Ordering::Relaxed), 3);

        unsafe {
            env::remove_var("NANOBOT_LLM_ADAPTIVE_PERMITS");
            env::remove_var("NANOBOT_LLM_CONCURRENCY_LIMIT");
            env::remove_var("NANOBOT_LLM_ADAPTIVE_MIN_PERMITS");
            env::remove_var("NANOBOT_LLM_ADAPTIVE_COOLDOWN_MS");
        }
    }

    #[tokio::test]
    async fn provider_setup_error_emits_error_then_done() {
        let (tx, mut rx) = mpsc::channel(8);
        emit_error_and_done(
            &tx,
            "test-session",
            "test-request",
            "provider_setup_failed",
            "provider init failed",
        )
        .await;

        match rx.recv().await {
            Some(StreamChunk::TextDelta(text)) => {
                assert!(text.contains("Error: provider init failed"));
            }
            other => panic!("expected error text delta, got {other:?}"),
        }

        match rx.recv().await {
            Some(StreamChunk::Done {
                kind: TerminalKind::ErrorDone { .. },
                ..
            }) => {}
            other => panic!("expected done chunk, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn terminal_emission_is_idempotent() {
        let (tx, mut rx) = mpsc::channel(8);
        let session_id = "term-session";
        let request_id = "term-request";

        emit_terminal(&tx, session_id, request_id, TerminalKind::SuccessDone).await;
        emit_terminal(
            &tx,
            session_id,
            request_id,
            TerminalKind::ErrorDone {
                code: "duplicate".to_string(),
                reason: "should_not_emit".to_string(),
            },
        )
        .await;

        match rx.recv().await {
            Some(StreamChunk::Done {
                kind: TerminalKind::SuccessDone,
                ..
            }) => {}
            other => panic!("expected single success terminal, got {other:?}"),
        }

        let second = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(second.is_err(), "duplicate terminal should not emit another frame");
    }

    #[test]
    fn provider_capability_matrix_is_declared() {
        let antigravity = declared_provider_capabilities("antigravity")
            .expect("antigravity capabilities missing");
        assert!(antigravity.supports_streaming);
        assert!(antigravity.supports_tool_calls);

        let openai = declared_provider_capabilities("openai").expect("openai capabilities missing");
        assert!(openai.supports_streaming);
        assert!(openai.supports_tool_calls);

        let google = declared_provider_capabilities("google").expect("google capabilities missing");
        assert!(google.supports_streaming);
        assert!(!google.supports_tool_calls);

        let meta = declared_provider_capabilities("meta").expect("meta capabilities missing");
        assert!(meta.supports_streaming);
        assert!(!meta.supports_tool_calls);

        let mock = declared_provider_capabilities("mock").expect("mock capabilities missing");
        assert!(mock.supports_streaming);
        assert!(!mock.supports_tool_calls);
    }

    #[test]
    fn provider_contract_matrix_declares_tool_behavior() {
        let cases = [
            ("antigravity", true),
            ("openai", true),
            ("google", false),
            ("meta", false),
            ("mock", false),
        ];

        for (provider, supports_tools) in cases {
            let caps = declared_provider_capabilities(provider)
                .unwrap_or_else(|| panic!("missing capabilities for provider {provider}"));
            assert!(caps.supports_streaming, "provider {provider} must support streaming");
            assert_eq!(
                caps.supports_tool_calls, supports_tools,
                "provider {provider} tool-call capability mismatch"
            );
        }
    }

    #[test]
    fn provider_contract_matrix_enforces_unsupported_tool_call_failure() {
        for provider in ["google", "meta", "mock"] {
            let caps = declared_provider_capabilities(provider)
                .unwrap_or_else(|| panic!("missing capabilities for provider {provider}"));
            let err = classify_stream_integrity_error(None, true, 1, caps, provider)
                .expect("unsupported provider tool call must hard-fail");
            assert_eq!(err.0, "provider_tool_calls_unsupported");
        }

        for provider in ["antigravity", "openai"] {
            let caps = declared_provider_capabilities(provider)
                .unwrap_or_else(|| panic!("missing capabilities for provider {provider}"));
            let err = classify_stream_integrity_error(None, true, 1, caps, provider);
            assert!(
                err.is_none(),
                "tool-capable provider {provider} should not fail tool-call integrity check"
            );
        }
    }

    #[test]
    fn parses_prefixed_tool_payload_into_structured_call() {
        let raw = r#"{"function":{"name":"read_file","arguments":{"path":"/tmp/a.txt"}}}"#;
        let (name, args) = parse_prefixed_tool_call_payload(raw).expect("payload should parse");
        assert_eq!(name, "read_file");
        assert_eq!(args.get("path").and_then(|v| v.as_str()), Some("/tmp/a.txt"));
    }

    #[test]
    fn rejects_prefixed_tool_payload_without_function_name() {
        let raw = r#"{"arguments":{"path":"/tmp/a.txt"}}"#;
        let err = parse_prefixed_tool_call_payload(raw).expect_err("payload should fail");
        assert!(err.contains("function.name"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_prefixed_tool_payload_with_non_object_arguments() {
        let raw = r#"{"function":{"name":"read_file","arguments":"{\"path\":\"/tmp/a.txt\"}"}}"#;
        let err = parse_prefixed_tool_call_payload(raw).expect_err("payload should fail");
        assert!(
            err.contains("arguments must be an object"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_prefixed_tool_payload_with_invalid_json() {
        let raw = r#"{"function":{"name":"read_file","arguments":{"path":"/tmp/a.txt"}"#;
        let err = parse_prefixed_tool_call_payload(raw).expect_err("payload should fail");
        assert!(err.contains("invalid tool payload json"), "unexpected error: {err}");
    }

    #[test]
    fn parses_mock_chunk_script_and_rejects_unknown_tokens() {
        let parsed = parse_mock_chunk_script("text:hello,tool:read_file,error:oops,end")
            .expect("script should parse");
        assert_eq!(parsed.len(), 4);
        assert!(matches!(parsed[0], MockChunkSpec::Text(_)));
        assert!(matches!(parsed[1], MockChunkSpec::Tool(_)));
        assert!(matches!(parsed[2], MockChunkSpec::Error(_)));
        assert!(matches!(parsed[3], MockChunkSpec::End));

        let invalid = parse_mock_chunk_script("text:ok,wat");
        assert!(invalid.is_none(), "unknown token should reject script");
    }

    #[test]
    fn parses_mock_chunk_script_sequence() {
        let parsed = parse_mock_chunk_script_sequence("tool:glob,end||text:ok,end")
            .expect("sequence should parse");
        assert_eq!(parsed.len(), 2);
        assert!(matches!(parsed[0][0], MockChunkSpec::Tool(_)));
        assert!(matches!(parsed[1][0], MockChunkSpec::Text(_)));

        let invalid = parse_mock_chunk_script_sequence("tool:glob,end||wat");
        assert!(invalid.is_none(), "invalid sequence token should reject script sequence");
    }

    #[tokio::test]
    async fn e2e_mock_tool_loop_persists_once_per_request_id() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");

        let mut root = std::env::temp_dir();
        root.push(format!(
            "nanobot-agent-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");

        let config_path = root.join("config.toml");
        std::fs::write(&config_path, "default_provider = \"mock\"\n[providers]\n")
            .expect("write config");

        let workspace_dir = root.join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("create workspace");

        let old_cwd = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(&root).expect("set cwd");

        unsafe {
            env::set_var("NANOBOT_CONFIG_PATH", &config_path);
            env::set_var("NANOBOT_WORKSPACE_DIR", &workspace_dir);
            env::set_var("NANOBOT_MOCK_PROVIDER", "1");
            env::set_var("NANOBOT_MOCK_SUPPORTS_TOOL_CALLS", "1");
            env::set_var(
                "NANOBOT_MOCK_CHUNK_SCRIPT_SEQUENCE",
                "tool:glob,end||text:final-tool-loop-answer,end",
            );
            env::set_var("NANOBOT_MOCK_SERVICE_MS", "10");
            env::remove_var("NANOBOT_LLM_BENCH_NO_PERSISTENCE");
        }

        let db_dir = root.join(".nanobot");
        std::fs::create_dir_all(&db_dir).expect("create db dir");
        let db_path = db_dir.join("context_tree.db");
        let bootstrap_pm = crate::persistence::PersistenceManager::new(db_path.clone());
        bootstrap_pm.init().expect("bootstrap persistence schema");

        let agent = AgentLoop::new().await.expect("agent init");
        let (tx, mut rx) = mpsc::channel(64);
        let request_id = "e2e-tool-loop-req".to_string();
        let session_id = "e2e-tool-loop-session".to_string();

        let msg = AgentMessage {
            session_id: session_id.clone(),
            tenant_id: "system".to_string(),
            request_id: request_id.clone(),
            content: "please use a tool then answer".to_string(),
            response_tx: tx,
            ingress_at: std::time::Instant::now(),
        };

        tokio::time::timeout(std::time::Duration::from_secs(8), agent.process_streaming(msg))
            .await
            .expect("agent handle timeout");

        let mut saw_tool_call = false;
        let mut saw_tool_result = false;
        let mut terminal_ok = false;
        let mut assistant_text = String::new();
        while let Ok(Some(chunk)) = tokio::time::timeout(std::time::Duration::from_millis(150), rx.recv()).await {
            match chunk {
                StreamChunk::ToolCall(_) => saw_tool_call = true,
                StreamChunk::ToolResult(_) => saw_tool_result = true,
                StreamChunk::TextDelta(t) => assistant_text.push_str(&t),
                StreamChunk::Done {
                    request_id: rid,
                    kind: TerminalKind::SuccessDone,
                } if rid == request_id => {
                    terminal_ok = true;
                    break;
                }
                _ => {}
            }
        }

        assert!(saw_tool_call, "tool call should be emitted in e2e flow");
        assert!(saw_tool_result, "tool result should be emitted in e2e flow");
        assert!(terminal_ok, "request should terminate with success_done");
        assert!(
            assistant_text.contains("final-tool-loop-answer"),
            "assistant output should be non-empty and meaningful"
        );

        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        let user_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'user' AND request_id = ?2",
                rusqlite::params![session_id, request_id],
                |row| row.get(0),
            )
            .expect("count user rows");
        let assistant_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'assistant' AND request_id = ?2",
                rusqlite::params!["e2e-tool-loop-session", "e2e-tool-loop-req"],
                |row| row.get(0),
            )
            .expect("count assistant rows");
        let user_markers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_request_commits WHERE session_id = ?1 AND role = 'user' AND request_id = ?2",
                rusqlite::params!["e2e-tool-loop-session", "e2e-tool-loop-req"],
                |row| row.get(0),
            )
            .expect("count user markers");
        let assistant_markers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message_request_commits WHERE session_id = ?1 AND role = 'assistant' AND request_id = ?2",
                rusqlite::params!["e2e-tool-loop-session", "e2e-tool-loop-req"],
                |row| row.get(0),
            )
            .expect("count assistant markers");

        assert_eq!(user_rows, 1, "exactly one persisted user row expected");
        assert_eq!(assistant_rows, 1, "exactly one persisted assistant row expected");
        assert_eq!(user_markers, 1, "exactly one user commit marker expected");
        assert_eq!(assistant_markers, 1, "exactly one assistant commit marker expected");

        unsafe {
            env::remove_var("NANOBOT_CONFIG_PATH");
            env::remove_var("NANOBOT_WORKSPACE_DIR");
            env::remove_var("NANOBOT_MOCK_PROVIDER");
            env::remove_var("NANOBOT_MOCK_SUPPORTS_TOOL_CALLS");
            env::remove_var("NANOBOT_MOCK_CHUNK_SCRIPT_SEQUENCE");
            env::remove_var("NANOBOT_MOCK_SERVICE_MS");
        }
        std::env::set_current_dir(old_cwd).expect("restore cwd");
        let _ = std::fs::remove_dir_all(root);
    }

    fn derive_stream_state(chunks: &[ProviderChunk]) -> (Option<String>, bool, usize) {
        let mut stream_error = None;
        let mut saw_text = false;
        let mut tool_calls = 0usize;

        for chunk in chunks {
            match chunk {
                ProviderChunk::TextDelta(text) => {
                    if !text.is_empty() {
                        saw_text = true;
                    }
                }
                ProviderChunk::ToolCall { .. } => {
                    tool_calls += 1;
                }
                ProviderChunk::Error(err) => {
                    stream_error = Some(err.clone());
                    break;
                }
                ProviderChunk::End => {}
            }
        }

        (stream_error, saw_text, tool_calls)
    }

    #[test]
    fn chaos_stream_end_first_classifies_empty_stream_error() {
        let chunks = vec![ProviderChunk::End];
        let (stream_error, saw_text, tool_calls) = derive_stream_state(&chunks);
        let err = classify_stream_integrity_error(
            stream_error.as_deref(),
            saw_text,
            tool_calls,
            ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: false,
            },
            "mock",
        )
        .expect("expected integrity error");
        assert_eq!(err.0, "empty_stream_no_content");
    }

    #[test]
    fn chaos_stream_error_mid_classifies_stream_error() {
        let chunks = vec![
            ProviderChunk::TextDelta("hello".to_string()),
            ProviderChunk::Error("boom".to_string()),
            ProviderChunk::End,
        ];
        let (stream_error, saw_text, tool_calls) = derive_stream_state(&chunks);
        let err = classify_stream_integrity_error(
            stream_error.as_deref(),
            saw_text,
            tool_calls,
            ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: false,
            },
            "mock",
        )
        .expect("expected integrity error");
        assert_eq!(err.0, "stream_error");
    }

    #[test]
    fn chaos_stream_tool_call_from_unsupported_provider_classifies_error() {
        let chunks = vec![ProviderChunk::ToolCall {
            name: "read_file".to_string(),
            arguments: json!({}),
        }];
        let (stream_error, saw_text, tool_calls) = derive_stream_state(&chunks);
        let err = classify_stream_integrity_error(
            stream_error.as_deref(),
            saw_text,
            tool_calls,
            ProviderCapabilities {
                supports_streaming: true,
                supports_tool_calls: false,
            },
            "mock",
        )
        .expect("expected integrity error");
        assert_eq!(err.0, "provider_tool_calls_unsupported");
    }

    #[tokio::test]
    async fn soft_limit_post_acquire_guard_prevents_overshoot() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_LLM_ADAPTIVE_PERMITS", "true");
            env::set_var("NANOBOT_LLM_ADAPTIVE_MIN_PERMITS", "1");
            env::set_var("NANOBOT_LLM_QUEUE_POLL_MS", "1");
        }

        let hard_limit = llm_task_concurrency_limit();
        ADAPTIVE_LLM_PERMIT_LIMIT.store(hard_limit, Ordering::Relaxed);

        let _one_inflight = LLM_TASK_SEMAPHORE
            .acquire()
            .await
            .expect("failed to acquire seed permit");

        TEST_FORCE_SOFT_LIMIT_DROP_ON_ACQUIRE.store(true, Ordering::Relaxed);
        let outcome = tokio::time::timeout(
            std::time::Duration::from_millis(120),
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(80)),
        )
        .await
        .expect("permit acquire should complete");
        TEST_FORCE_SOFT_LIMIT_DROP_ON_ACQUIRE.store(false, Ordering::Relaxed);

        assert!(matches!(
            outcome,
            LlmPermitAcquireOutcome::BudgetExceeded { .. }
        ));

        unsafe {
            env::remove_var("NANOBOT_LLM_ADAPTIVE_PERMITS");
            env::remove_var("NANOBOT_LLM_ADAPTIVE_MIN_PERMITS");
            env::remove_var("NANOBOT_LLM_QUEUE_POLL_MS");
        }
    }

    #[tokio::test]
    async fn admission_fifo_order_is_preserved_under_contention() {
        let hard_limit = llm_task_concurrency_limit();
        ADAPTIVE_LLM_PERMIT_LIMIT.store(hard_limit, Ordering::Relaxed);
        {
            let mut q = LLM_ADMISSION_QUEUE.lock().await;
            q.clear();
        }
        LLM_ADMISSION_QUEUE_NOTIFY.notify_waiters();

        let mut held = Vec::new();
        for _ in 0..hard_limit {
            held.push(
                LLM_TASK_SEMAPHORE
                    .acquire()
                    .await
                    .expect("failed to acquire seed permit"),
            );
        }

        let first = tokio::spawn(async {
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(400)).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let mut second = tokio::spawn(async {
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(400)).await
        });

        held.pop();
        let first_outcome = tokio::time::timeout(std::time::Duration::from_millis(250), first)
            .await
            .expect("first waiter should complete")
            .expect("first waiter join should succeed");
        assert!(matches!(first_outcome, LlmPermitAcquireOutcome::Acquired(_, _)));

        let still_waiting =
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut second).await;
        assert!(still_waiting.is_err(), "second waiter should still be queued before next permit");

        held.pop();
        let second_outcome = tokio::time::timeout(std::time::Duration::from_millis(250), second)
            .await
            .expect("second waiter should complete")
            .expect("second waiter join should succeed");
        assert!(matches!(second_outcome, LlmPermitAcquireOutcome::Acquired(_, _)));
    }

    #[tokio::test]
    async fn timed_out_head_does_not_starve_next_waiter() {
        let hard_limit = llm_task_concurrency_limit();
        ADAPTIVE_LLM_PERMIT_LIMIT.store(hard_limit, Ordering::Relaxed);
        {
            let mut q = LLM_ADMISSION_QUEUE.lock().await;
            q.clear();
        }
        LLM_ADMISSION_QUEUE_NOTIFY.notify_waiters();

        let mut held = Vec::new();
        for _ in 0..hard_limit {
            held.push(
                LLM_TASK_SEMAPHORE
                    .acquire()
                    .await
                    .expect("failed to acquire seed permit"),
            );
        }

        let short = tokio::spawn(async {
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(40)).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let long = tokio::spawn(async {
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(400)).await
        });

        let short_outcome = tokio::time::timeout(std::time::Duration::from_millis(250), short)
            .await
            .expect("short waiter should complete")
            .expect("short waiter join should succeed");
        assert!(matches!(
            short_outcome,
            LlmPermitAcquireOutcome::BudgetExceeded { .. }
        ));

        held.pop();
        let long_outcome = tokio::time::timeout(std::time::Duration::from_millis(250), long)
            .await
            .expect("long waiter should complete")
            .expect("long waiter join should succeed");
        assert!(matches!(long_outcome, LlmPermitAcquireOutcome::Acquired(_, _)));
    }

    #[tokio::test]
    async fn admission_queue_over_capacity_rejects_fast() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_LLM_ADMISSION_QUEUE_MAX", "1");
        }

        let hard_limit = llm_task_concurrency_limit();
        ADAPTIVE_LLM_PERMIT_LIMIT.store(hard_limit, Ordering::Relaxed);
        {
            let mut q = LLM_ADMISSION_QUEUE.lock().await;
            q.clear();
        }
        LLM_ADMISSION_QUEUE_NOTIFY.notify_waiters();

        let mut held = Vec::new();
        for _ in 0..hard_limit {
            held.push(
                LLM_TASK_SEMAPHORE
                    .acquire()
                    .await
                    .expect("failed to acquire seed permit"),
            );
        }

        let first = tokio::spawn(async {
            acquire_llm_permit_with_timeout(std::time::Duration::from_millis(300)).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let second = acquire_llm_permit_with_timeout(std::time::Duration::from_millis(200)).await;
        assert!(matches!(
            second,
            LlmPermitAcquireOutcome::QueueOverCapacity { .. }
        ));

        held.pop();
        let first_outcome = tokio::time::timeout(std::time::Duration::from_millis(250), first)
            .await
            .expect("first waiter should complete")
            .expect("first waiter join should succeed");
        assert!(matches!(first_outcome, LlmPermitAcquireOutcome::Acquired(_, _)));

        unsafe {
            env::remove_var("NANOBOT_LLM_ADMISSION_QUEUE_MAX");
        }
    }

    #[tokio::test]
    async fn global_admission_mode_falls_back_to_local_backend() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        unsafe {
            env::set_var("NANOBOT_ADMISSION_MODE", "global");
            env::remove_var("NANOBOT_ADMISSION_STRICT");
        }

        ADMISSION_GLOBAL_FALLBACK_WARNED.store(false, Ordering::Relaxed);
        let outcome = acquire_llm_permit_with_timeout(std::time::Duration::from_millis(200)).await;
        match outcome {
            LlmPermitAcquireOutcome::Acquired(_permit, _) => {}
            _ => panic!("expected local fallback to acquire permit"),
        }

        assert!(
            ADMISSION_GLOBAL_FALLBACK_WARNED.load(Ordering::Relaxed),
            "global admission fallback warning flag should be set"
        );

        unsafe {
            env::remove_var("NANOBOT_ADMISSION_MODE");
            env::remove_var("NANOBOT_ADMISSION_STRICT");
        }
    }

    #[cfg(feature = "distributed-redis")]
    fn ci_env_enabled() -> bool {
        env::var("CI")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    }

    #[cfg(feature = "distributed-redis")]
    async fn redis_test_url() -> Option<String> {
        let Some(url) = env::var("NANOBOT_REDIS_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            if ci_env_enabled() {
                panic!(
                    "CI requires reachable NANOBOT_REDIS_URL for redis integration tests: env var is missing"
                );
            }
            return None;
        };

        let client = match redis::Client::open(url.clone()) {
            Ok(c) => c,
            Err(err) => {
                if ci_env_enabled() {
                    panic!(
                        "CI requires reachable NANOBOT_REDIS_URL for redis integration tests: invalid URL: {}",
                        err
                    );
                }
                eprintln!("skipping redis harness test: NANOBOT_REDIS_URL invalid: {}", err);
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
                    panic!(
                        "CI requires reachable NANOBOT_REDIS_URL for redis integration tests: connect failed: {}",
                        err
                    );
                }
                eprintln!("skipping redis harness test: redis connect failed: {}", err);
                return None;
            }
            Err(_) => {
                if ci_env_enabled() {
                    panic!(
                        "CI requires reachable NANOBOT_REDIS_URL for redis integration tests: connect timed out"
                    );
                }
                eprintln!("skipping redis harness test: redis connect timed out");
                return None;
            }
        };

        let ping: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        match ping {
            Ok(_) => Some(url),
            Err(err) => {
                if ci_env_enabled() {
                    panic!(
                        "CI requires reachable NANOBOT_REDIS_URL for redis integration tests: ping failed: {}",
                        err
                    );
                }
                eprintln!("skipping redis harness test: redis ping failed: {}", err);
                None
            }
        }
    }

    #[cfg(feature = "distributed-redis")]
    fn read_limiter_counter(metric_name: &str) -> f64 {
        read_counter_from_metrics(metric_name)
    }

    #[cfg(feature = "distributed-redis")]
    fn read_limiter_counter_by_prefix(metric_prefix: &str) -> f64 {
        crate::metrics::GLOBAL_METRICS
            .export_prometheus()
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with(metric_prefix) {
                    trimmed
                        .split_whitespace()
                        .last()
                        .and_then(|v| v.parse::<f64>().ok())
                } else {
                    None
                }
            })
            .sum()
    }

    #[cfg(feature = "distributed-redis")]
    struct NodeLimiterBurstResult {
        total_requests: usize,
        allowed: usize,
        denied: usize,
        success_terminals: usize,
        denied_terminals: usize,
        correlated_terminals: usize,
        malformed_terminals: usize,
        terminal_violations: usize,
        deny_signal_latencies_ms: Vec<f64>,
    }

    #[cfg(feature = "distributed-redis")]
    async fn run_gateway_node_limiter_burst(
        node_id: &str,
        attempts: usize,
    ) -> NodeLimiterBurstResult {
        run_gateway_node_limiter_burst_with_offset(node_id, 0, attempts).await
    }

    #[cfg(feature = "distributed-redis")]
    async fn run_gateway_node_limiter_burst_with_offset(
        node_id: &str,
        start_index: usize,
        attempts: usize,
    ) -> NodeLimiterBurstResult {
        use std::collections::{HashMap, HashSet};

        let mut denied_request_ids = std::collections::HashSet::new();
        let mut terminal_request_ids = std::collections::HashSet::new();
        let (tx, mut rx) = mpsc::channel(256);
        let mut expected_request_ids: HashSet<String> = HashSet::new();
        let deny_latencies = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let mut tasks = Vec::with_capacity(attempts);
        for i in 0..attempts {
            let request_id = format!("{}-req-{}", node_id, start_index + i);
            expected_request_ids.insert(request_id.clone());
            let tx_clone = tx.clone();
            let deny_latencies_clone = std::sync::Arc::clone(&deny_latencies);
            let session_id = format!("session-{}", node_id);
            tasks.push(tokio::spawn(async move {
                let request_started = std::time::Instant::now();
                if crate::distributed::allow_provider_request("openai").await {
                    emit_terminal(
                        &tx_clone,
                        &session_id,
                        &request_id,
                        TerminalKind::SuccessDone,
                    )
                    .await;
                    return (request_id, true);
                }

                let elapsed_ms = request_started.elapsed().as_secs_f64() * 1000.0;
                {
                    let mut latencies = deny_latencies_clone.lock().await;
                    latencies.push(elapsed_ms);
                }
                emit_error_and_done(
                    &tx_clone,
                    &session_id,
                    &request_id,
                    "provider_rate_limited_global",
                    "global provider rate limit reached for 'openai'",
                )
                .await;
                (request_id, false)
            }));
        }

        let mut allowed = 0usize;
        let mut denied = 0usize;
        for task in tasks {
            let (request_id, was_allowed) = task.await.expect("limiter burst task join should succeed");
            if was_allowed {
                allowed += 1;
            } else {
                denied += 1;
                denied_request_ids.insert(request_id);
            }
        }

        drop(tx);

        let mut terminal_counts: HashMap<String, usize> = HashMap::new();
        let mut denied_terminals = 0usize;
        let mut success_terminals = 0usize;
        let mut malformed_terminals = 0usize;
        let mut terminal_violations = 0usize;
        while let Some(chunk) = rx.recv().await {
            if let StreamChunk::Done {
                request_id,
                kind,
            } = chunk
            {
                *terminal_counts.entry(request_id.clone()).or_insert(0) += 1;
                if !expected_request_ids.contains(&request_id) {
                    malformed_terminals += 1;
                    terminal_violations += 1;
                    continue;
                }

                match kind {
                    TerminalKind::SuccessDone => {
                        success_terminals += 1;
                    }
                    TerminalKind::ErrorDone { code, .. }
                        if code == "provider_rate_limited_global" =>
                    {
                        denied_terminals += 1;
                        terminal_request_ids.insert(request_id);
                    }
                    _ => {
                        malformed_terminals += 1;
                    }
                }
            }
        }

        for request_id in &expected_request_ids {
            let count = terminal_counts.get(request_id).copied().unwrap_or(0);
            if count != 1 {
                terminal_violations += 1;
            }
        }

        let correlated_terminals = terminal_request_ids
            .iter()
            .filter(|request_id| denied_request_ids.contains(*request_id))
            .count();

        NodeLimiterBurstResult {
            total_requests: attempts,
            allowed,
            denied,
            success_terminals,
            denied_terminals,
            correlated_terminals,
            malformed_terminals,
            terminal_violations,
            deny_signal_latencies_ms: deny_latencies.lock().await.clone(),
        }
    }

    #[cfg(feature = "distributed-redis")]
    fn percentile_ms(samples: &[f64], percentile: f64) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len().saturating_sub(1)) as f64 * percentile).round() as usize;
        sorted[idx.min(sorted.len().saturating_sub(1))]
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn redis_two_node_harness_enforces_aggregate_qps_with_clean_terminals() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!(
                    "CI requires NANOBOT_REDIS_URL to run redis integration harness tests"
                );
            }
            eprintln!("skipping redis harness test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        crate::metrics::GLOBAL_METRICS.reset();

        let qps = 8usize;
        let attempts_per_node = 20usize;
        let unique_prefix = format!(
            "nanobot-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", redis_url);
            env::set_var("NANOBOT_REDIS_KEY_PREFIX", unique_prefix);
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", qps.to_string());
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "cluster-shared");
        }

        let deny_metric_before = read_limiter_counter(
            "provider_global_limiter_checks_total{provider=openai,result=deny}",
        );

        while std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_millis()
            > 100
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let b1 = barrier.clone();
        let b2 = barrier.clone();

        let node_a = tokio::spawn(async move {
            b1.wait().await;
            run_gateway_node_limiter_burst("node-a", attempts_per_node).await
        });
        let node_b = tokio::spawn(async move {
            b2.wait().await;
            run_gateway_node_limiter_burst("node-b", attempts_per_node).await
        });

        let a = node_a.await.expect("node a join should succeed");
        let b = node_b.await.expect("node b join should succeed");

        let total_allowed = a.allowed + b.allowed;
        let total_denied = a.denied + b.denied;
        let total_requests = attempts_per_node * 2;

        assert!(
            total_allowed <= qps + 1,
            "aggregate allowed={} should stay near configured qps={} across both nodes",
            total_allowed,
            qps
        );
        assert!(a.denied > 0, "node a should observe global limiter denies");
        assert!(b.denied > 0, "node b should observe global limiter denies");
        assert_eq!(a.total_requests, attempts_per_node, "node a request accounting mismatch");
        assert_eq!(b.total_requests, attempts_per_node, "node b request accounting mismatch");
        assert_eq!(a.success_terminals, a.allowed, "node a success terminal mismatch");
        assert_eq!(b.success_terminals, b.allowed, "node b success terminal mismatch");
        assert_eq!(a.denied_terminals, a.denied, "node a deny terminal mismatch");
        assert_eq!(b.denied_terminals, b.denied, "node b deny terminal mismatch");
        assert_eq!(
            a.correlated_terminals, a.denied,
            "node a denied terminals must be request-correlated"
        );
        assert_eq!(
            b.correlated_terminals, b.denied,
            "node b denied terminals must be request-correlated"
        );
        assert_eq!(a.malformed_terminals, 0, "node a should not emit non-error terminals");
        assert_eq!(b.malformed_terminals, 0, "node b should not emit non-error terminals");
        assert_eq!(a.terminal_violations, 0, "node a terminal contract violated");
        assert_eq!(b.terminal_violations, 0, "node b terminal contract violated");

        let deny_metric_after = read_limiter_counter(
            "provider_global_limiter_checks_total{provider=openai,result=deny}",
        );
        assert!(
            deny_metric_after >= deny_metric_before + total_denied as f64,
            "deny metric should reflect cross-node denies"
        );

        let error_metric_after = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");
        eprintln!(
            "GLOBAL LIMIT TEST SUMMARY:\n  qps_limit: {}\n  total_requests: {}\n  allowed: {}\n  denied: {}\n  node_a_denies: {}\n  node_b_denies: {}\n  malformed_terminals: {}\n  terminal_violations: {}\n  error_metrics: {}",
            qps,
            total_requests,
            total_allowed,
            total_denied,
            a.denied,
            b.denied,
            a.malformed_terminals + b.malformed_terminals,
            a.terminal_violations + b.terminal_violations,
            error_metric_after,
        );
        assert_eq!(
            error_metric_after,
            0.0,
            "normal two-node limiter path should not increment limiter error metrics"
        );

        unsafe {
            env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn redis_failure_mode_open_allows_load_and_records_errors() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        crate::metrics::GLOBAL_METRICS.reset();

        let original_redis_url = env::var("NANOBOT_REDIS_URL").ok();

        let error_before = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", "redis://127.0.0.1:6399/");
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "1");
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "open");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "failure-open");
        }

        let burst =
            run_gateway_node_limiter_burst("node-open", 8).await;

        assert_eq!(burst.allowed, 8, "fail-open should allow all requests");
        assert_eq!(burst.denied, 0, "fail-open should not deny requests");
        assert_eq!(burst.success_terminals, burst.allowed, "fail-open success terminal mismatch");
        assert_eq!(
            burst.denied_terminals, 0,
            "fail-open should not emit rate-limit terminals"
        );
        assert_eq!(burst.malformed_terminals, 0, "fail-open malformed terminals detected");
        assert_eq!(burst.terminal_violations, 0, "fail-open terminal contract violated");

        let error_after = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");
        assert!(
            error_after > error_before,
            "fail-open redis errors should increment limiter error metric"
        );

        unsafe {
            if let Some(v) = original_redis_url {
                env::set_var("NANOBOT_REDIS_URL", v);
            } else {
                env::remove_var("NANOBOT_REDIS_URL");
            }
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn redis_failure_mode_closed_sheds_load_fast_with_terminals() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        crate::metrics::GLOBAL_METRICS.reset();

        let original_redis_url = env::var("NANOBOT_REDIS_URL").ok();

        let error_before = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", "redis://127.0.0.1:6399/");
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "1");
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "failure-closed");
        }

        let started = std::time::Instant::now();
        let burst =
            run_gateway_node_limiter_burst("node-closed", 8).await;
        let elapsed = started.elapsed();

        assert_eq!(
            burst.allowed, 0,
            "fail-closed should deny all requests when redis is down"
        );
        assert_eq!(burst.denied, 8, "fail-closed should shed load immediately");
        assert_eq!(
            burst.denied_terminals, burst.denied,
            "fail-closed denies should emit provider_rate_limited_global terminals"
        );
        assert_eq!(
            burst.success_terminals, 0,
            "fail-closed timeout path should not emit success terminals"
        );
        assert_eq!(
            burst.correlated_terminals, burst.denied,
            "fail-closed deny terminals should remain request-correlated"
        );
        assert_eq!(
            burst.malformed_terminals, 0,
            "fail-closed path should only emit error_done terminals"
        );
        assert_eq!(
            burst.terminal_violations, 0,
            "fail-closed path should preserve terminal contract"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "fail-closed shedding should be fast"
        );

        let error_after = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");
        assert!(
            error_after > error_before,
            "fail-closed redis errors should increment limiter error metric"
        );

        unsafe {
            if let Some(v) = original_redis_url {
                env::set_var("NANOBOT_REDIS_URL", v);
            } else {
                env::remove_var("NANOBOT_REDIS_URL");
            }
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
        }
    }

    #[cfg(feature = "distributed-redis")]
    async fn run_two_node_burst_with_timeout(
        attempts_per_node: usize,
        timeout: std::time::Duration,
    ) -> (NodeLimiterBurstResult, NodeLimiterBurstResult) {
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let b1 = barrier.clone();
        let b2 = barrier.clone();
        let node_a = tokio::spawn(async move {
            b1.wait().await;
            run_gateway_node_limiter_burst("chaos-node-a", attempts_per_node).await
        });
        let node_b = tokio::spawn(async move {
            b2.wait().await;
            run_gateway_node_limiter_burst("chaos-node-b", attempts_per_node).await
        });

        tokio::time::timeout(timeout, async {
            let a = node_a.await.expect("chaos node a join should succeed");
            let b = node_b.await.expect("chaos node b join should succeed");
            (a, b)
        })
        .await
        .expect("chaos scenario exceeded timeout bound")
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn redis_chaos_slow_alive_fail_closed_sheds_with_bounded_latency() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for redis chaos integration tests");
            }
            eprintln!("skipping redis chaos fail-closed test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        crate::metrics::GLOBAL_METRICS.reset();
        crate::distributed::reset_provider_limiter_test_state();

        let panic_before = read_limiter_counter_by_prefix("agent_session_task_panics_total")
            + read_limiter_counter_by_prefix("agent_event_task_panics_total");
        let error_before = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");

        let qps = 2usize;
        let redis_timeout_ms = 100u64;
        let injected_sleep_ms = 300u64;
        let attempts_per_node = 10usize;
        let scenario = "slow_alive_fail_closed";
        let unique_prefix = format!(
            "nanobot-chaos-{}-{}",
            scenario,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", redis_url);
            env::set_var("NANOBOT_REDIS_KEY_PREFIX", unique_prefix);
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", qps.to_string());
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "chaos-shared");
            env::set_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS", redis_timeout_ms.to_string());
            env::set_var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS", injected_sleep_ms.to_string());
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N");
        }

        let (a, b) = run_two_node_burst_with_timeout(attempts_per_node, std::time::Duration::from_secs(15)).await;
        let total_allowed = a.allowed + b.allowed;
        let total_denied = a.denied + b.denied;
        let terminal_violations = a.terminal_violations + b.terminal_violations;
        let malformed_terminals = a.malformed_terminals + b.malformed_terminals;

        let mut deny_latencies_ms = a.deny_signal_latencies_ms.clone();
        deny_latencies_ms.extend(b.deny_signal_latencies_ms.iter().copied());
        let deny_latency_p50 = percentile_ms(&deny_latencies_ms, 0.50);
        let deny_latency_p95 = percentile_ms(&deny_latencies_ms, 0.95);

        let error_after = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");
        let panic_after = read_limiter_counter_by_prefix("agent_session_task_panics_total")
            + read_limiter_counter_by_prefix("agent_event_task_panics_total");

        let summary = json!({
            "scenario": scenario,
            "qps_limit": qps,
            "redis_timeout_ms": redis_timeout_ms,
            "injected_sleep_ms": injected_sleep_ms,
            "total_requests": attempts_per_node * 2,
            "allowed": total_allowed,
            "denied": total_denied,
            "errors": (error_after - error_before).max(0.0),
            "deny_signal_latency_ms_p50": deny_latency_p50,
            "deny_signal_latency_ms_p95": deny_latency_p95,
            "terminal_violations": terminal_violations,
            "malformed_terminals": malformed_terminals,
        });
        eprintln!("CHAOS_SUMMARY {}", summary);

        assert!(total_denied > 0, "fail-closed chaos should deny requests under redis slowness");
        assert_eq!(malformed_terminals, 0, "malformed terminals must stay at zero");
        assert_eq!(terminal_violations, 0, "terminal contract must hold under chaos");
        assert!(
            deny_latency_p95 <= (redis_timeout_ms as f64 + 250.0),
            "reject signal p95={}ms exceeds bound {}ms",
            deny_latency_p95,
            redis_timeout_ms as f64 + 250.0
        );
        assert!(error_after > error_before, "slow redis timeout should increment limiter errors");
        assert_eq!(panic_after, panic_before, "chaos scenario should not panic gateway/agent tasks");

        unsafe {
            env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS");
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS");
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn redis_chaos_slow_alive_fail_open_allows_and_preserves_contract() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for redis chaos integration tests");
            }
            eprintln!("skipping redis chaos fail-open test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        crate::metrics::GLOBAL_METRICS.reset();
        crate::distributed::reset_provider_limiter_test_state();

        let panic_before = read_limiter_counter_by_prefix("agent_session_task_panics_total")
            + read_limiter_counter_by_prefix("agent_event_task_panics_total");
        let error_before = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");

        let qps = 2usize;
        let redis_timeout_ms = 100u64;
        let injected_sleep_ms = 300u64;
        let attempts_per_node = 10usize;
        let scenario = "slow_alive_fail_open";
        let unique_prefix = format!(
            "nanobot-chaos-{}-{}",
            scenario,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", redis_url);
            env::set_var("NANOBOT_REDIS_KEY_PREFIX", unique_prefix);
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", qps.to_string());
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "open");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "chaos-shared");
            env::set_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS", redis_timeout_ms.to_string());
            env::set_var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS", injected_sleep_ms.to_string());
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N");
        }

        let (a, b) = run_two_node_burst_with_timeout(attempts_per_node, std::time::Duration::from_secs(15)).await;
        let total_allowed = a.allowed + b.allowed;
        let total_denied = a.denied + b.denied;
        let terminal_violations = a.terminal_violations + b.terminal_violations;
        let malformed_terminals = a.malformed_terminals + b.malformed_terminals;
        let error_after = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");
        let panic_after = read_limiter_counter_by_prefix("agent_session_task_panics_total")
            + read_limiter_counter_by_prefix("agent_event_task_panics_total");

        let summary = json!({
            "scenario": scenario,
            "qps_limit": qps,
            "redis_timeout_ms": redis_timeout_ms,
            "injected_sleep_ms": injected_sleep_ms,
            "total_requests": attempts_per_node * 2,
            "allowed": total_allowed,
            "denied": total_denied,
            "errors": (error_after - error_before).max(0.0),
            "deny_signal_latency_ms_p50": 0.0,
            "deny_signal_latency_ms_p95": 0.0,
            "terminal_violations": terminal_violations,
            "malformed_terminals": malformed_terminals,
        });
        eprintln!("CHAOS_SUMMARY {}", summary);

        assert!(total_allowed > 0, "fail-open chaos should allow traffic");
        assert_eq!(malformed_terminals, 0, "malformed terminals must stay at zero");
        assert_eq!(terminal_violations, 0, "terminal contract must hold under chaos");
        assert!(error_after > error_before, "slow redis timeout should increment limiter errors");
        assert_eq!(panic_after, panic_before, "chaos scenario should not panic gateway/agent tasks");

        unsafe {
            env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS");
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS");
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    async fn redis_chaos_flaky_alternating_keeps_system_responsive() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for redis chaos integration tests");
            }
            eprintln!("skipping redis chaos flaky test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        crate::metrics::GLOBAL_METRICS.reset();
        crate::distributed::reset_provider_limiter_test_state();

        let panic_before = read_limiter_counter_by_prefix("agent_session_task_panics_total")
            + read_limiter_counter_by_prefix("agent_event_task_panics_total");
        let error_before = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");

        let qps = 3usize;
        let redis_timeout_ms = 100u64;
        let injected_sleep_ms = 300u64;
        let attempts_per_node = 15usize;
        let scenario = "flaky_alternating";
        let unique_prefix = format!(
            "nanobot-chaos-{}-{}",
            scenario,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", redis_url);
            env::set_var("NANOBOT_REDIS_KEY_PREFIX", unique_prefix);
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", qps.to_string());
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "open");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "chaos-shared");
            env::set_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS", redis_timeout_ms.to_string());
            env::set_var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS", injected_sleep_ms.to_string());
            env::set_var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N", "3");
        }

        let (a, b) = run_two_node_burst_with_timeout(attempts_per_node, std::time::Duration::from_secs(15)).await;
        let total_allowed = a.allowed + b.allowed;
        let total_denied = a.denied + b.denied;
        let terminal_violations = a.terminal_violations + b.terminal_violations;
        let malformed_terminals = a.malformed_terminals + b.malformed_terminals;
        let error_after = read_limiter_counter_by_prefix("provider_global_limiter_errors_total");
        let panic_after = read_limiter_counter_by_prefix("agent_session_task_panics_total")
            + read_limiter_counter_by_prefix("agent_event_task_panics_total");

        let summary = json!({
            "scenario": scenario,
            "qps_limit": qps,
            "redis_timeout_ms": redis_timeout_ms,
            "injected_sleep_ms": injected_sleep_ms,
            "total_requests": attempts_per_node * 2,
            "allowed": total_allowed,
            "denied": total_denied,
            "errors": (error_after - error_before).max(0.0),
            "deny_signal_latency_ms_p50": percentile_ms(&{
                let mut v = a.deny_signal_latencies_ms.clone();
                v.extend(b.deny_signal_latencies_ms.iter().copied());
                v
            }, 0.50),
            "deny_signal_latency_ms_p95": percentile_ms(&{
                let mut v = a.deny_signal_latencies_ms.clone();
                v.extend(b.deny_signal_latencies_ms.iter().copied());
                v
            }, 0.95),
            "terminal_violations": terminal_violations,
            "malformed_terminals": malformed_terminals,
        });
        eprintln!("CHAOS_SUMMARY {}", summary);

        assert!(total_allowed > 0, "flaky chaos scenario should continue allowing some traffic");
        assert!(error_after > error_before, "flaky chaos scenario should record limiter errors");
        assert_eq!(malformed_terminals, 0, "malformed terminals must stay at zero");
        assert_eq!(terminal_violations, 0, "terminal contract must hold under chaos");
        assert_eq!(panic_after, panic_before, "chaos scenario should not panic gateway/agent tasks");

        unsafe {
            env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS");
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_MS");
            env::remove_var("NANOBOT_TEST_REDIS_LUA_SLEEP_EVERY_N");
        }
    }

    #[cfg(feature = "distributed-redis")]
    #[tokio::test]
    #[ignore = "nightly soak"]
    async fn redis_rolling_restart_soak_nightly() {
        let _guard = TEST_ENV_LOCK.lock().expect("env test lock");
        let Some(redis_url) = redis_test_url().await else {
            if ci_env_enabled() {
                panic!("CI requires NANOBOT_REDIS_URL for rolling restart soak");
            }
            eprintln!("skipping rolling restart soak test: NANOBOT_REDIS_URL is not set/reachable");
            return;
        };

        crate::metrics::GLOBAL_METRICS.reset();

        let qps = env::var("NANOBOT_ROLLING_SOAK_QPS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(6);
        let total_waves = env::var("NANOBOT_ROLLING_SOAK_WAVES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v >= 10)
            .unwrap_or(24);
        let requests_per_wave = env::var("NANOBOT_ROLLING_SOAK_REQUESTS_PER_WAVE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(4);
        let redis_timeout_ms = env::var("NANOBOT_ROLLING_SOAK_REDIS_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(150);
        let dedupe_ttl_secs = env::var("NANOBOT_ROLLING_SOAK_DEDUPE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(5);
        let down_start_wave = total_waves * 40 / 100;
        let restart_wave = total_waves * 60 / 100;
        let unique_prefix = format!(
            "nanobot-rolling-soak-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        unsafe {
            env::set_var("NANOBOT_REDIS_URL", redis_url);
            env::set_var("NANOBOT_REDIS_KEY_PREFIX", &unique_prefix);
            env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", qps.to_string());
            env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
            env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
            env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "rolling-restart-cluster");
            env::set_var(
                "NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS",
                redis_timeout_ms.to_string(),
            );
            env::set_var("NANOBOT_TERMINAL_DEDUPE_TTL_SECS", dedupe_ttl_secs.to_string());
        }

        let start_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let started = std::time::Instant::now();

        let mut node_a_request_cursor: usize = 0;
        let mut node_b_request_cursor: usize = 0;
        let mut node_a_down_waves = 0usize;
        let mut node_a_restart_events = 0usize;
        let mut amplification_violations = 0usize;

        let mut total_requests = 0usize;
        let mut total_allowed = 0usize;
        let mut total_denied = 0usize;
        let mut total_success_terminals = 0usize;
        let mut total_denied_terminals = 0usize;
        let mut total_malformed_terminals = 0usize;
        let mut total_terminal_violations = 0usize;

        for wave in 0..total_waves {
            while std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_millis()
                > 100
            {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }

            let node_a_active = wave < down_start_wave || wave >= restart_wave;
            if !node_a_active {
                node_a_down_waves += 1;
            }
            if wave == restart_wave {
                node_a_restart_events += 1;
            }

            let barrier_parties = if node_a_active { 2 } else { 1 };
            let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(barrier_parties));

            let b_barrier = barrier.clone();
            let b_start = node_b_request_cursor;
            node_b_request_cursor += requests_per_wave;
            let node_b = tokio::spawn(async move {
                b_barrier.wait().await;
                run_gateway_node_limiter_burst_with_offset(
                    "rolling-node-b",
                    b_start,
                    requests_per_wave,
                )
                .await
            });

            let node_a = if node_a_active {
                let a_barrier = barrier.clone();
                let a_start = node_a_request_cursor;
                node_a_request_cursor += requests_per_wave;
                Some(tokio::spawn(async move {
                    a_barrier.wait().await;
                    run_gateway_node_limiter_burst_with_offset(
                        "rolling-node-a",
                        a_start,
                        requests_per_wave,
                    )
                    .await
                }))
            } else {
                None
            };

            let b = node_b.await.expect("rolling node b join should succeed");
            let a = match node_a {
                Some(handle) => Some(handle.await.expect("rolling node a join should succeed")),
                None => None,
            };

            let wave_allowed = b.allowed + a.as_ref().map(|r| r.allowed).unwrap_or(0);
            let wave_requests = b.total_requests + a.as_ref().map(|r| r.total_requests).unwrap_or(0);
            if wave_allowed > qps + 1 {
                amplification_violations += 1;
            }

            total_requests += wave_requests;
            total_allowed += wave_allowed;
            total_denied += b.denied + a.as_ref().map(|r| r.denied).unwrap_or(0);
            total_success_terminals +=
                b.success_terminals + a.as_ref().map(|r| r.success_terminals).unwrap_or(0);
            total_denied_terminals +=
                b.denied_terminals + a.as_ref().map(|r| r.denied_terminals).unwrap_or(0);
            total_malformed_terminals +=
                b.malformed_terminals + a.as_ref().map(|r| r.malformed_terminals).unwrap_or(0);
            total_terminal_violations +=
                b.terminal_violations + a.as_ref().map(|r| r.terminal_violations).unwrap_or(0);
        }

        tokio::time::sleep(std::time::Duration::from_secs(dedupe_ttl_secs + 2)).await;

        let client = redis::Client::open(
            env::var("NANOBOT_REDIS_URL").expect("redis url should be present"),
        )
        .expect("redis client should initialize");
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .expect("redis should connect for rolling verification");
        let dedupe_pattern = format!("{}:terminal_dedupe:*", unique_prefix);
        let dedupe_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&dedupe_pattern)
            .query_async(&mut conn)
            .await
            .expect("rolling dedupe key query should succeed");
        let corr_pattern = format!("{}:corr:*", unique_prefix);
        let corr_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&corr_pattern)
            .query_async(&mut conn)
            .await
            .expect("rolling correlation key query should succeed");

        let elapsed = started.elapsed();
        let end_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let total_terminals =
            total_success_terminals + total_denied_terminals + total_malformed_terminals;
        let stuck_requests = total_requests.saturating_sub(total_terminals);

        let summary = json!({
            "schema": 1,
            "scenario": "redis_rolling_restart_soak",
            "status": "pass",
            "start_epoch_ms": start_epoch_ms,
            "end_epoch_ms": end_epoch_ms,
            "elapsed_ms": elapsed.as_millis() as u64,
            "total_waves": total_waves,
            "node_a_down_waves": node_a_down_waves,
            "node_a_restart_events": node_a_restart_events,
            "requests_per_wave": requests_per_wave,
            "qps_limit": qps,
            "total_requests": total_requests,
            "total_allowed": total_allowed,
            "total_denied": total_denied,
            "total_terminals": total_terminals,
            "stuck_requests": stuck_requests,
            "terminal_violations": total_terminal_violations,
            "malformed_terminals": total_malformed_terminals,
            "amplification_violation_count": amplification_violations,
            "dedupe_key_count_end": dedupe_keys.len(),
            "correlation_key_count_end": corr_keys.len(),
        });
        eprintln!("ROLLING_SOAK_SUMMARY {}", summary);
        eprintln!(
            "ROLLING_SOAK_VERDICT schema=1 ok=1 reasons=[] topology={{\"down_waves\":{},\"restart_events\":{}}}",
            node_a_down_waves,
            node_a_restart_events,
        );

        assert!(node_a_down_waves > 0, "rolling soak should include node-a down phase");
        assert_eq!(
            node_a_restart_events, 1,
            "rolling soak should include exactly one restart event"
        );
        assert_eq!(
            total_requests,
            total_allowed + total_denied,
            "request accounting must be exact"
        );
        assert_eq!(
            total_success_terminals,
            total_allowed,
            "success terminals must match allowed requests"
        );
        assert_eq!(
            total_denied_terminals,
            total_denied,
            "deny terminals must match denied requests"
        );
        assert_eq!(
            total_malformed_terminals, 0,
            "malformed terminals must stay at zero"
        );
        assert_eq!(
            total_terminal_violations, 0,
            "terminal contract must hold during rolling restart"
        );
        assert_eq!(stuck_requests, 0, "no requests should be left without a terminal");
        assert_eq!(
            amplification_violations, 0,
            "global limiter should not amplify allows during rolling restart"
        );
        assert!(
            corr_keys.is_empty(),
            "correlation keys must be cleaned up by end of rolling soak"
        );
        assert!(
            dedupe_keys.is_empty(),
            "terminal dedupe keys must expire by end of rolling soak"
        );

        let cleanup_pattern = format!("{}:*", unique_prefix);
        let cleanup_keys: Vec<String> = redis::cmd("KEYS")
            .arg(&cleanup_pattern)
            .query_async(&mut conn)
            .await
            .expect("rolling cleanup key query should succeed");
        if !cleanup_keys.is_empty() {
            let _: () = redis::cmd("DEL")
                .arg(cleanup_keys)
                .query_async(&mut conn)
                .await
                .expect("rolling cleanup delete should succeed");
        }

        unsafe {
            env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
            env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
            env::remove_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS");
            env::remove_var("NANOBOT_TERMINAL_DEDUPE_TTL_SECS");
        }
    }
}
