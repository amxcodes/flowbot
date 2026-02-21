//! Performance monitoring and metrics collection system
//!
//! Provides real-time performance insights:
//! - Request latency tracking
//! - Throughput measurement
//! - Resource utilization monitoring
//! - Custom metrics collection
//! - Prometheus-compatible export

use dashmap::DashMap;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::interval;

/// Metric types supported by the system
#[derive(Debug, Clone)]
pub enum MetricValue {
    Counter(u64),
    Gauge(f64),
    Histogram(Vec<f64>),
    Timing(Duration),
}

/// A single metric measurement
#[derive(Debug, Clone)]
pub struct Metric {
    pub name: String,
    pub value: MetricValue,
    pub timestamp: Instant,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MetricSeriesKey {
    name: String,
    labels: Vec<(String, String)>,
}

/// Performance statistics for operations
#[derive(Debug, Default, Clone)]
pub struct OperationStats {
    pub count: u64,
    pub total_duration: Duration,
    pub min_duration: Option<Duration>,
    pub max_duration: Option<Duration>,
    pub errors: u64,
}

impl OperationStats {
    pub fn record(&mut self, duration: Duration, success: bool) {
        self.count += 1;
        self.total_duration += duration;

        if success {
            self.min_duration = Some(self.min_duration.map_or(duration, |d| d.min(duration)));
            self.max_duration = Some(self.max_duration.map_or(duration, |d| d.max(duration)));
        } else {
            self.errors += 1;
        }
    }

    pub fn avg_duration(&self) -> Option<Duration> {
        if self.count == 0 {
            None
        } else {
            Some(self.total_duration / self.count as u32)
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            (self.count - self.errors) as f64 / self.count as f64
        }
    }
}

/// System resource metrics
#[derive(Debug, Clone)]
pub struct ResourceMetrics {
    pub cpu_usage_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub open_file_descriptors: u64,
    pub thread_count: usize,
}

impl Default for ResourceMetrics {
    fn default() -> Self {
        Self {
            cpu_usage_percent: 0.0,
            memory_used_bytes: 0,
            memory_total_bytes: 0,
            disk_used_bytes: 0,
            disk_total_bytes: 0,
            open_file_descriptors: 0,
            thread_count: 0,
        }
    }
}

fn sanitize_metric_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "metric".to_string();
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
    }
    out
}

fn parse_metric_series(input: &str) -> MetricSeriesKey {
    let trimmed = input.trim();
    if let Some(start) = trimmed.find('{')
        && let Some(end) = trimmed.rfind('}')
        && end > start
    {
        let name = sanitize_metric_name(trimmed[..start].trim());
        let labels_raw = &trimmed[start + 1..end];
        let mut labels = Vec::new();
        for pair in labels_raw.split(',') {
            let p = pair.trim();
            if p.is_empty() {
                continue;
            }
            let mut parts = p.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim();
            let val = parts.next().unwrap_or("").trim();
            if key.is_empty() {
                continue;
            }
            let cleaned_key = sanitize_metric_name(key);
            let cleaned_val = val.trim_matches('"').trim_matches('\'').to_string();
            labels.push((cleaned_key, cleaned_val));
        }
        labels.sort();
        labels.dedup();
        return MetricSeriesKey { name, labels };
    }

    MetricSeriesKey {
        name: sanitize_metric_name(trimmed),
        labels: Vec::new(),
    }
}

fn labels_to_prometheus(labels: &[(String, String)]) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let encoded = labels
        .iter()
        .map(|(k, v)| {
            let escaped = v
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            format!("{}=\"{}\"", k, escaped)
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{}}}", encoded)
}

fn allowed_metric_labels() -> &'static [&'static str] {
    &[
        "backend", "channel", "code", "gateway", "le", "method", "op", "pool",
        "provider", "reason", "result", "route", "stage", "status", "tool", "type",
    ]
}

fn forbidden_metric_labels() -> &'static [&'static str] {
    &[
        "request_id",
        "session_id",
        "user_id",
        "tenant_id",
        "ip",
        "url",
        "path",
        "error",
        "message",
        "stack",
        "nonce",
        "token",
        "model",
    ]
}

