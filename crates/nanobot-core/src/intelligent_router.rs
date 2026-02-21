//! Fixed intelligent router - Production Ready
//!
//! Simplified version that compiles and integrates

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const MAX_SUCCESSFUL_SEQUENCES: usize = 256;
const MAX_STEP_PATTERN_STATS: usize = 512;

/// Message classification
#[derive(Debug, Clone)]
pub struct MessageClassification {
    pub category: MessageCategory,
    pub urgency: UrgencyLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageCategory {
    Command,
    Question,
    Conversation,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UrgencyLevel {
    Critical,
    High,
    Normal,
}

/// Incoming message
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    // Request metadata is carried for downstream integrations and tracing hooks.
    #[allow(dead_code)]
    pub id: String,
    pub content: String,
    pub user_id: String,
    #[allow(dead_code)]
    pub channel: String,
    #[allow(dead_code)]
    pub timestamp: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    Command,
    Help,
    Debug,
    Execute,
    Chat,
}

#[derive(Debug, Clone)]
pub struct RoutePlan {
    #[allow(dead_code)]
    pub intent: IntentKind,
    pub category: MessageCategory,
    pub urgency: UrgencyLevel,
    #[allow(dead_code)]
    pub confidence: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserLearningProfile {
    pub successful_tasks: u64,
    pub failed_tasks: u64,
    pub repeated_failure_signals: u64,
    pub avg_response_ms: f64,
    pub tool_stats: std::collections::HashMap<String, ToolOutcomeStats>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolOutcomeStats {
    pub success: u64,
    pub fail: u64,
    pub consecutive_success: u64,
    pub consecutive_fail: u64,
}

/// Fixed intelligent router
pub struct IntelligentRouter {
    /// Message classification patterns
    command_patterns: Arc<RwLock<Vec<(regex::Regex, MessageCategory)>>>,

    /// Agent assignments
    agent_assignments: Arc<DashMap<String, String>>, // user_id -> agent_id

    /// Rate limiting
    rate_limits: Arc<DashMap<String, TokenBucket>>,

    /// Queue depths
    queue_depths: Arc<DashMap<UrgencyLevel, usize>>,

    /// Lightweight adaptive profile per user
    learning_profiles: Arc<DashMap<String, UserLearningProfile>>,

    /// Planner telemetry counters
    planner_evaluations: AtomicU64,
    planner_fallback_suggestions: AtomicU64,
    planner_fallback_auto_selected: AtomicU64,
    step_pattern_updates: AtomicU64,

    /// Learned successful step sequences: signature -> usage count
    successful_sequences: Arc<DashMap<String, u64>>,

    /// Learned step-pattern outcomes: tool:action-key -> stats
    step_pattern_stats: Arc<DashMap<String, ToolOutcomeStats>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PlannerTelemetry {
    evaluations: u64,
    fallback_suggestions: u64,
    fallback_auto_selected: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct RouterPersistence {
    profiles: std::collections::HashMap<String, UserLearningProfile>,
    planner: PlannerTelemetry,
    successful_sequences: std::collections::HashMap<String, u64>,
    step_pattern_stats: std::collections::HashMap<String, ToolOutcomeStats>,
}

#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

/// Routing result
#[derive(Debug, Clone)]
pub enum RoutingResult {
    Routed {
        #[allow(dead_code)]
        agent: String,
        #[allow(dead_code)]
        estimated_wait: Duration,
    },
    Throttled {
        retry_after: Duration,
    },
    Command {
        command: String,
    },
}

#[derive(Debug, Clone)]
pub struct ToolPlanCandidate {
    pub tool: String,
    pub decision: crate::intelligent_policy::Decision,
    pub risk: crate::intelligent_policy::RiskLevel,
    pub score: f32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStep {
    pub id: String,
    pub action: String,
    pub dependencies: Vec<String>,
    pub suggested_tool: String,
    pub inferred_args: serde_json::Value,
    pub confidence: f32,
    pub expected_inputs: Vec<String>,
    pub expected_outputs: Vec<String>,
    pub expected_assertions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub prompt: String,
    pub domain: String,
    pub steps: Vec<TaskStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecutionPreview {
    pub id: String,
    pub suggested_tool: String,
    pub ready: bool,
    pub blocked_by: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecutionPreview {
    pub ready_to_run: bool,
    pub steps: Vec<StepExecutionPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanCritique {
    pub issues: Vec<String>,
    pub confidence_average: f32,
}

impl IntelligentRouter {
    pub fn new() -> Self {
        let patterns = vec![
            (
                regex::Regex::new(r"^/(help|status|info)").unwrap(),
                MessageCategory::Command,
            ),
            (
                regex::Regex::new(r"^/(start|stop|restart)").unwrap(),
                MessageCategory::Command,
            ),
        ];

        let router = Self {
            command_patterns: Arc::new(RwLock::new(patterns)),
            agent_assignments: Arc::new(DashMap::new()),
            rate_limits: Arc::new(DashMap::new()),
            queue_depths: Arc::new(DashMap::new()),
            learning_profiles: Arc::new(DashMap::new()),
            planner_evaluations: AtomicU64::new(0),
            planner_fallback_suggestions: AtomicU64::new(0),
            planner_fallback_auto_selected: AtomicU64::new(0),
            step_pattern_updates: AtomicU64::new(0),
            successful_sequences: Arc::new(DashMap::new()),
            step_pattern_stats: Arc::new(DashMap::new()),
        };

        router.load_profiles_from_disk();
        router
    }

    /// Route a message to appropriate agent
    pub async fn route(&self, message: IncomingMessage) -> RoutingResult {
        let plan = self.plan_route(&message).await;

        // Check rate limit
        if let Some(wait_time) = self.check_rate_limit(&message.user_id, plan.urgency).await {
            return RoutingResult::Throttled {
                retry_after: wait_time,
            };
        }

        // Check for commands
        if plan.category == MessageCategory::Command
            && let Some(cmd) = self.extract_command(&message.content)
        {
            return RoutingResult::Command { command: cmd };
        }

        // Get or assign agent
        let agent_id = self.get_or_assign_agent(&message.user_id).await;

        // Calculate estimated wait
        let wait = self.calculate_wait_time(plan.urgency).await;

        // Update queue depth
        self.queue_depths
            .entry(plan.urgency)
            .and_modify(|v| *v = v.saturating_add(1))
            .or_insert(1);

        // Queue depth is an estimate of in-flight work. Decrement after estimated wait
        // so stats do not grow monotonically under sustained traffic.
        let queue_depths = Arc::clone(&self.queue_depths);
        let urgency = plan.urgency;
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            if let Some(mut entry) = queue_depths.get_mut(&urgency) {
                if *entry > 1 {
                    *entry -= 1;
                } else {
                    drop(entry);
                    queue_depths.remove(&urgency);
                }
            }
        });

        RoutingResult::Routed {
            agent: agent_id,
            estimated_wait: wait,
        }
    }

    /// Plan route with light adaptive behavior.
    pub async fn plan_route(&self, message: &IncomingMessage) -> RoutePlan {
        let classification = self.classify(&message.content).await;
        let mut urgency = classification.urgency;
        let content_lower = message.content.to_lowercase();

        if is_repeated_failure_signal(&content_lower) {
            self.learning_profiles
                .entry(message.user_id.clone())
                .and_modify(|p| {
                    p.repeated_failure_signals = p.repeated_failure_signals.saturating_add(1)
                })
                .or_insert_with(|| UserLearningProfile {
                    repeated_failure_signals: 1,
                    ..UserLearningProfile::default()
                });

            if matches!(urgency, UrgencyLevel::Normal) {
                urgency = UrgencyLevel::High;
            }
        }

        if let Some(profile) = self.learning_profiles.get(&message.user_id) {
            let p = profile.value();
            if p.failed_tasks > p.successful_tasks.saturating_add(2)
                && matches!(urgency, UrgencyLevel::Normal)
            {
                urgency = UrgencyLevel::High;
            }
        }

        let intent = classify_intent(&content_lower, classification.category);
        let confidence = estimate_confidence(&content_lower, classification.category, intent);

        RoutePlan {
            intent,
            category: classification.category,
            urgency,
            confidence,
        }
    }

    /// Classify message
    async fn classify(&self, content: &str) -> MessageClassification {
        let content_lower = content.to_lowercase();

        // Check for commands
        let patterns = self.command_patterns.read().await;
        for (pattern, category) in patterns.iter() {
            if pattern.is_match(content) {
                return MessageClassification {
                    category: *category,
                    urgency: UrgencyLevel::High,
                };
            }
        }
        drop(patterns);

        // Check for questions
        let category = if content_lower.ends_with("?") {
            MessageCategory::Question
        } else if content_lower.contains("error") || content_lower.contains("fail") {
            MessageCategory::Task
        } else {
            MessageCategory::Conversation
        };

        // Determine urgency
        let urgency = if content_lower.contains("urgent") || content_lower.contains("asap") {
            UrgencyLevel::Critical
        } else if content_lower.contains("help") || content_lower.contains("error") {
            UrgencyLevel::High
        } else {
            UrgencyLevel::Normal
        };

        MessageClassification { category, urgency }
    }

    /// Extract command from message
    fn extract_command(&self, content: &str) -> Option<String> {
        content.split_whitespace().next().map(|s| s.to_string())
    }

    /// Get or assign agent for user
    async fn get_or_assign_agent(&self, user_id: &str) -> String {
        // Check if user already has an agent
        if let Some(agent) = self.agent_assignments.get(user_id) {
            return agent.clone();
        }

        // Assign new agent (simple round-robin logic could go here)
        let agent_id = format!("agent_{}", user_id);
        self.agent_assignments
            .insert(user_id.to_string(), agent_id.clone());

        agent_id
    }

    /// Check rate limit for user
    async fn check_rate_limit(&self, user_id: &str, urgency: UrgencyLevel) -> Option<Duration> {
        let mut limiter = self
            .rate_limits
            .entry(user_id.to_string())
            .or_insert_with(|| TokenBucket {
                tokens: 100.0,
                max_tokens: 100.0,
                refill_rate: 10.0, // 10 per second
                last_refill: Instant::now(),
            })
            .clone();

        // Refill tokens
        let elapsed = limiter.last_refill.elapsed().as_secs_f64();
        limiter.tokens = (limiter.tokens + elapsed * limiter.refill_rate).min(limiter.max_tokens);
        limiter.last_refill = Instant::now();

        let mut token_cost = match urgency {
            UrgencyLevel::Critical => 2.0,
            UrgencyLevel::High => 1.25,
            UrgencyLevel::Normal => 1.0,
        };

        if let Some(profile) = self.learning_profiles.get(user_id)
            && profile.repeated_failure_signals > 0
            && matches!(urgency, UrgencyLevel::High | UrgencyLevel::Critical)
        {
            token_cost = (token_cost - 0.25_f64).max(0.5_f64);
        }

        // Check if allowed
        if limiter.tokens >= token_cost {
            limiter.tokens -= token_cost;
            self.rate_limits.insert(user_id.to_string(), limiter);
            None
        } else {
            let wait_secs = (token_cost - limiter.tokens) / limiter.refill_rate;
            self.rate_limits.insert(user_id.to_string(), limiter);
            Some(Duration::from_secs_f64(wait_secs))
        }
    }

    /// Calculate estimated wait time
    async fn calculate_wait_time(&self, urgency: UrgencyLevel) -> Duration {
        let base_time = match urgency {
            UrgencyLevel::Critical => Duration::from_millis(10),
            UrgencyLevel::High => Duration::from_millis(50),
            UrgencyLevel::Normal => Duration::from_millis(200),
        };

        // Add queue delay
        let queue_depth = self.queue_depths.get(&urgency).map(|v| *v).unwrap_or(0);
        let queue_delay = Duration::from_millis(queue_depth as u64 * 10);

        base_time + queue_delay
    }

    /// Feed task outcome signals for adaptive routing.
    pub fn record_outcome(&self, user_id: &str, success: bool, response_time: Duration) {
        self.record_tool_outcome(user_id, "__general__", success, response_time);
    }

    pub fn record_tool_outcome(
        &self,
        user_id: &str,
        tool: &str,
        success: bool,
        response_time: Duration,
    ) {
        let rt_ms = response_time.as_millis() as f64;
        self.learning_profiles
            .entry(user_id.to_string())
            .and_modify(|p| {
                if success {
                    p.successful_tasks = p.successful_tasks.saturating_add(1);
                } else {
                    p.failed_tasks = p.failed_tasks.saturating_add(1);
                }
                if p.avg_response_ms <= 0.0 {
                    p.avg_response_ms = rt_ms;
                } else {
                    p.avg_response_ms = (p.avg_response_ms * 0.8) + (rt_ms * 0.2);
                }
                let entry = p.tool_stats.entry(tool.to_string()).or_default();
                if success {
                    entry.success = entry.success.saturating_add(1);
                    entry.consecutive_success = entry.consecutive_success.saturating_add(1);
                    entry.consecutive_fail = 0;
                } else {
                    entry.fail = entry.fail.saturating_add(1);
                    entry.consecutive_fail = entry.consecutive_fail.saturating_add(1);
                    entry.consecutive_success = 0;
                }
            })
            .or_insert_with(|| UserLearningProfile {
                successful_tasks: if success { 1 } else { 0 },
                failed_tasks: if success { 0 } else { 1 },
                repeated_failure_signals: 0,
                avg_response_ms: rt_ms,
                tool_stats: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        tool.to_string(),
                        ToolOutcomeStats {
                            success: if success { 1 } else { 0 },
                            fail: if success { 0 } else { 1 },
                            consecutive_success: if success { 1 } else { 0 },
                            consecutive_fail: if success { 0 } else { 1 },
                        },
                    );
                    m
                },
            });

        self.persist_profiles_to_disk();
    }

    pub fn user_profile(&self, user_id: &str) -> Option<UserLearningProfile> {
        self.learning_profiles
            .get(user_id)
            .map(|p| p.value().clone())
    }

    pub fn record_fallback_auto_selected(&self) {
        self.planner_fallback_auto_selected
            .fetch_add(1, Ordering::Relaxed);
        self.persist_profiles_to_disk();
    }

    pub async fn rank_tool_candidates(
        &self,
        user_id: &str,
        candidates: &[&str],
    ) -> Vec<ToolPlanCandidate> {
        self.rank_tool_candidates_with_policy(
            &crate::intelligent_policy::INTELLIGENT_POLICY,
            user_id,
            candidates,
        )
        .await
    }

    pub async fn rank_tool_candidates_with_policy(
        &self,
        policy: &crate::intelligent_policy::IntelligentPolicy,
        user_id: &str,
        candidates: &[&str],
    ) -> Vec<ToolPlanCandidate> {
        self.planner_evaluations.fetch_add(1, Ordering::Relaxed);
        let profile = self.user_profile(user_id).unwrap_or_default();
        let failure_bias = if profile.failed_tasks > profile.successful_tasks {
            -5.0_f32
        } else {
            0.0_f32
        };

        let mut ranked = Vec::with_capacity(candidates.len());
        for tool in candidates {
            let check = policy.check_tool(tool, user_id).await;
            let risk_cost = risk_cost(check.risk_level);
            let decision_cost = decision_cost(check.decision);
            let tool_bias = profile
                .tool_stats
                .get(*tool)
                .map(|stats| tool_success_bias(stats) + tool_streak_bias(stats))
                .unwrap_or(0.0);
            let score = (100.0_f32 - risk_cost - decision_cost + failure_bias + tool_bias).max(0.0);
            ranked.push(ToolPlanCandidate {
                tool: (*tool).to_string(),
                decision: check.decision,
                risk: check.risk_level,
                score,
                reason: check.reason,
            });
        }

        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some(requested) = candidates.first()
            && let Some(top) = ranked.first()
            && top.tool != *requested
        {
            self.planner_fallback_suggestions
                .fetch_add(1, Ordering::Relaxed);
        }

        self.persist_profiles_to_disk();
        ranked
    }

    fn profile_store_path() -> std::path::PathBuf {
        if let Ok(custom) = std::env::var("NANOBOT_INTEL_PROFILE_PATH") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return std::path::PathBuf::from(trimmed);
            }
        }
        crate::workspace::default_workspace_dir()
            .join("intelligence")
            .join("router_profiles.json")
    }

    fn load_profiles_from_disk(&self) {
        let path = Self::profile_store_path();
        let Ok(data) = std::fs::read_to_string(&path) else {
            return;
        };
        if let Ok(persisted) = serde_json::from_str::<RouterPersistence>(&data) {
            for (user, profile) in persisted.profiles {
                self.learning_profiles.insert(user, profile);
            }
            self.planner_evaluations
                .store(persisted.planner.evaluations, Ordering::Relaxed);
            self.planner_fallback_suggestions
                .store(persisted.planner.fallback_suggestions, Ordering::Relaxed);
            self.planner_fallback_auto_selected
                .store(persisted.planner.fallback_auto_selected, Ordering::Relaxed);
            for (sig, count) in persisted.successful_sequences {
                self.successful_sequences.insert(sig, count);
            }
            for (k, v) in persisted.step_pattern_stats {
                self.step_pattern_stats.insert(k, v);
            }
            return;
        }

        // Backward compatibility for previous map-only storage format.
        if let Ok(map) =
            serde_json::from_str::<std::collections::HashMap<String, UserLearningProfile>>(&data)
        {
            for (user, profile) in map {
                self.learning_profiles.insert(user, profile);
            }
        }
    }

    fn persist_profiles_to_disk(&self) {
        let path = Self::profile_store_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let profiles: std::collections::HashMap<String, UserLearningProfile> = self
            .learning_profiles
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        let snapshot = RouterPersistence {
            profiles,
            planner: PlannerTelemetry {
                evaluations: self.planner_evaluations.load(Ordering::Relaxed),
                fallback_suggestions: self.planner_fallback_suggestions.load(Ordering::Relaxed),
                fallback_auto_selected: self.planner_fallback_auto_selected.load(Ordering::Relaxed),
            },
            successful_sequences: self
                .successful_sequences
                .iter()
                .map(|e| (e.key().clone(), *e.value()))
                .collect(),
            step_pattern_stats: self
                .step_pattern_stats
                .iter()
                .map(|e| (e.key().clone(), e.value().clone()))
                .collect(),
        };
        if let Ok(payload) = serde_json::to_string_pretty(&snapshot) {
            let _ = std::fs::write(path, payload);
        }
    }

    /// Get router statistics
    pub async fn get_stats(&self) -> RouterStats {
        RouterStats {
            active_routes: self.agent_assignments.len(),
            queued_messages: self.queue_depths.iter().map(|e| *e.value()).sum(),
            rate_limited_users: self.rate_limits.len(),
        }
    }

    pub fn get_adaptive_stats(&self) -> AdaptiveStats {
        let mut successful = 0_u64;
        let mut failed = 0_u64;
        let mut repeated_failure_users = 0_usize;
        let mut tracked_tools = 0_usize;
        let mut total_tool_success = 0_u64;
        let mut total_tool_fail = 0_u64;

        for profile in self.learning_profiles.iter() {
            successful = successful.saturating_add(profile.successful_tasks);
            failed = failed.saturating_add(profile.failed_tasks);
            if profile.repeated_failure_signals > 0 {
                repeated_failure_users += 1;
            }
            tracked_tools = tracked_tools.saturating_add(profile.tool_stats.len());
            for stats in profile.tool_stats.values() {
                total_tool_success = total_tool_success.saturating_add(stats.success);
                total_tool_fail = total_tool_fail.saturating_add(stats.fail);
            }
        }

        AdaptiveStats {
            users_tracked: self.learning_profiles.len(),
            total_successful_tasks: successful,
            total_failed_tasks: failed,
            repeated_failure_users,
            tracked_tools,
            total_tool_success,
            total_tool_fail,
            learned_sequence_count: self.successful_sequences.len(),
            learned_step_pattern_count: self.step_pattern_stats.len(),
            planner_evaluations: self.planner_evaluations.load(Ordering::Relaxed),
            planner_fallback_suggestions: self.planner_fallback_suggestions.load(Ordering::Relaxed),
            planner_fallback_auto_selected: self
                .planner_fallback_auto_selected
                .load(Ordering::Relaxed),
            top_sequences: self.top_successful_sequences(3),
        }
    }

    /// Build a structured response for built-in command messages.
    pub async fn command_response(&self, command: &str, session_id: &str) -> serde_json::Value {
        let cmd = command.trim().to_ascii_lowercase();
        match cmd.as_str() {
            "/help" => json!({
                "ok": true,
                "command": command,
                "message": "Available commands: /help, /status, /info, /start, /stop, /restart"
            }),
            "/status" | "/info" => {
                let stats = self.get_stats().await;
                let adaptive = self.get_adaptive_stats();
                json!({
                    "ok": true,
                    "command": command,
                    "session_id": session_id,
                    "router": {
                        "active_routes": stats.active_routes,
                        "queued_messages": stats.queued_messages,
                        "rate_limited_users": stats.rate_limited_users,
                    },
                    "adaptive": {
                        "users_tracked": adaptive.users_tracked,
                        "total_successful_tasks": adaptive.total_successful_tasks,
                        "total_failed_tasks": adaptive.total_failed_tasks,
                        "repeated_failure_users": adaptive.repeated_failure_users,
                        "tracked_tools": adaptive.tracked_tools,
                        "total_tool_success": adaptive.total_tool_success,
                        "total_tool_fail": adaptive.total_tool_fail,
                        "learned_sequence_count": adaptive.learned_sequence_count,
                        "learned_step_pattern_count": adaptive.learned_step_pattern_count,
                        "planner_evaluations": adaptive.planner_evaluations,
                        "planner_fallback_suggestions": adaptive.planner_fallback_suggestions,
                        "planner_fallback_auto_selected": adaptive.planner_fallback_auto_selected,
                        "top_sequences": adaptive.top_sequences,
                    }
                })
            }
            "/start" => json!({
                "ok": true,
                "command": command,
                "message": "Session is active. Send a message to begin."
            }),
            "/stop" => json!({
                "ok": true,
                "command": command,
                "message": "No stop action is wired for this channel. Session remains available."
            }),
            "/restart" => json!({
                "ok": true,
                "command": command,
                "message": "No restart action is wired for this channel. Use /status to inspect health."
            }),
            _ => json!({
                "ok": false,
                "command": command,
                "message": "Unknown command"
            }),
        }
    }

    pub fn decompose_task(&self, prompt: &str) -> TaskPlan {
        let normalized = prompt
            .replace("\r\n", "\n")
            .replace(" then ", "\n")
            .replace(" and then ", "\n")
            .replace(";", "\n");

        let mut raw_steps: Vec<String> = normalized
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.trim_start_matches('-')
                    .trim_start_matches('*')
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        if raw_steps.is_empty() {
            raw_steps.push(prompt.trim().to_string());
        }

        let mut steps = Vec::with_capacity(raw_steps.len());
        for (idx, action) in raw_steps.into_iter().enumerate() {
            let step_id = format!("s{}", idx + 1);
            let deps = if idx == 0 {
                Vec::new()
            } else {
                vec![format!("s{}", idx)]
            };
            let suggested_tool = suggest_tool_for_step(&action);
            let inferred_args = infer_args_for_step(&action, &suggested_tool);
            let mut confidence = infer_step_confidence(&action, &suggested_tool);
            let (expected_inputs, expected_outputs) =
                infer_artifact_hints(&action, &suggested_tool, &inferred_args);
            let expected_assertions =
                infer_output_assertions(&action, &suggested_tool, &inferred_args);
            let key = step_pattern_key_parts(&suggested_tool, &action);
            if let Some(stats) = self.step_pattern_stats.get(&key) {
                confidence =
                    (confidence + tool_success_bias(stats.value()) * 0.02).clamp(0.2, 0.98);
            }
            steps.push(TaskStep {
                id: step_id,
                action,
                dependencies: deps,
                suggested_tool,
                inferred_args,
                confidence,
                expected_inputs,
                expected_outputs,
                expected_assertions,
            });
        }

        let domain = classify_task_domain(prompt);
        self.apply_sequence_prior(&domain, &mut steps);

        TaskPlan {
            prompt: prompt.to_string(),
            domain,
            steps,
        }
    }

    pub fn validate_task_plan(&self, plan: &TaskPlan) -> Result<(), String> {
        if plan.steps.is_empty() {
            return Err("task plan has no steps".to_string());
        }

        let known: std::collections::HashSet<&str> =
            plan.steps.iter().map(|s| s.id.as_str()).collect();

        for step in &plan.steps {
            for dep in &step.dependencies {
                if !known.contains(dep.as_str()) {
                    return Err(format!(
                        "step '{}' has unknown dependency '{}'",
                        step.id, dep
                    ));
                }
            }
        }

        // Acyclic validation for dependency graph
        let mut visiting = std::collections::HashSet::new();
        let mut visited = std::collections::HashSet::new();
        let map: std::collections::HashMap<&str, &TaskStep> =
            plan.steps.iter().map(|s| (s.id.as_str(), s)).collect();

        fn dfs<'a>(
            id: &'a str,
            map: &std::collections::HashMap<&'a str, &'a TaskStep>,
            visiting: &mut std::collections::HashSet<&'a str>,
            visited: &mut std::collections::HashSet<&'a str>,
        ) -> bool {
            if visited.contains(id) {
                return true;
            }
            if !visiting.insert(id) {
                return false;
            }
            if let Some(step) = map.get(id) {
                for dep in &step.dependencies {
                    if !dfs(dep.as_str(), map, visiting, visited) {
                        return false;
                    }
                }
            }
            visiting.remove(id);
            visited.insert(id);
            true
        }

        for step in &plan.steps {
            if !dfs(step.id.as_str(), &map, &mut visiting, &mut visited) {
                return Err("task plan dependency cycle detected".to_string());
            }
        }

        Ok(())
    }

    pub fn execution_order(&self, plan: &TaskPlan) -> Result<Vec<TaskStep>, String> {
        self.validate_task_plan(plan)?;

        let preferred = self
            .top_successful_sequences(1)
            .into_iter()
            .next()
            .map(|(sig, count)| {
                if count >= 2 {
                    sig.split("->")
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();

        let mut indegree: std::collections::HashMap<String, usize> =
            plan.steps.iter().map(|s| (s.id.clone(), 0)).collect();
        let mut outgoing: std::collections::HashMap<String, Vec<String>> = plan
            .steps
            .iter()
            .map(|s| (s.id.clone(), Vec::new()))
            .collect();
        let by_id: std::collections::HashMap<String, TaskStep> = plan
            .steps
            .iter()
            .map(|s| (s.id.clone(), s.clone()))
            .collect();

        for step in &plan.steps {
            for dep in &step.dependencies {
                *indegree.entry(step.id.clone()).or_insert(0) += 1;
                outgoing
                    .entry(dep.clone())
                    .and_modify(|v| v.push(step.id.clone()))
                    .or_insert_with(|| vec![step.id.clone()]);
            }
        }

        let mut queue = Vec::new();
        for step in &plan.steps {
            if indegree.get(&step.id).copied().unwrap_or(0) == 0 {
                queue.push(step.id.clone());
            }
        }

        let mut ordered = Vec::with_capacity(plan.steps.len());
        let mut position = 0_usize;
        while !queue.is_empty() {
            let next_idx = queue
                .iter()
                .enumerate()
                .min_by_key(|(_, id)| {
                    if let Some(step) = by_id.get(*id) {
                        preferred
                            .get(position)
                            .and_then(|tool| {
                                if *tool == step.suggested_tool {
                                    Some(0)
                                } else {
                                    preferred
                                        .iter()
                                        .position(|t| t == &step.suggested_tool)
                                        .map(|p| p + 1)
                                }
                            })
                            .unwrap_or(usize::MAX / 2)
                    } else {
                        usize::MAX
                    }
                })
                .map(|(idx, _)| idx)
                .unwrap_or(0);

            let id = queue.remove(next_idx);
            if let Some(step) = by_id.get(&id) {
                ordered.push(step.clone());
            }
            if let Some(children) = outgoing.get(&id) {
                for child in children {
                    if let Some(v) = indegree.get_mut(child) {
                        *v = v.saturating_sub(1);
                        if *v == 0 {
                            queue.push(child.clone());
                        }
                    }
                }
            }
            position = position.saturating_add(1);
        }

        if ordered.len() != plan.steps.len() {
            return Err("unable to compute full execution order".to_string());
        }

        Ok(ordered)
    }

    pub fn rollback_hints(&self, plan: &TaskPlan) -> Vec<String> {
        let mut hints = Vec::new();
        for step in &plan.steps {
            let hint = match step.suggested_tool.as_str() {
                "write_file" | "edit_file" | "apply_patch" => Some(format!(
                    "{}: keep backup or patch diff before applying changes",
                    step.id
                )),
                "run_command" => Some(format!(
                    "{}: capture command output and exit code for rollback decisions",
                    step.id
                )),
                "web_fetch" | "web_search" => Some(format!(
                    "{}: cache fetched response for deterministic retries",
                    step.id
                )),
                _ => None,
            };
            if let Some(h) = hint {
                hints.push(h);
            }
        }
        hints
    }

    pub fn record_successful_sequence(&self, plan: &TaskPlan) {
        let order = self
            .execution_order(plan)
            .unwrap_or_else(|_| plan.steps.clone());
        if order.is_empty() {
            return;
        }
        let sequence = order
            .iter()
            .map(|s| s.suggested_tool.as_str())
            .collect::<Vec<_>>()
            .join("->");
        let signature = format!("{}|{}", plan.domain, sequence);
        self.successful_sequences
            .entry(signature)
            .and_modify(|v| *v = v.saturating_add(1))
            .or_insert(1);
        self.prune_successful_sequences();
        self.persist_profiles_to_disk();
    }

    pub fn record_step_pattern_outcome(&self, step: &TaskStep, success: bool) {
        let key = step_pattern_key(step);
        self.step_pattern_stats
            .entry(key)
            .and_modify(|stats| {
                let attempts = stats.success.saturating_add(stats.fail);
                if attempts > 40 {
                    stats.success = stats.success.saturating_sub(1);
                    stats.fail = stats.fail.saturating_sub(1);
                }
                if success {
                    stats.success = stats.success.saturating_add(1);
                    stats.consecutive_success = stats.consecutive_success.saturating_add(1);
                    stats.consecutive_fail = 0;
                } else {
                    stats.fail = stats.fail.saturating_add(1);
                    stats.consecutive_fail = stats.consecutive_fail.saturating_add(1);
                    stats.consecutive_success = 0;
                }
            })
            .or_insert_with(|| ToolOutcomeStats {
                success: if success { 1 } else { 0 },
                fail: if success { 0 } else { 1 },
                consecutive_success: if success { 1 } else { 0 },
                consecutive_fail: if success { 0 } else { 1 },
            });

        let updates = self.step_pattern_updates.fetch_add(1, Ordering::Relaxed) + 1;
        if updates.is_multiple_of(250) {
            self.decay_step_pattern_stats();
        }
        self.prune_step_pattern_stats();
        self.persist_profiles_to_disk();
    }

    pub fn step_pattern_fail_streak(&self, step: &TaskStep) -> u64 {
        let key = step_pattern_key(step);
        self.step_pattern_stats
            .get(&key)
            .map(|s| s.consecutive_fail)
            .unwrap_or(0)
    }

    pub fn rewrite_step_for_reliability(
        &self,
        step: &TaskStep,
        supported_tools: &std::collections::HashSet<String>,
    ) -> TaskStep {
        let mut rewritten = step.clone();
        let fail_streak = self.step_pattern_fail_streak(step);

        if (fail_streak >= 3 || rewritten.confidence < 0.4)
            && let Some(safer) = self.safer_tool_for_step(&rewritten, supported_tools)
            && safer != rewritten.suggested_tool
        {
            rewritten.suggested_tool = safer;
            rewritten.inferred_args =
                infer_args_for_step(&rewritten.action, &rewritten.suggested_tool);
            let (expected_inputs, expected_outputs) = infer_artifact_hints(
                &rewritten.action,
                &rewritten.suggested_tool,
                &rewritten.inferred_args,
            );
            rewritten.expected_inputs = expected_inputs;
            rewritten.expected_outputs = expected_outputs;
            rewritten.expected_assertions = infer_output_assertions(
                &rewritten.action,
                &rewritten.suggested_tool,
                &rewritten.inferred_args,
            );
            rewritten.confidence = (rewritten.confidence + 0.12).min(0.92);
        }

        rewritten
    }

    pub fn top_successful_sequences(&self, limit: usize) -> Vec<(String, u64)> {
        let mut items = self
            .successful_sequences
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect::<Vec<_>>();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        items.truncate(limit);
        items
    }

    pub fn top_successful_sequences_for_domain(
        &self,
        domain: &str,
        limit: usize,
    ) -> Vec<(String, u64)> {
        let mut items = self
            .successful_sequences
            .iter()
            .filter_map(|e| {
                let key = e.key();
                let (d, seq) = key.split_once('|')?;
                if d == domain {
                    Some((seq.to_string(), *e.value()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| b.1.cmp(&a.1));
        items.truncate(limit);
        items
    }

    fn apply_sequence_prior(&self, plan_domain: &str, steps: &mut [TaskStep]) {
        let Some((sig, count)) = self
            .top_successful_sequences_for_domain(plan_domain, 1)
            .into_iter()
            .next()
        else {
            return;
        };
        if count < 3 {
            return;
        }

        let sequence = sig
            .split("->")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();

        if sequence.len() != steps.len() {
            return;
        }

        for (step, preferred_tool) in steps.iter_mut().zip(sequence.into_iter()) {
            if step.confidence < 0.55 || step.suggested_tool == "task" {
                step.suggested_tool = preferred_tool.clone();
                step.inferred_args = infer_args_for_step(&step.action, &preferred_tool);
                let (expected_inputs, expected_outputs) =
                    infer_artifact_hints(&step.action, &preferred_tool, &step.inferred_args);
                step.expected_inputs = expected_inputs;
                step.expected_outputs = expected_outputs;
                step.expected_assertions =
                    infer_output_assertions(&step.action, &preferred_tool, &step.inferred_args);
                step.confidence = (step.confidence + 0.1).min(0.95);
            }
        }
    }

    fn prune_successful_sequences(&self) {
        if self.successful_sequences.len() <= MAX_SUCCESSFUL_SEQUENCES {
            return;
        }

        let mut entries = self
            .successful_sequences
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        let remove_count = entries.len().saturating_sub(MAX_SUCCESSFUL_SEQUENCES);
        for (k, _) in entries.into_iter().take(remove_count) {
            self.successful_sequences.remove(&k);
        }
    }

    fn prune_step_pattern_stats(&self) {
        if self.step_pattern_stats.len() <= MAX_STEP_PATTERN_STATS {
            return;
        }

        let mut entries = self
            .step_pattern_stats
            .iter()
            .map(|e| {
                let v = e.value();
                (e.key().clone(), v.success.saturating_add(v.fail))
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        let remove_count = entries.len().saturating_sub(MAX_STEP_PATTERN_STATS);
        for (k, _) in entries.into_iter().take(remove_count) {
            self.step_pattern_stats.remove(&k);
        }
    }

    fn decay_step_pattern_stats(&self) {
        for mut entry in self.step_pattern_stats.iter_mut() {
            let stats = entry.value_mut();
            stats.success = (stats.success.saturating_mul(97)) / 100;
            stats.fail = (stats.fail.saturating_mul(97)) / 100;
        }
    }

    pub fn build_execution_preview(
        &self,
        plan: &TaskPlan,
        supported_tools: &std::collections::HashSet<String>,
    ) -> TaskExecutionPreview {
        let ordered = self
            .execution_order(plan)
            .unwrap_or_else(|_| plan.steps.clone());
        let mut completed = std::collections::HashSet::new();
        let mut previews = Vec::with_capacity(ordered.len());

        for step in ordered {
            let mut blocked_by = Vec::new();
            for dep in &step.dependencies {
                if !completed.contains(dep) {
                    blocked_by.push(format!("dependency:{}", dep));
                }
            }
            if !supported_tools.contains(&step.suggested_tool) {
                blocked_by.push(format!("unsupported_tool:{}", step.suggested_tool));
            }

            let ready = blocked_by.is_empty();
            if ready {
                completed.insert(step.id.clone());
            }
            let reason = if ready {
                "step ready".to_string()
            } else {
                "step blocked".to_string()
            };

            previews.push(StepExecutionPreview {
                id: step.id,
                suggested_tool: step.suggested_tool,
                ready,
                blocked_by,
                reason,
            });
        }

        TaskExecutionPreview {
            ready_to_run: previews.iter().all(|p| p.ready),
            steps: previews,
        }
    }

    pub fn critique_task_plan(
        &self,
        plan: &TaskPlan,
        supported_tools: &std::collections::HashSet<String>,
    ) -> PlanCritique {
        let mut issues = Vec::new();
        let mut sum_conf = 0.0_f32;

        for step in &plan.steps {
            sum_conf += step.confidence;
            if step.confidence < 0.45 {
                issues.push(format!(
                    "{} low confidence ({:.2}) for tool '{}'",
                    step.id, step.confidence, step.suggested_tool
                ));
            }
            if !supported_tools.contains(&step.suggested_tool) {
                issues.push(format!(
                    "{} unsupported tool '{}'",
                    step.id, step.suggested_tool
                ));
            }

            match step.suggested_tool.as_str() {
                "read_file" | "edit_file" | "write_file" | "apply_patch" => {
                    if !step
                        .inferred_args
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                    {
                        issues.push(format!("{} missing inferred path", step.id));
                    }
                }
                "web_fetch" => {
                    if !step
                        .inferred_args
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                    {
                        issues.push(format!("{} missing inferred url", step.id));
                    }
                }
                "run_command" => {
                    if !step
                        .inferred_args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                    {
                        issues.push(format!("{} missing inferred command", step.id));
                    }
                }
                _ => {}
            }
        }

        let confidence_average = if plan.steps.is_empty() {
            0.0
        } else {
            sum_conf / plan.steps.len() as f32
        };

        PlanCritique {
            issues,
            confidence_average,
        }
    }

    pub fn refine_task_plan(
        &self,
        plan: &TaskPlan,
        supported_tools: &std::collections::HashSet<String>,
    ) -> TaskPlan {
        let mut refined = plan.clone();
        for step in &mut refined.steps {
            let pattern_key = step_pattern_key(step);
            let pattern_fail_streak = self
                .step_pattern_stats
                .get(&pattern_key)
                .map(|s| s.consecutive_fail)
                .unwrap_or(0);

            if !supported_tools.contains(&step.suggested_tool) {
                let alt = suggest_tool_for_step(&step.action);
                if supported_tools.contains(&alt) {
                    step.suggested_tool = alt;
                } else {
                    step.suggested_tool = "read_file".to_string();
                }
                step.inferred_args = infer_args_for_step(&step.action, &step.suggested_tool);
                let (expected_inputs, expected_outputs) =
                    infer_artifact_hints(&step.action, &step.suggested_tool, &step.inferred_args);
                step.expected_inputs = expected_inputs;
                step.expected_outputs = expected_outputs;
                step.expected_assertions = infer_output_assertions(
                    &step.action,
                    &step.suggested_tool,
                    &step.inferred_args,
                );
                step.confidence = (step.confidence + 0.08).min(0.9);
            }

            if (step.confidence < 0.45 || pattern_fail_streak >= 2)
                && let Some(safer) = self.safer_tool_for_step(step, supported_tools)
            {
                step.suggested_tool = safer;
                step.inferred_args = infer_args_for_step(&step.action, &step.suggested_tool);
                let (expected_inputs, expected_outputs) =
                    infer_artifact_hints(&step.action, &step.suggested_tool, &step.inferred_args);
                step.expected_inputs = expected_inputs;
                step.expected_outputs = expected_outputs;
                step.expected_assertions = infer_output_assertions(
                    &step.action,
                    &step.suggested_tool,
                    &step.inferred_args,
                );
                step.confidence = (step.confidence + 0.07).min(0.9);
            }
        }
        refined
    }

    pub fn safer_tool_for_step(
        &self,
        step: &TaskStep,
        supported_tools: &std::collections::HashSet<String>,
    ) -> Option<String> {
        let candidates: &[&str] = match step.suggested_tool.as_str() {
            "run_command" => &["grep", "read_file", "web_fetch"],
            "apply_patch" | "write_file" | "edit_file" => &["read_file", "grep"],
            "web_fetch" => &["web_search", "read_file"],
            _ => &["read_file", "grep"],
        };
        for c in candidates {
            if supported_tools.contains(*c) {
                return Some((*c).to_string());
            }
        }
        None
    }
}

impl Default for IntelligentRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Router statistics
#[derive(Debug, Clone)]
pub struct RouterStats {
    pub active_routes: usize,
    pub queued_messages: usize,
    pub rate_limited_users: usize,
}

#[derive(Debug, Clone)]
pub struct AdaptiveStats {
    pub users_tracked: usize,
    pub total_successful_tasks: u64,
    pub total_failed_tasks: u64,
    pub repeated_failure_users: usize,
    pub tracked_tools: usize,
    pub total_tool_success: u64,
    pub total_tool_fail: u64,
    pub learned_sequence_count: usize,
    pub learned_step_pattern_count: usize,
    pub planner_evaluations: u64,
    pub planner_fallback_suggestions: u64,
    pub planner_fallback_auto_selected: u64,
    pub top_sequences: Vec<(String, u64)>,
}

// Global intelligent router instance
lazy_static::lazy_static! {
    pub static ref INTELLIGENT_ROUTER: IntelligentRouter = IntelligentRouter::new();
}

fn is_repeated_failure_signal(content_lower: &str) -> bool {
    [
        "still failing",
        "didn't work",
        "did not work",
        "not working",
        "again",
        "retry failed",
    ]
    .iter()
    .any(|p| content_lower.contains(p))
}

fn classify_intent(content_lower: &str, category: MessageCategory) -> IntentKind {
    if category == MessageCategory::Command {
        return IntentKind::Command;
    }
    if content_lower.contains("debug") || content_lower.contains("trace") {
        return IntentKind::Debug;
    }
    if content_lower.contains("run")
        || content_lower.contains("execute")
        || content_lower.contains("apply")
    {
        return IntentKind::Execute;
    }
    if content_lower.contains("help") || content_lower.contains("how") {
        return IntentKind::Help;
    }
    IntentKind::Chat
}

fn estimate_confidence(content_lower: &str, category: MessageCategory, intent: IntentKind) -> f32 {
    if category == MessageCategory::Command && content_lower.starts_with('/') {
        return 0.98;
    }
    let mut score: f32 = 0.62;
    if category == MessageCategory::Question {
        score += 0.08;
    }
    if matches!(intent, IntentKind::Debug | IntentKind::Execute) {
        score += 0.1;
    }
    if content_lower.len() < 6 {
        score -= 0.1;
    }
    score.clamp(0.35, 0.97)
}

fn risk_cost(risk: crate::intelligent_policy::RiskLevel) -> f32 {
    match risk {
        crate::intelligent_policy::RiskLevel::Low => 8.0,
        crate::intelligent_policy::RiskLevel::Medium => 20.0,
        crate::intelligent_policy::RiskLevel::High => 45.0,
        crate::intelligent_policy::RiskLevel::Critical => 70.0,
    }
}

fn decision_cost(decision: crate::intelligent_policy::Decision) -> f32 {
    match decision {
        crate::intelligent_policy::Decision::Allow => 0.0,
        crate::intelligent_policy::Decision::Escalate => 30.0,
        crate::intelligent_policy::Decision::Deny => 100.0,
    }
}

fn tool_success_bias(stats: &ToolOutcomeStats) -> f32 {
    let attempts = stats.success.saturating_add(stats.fail);
    if attempts < 3 {
        return 0.0;
    }
    let rate = stats.success as f32 / attempts as f32;
    ((rate - 0.5) * 12.0).clamp(-6.0, 6.0)
}

fn tool_streak_bias(stats: &ToolOutcomeStats) -> f32 {
    let fail_penalty = (stats.consecutive_fail.min(5) as f32) * 2.0;
    let success_bonus = (stats.consecutive_success.min(5) as f32) * 1.2;
    (success_bonus - fail_penalty).clamp(-8.0, 6.0)
}

fn suggest_tool_for_step(action: &str) -> String {
    let a = action.to_ascii_lowercase();
    if a.contains("search") || a.contains("find") || a.contains("match") {
        return "grep".to_string();
    }
    if a.contains("read") || a.contains("inspect") {
        return "read_file".to_string();
    }
    if a.contains("write") || a.contains("create") {
        return "write_file".to_string();
    }
    if a.contains("edit") || a.contains("update") || a.contains("modify") {
        return "edit_file".to_string();
    }
    if a.contains("patch") {
        return "apply_patch".to_string();
    }
    if a.contains("run") || a.contains("execute") || a.contains("test") {
        return "run_command".to_string();
    }
    if a.contains("fetch") || a.contains("url") || a.contains("http") {
        return "web_fetch".to_string();
    }
    "task".to_string()
}

fn infer_step_confidence(action: &str, suggested_tool: &str) -> f32 {
    let lower = action.to_ascii_lowercase();
    let mut score: f32 = 0.45;

    if lower.len() > 12 {
        score += 0.1;
    }
    if extract_hint_path(action).is_some() || extract_url(action).is_some() {
        score += 0.15;
    }
    if matches!(
        suggested_tool,
        "read_file" | "grep" | "run_command" | "web_fetch" | "edit_file" | "write_file"
    ) {
        score += 0.12;
    }
    if suggested_tool == "task" {
        score -= 0.08;
    }

    score.clamp(0.25, 0.95)
}

fn step_pattern_key(step: &TaskStep) -> String {
    step_pattern_key_parts(&step.suggested_tool, &step.action)
}

fn step_pattern_key_parts(tool: &str, action: &str) -> String {
    let action_key = action
        .split_whitespace()
        .take(6)
        .map(|s| s.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{}:{}", tool, action_key)
}

fn classify_task_domain(prompt: &str) -> String {
    let p = prompt.to_ascii_lowercase();
    if p.contains("http") || p.contains("url") || p.contains("api") || p.contains("fetch") {
        return "network".to_string();
    }
    if p.contains("file") || p.contains("path") || p.contains("read") || p.contains("write") {
        return "filesystem".to_string();
    }
    if p.contains("run") || p.contains("execute") || p.contains("command") || p.contains("test") {
        return "execution".to_string();
    }
    "general".to_string()
}

fn infer_args_for_step(action: &str, suggested_tool: &str) -> serde_json::Value {
    match suggested_tool {
        "read_file" => {
            if let Some(path) = extract_hint_path(action) {
                serde_json::json!({ "path": path })
            } else {
                serde_json::json!({})
            }
        }
        "grep" => {
            if let Some((pattern, path)) = extract_grep_hints(action) {
                serde_json::json!({ "pattern": pattern, "path": path })
            } else {
                serde_json::json!({})
            }
        }
        "web_fetch" => {
            if let Some(url) = extract_url(action) {
                serde_json::json!({ "url": url })
            } else {
                serde_json::json!({})
            }
        }
        "run_command" => {
            if let Some(command) = extract_command_hint(action) {
                serde_json::json!({ "command": command })
            } else {
                serde_json::json!({})
            }
        }
        _ => serde_json::json!({}),
    }
}

fn infer_artifact_hints(
    action: &str,
    suggested_tool: &str,
    inferred_args: &serde_json::Value,
) -> (Vec<String>, Vec<String>) {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();

    if action.to_ascii_lowercase().contains("use previous")
        || action.to_ascii_lowercase().contains("from previous")
    {
        inputs.push("step:previous_output".to_string());
    }

    match suggested_tool {
        "read_file" | "edit_file" | "write_file" | "apply_patch" => {
            if let Some(path) = inferred_args.get("path").and_then(|v| v.as_str()) {
                let p = path.trim();
                if !p.is_empty() {
                    outputs.push(format!("file:{}", p));
                    if suggested_tool != "read_file" {
                        inputs.push(format!("file:{}", p));
                    }
                }
            }
        }
        "web_fetch" => {
            if let Some(url) = inferred_args.get("url").and_then(|v| v.as_str()) {
                let u = url.trim();
                if !u.is_empty() {
                    outputs.push(format!("url:{}", u));
                }
            }
        }
        "run_command" => {
            if let Some(cmd) = inferred_args.get("command").and_then(|v| v.as_str()) {
                let c = cmd.trim();
                if !c.is_empty() {
                    outputs.push(format!("cmd:{}", c));
                }
            }
        }
        _ => {}
    }

    (inputs, outputs)
}

fn infer_output_assertions(
    action: &str,
    suggested_tool: &str,
    inferred_args: &serde_json::Value,
) -> Vec<String> {
    let mut assertions = Vec::new();
    let lower = action.to_ascii_lowercase();

    match suggested_tool {
        "read_file" => {
            if let Some(path) = inferred_args.get("path").and_then(|v| v.as_str()) {
                let file = std::path::Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path)
                    .to_ascii_lowercase();
                assertions.push(format!("contains:{}", file));
            }
        }
        "run_command" => {
            if lower.contains("test") {
                assertions.push("contains:ok".to_string());
            }
            if lower.contains("build") {
                assertions.push("not_contains:error".to_string());
            }
            if lower.contains("json") {
                assertions.push("json_valid".to_string());
                if lower.contains("status") {
                    assertions.push("json_key:status".to_string());
                }
            }
        }
        "web_fetch" => {
            assertions.push("min_len:20".to_string());
            if let Some(url) = inferred_args.get("url").and_then(|v| v.as_str()) {
                assertions.push(format!("contains:{}", url.to_ascii_lowercase()));
            }
            if lower.contains("json") || lower.contains("api") {
                assertions.push("json_valid".to_string());
            }
        }
        _ => {}
    }

    assertions
}

fn extract_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|t| t.starts_with("http://") || t.starts_with("https://"))
        .map(|s| s.trim_end_matches([',', '.', ';']).to_string())
}

fn extract_hint_path(text: &str) -> Option<String> {
    let quoted = text
        .split('`')
        .nth(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    if quoted.is_some() {
        return quoted;
    }

    text.split_whitespace()
        .map(|s| s.trim_matches([',', '.', ';', '"', '\'', '(', ')']))
        .find(|token| {
            token.contains('/')
                || token.contains('\\')
                || token.ends_with(".rs")
                || token.ends_with(".md")
                || token.ends_with(".toml")
                || token.ends_with(".json")
                || token.ends_with(".txt")
        })
        .map(ToString::to_string)
}

fn extract_command_hint(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let markers = ["run ", "execute ", "test "];
    for marker in markers {
        if let Some(idx) = lower.find(marker) {
            let cmd = text[idx + marker.len()..].trim();
            if !cmd.is_empty() {
                return Some(cmd.to_string());
            }
        }
    }
    None
}

fn extract_grep_hints(text: &str) -> Option<(String, String)> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("for ") && !lower.contains("grep") {
        return None;
    }
    let pattern = text
        .split('`')
        .nth(1)
        .map(|s| s.trim().to_string())
        .or_else(|| {
            lower
                .find("for ")
                .map(|idx| text[idx + 4..].trim().to_string())
        })?;
    let path = extract_hint_path(text).unwrap_or_else(|| ".".to_string());
    Some((pattern, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_message_classification() {
        let router = IntelligentRouter::new();

        let msg = IncomingMessage {
            id: "1".to_string(),
            content: "/help".to_string(),
            user_id: "user1".to_string(),
            channel: "test".to_string(),
            timestamp: Instant::now(),
        };

        let result = router.route(msg).await;

        match result {
            RoutingResult::Command { command } => {
                assert_eq!(command, "/help");
            }
            _ => panic!("Expected command result"),
        }
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let router = IntelligentRouter::new();
        let user_id = "user1";

        let should_pass = router.check_rate_limit(user_id, UrgencyLevel::Normal).await;
        assert!(should_pass.is_none());
    }

    #[tokio::test]
    async fn test_queue_depth_recovers_after_estimated_wait() {
        let router = IntelligentRouter::new();
        let msg = IncomingMessage {
            id: "2".to_string(),
            content: "hello there".to_string(),
            user_id: "user2".to_string(),
            channel: "test".to_string(),
            timestamp: Instant::now(),
        };

        let result = router.route(msg).await;
        let wait = match result {
            RoutingResult::Routed { estimated_wait, .. } => estimated_wait,
            _ => panic!("Expected routed result"),
        };

        let stats_now = router.get_stats().await;
        assert!(stats_now.queued_messages >= 1);

        tokio::time::sleep(wait + Duration::from_millis(50)).await;
        let stats_after = router.get_stats().await;
        assert_eq!(stats_after.queued_messages, 0);
    }

    #[tokio::test]
    async fn test_command_response_status_shape() {
        let router = IntelligentRouter::new();
        router.record_outcome("s1", true, Duration::from_millis(120));
        let res = router.command_response("/status", "s1").await;
        assert_eq!(res["ok"], true);
        assert_eq!(res["command"], "/status");
        assert!(res["router"].is_object());
        assert!(res["adaptive"].is_object());
    }

    #[tokio::test]
    async fn test_repeated_failure_signal_increases_urgency() {
        let router = IntelligentRouter::new();
        let msg = IncomingMessage {
            id: "3".to_string(),
            content: "this is still failing again".to_string(),
            user_id: "user3".to_string(),
            channel: "test".to_string(),
            timestamp: Instant::now(),
        };
        let plan = router.plan_route(&msg).await;
        assert!(matches!(
            plan.urgency,
            UrgencyLevel::High | UrgencyLevel::Critical
        ));
    }

    #[test]
    fn test_record_outcome_updates_profile() {
        let router = IntelligentRouter::new();
        let user_id = format!(
            "u-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        router.record_outcome(&user_id, true, Duration::from_millis(100));
        router.record_outcome(&user_id, false, Duration::from_millis(300));
        let profile = router.user_profile(&user_id).expect("profile should exist");
        assert_eq!(profile.successful_tasks, 1);
        assert_eq!(profile.failed_tasks, 1);
        assert!(profile.avg_response_ms > 0.0);
        assert!(profile.tool_stats.contains_key("__general__"));
    }

    #[test]
    fn test_record_tool_outcome_tracks_specific_tool() {
        let router = IntelligentRouter::new();
        let user_id = format!(
            "u-tool-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        router.record_tool_outcome(&user_id, "read_file", true, Duration::from_millis(50));
        router.record_tool_outcome(&user_id, "read_file", false, Duration::from_millis(70));
        let profile = router.user_profile(&user_id).expect("profile should exist");
        let stats = profile
            .tool_stats
            .get("read_file")
            .expect("tool stats should exist");
        assert_eq!(stats.success, 1);
        assert_eq!(stats.fail, 1);
        assert_eq!(stats.consecutive_fail, 1);
        assert_eq!(stats.consecutive_success, 0);
    }

    #[tokio::test]
    async fn test_rank_tool_candidates_denied_tool_last() {
        let router = IntelligentRouter::new();
        let policy = crate::intelligent_policy::IntelligentPolicy::new();
        policy.deny_tool("run_command").await;

        let ranked = router
            .rank_tool_candidates_with_policy(&policy, "u-rank-1", &["read_file", "run_command"])
            .await;

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].tool, "read_file");
        assert_eq!(
            ranked[1].decision,
            crate::intelligent_policy::Decision::Deny
        );
    }

    #[tokio::test]
    async fn test_rank_tool_candidates_escalated_below_allowed() {
        let router = IntelligentRouter::new();
        let policy = crate::intelligent_policy::IntelligentPolicy::new();
        policy.require_approval("apply_patch").await;

        let ranked = router
            .rank_tool_candidates_with_policy(&policy, "u-rank-2", &["read_file", "apply_patch"])
            .await;

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].tool, "read_file");
        assert_eq!(
            ranked[1].decision,
            crate::intelligent_policy::Decision::Escalate
        );
    }

    #[test]
    fn test_decompose_task_builds_ordered_steps() {
        let router = IntelligentRouter::new();
        let plan = router.decompose_task("read config then update key then run tests");
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].id, "s1");
        assert_eq!(plan.steps[1].dependencies, vec!["s1".to_string()]);
        assert_eq!(plan.steps[2].dependencies, vec!["s2".to_string()]);
        assert!(router.validate_task_plan(&plan).is_ok());
    }

    #[test]
    fn test_validate_task_plan_detects_missing_dependency() {
        let router = IntelligentRouter::new();
        let plan = TaskPlan {
            prompt: "x".to_string(),
            domain: "execution".to_string(),
            steps: vec![TaskStep {
                id: "s1".to_string(),
                action: "run".to_string(),
                dependencies: vec!["s9".to_string()],
                suggested_tool: "run_command".to_string(),
                inferred_args: serde_json::json!({}),
                confidence: 0.7,
                expected_inputs: vec![],
                expected_outputs: vec![],
                expected_assertions: vec![],
            }],
        };
        assert!(router.validate_task_plan(&plan).is_err());
    }

    #[test]
    fn test_execution_order_respects_dependencies() {
        let router = IntelligentRouter::new();
        let plan = TaskPlan {
            prompt: "x".to_string(),
            domain: "filesystem".to_string(),
            steps: vec![
                TaskStep {
                    id: "s1".to_string(),
                    action: "read".to_string(),
                    dependencies: vec![],
                    suggested_tool: "read_file".to_string(),
                    inferred_args: serde_json::json!({"path":"README.md"}),
                    confidence: 0.8,
                    expected_inputs: vec![],
                    expected_outputs: vec!["file:README.md".to_string()],
                    expected_assertions: vec![],
                },
                TaskStep {
                    id: "s2".to_string(),
                    action: "edit".to_string(),
                    dependencies: vec!["s1".to_string()],
                    suggested_tool: "edit_file".to_string(),
                    inferred_args: serde_json::json!({"path":"README.md"}),
                    confidence: 0.8,
                    expected_inputs: vec!["file:README.md".to_string()],
                    expected_outputs: vec!["file:README.md".to_string()],
                    expected_assertions: vec![],
                },
                TaskStep {
                    id: "s3".to_string(),
                    action: "test".to_string(),
                    dependencies: vec!["s2".to_string()],
                    suggested_tool: "run_command".to_string(),
                    inferred_args: serde_json::json!({"command":"cargo test"}),
                    confidence: 0.8,
                    expected_inputs: vec![],
                    expected_outputs: vec!["cmd:cargo test".to_string()],
                    expected_assertions: vec![],
                },
            ],
        };
        let order = router.execution_order(&plan).expect("order should succeed");
        assert_eq!(order[0].id, "s1");
        assert_eq!(order[1].id, "s2");
        assert_eq!(order[2].id, "s3");
    }

    #[test]
    fn test_execution_preview_blocks_unsupported_tools() {
        let router = IntelligentRouter::new();
        let plan = TaskPlan {
            prompt: "x".to_string(),
            domain: "general".to_string(),
            steps: vec![TaskStep {
                id: "s1".to_string(),
                action: "unknown".to_string(),
                dependencies: vec![],
                suggested_tool: "nonexistent_tool".to_string(),
                inferred_args: serde_json::json!({}),
                confidence: 0.4,
                expected_inputs: vec![],
                expected_outputs: vec![],
                expected_assertions: vec![],
            }],
        };
        let supported = std::collections::HashSet::from(["read_file".to_string()]);
        let preview = router.build_execution_preview(&plan, &supported);
        assert!(!preview.ready_to_run);
        assert_eq!(preview.steps.len(), 1);
        assert!(!preview.steps[0].ready);
        assert!(
            preview.steps[0]
                .blocked_by
                .iter()
                .any(|b| b.contains("unsupported_tool"))
        );
    }

    #[test]
    fn test_record_successful_sequence_tracks_signature() {
        let router = IntelligentRouter::new();
        let plan = TaskPlan {
            prompt: "x".to_string(),
            domain: "filesystem".to_string(),
            steps: vec![TaskStep {
                id: "s1".to_string(),
                action: "read".to_string(),
                dependencies: vec![],
                suggested_tool: "read_file".to_string(),
                inferred_args: serde_json::json!({}),
                confidence: 0.8,
                expected_inputs: vec![],
                expected_outputs: vec![],
                expected_assertions: vec![],
            }],
        };
        router.record_successful_sequence(&plan);
        let top = router.top_successful_sequences(1);
        assert!(!top.is_empty());
    }

    #[test]
    fn test_sequence_store_prunes_to_cap() {
        let router = IntelligentRouter::new();
        for i in 0..(MAX_SUCCESSFUL_SEQUENCES + 40) {
            let plan = TaskPlan {
                prompt: "x".to_string(),
                domain: "filesystem".to_string(),
                steps: vec![TaskStep {
                    id: "s1".to_string(),
                    action: format!("read file {}", i),
                    dependencies: vec![],
                    suggested_tool: format!("read_file_{}", i),
                    inferred_args: serde_json::json!({}),
                    confidence: 0.8,
                    expected_inputs: vec![],
                    expected_outputs: vec![],
                    expected_assertions: vec![],
                }],
            };
            router.record_successful_sequence(&plan);
        }
        let stats = router.get_adaptive_stats();
        assert!(stats.learned_sequence_count <= MAX_SUCCESSFUL_SEQUENCES);
    }

    #[test]
    fn test_refine_task_plan_replaces_unsupported_tool() {
        let router = IntelligentRouter::new();
        let plan = TaskPlan {
            prompt: "x".to_string(),
            domain: "filesystem".to_string(),
            steps: vec![TaskStep {
                id: "s1".to_string(),
                action: "read `README.md`".to_string(),
                dependencies: vec![],
                suggested_tool: "unknown_tool".to_string(),
                inferred_args: serde_json::json!({}),
                confidence: 0.3,
                expected_inputs: vec![],
                expected_outputs: vec![],
                expected_assertions: vec![],
            }],
        };
        let supported =
            std::collections::HashSet::from(["read_file".to_string(), "grep".to_string()]);
        let refined = router.refine_task_plan(&plan, &supported);
        assert!(supported.contains(&refined.steps[0].suggested_tool));
    }

    #[test]
    fn test_rewrite_step_for_reliability_after_fail_streak() {
        let router = IntelligentRouter::new();
        let step = TaskStep {
            id: "s1".to_string(),
            action: "run tests".to_string(),
            dependencies: vec![],
            suggested_tool: "run_command".to_string(),
            inferred_args: serde_json::json!({"command":"cargo test"}),
            confidence: 0.5,
            expected_inputs: vec![],
            expected_outputs: vec!["cmd:cargo test".to_string()],
            expected_assertions: vec!["contains:ok".to_string()],
        };
        router.record_step_pattern_outcome(&step, false);
        router.record_step_pattern_outcome(&step, false);
        router.record_step_pattern_outcome(&step, false);

        let supported = std::collections::HashSet::from([
            "run_command".to_string(),
            "grep".to_string(),
            "read_file".to_string(),
        ]);
        let rewritten = router.rewrite_step_for_reliability(&step, &supported);
        assert_ne!(rewritten.suggested_tool, "task");
        assert!(rewritten.confidence >= step.confidence);
    }
}