fn metrics_label_policy_strict() -> bool {
    if cfg!(test) {
        return true;
    }
    if std::env::var("CI").ok().as_deref() == Some("true") {
        return true;
    }
    std::env::var("NANOBOT_METRICS_LABEL_POLICY_STRICT")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn validate_metric_series(key: &MetricSeriesKey) -> Result<(), String> {
    let allowed = allowed_metric_labels();
    let forbidden = forbidden_metric_labels();

    for (label, value) in &key.labels {
        if forbidden.contains(&label.as_str()) {
            return Err(format!(
                "forbidden metric label '{}' on metric '{}'",
                label, key.name
            ));
        }
        if !allowed.contains(&label.as_str()) {
            return Err(format!(
                "label '{}' not in allowlist for metric '{}'",
                label, key.name
            ));
        }
        if value.len() > 128 {
            return Err(format!(
                "label '{}' value too long ({} chars) on metric '{}'",
                label,
                value.len(),
                key.name
            ));
        }
    }

    Ok(())
}

/// High-performance metrics collector
pub struct MetricsCollector {
    /// Operation statistics
    operations: Arc<DashMap<MetricSeriesKey, OperationStats>>,
    /// Custom metrics
    custom_metrics: Arc<DashMap<MetricSeriesKey, MetricValue>>,
    /// Resource metrics
    resource_metrics: Arc<RwLock<ResourceMetrics>>,
    /// Active timers
    active_timers: Arc<DashMap<String, Instant>>,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        let collector = Self {
            operations: Arc::new(DashMap::new()),
            custom_metrics: Arc::new(DashMap::new()),
            resource_metrics: Arc::new(RwLock::new(ResourceMetrics::default())),
            active_timers: Arc::new(DashMap::new()),
        };

        // Start resource monitoring
        collector.start_resource_monitoring();

        collector
    }

    /// Start a timer for an operation
    pub fn start_timer(&self, operation: &str) {
        self.active_timers
            .insert(operation.to_string(), Instant::now());
    }

    /// Stop timer and record duration
    pub fn stop_timer(&self, operation: &str, success: bool) {
        if let Some((_, start)) = self.active_timers.remove(operation) {
            let duration = start.elapsed();
            self.record_duration(operation, duration, success);
        }
    }

    fn record_policy_violation(&self, reason: &str) {
        let key = parse_metric_series("metrics_policy_violations_total");
        self.custom_metrics
            .entry(key)
            .and_modify(|v| {
                if let MetricValue::Counter(c) = v {
                    *c += 1;
                }
            })
            .or_insert(MetricValue::Counter(1));
        tracing::warn!(reason = reason, "metric label policy violation");
    }

    fn parse_and_validate_series(&self, name: &str) -> Option<MetricSeriesKey> {
        let key = parse_metric_series(name);
        if let Err(reason) = validate_metric_series(&key) {
            if metrics_label_policy_strict() {
                panic!("metric label policy violation: {}", reason);
            }
            self.record_policy_violation(&reason);
            return None;
        }
        Some(key)
    }

    /// Record operation duration directly
    pub fn record_duration(&self, operation: &str, duration: Duration, success: bool) {
        let Some(key) = self.parse_and_validate_series(operation) else {
            return;
        };
        self.operations
            .entry(key)
            .and_modify(|stats| stats.record(duration, success))
            .or_insert_with(|| {
                let mut stats = OperationStats::default();
                stats.record(duration, success);
                stats
            });
    }

    /// Increment a counter metric
    pub fn increment_counter(&self, name: &str, value: u64) {
        let Some(key) = self.parse_and_validate_series(name) else {
            return;
        };
        self.custom_metrics
            .entry(key)
            .and_modify(|v| {
                if let MetricValue::Counter(c) = v {
                    *c += value;
                }
            })
            .or_insert(MetricValue::Counter(value));
    }

    /// Set a gauge metric
    pub fn set_gauge(&self, name: &str, value: f64) {
        let Some(key) = self.parse_and_validate_series(name) else {
            return;
        };
        self.custom_metrics.insert(key, MetricValue::Gauge(value));
    }

    /// Record a timing metric
    pub fn record_timing(&self, name: &str, duration: Duration) {
        let Some(key) = self.parse_and_validate_series(name) else {
            return;
        };
        self.custom_metrics
            .entry(key)
            .and_modify(|v| {
                if let MetricValue::Timing(t) = v {
                    *t = duration;
                }
            })
            .or_insert(MetricValue::Timing(duration));
    }

    /// Get operation statistics
    pub fn get_operation_stats(&self, operation: &str) -> Option<OperationStats> {
        let key = parse_metric_series(operation);
        self.operations.get(&key).map(|s| s.clone())
    }

    /// Get all operation statistics
    pub fn get_all_operation_stats(&self) -> HashMap<String, OperationStats> {
        self.operations
            .iter()
            .map(|entry| {
                let key = entry.key();
                let rendered = if key.labels.is_empty() {
                    key.name.clone()
                } else {
                    let labels = key
                        .labels
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("{}{{{}}}", key.name, labels)
                };
                (rendered, entry.value().clone())
            })
            .collect()
    }

    /// Get custom metric value
    pub fn get_custom_metric(&self, name: &str) -> Option<MetricValue> {
        let key = parse_metric_series(name);
        self.custom_metrics.get(&key).map(|v| v.clone())
    }

    /// Get resource metrics
    pub async fn get_resource_metrics(&self) -> ResourceMetrics {
        self.resource_metrics.read().await.clone()
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.operations.clear();
        self.custom_metrics.clear();
        self.active_timers.clear();
    }

    /// Export metrics in Prometheus format
    pub fn export_prometheus(&self) -> String {
        fn emit_help_and_type(
            lines: &mut Vec<String>,
            emitted_types: &mut BTreeMap<String, &'static str>,
            metric: &str,
            kind: &'static str,
            help: &str,
        ) {
            match emitted_types.get(metric) {
                Some(existing) if *existing != kind => {
                    tracing::warn!(
                        metric = metric,
                        existing_type = *existing,
                        requested_type = kind,
                        "Skipping metric due to Prometheus type conflict"
                    );
                }
                Some(_) => {}
                None => {
                    lines.push(format!("# HELP {} {}", metric, help));
                    lines.push(format!("# TYPE {} {}", metric, kind));
                    emitted_types.insert(metric.to_string(), kind);
                }
            }
        }

        let mut lines = Vec::new();
        let mut emitted_types: BTreeMap<String, &'static str> = BTreeMap::new();

        for entry in self.operations.iter() {
            let key = entry.key();
            let stats = entry.value();
            let labels = labels_to_prometheus(&key.labels);

            let total_name = format!("{}_total", key.name);
            emit_help_and_type(
                &mut lines,
                &mut emitted_types,
                &total_name,
                "counter",
                &format!("Total number of {} operations", key.name),
            );
            lines.push(format!("{}{} {}", total_name, labels, stats.count));

            let duration_name = format!("{}_duration_seconds", key.name);
            emit_help_and_type(
                &mut lines,
                &mut emitted_types,
                &duration_name,
                "counter",
                &format!("Total duration of {} operations", key.name),
            );
            lines.push(format!(
                "{}{} {}",
                duration_name,
                labels,
                stats.total_duration.as_secs_f64()
            ));

            let errors_name = format!("{}_errors_total", key.name);
            emit_help_and_type(
                &mut lines,
                &mut emitted_types,
                &errors_name,
                "counter",
                &format!("Total number of {} errors", key.name),
            );
            lines.push(format!("{}{} {}", errors_name, labels, stats.errors));
        }

        for entry in self.custom_metrics.iter() {
            let key = entry.key();
            let labels = labels_to_prometheus(&key.labels);
            match entry.value() {
                MetricValue::Counter(c) => {
                    emit_help_and_type(
                        &mut lines,
                        &mut emitted_types,
                        &key.name,
                        "counter",
                        &format!("Counter metric {}", key.name),
                    );
                    lines.push(format!("{}{} {}", key.name, labels, c));
                }
                MetricValue::Gauge(g) => {
                    emit_help_and_type(
                        &mut lines,
                        &mut emitted_types,
                        &key.name,
                        "gauge",
                        &format!("Gauge metric {}", key.name),
                    );
                    lines.push(format!("{}{} {}", key.name, labels, g));
                }
                MetricValue::Timing(t) => {
                    let timing_name = format!("{}_seconds", key.name);
                    emit_help_and_type(
                        &mut lines,
                        &mut emitted_types,
                        &timing_name,
                        "gauge",
                        &format!("Timing metric {} in seconds", key.name),
                    );
                    lines.push(format!("{}{} {}", timing_name, labels, t.as_secs_f64()));
                }
                MetricValue::Histogram(values) => {
                    let summary_name = format!("{}_samples", key.name);
                    emit_help_and_type(
                        &mut lines,
                        &mut emitted_types,
                        &summary_name,
                        "gauge",
                        &format!("Histogram sample count for {}", key.name),
                    );
                    lines.push(format!("{}{} {}", summary_name, labels, values.len()));
                }
            }
        }

        lines.sort();
        let mut output = lines.join("\n");
        output.push('\n');
        output
    }

    fn start_resource_monitoring(&self) {
        let resource_metrics = Arc::clone(&self.resource_metrics);

        tokio::spawn(async move {
            let mut sys = sysinfo::System::new_all();
            let mut interval = interval(Duration::from_secs(5));

            loop {
                interval.tick().await;

                sys.refresh_all();

                let cpu_usage = sys.global_cpu_info().cpu_usage() as f64;
                let memory_used = sys.used_memory();
                let memory_total = sys.total_memory();

                let mut metrics = resource_metrics.write().await;
                metrics.cpu_usage_percent = cpu_usage;
                metrics.memory_used_bytes = memory_used;
                metrics.memory_total_bytes = memory_total;
                metrics.thread_count = 0; // Simplified for now
            }
        });
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

// Global metrics collector instance
lazy_static::lazy_static! {
    pub static ref GLOBAL_METRICS: MetricsCollector = MetricsCollector::new();
}

/// Convenience macro for timing operations
#[macro_export]
macro_rules! timed {
    ($name:expr, $block:block) => {{
        $crate::metrics::GLOBAL_METRICS.start_timer($name);
        let result = $block;
        $crate::metrics::GLOBAL_METRICS.stop_timer($name, result.is_ok());
        result
    }};
}

/// Convenience macro for counting
#[macro_export]
macro_rules! count {
    ($name:expr, $value:expr) => {
        $crate::metrics::GLOBAL_METRICS.increment_counter($name, $value)
    };
}

/// Performance report for dashboards
#[derive(Debug, Clone)]
pub struct PerformanceReport {
    pub timestamp: Instant,
    pub operations: HashMap<String, OperationStats>,
    pub resources: ResourceMetrics,
    pub custom_metrics: HashMap<String, MetricValue>,
}

impl MetricsCollector {
    /// Generate a comprehensive performance report
    pub async fn generate_report(&self) -> PerformanceReport {
        PerformanceReport {
            timestamp: Instant::now(),
            operations: self.get_all_operation_stats(),
            resources: self.get_resource_metrics().await,
            custom_metrics: self
                .custom_metrics
                .iter()
                .map(|e| {
                    let key = e.key();
                    let rendered = if key.labels.is_empty() {
                        key.name.clone()
                    } else {
                        let labels = key
                            .labels
                            .iter()
                            .map(|(k, v)| format!("{}={}", k, v))
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("{}{{{}}}", key.name, labels)
                    };
                    (rendered, e.value().clone())
                })
                .collect(),
        }
    }
}

impl PerformanceReport {
    /// Format as human-readable text
    pub fn format_text(&self) -> String {
        let mut output = String::new();

        output.push_str("=== Performance Report ===\n\n");

        // Operations
        output.push_str("Operations:\n");
        for (name, stats) in &self.operations {
            output.push_str(&format!(
                "  {}: {} ops, avg: {:?}, success: {:.1}%\n",
                name,
                stats.count,
                stats.avg_duration(),
                stats.success_rate() * 100.0
            ));
        }

        // Resources
        output.push_str("\nResources:\n");
        output.push_str(&format!(
            "  CPU: {:.1}%\n",
            self.resources.cpu_usage_percent
        ));
        output.push_str(&format!(
            "  Memory: {} MB / {} MB\n",
            self.resources.memory_used_bytes / 1024 / 1024,
            self.resources.memory_total_bytes / 1024 / 1024
        ));
        output.push_str(&format!("  Threads: {}\n", self.resources.thread_count));

        output
    }

    /// Check if performance is healthy
    pub fn is_healthy(&self) -> bool {
        // CPU under 80%
        if self.resources.cpu_usage_percent > 80.0 {
            return false;
        }

        // Memory under 90%
        let memory_usage = if self.resources.memory_total_bytes > 0 {
            self.resources.memory_used_bytes as f64 / self.resources.memory_total_bytes as f64
        } else {
            0.0
        };

        if memory_usage > 0.9 {
            return false;
        }

        // All operations have >95% success rate
        for stats in self.operations.values() {
            if stats.success_rate() < 0.95 && stats.count > 10 {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_collection() {
        let metrics = MetricsCollector::new();

        // Record some operations
        metrics.record_duration("test_op", Duration::from_millis(100), true);
        metrics.record_duration("test_op", Duration::from_millis(150), true);
        metrics.record_duration("test_op", Duration::from_millis(200), false);

        let stats = metrics.get_operation_stats("test_op").unwrap();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.errors, 1);
        assert!(stats.success_rate() > 0.6);
    }

    #[tokio::test]
    async fn test_counter() {
        let metrics = MetricsCollector::new();

        metrics.increment_counter("requests", 1);
        metrics.increment_counter("requests", 1);
        metrics.increment_counter("requests", 1);

        if let Some(MetricValue::Counter(c)) = metrics.get_custom_metric("requests") {
            assert_eq!(c, 3);
        } else {
            panic!("Expected counter metric");
        }
    }

    #[tokio::test]
    async fn test_prometheus_export() {
        let metrics = MetricsCollector::new();

        metrics.record_duration("api_call", Duration::from_millis(100), true);
        metrics.increment_counter("requests", 5);
        metrics.set_gauge("active_connections", 10.0);

        let prometheus = metrics.export_prometheus();
        assert!(prometheus.contains("api_call_total"));
        assert!(prometheus.contains("requests"));
        assert!(prometheus.contains("active_connections"));
    }

    #[tokio::test]
    async fn test_prometheus_label_rendering_and_sanitization() {
        let metrics = MetricsCollector::new();
        metrics.increment_counter("llm_rejected_total{reason=provider timeout,stage=stream}", 2);
        let prometheus = metrics.export_prometheus();
        assert!(
            prometheus.contains("llm_rejected_total{reason=\"provider timeout\",stage=\"stream\"} 2")
                || prometheus.contains(
                    "llm_rejected_total{stage=\"stream\",reason=\"provider timeout\"} 2"
                )
        );
        assert!(!prometheus.contains("llm_rejected_total{reason=provider timeout"));
    }

    #[tokio::test]
    async fn test_prometheus_metric_name_cleanup() {
        let metrics = MetricsCollector::new();
        metrics.increment_counter("gateway.ws/send-latency{stage=send}", 1);
        let prometheus = metrics.export_prometheus();
        assert!(prometheus.contains("gateway_ws_send_latency"));
    }

    #[tokio::test]
    async fn test_prometheus_export_golden_scrape_shape() {
        let metrics = MetricsCollector::new();
        metrics.record_duration("api_call{route=/chat}", Duration::from_millis(100), true);
        metrics.increment_counter("requests_total{route=/ws,code=200}", 3);
        metrics.set_gauge("active_connections", 10.0);

        let scrape = metrics.export_prometheus();
        assert!(scrape.contains("# HELP api_call_total"));
        assert!(scrape.contains("# TYPE api_call_total counter"));
        assert!(scrape.contains("api_call_total{route=\"/chat\"} 1"));
        assert!(scrape.contains("# HELP requests_total"));
        assert!(scrape.contains("requests_total{code=\"200\",route=\"/ws\"} 3"));
        assert!(scrape.contains("# TYPE active_connections gauge"));
        assert!(scrape.contains("active_connections 10"));
    }

    #[tokio::test]
    async fn emit_prometheus_fixture_for_promtool() {
        let metrics = MetricsCollector::new();
        metrics.record_duration("gateway_ws_send_wait_seconds{channel=web,status=ok}", Duration::from_millis(42), true);
        metrics.increment_counter("llm_rejected_total{reason=provider_unhealthy}", 7);
        metrics.set_gauge("llm_permits_current", 3.0);

        let scrape = metrics.export_prometheus();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/prometheus_fixture.prom");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture dir");
        }
        std::fs::write(&path, scrape).expect("write prometheus fixture");
    }

    #[tokio::test]
    async fn metrics_label_policy_rejects_forbidden_labels_in_tests() {
        let metrics = MetricsCollector::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            metrics.increment_counter("llm_rejected_total{request_id=req-123}", 1);
        }));
        assert!(result.is_err(), "forbidden metric label should panic in strict mode");
    }
}
