//! Quantum Policy Engine - Multi-Dimensional Access Control
//!
//! A superior alternative to OpenClaw's 9-layer linear cascade.
//! Uses a multi-dimensional policy matrix with composable rules,
//! adaptive risk scoring, and compile-time verification.
//!
//! Dimensions:
//! - Scope (Global → Provider → Agent → Session)
//! - Context (Channel → User → Time → Location)
//! - Risk (Static classification + Dynamic behavior)
//! - Capability (Tool → Resource → Network → System)
//! - Inheritance (Parent → Child with constraint propagation)

use anyhow::Result;
use dashmap::DashMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Policy decision outcome
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny,
    Escalate,
    Quarantine,
}

/// Risk classification for operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Minimal = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl RiskLevel {
    pub fn from_score(score: u32) -> Self {
        match score {
            0 => RiskLevel::Minimal,
            1..=30 => RiskLevel::Low,
            31..=60 => RiskLevel::Medium,
            61..=85 => RiskLevel::High,
            _ => RiskLevel::Critical,
        }
    }
}

/// Multi-dimensional policy scope
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PolicyScope {
    Global,
    Provider(String),
    Tenant(String),
    Agent(String),
    Session(String),
    Subagent { parent: String, child: String },
}

/// Execution context for policy evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub scope: PolicyScope,
    pub user_id: String,
    pub channel: String,
    pub channel_type: ChannelType,
    pub message_id: String,
    pub timestamp: SystemTime,
    pub source_ip: Option<IpAddr>,
    pub is_admin: bool,
    pub auth_method: AuthMethod,
    pub session_duration: Duration,
    pub previous_violations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    DirectMessage,
    Group,
    PublicChannel,
    WebInterface,
    Api,
    Cron,
    Webhook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMethod {
    OAuth { provider: String },
    Token,
    Certificate,
    Anonymous,
}

/// Tool capability classification
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    FileRead,
    FileWrite,
    FileDelete,
    CommandExecute,
    NetworkOutbound,
    NetworkInbound,
    BrowserAutomation,
    SystemInfo,
    ProcessSpawn,
    DatabaseAccess,
    ExternalApi { service: String },
    CodeExecution,
    PrivilegeEscalation,
}

impl Capability {
    pub fn base_risk_level(&self) -> RiskLevel {
        match self {
            Capability::FileRead => RiskLevel::Minimal,
            Capability::SystemInfo => RiskLevel::Minimal,
            Capability::FileWrite => RiskLevel::Low,
            Capability::NetworkOutbound => RiskLevel::Low,
            Capability::FileDelete => RiskLevel::Medium,
            Capability::ProcessSpawn => RiskLevel::Medium,
            Capability::CommandExecute => RiskLevel::High,
            Capability::BrowserAutomation => RiskLevel::High,
            Capability::ExternalApi { .. } => RiskLevel::Medium,
            Capability::DatabaseAccess => RiskLevel::High,
            Capability::NetworkInbound => RiskLevel::High,
            Capability::CodeExecution => RiskLevel::Critical,
            Capability::PrivilegeEscalation => RiskLevel::Critical,
        }
    }
}

/// Resource constraint limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_file_size: u64,
    pub max_memory_mb: u64,
    pub max_cpu_percent: f32,
    pub max_execution_time: Duration,
    pub max_concurrent_operations: u32,
    pub max_daily_api_calls: u32,
    pub max_network_egress_mb: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_file_size: 100 * 1024 * 1024, // 100MB
            max_memory_mb: 1024,
            max_cpu_percent: 50.0,
            max_execution_time: Duration::from_secs(300),
            max_concurrent_operations: 10,
            max_daily_api_calls: 1000,
            max_network_egress_mb: 1024,
        }
    }
}

/// Policy rule trait - composable and extensible
#[async_trait::async_trait]
pub trait PolicyRule: Send + Sync {
    async fn evaluate(&self, tool: &ToolRequest, context: &ExecutionContext) -> RuleResult;
    fn priority(&self) -> i32;
    fn name(&self) -> &str;
}

/// Result from individual rule evaluation
#[derive(Debug, Clone)]
pub struct RuleResult {
    pub decision: PolicyDecision,
    pub risk_modifier: i32,
    pub reason: String,
    pub rule_name: String,
}

/// Tool request being evaluated
#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub tool_name: String,
    pub tool_id: String,
    pub capabilities: Vec<Capability>,
    pub parameters: HashMap<String, serde_json::Value>,
    pub target_resources: Vec<ResourceTarget>,
}

/// Target resource for operation
#[derive(Debug, Clone)]
pub enum ResourceTarget {
    File(PathBuf),
    Directory(PathBuf),
    Url(String),
    Database(String),
    Process(u32),
    Service(String),
}

/// Quantum Policy Engine - Multi-dimensional policy evaluation
pub struct QuantumPolicyEngine {
    /// Scope-based policies (hierarchical)
    scope_policies: DashMap<PolicyScope, Arc<RwLock<ScopePolicy>>>,

    /// Composable rule chain
    rules: Vec<Box<dyn PolicyRule>>,

    /// Dynamic risk assessment
    risk_engine: Arc<RiskAssessmentEngine>,

    /// Behavior tracking for ML-like analysis
    behavior_tracker: Arc<BehaviorTracker>,

    /// Policy cache for performance
    decision_cache: DashMap<String, CachedDecision>,

    /// Audit logger
    audit_logger: Arc<dyn AuditLogger>,
}

/// Policy configuration for a specific scope
#[derive(Debug, Clone)]
pub struct ScopePolicy {
    pub scope: PolicyScope,
    pub allowed_capabilities: HashSet<Capability>,
    pub denied_capabilities: HashSet<Capability>,
    pub allowed_tools: Option<HashSet<String>>,
    pub denied_tools: HashSet<String>,
    pub allowed_paths: Vec<PathPattern>,
    pub denied_paths: Vec<PathPattern>,
    pub allowed_commands: Option<HashSet<String>>,
    pub resource_limits: ResourceLimits,
    pub require_approval_for: HashSet<Capability>,
    pub time_restrictions: Option<TimeRestrictions>,
    pub network_policy: NetworkPolicy,
    pub inheritance_mode: InheritanceMode,
}

#[derive(Debug, Clone)]
pub enum PathPattern {
    Exact(String),
    Prefix(String),
    Glob(String),
    Regex(Regex),
}

#[derive(Debug, Clone)]
pub struct TimeRestrictions {
    pub allowed_days: Vec<u8>,   // 0-6 for Sun-Sat
    pub allowed_hours_start: u8, // 0-23
    pub allowed_hours_end: u8,
    pub timezone: String,
}

#[derive(Debug, Clone)]
pub struct NetworkPolicy {
    pub allow_outbound: bool,
    pub allowed_domains: Vec<String>,
    pub denied_domains: Vec<String>,
    pub allowed_ips: Vec<IpAddr>,
    pub denied_ips: Vec<IpAddr>,
    pub require_dns_pinning: bool,
    pub max_request_size: u64,
    pub blocked_schemes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum InheritanceMode {
    Strict,     // Child cannot exceed parent
    Permissive, // Child can add capabilities
    Override,   // Child completely overrides parent
}

/// Cached policy decision
#[derive(Debug, Clone)]
struct CachedDecision {
    decision: PolicyDecision,
    risk_level: RiskLevel,
    timestamp: Instant,
    ttl: Duration,
}

/// Risk assessment engine with dynamic scoring
pub struct RiskAssessmentEngine {
    /// Base risk scores for capabilities
    base_risks: HashMap<Capability, u32>,

    /// Contextual risk modifiers
    context_modifiers: DashMap<String, ContextModifier>,

    /// Real-time threat intelligence
    threat_feed: Arc<RwLock<ThreatIntelligence>>,
}

#[derive(Debug, Clone)]
struct ContextModifier {
    condition: RiskCondition,
    modifier: i32,
}

#[derive(Debug, Clone)]
enum RiskCondition {
    TimeOfDay { start: u8, end: u8 },
    NewUser { max_age_hours: u32 },
    HighViolationCount { threshold: u32 },
    UnusualLocation,
    AnonymousAuth,
}

/// Threat intelligence feed
#[derive(Debug, Clone)]
struct ThreatIntelligence {
    known_bad_ips: HashSet<IpAddr>,
    known_bad_domains: HashSet<String>,
    attack_patterns: Vec<Regex>,
    last_updated: Instant,
}

impl Default for ThreatIntelligence {
    fn default() -> Self {
        Self {
            known_bad_ips: HashSet::new(),
            known_bad_domains: HashSet::new(),
            attack_patterns: Vec::new(),
            last_updated: Instant::now(),
        }
    }
}

/// Behavior tracking for ML-like analysis
pub struct BehaviorTracker {
    /// User behavior profiles
    user_profiles: DashMap<String, UserBehaviorProfile>,

    /// Tool usage patterns
    tool_patterns: DashMap<String, ToolUsagePattern>,
}

#[derive(Debug, Clone)]
struct UserBehaviorProfile {
    typical_tools: HashSet<String>,
    typical_hours: Vec<u8>,
    typical_channels: Vec<String>,
    session_history: Vec<SessionRecord>,
    risk_score: f32,
    created_at: Instant,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    start: Instant,
    duration: Duration,
    tools_used: Vec<String>,
    violations: u32,
}

#[derive(Debug, Clone)]
struct ToolUsagePattern {
    typical_parameters: HashMap<String, ParameterStats>,
    execution_times: Vec<Duration>,
    success_rate: f32,
}

#[derive(Debug, Clone)]
struct ParameterStats {
    field: String,
    common_values: Vec<String>,
    value_types: HashSet<String>,
}

/// Audit logger trait
#[async_trait::async_trait]
pub trait AuditLogger: Send + Sync {
    async fn log_policy_decision(&self, record: AuditRecord);
    async fn log_violation(&self, record: ViolationRecord);
    async fn log_behavior_anomaly(&self, record: AnomalyRecord);
}

/// Audit record for policy decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub timestamp: SystemTime,
    pub decision: PolicyDecision,
    pub tool_name: String,
    pub user_id: String,
    pub scope: PolicyScope,
    pub risk_level: RiskLevel,
    pub risk_score: u32,
    pub context: ExecutionContext,
    pub rules_evaluated: Vec<String>,
    pub reasoning: String,
}

/// Violation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationRecord {
    pub timestamp: SystemTime,
    pub violation_type: ViolationType,
    pub tool_name: String,
    pub user_id: String,
    pub attempted_action: String,
    pub blocked_by: String,
    pub context: ExecutionContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ViolationType {
    ToolNotAllowed,
    PathNotAllowed,
    CapabilityNotAllowed,
    ResourceLimitExceeded,
    TimeRestrictionViolated,
    NetworkPolicyViolated,
    RateLimitExceeded,
    BehavioralAnomaly,
}

/// Anomaly record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyRecord {
    pub timestamp: SystemTime,
    pub anomaly_type: AnomalyType,
    pub user_id: String,
    pub description: String,
    pub severity: RiskLevel,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnomalyType {
    UnusualToolUsage,
    UnusualTimePattern,
    UnusualChannel,
    RapidToolExecution,
    SuspiciousParameterValues,
    PotentialInjectionAttempt,
    ImpossibleTravel,
}

impl QuantumPolicyEngine {
    pub fn new(audit_logger: Arc<dyn AuditLogger>) -> Self {
        Self {
            scope_policies: DashMap::new(),
            rules: Vec::new(),
            risk_engine: Arc::new(RiskAssessmentEngine::new()),
            behavior_tracker: Arc::new(BehaviorTracker::new()),
            decision_cache: DashMap::new(),
            audit_logger,
        }
    }

    /// Evaluate policy for a tool execution request
    pub async fn evaluate(
        &self,
        tool: &ToolRequest,
        context: &ExecutionContext,
    ) -> Result<PolicyEvaluation> {
        let start_time = Instant::now();

        // Check cache first
        let cache_key = format!("{}:{}:{}", context.user_id, tool.tool_name, tool.tool_id);
        if let Some(cached) = self.decision_cache.get(&cache_key)
            && cached.timestamp.elapsed() < cached.ttl
        {
            return Ok(PolicyEvaluation {
                decision: cached.decision,
                risk_level: cached.risk_level,
                risk_score: 0,
                requires_approval: cached.decision == PolicyDecision::Escalate,
                cache_hit: true,
                evaluation_time_ms: start_time.elapsed().as_millis() as u64,
                rules_applied: vec!["cached".to_string()],
                reasoning: "Retrieved from cache".to_string(),
            });
        }

        // Get scope policy
        let scope_policy = self.get_scope_policy(&context.scope).await?;

        // Evaluate all rules
        let mut rule_results = Vec::new();
        let mut total_risk = 0i32;

        for rule in &self.rules {
            let result = rule.evaluate(tool, context).await;
            total_risk += result.risk_modifier;
            rule_results.push(result);
        }

        // Assess base + contextual risk from risk engine
        let base_risk = self.risk_engine.calculate_risk(tool, context).await;

        // Get dynamic risk from behavior analysis
        let behavior_risk = self.behavior_tracker.assess_risk(context, tool).await;

        // Calculate final risk score
        let risk_score = (base_risk as i32 + total_risk + behavior_risk).clamp(0, 100) as u32;
        let risk_level = RiskLevel::from_score(risk_score);

        // Determine final decision
        let decision = self
            .calculate_final_decision(&rule_results, risk_level, &scope_policy, tool)
            .await;

        // Check for behavioral anomalies
        self.behavior_tracker
            .record_usage(context, tool, &decision)
            .await;

        // Cache the decision
        self.decision_cache.insert(
            cache_key,
            CachedDecision {
                decision,
                risk_level,
                timestamp: Instant::now(),
                ttl: Duration::from_secs(60),
            },
        );

        // Log audit record
        let audit_record = AuditRecord {
            timestamp: SystemTime::now(),
            decision,
            tool_name: tool.tool_name.clone(),
            user_id: context.user_id.clone(),
            scope: context.scope.clone(),
            risk_level,
            risk_score,
            context: context.clone(),
            rules_evaluated: rule_results.iter().map(|r| r.rule_name.clone()).collect(),
            reasoning: self.build_reasoning(&rule_results),
        };
        self.audit_logger.log_policy_decision(audit_record).await;

        Ok(PolicyEvaluation {
            decision,
            risk_level,
            risk_score,
            requires_approval: decision == PolicyDecision::Escalate
                || scope_policy
                    .require_approval_for
                    .iter()
                    .any(|c| tool.capabilities.contains(c)),
            cache_hit: false,
            evaluation_time_ms: start_time.elapsed().as_millis() as u64,
            rules_applied: rule_results.iter().map(|r| r.rule_name.clone()).collect(),
            reasoning: self.build_reasoning(&rule_results),
        })
    }

    async fn get_scope_policy(&self, scope: &PolicyScope) -> Result<ScopePolicy> {
        // Try to get exact scope policy
        if let Some(policy) = self.scope_policies.get(scope) {
            return Ok(policy.read().await.clone());
        }

        // Fall back to parent scope
        let parent_scope = self.get_parent_scope(scope);
        if let Some(parent) = parent_scope
            && let Some(policy) = self.scope_policies.get(&parent)
        {
            let parent_policy = policy.read().await.clone();
            return Ok(self.inherit_policy(parent_policy, scope));
        }

        // Return default policy
        Ok(ScopePolicy::default_global())
    }

    fn get_parent_scope(&self, scope: &PolicyScope) -> Option<PolicyScope> {
        match scope {
            PolicyScope::Subagent { parent, .. } => Some(PolicyScope::Agent(parent.clone())),
            PolicyScope::Agent(_) => Some(PolicyScope::Global),
            PolicyScope::Session(_) => Some(PolicyScope::Global),
            PolicyScope::Tenant(_) => Some(PolicyScope::Global),
            PolicyScope::Provider(_) => Some(PolicyScope::Global),
            PolicyScope::Global => None,
        }
    }

    fn inherit_policy(&self, parent: ScopePolicy, child_scope: &PolicyScope) -> ScopePolicy {
        match parent.inheritance_mode {
            InheritanceMode::Override => parent,
            InheritanceMode::Strict => {
                // Child cannot exceed parent
                let mut child = parent.clone();
                child.scope = child_scope.clone();
                child
            }
            InheritanceMode::Permissive => {
                // Child can add capabilities but not remove denials
                let mut child = parent.clone();
                child.scope = child_scope.clone();
                // Keep parent's denials
                child
            }
        }
    }

    async fn calculate_final_decision(
        &self,
        rule_results: &[RuleResult],
        risk_level: RiskLevel,
        scope_policy: &ScopePolicy,
        tool: &ToolRequest,
    ) -> PolicyDecision {
        // Check for explicit denials first
        for result in rule_results {
            if result.decision == PolicyDecision::Deny {
                return PolicyDecision::Deny;
            }
        }

        // Check capability restrictions
        for cap in &tool.capabilities {
            if scope_policy.denied_capabilities.contains(cap) {
                return PolicyDecision::Deny;
            }
            if !scope_policy.allowed_capabilities.contains(cap) {
                return PolicyDecision::Deny;
            }
        }

        // Check tool restrictions
        if scope_policy.denied_tools.contains(&tool.tool_name) {
            return PolicyDecision::Deny;
        }
        if let Some(ref allowed) = scope_policy.allowed_tools
            && !allowed.contains(&tool.tool_name)
        {
            return PolicyDecision::Deny;
        }

        // Risk-based escalation
        match risk_level {
            RiskLevel::Critical => PolicyDecision::Quarantine,
            RiskLevel::High => PolicyDecision::Escalate,
            RiskLevel::Medium => {
                // Check if requires approval
                if scope_policy
                    .require_approval_for
                    .iter()
                    .any(|c| tool.capabilities.contains(c))
                {
                    PolicyDecision::Escalate
                } else {
                    PolicyDecision::Allow
                }
            }
            _ => PolicyDecision::Allow,
        }
    }

    fn build_reasoning(&self, rule_results: &[RuleResult]) -> String {
        rule_results
            .iter()
            .map(|r| {
                format!(
                    "{}: {} (modifier: {})",
                    r.rule_name, r.reason, r.risk_modifier
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Add a policy rule
    pub fn add_rule(&mut self, rule: Box<dyn PolicyRule>) {
        self.rules.push(rule);
        self.rules.sort_by_key(|r| -r.priority()); // Higher priority first
    }

    /// Set policy for a scope
    pub async fn set_scope_policy(&self, scope: PolicyScope, policy: ScopePolicy) {
        self.scope_policies
            .insert(scope, Arc::new(RwLock::new(policy)));
    }
}

/// Policy evaluation result
#[derive(Debug, Clone)]
pub struct PolicyEvaluation {
    pub decision: PolicyDecision,
    pub risk_level: RiskLevel,
    pub risk_score: u32,
    pub requires_approval: bool,
    pub cache_hit: bool,
    pub evaluation_time_ms: u64,
    pub rules_applied: Vec<String>,
    pub reasoning: String,
}

impl ScopePolicy {
    pub fn default_global() -> Self {
        Self {
            scope: PolicyScope::Global,
            allowed_capabilities: HashSet::from([
                Capability::FileRead,
                Capability::FileWrite,
                Capability::NetworkOutbound,
                Capability::SystemInfo,
            ]),
            denied_capabilities: HashSet::from([
                Capability::CodeExecution,
                Capability::PrivilegeEscalation,
                Capability::NetworkInbound,
            ]),
            allowed_tools: None,
            denied_tools: HashSet::new(),
            allowed_paths: vec![PathPattern::Glob("**".to_string())],
            denied_paths: vec![],
            allowed_commands: None,
            resource_limits: ResourceLimits::default(),
            require_approval_for: HashSet::from([
                Capability::CommandExecute,
                Capability::BrowserAutomation,
                Capability::ProcessSpawn,
                Capability::DatabaseAccess,
            ]),
            time_restrictions: None,
            network_policy: NetworkPolicy {
                allow_outbound: true,
                allowed_domains: vec![],
                denied_domains: vec![],
                allowed_ips: vec![],
                denied_ips: vec![],
                require_dns_pinning: false,
                max_request_size: 10 * 1024 * 1024,
                blocked_schemes: vec!["file".to_string(), "gopher".to_string()],
            },
            inheritance_mode: InheritanceMode::Strict,
        }
    }
}

impl Default for RiskAssessmentEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RiskAssessmentEngine {
    pub fn new() -> Self {
        let mut base_risks = HashMap::new();
        base_risks.insert(Capability::FileRead, 5);
        base_risks.insert(Capability::FileWrite, 15);
        base_risks.insert(Capability::CommandExecute, 60);
        base_risks.insert(Capability::BrowserAutomation, 55);
        base_risks.insert(Capability::CodeExecution, 95);
        base_risks.insert(Capability::NetworkOutbound, 20);
        base_risks.insert(Capability::ProcessSpawn, 40);
        base_risks.insert(Capability::DatabaseAccess, 50);
        base_risks.insert(Capability::FileDelete, 35);
        base_risks.insert(Capability::PrivilegeEscalation, 100);

        let engine = Self {
            base_risks,
            context_modifiers: DashMap::new(),
            threat_feed: Arc::new(RwLock::new(ThreatIntelligence::default())),
        };

        engine.context_modifiers.insert(
            "off_hours".to_string(),
            ContextModifier {
                condition: RiskCondition::TimeOfDay { start: 0, end: 6 },
                modifier: 8,
            },
        );
        engine.context_modifiers.insert(
            "new_user".to_string(),
            ContextModifier {
                condition: RiskCondition::NewUser { max_age_hours: 24 },
                modifier: 5,
            },
        );
        engine.context_modifiers.insert(
            "high_violations".to_string(),
            ContextModifier {
                condition: RiskCondition::HighViolationCount { threshold: 3 },
                modifier: 15,
            },
        );
        engine.context_modifiers.insert(
            "anonymous_auth".to_string(),
            ContextModifier {
                condition: RiskCondition::AnonymousAuth,
                modifier: 10,
            },
        );
        engine.context_modifiers.insert(
            "unusual_location".to_string(),
            ContextModifier {
                condition: RiskCondition::UnusualLocation,
                modifier: 3,
            },
        );

        engine
    }

    pub async fn calculate_risk(&self, tool: &ToolRequest, context: &ExecutionContext) -> u32 {
        let mut score = 0u32;

        // Base capability risks
        for cap in &tool.capabilities {
            if let Some(risk) = self.base_risks.get(cap) {
                score += risk;
            }
        }

        // Apply context modifiers
        for modifier in self.context_modifiers.iter() {
            if self.matches_condition(&modifier.condition, context) {
                score = (score as i32 + modifier.modifier).max(0) as u32;
            }
        }

        // Check threat intelligence
        if let Some(ip) = context.source_ip {
            let threat_feed = self.threat_feed.read().await;
            if threat_feed.known_bad_ips.contains(&ip) {
                score += 50;
            }

            if let Some(url) = tool.parameters.get("url").and_then(|v| v.as_str()) {
                if threat_feed
                    .known_bad_domains
                    .iter()
                    .any(|d| url.contains(d))
                {
                    score += 25;
                }
                if threat_feed.attack_patterns.iter().any(|p| p.is_match(url)) {
                    score += 20;
                }
            }

            if threat_feed.last_updated.elapsed() > Duration::from_secs(24 * 3600) {
                score += 2;
            }
        }

        score.min(100)
    }

    fn matches_condition(&self, condition: &RiskCondition, context: &ExecutionContext) -> bool {
        match condition {
            RiskCondition::TimeOfDay { start, end } => {
                let now_secs = context
                    .timestamp
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let hour = ((now_secs / 3600) % 24) as u8;
                if start <= end {
                    hour >= *start && hour <= *end
                } else {
                    hour >= *start || hour <= *end
                }
            }
            RiskCondition::NewUser { max_age_hours } => {
                context.session_duration < Duration::from_secs(*max_age_hours as u64 * 3600)
            }
            RiskCondition::HighViolationCount { threshold } => {
                context.previous_violations > *threshold
            }
            RiskCondition::UnusualLocation => false, // Would check against typical locations
            RiskCondition::AnonymousAuth => matches!(context.auth_method, AuthMethod::Anonymous),
        }
    }
}

impl Default for BehaviorTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl BehaviorTracker {
    pub fn new() -> Self {
        Self {
            user_profiles: DashMap::new(),
            tool_patterns: DashMap::new(),
        }
    }

    pub async fn assess_risk(&self, context: &ExecutionContext, tool: &ToolRequest) -> i32 {
        let mut risk_modifier = 0i32;

        // Check user profile
        if let Some(profile) = self.user_profiles.get(&context.user_id) {
            let profile = profile.value();

            // New tool for this user
            if !profile.typical_tools.contains(&tool.tool_name) {
                risk_modifier += 10;
            }

            if !profile
                .typical_channels
                .iter()
                .any(|c| c == &context.channel)
            {
                risk_modifier += 8;
            }

            let hour = context
                .timestamp
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let hour = ((hour / 3600) % 24) as u8;
            if !profile.typical_hours.contains(&hour) {
                risk_modifier += 4;
            }

            // Unusual time
            // Simplified - would check typical hours

            // High violation history
            if context.previous_violations > 3 {
                risk_modifier += 15;
            }

            let recent_violations: u32 = profile
                .session_history
                .iter()
                .filter(|s| s.start.elapsed() <= Duration::from_secs(3600))
                .map(|s| s.violations)
                .sum();
            risk_modifier += recent_violations.min(10) as i32;

            let recent_duration_secs: u64 = profile
                .session_history
                .iter()
                .filter(|s| s.start.elapsed() <= Duration::from_secs(3600))
                .map(|s| s.duration.as_secs())
                .sum();
            if recent_duration_secs > 3600 {
                risk_modifier += 3;
            }

            let recent_tool_events: usize = profile
                .session_history
                .iter()
                .map(|s| s.tools_used.len())
                .sum();
            if recent_tool_events > 20 {
                risk_modifier += 4;
            }

            if profile.risk_score > 0.7 {
                risk_modifier += 6;
            }

            if profile.created_at.elapsed() < Duration::from_secs(24 * 3600) {
                risk_modifier += 3;
            }
        } else {
            // New user - moderate risk
            risk_modifier += 5;
        }

        if let Some(pattern) = self.tool_patterns.get(&tool.tool_name) {
            let pattern = pattern.value();
            if pattern.execution_times.len() > 20 {
                risk_modifier += 2;
            }
            if pattern.success_rate < 0.8 {
                risk_modifier += 5;
            }
            if !pattern.typical_parameters.is_empty() && tool.parameters.is_empty() {
                risk_modifier += 1;
            }
            for stats in pattern.typical_parameters.values() {
                if !stats.common_values.is_empty() && stats.value_types.len() == 1 {
                    risk_modifier += 1;
                }
                if stats.field.is_empty() {
                    risk_modifier += 1;
                }
            }
        }

        risk_modifier
    }

    pub async fn record_usage(
        &self,
        context: &ExecutionContext,
        tool: &ToolRequest,
        decision: &PolicyDecision,
    ) {
        // Update user profile
        self.user_profiles
            .entry(context.user_id.clone())
            .and_modify(|profile| {
                profile.typical_tools.insert(tool.tool_name.clone());
                let hour = context
                    .timestamp
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let hour = ((hour / 3600) % 24) as u8;
                if !profile.typical_hours.contains(&hour) {
                    profile.typical_hours.push(hour);
                }
                if !profile
                    .typical_channels
                    .iter()
                    .any(|c| c == &context.channel)
                {
                    profile.typical_channels.push(context.channel.clone());
                }
                profile.session_history.push(SessionRecord {
                    start: Instant::now(),
                    duration: context.session_duration,
                    tools_used: vec![tool.tool_name.clone()],
                    violations: if *decision == PolicyDecision::Deny {
                        1
                    } else {
                        0
                    },
                });
                let total_violations: u32 =
                    profile.session_history.iter().map(|s| s.violations).sum();
                let sessions = profile.session_history.len().max(1) as f32;
                profile.risk_score = (total_violations as f32 / sessions).min(1.0);
            })
            .or_insert_with(|| UserBehaviorProfile {
                typical_tools: HashSet::from([tool.tool_name.clone()]),
                typical_hours: vec![
                    ((context
                        .timestamp
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        / 3600)
                        % 24) as u8,
                ],
                typical_channels: vec![context.channel.clone()],
                session_history: vec![],
                risk_score: 0.0,
                created_at: Instant::now(),
            });

        // Update tool patterns
        self.tool_patterns
            .entry(tool.tool_name.clone())
            .and_modify(|pattern| {
                pattern.execution_times.push(context.session_duration);
                let is_success = *decision != PolicyDecision::Deny;
                let history_len = pattern.execution_times.len() as f32;
                if history_len > 0.0 {
                    let prev = pattern.success_rate * (history_len - 1.0);
                    pattern.success_rate =
                        (prev + if is_success { 1.0 } else { 0.0 }) / history_len;
                }
                for (k, v) in &tool.parameters {
                    let entry = pattern
                        .typical_parameters
                        .entry(k.clone())
                        .or_insert_with(|| ParameterStats {
                            field: k.clone(),
                            common_values: Vec::new(),
                            value_types: HashSet::new(),
                        });
                    entry.value_types.insert(
                        match v {
                            serde_json::Value::Null => "null",
                            serde_json::Value::Bool(_) => "bool",
                            serde_json::Value::Number(_) => "number",
                            serde_json::Value::String(_) => "string",
                            serde_json::Value::Array(_) => "array",
                            serde_json::Value::Object(_) => "object",
                        }
                        .to_string(),
                    );
                    if let Some(s) = v.as_str()
                        && !entry.common_values.iter().any(|x| x == s)
                        && entry.common_values.len() < 5
                    {
                        entry.common_values.push(s.to_string());
                    }
                }
            })
            .or_insert_with(|| ToolUsagePattern {
                typical_parameters: HashMap::new(),
                execution_times: vec![context.session_duration],
                success_rate: if *decision == PolicyDecision::Deny {
                    0.0
                } else {
                    1.0
                },
            });
    }
}

// Built-in policy rules

/// Time-based policy rule
pub struct TimeRestrictionRule;

#[async_trait::async_trait]
impl PolicyRule for TimeRestrictionRule {
    async fn evaluate(&self, _tool: &ToolRequest, context: &ExecutionContext) -> RuleResult {
        let hour = context
            .timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let hour = ((hour / 3600) % 24) as u8;
        let off_hours = !(6..=22).contains(&hour);

        RuleResult {
            decision: if off_hours {
                PolicyDecision::Escalate
            } else {
                PolicyDecision::Allow
            },
            risk_modifier: if off_hours { 8 } else { 0 },
            reason: if off_hours {
                "Outside standard operating hours".to_string()
            } else {
                "Within standard operating hours".to_string()
            },
            rule_name: "TimeRestriction".to_string(),
        }
    }

    fn priority(&self) -> i32 {
        50
    }
    fn name(&self) -> &str {
        "TimeRestriction"
    }
}

/// Rate limiting rule
pub struct RateLimitRule {
    max_requests_per_minute: u32,
    request_counts: DashMap<String, Vec<Instant>>,
}

#[async_trait::async_trait]
impl PolicyRule for RateLimitRule {
    async fn evaluate(&self, tool: &ToolRequest, context: &ExecutionContext) -> RuleResult {
        let key = format!("{}:{}", context.user_id, tool.tool_name);
        let now = Instant::now();
        let window = Duration::from_secs(60);

        let mut counts = self.request_counts.entry(key).or_default();
        counts.retain(|&t| now.duration_since(t) < window);

        if counts.len() as u32 >= self.max_requests_per_minute {
            return RuleResult {
                decision: PolicyDecision::Deny,
                risk_modifier: 30,
                reason: "Rate limit exceeded".to_string(),
                rule_name: "RateLimit".to_string(),
            };
        }

        counts.push(now);

        RuleResult {
            decision: PolicyDecision::Allow,
            risk_modifier: 0,
            reason: "Rate limit OK".to_string(),
            rule_name: "RateLimit".to_string(),
        }
    }

    fn priority(&self) -> i32 {
        100
    }
    fn name(&self) -> &str {
        "RateLimit"
    }
}

/// Resource limit rule
pub struct ResourceLimitRule;

#[async_trait::async_trait]
impl PolicyRule for ResourceLimitRule {
    async fn evaluate(&self, tool: &ToolRequest, _context: &ExecutionContext) -> RuleResult {
        if let Some(size) = tool.parameters.get("file_size").and_then(|v| v.as_u64())
            && size > 100 * 1024 * 1024
        {
            return RuleResult {
                decision: PolicyDecision::Deny,
                risk_modifier: 25,
                reason: "Requested file size exceeds 100MB safety limit".to_string(),
                rule_name: "ResourceLimit".to_string(),
            };
        }

        RuleResult {
            decision: PolicyDecision::Allow,
            risk_modifier: 0,
            reason: "Resource limits checked".to_string(),
            rule_name: "ResourceLimit".to_string(),
        }
    }

    fn priority(&self) -> i32 {
        80
    }
    fn name(&self) -> &str {
        "ResourceLimit"
    }
}

/// Network security rule
pub struct NetworkSecurityRule;

#[async_trait::async_trait]
impl PolicyRule for NetworkSecurityRule {
    async fn evaluate(&self, tool: &ToolRequest, _context: &ExecutionContext) -> RuleResult {
        // Check for SSRF indicators
        if let Some(url) = tool.parameters.get("url").and_then(|v| v.as_str())
            && (is_internal_ip(url) || is_dangerous_scheme(url))
        {
            return RuleResult {
                decision: PolicyDecision::Deny,
                risk_modifier: 50,
                reason: "Potential SSRF attempt".to_string(),
                rule_name: "NetworkSecurity".to_string(),
            };
        }

        RuleResult {
            decision: PolicyDecision::Allow,
            risk_modifier: 0,
            reason: "Network security OK".to_string(),
            rule_name: "NetworkSecurity".to_string(),
        }
    }

    fn priority(&self) -> i32 {
        90
    }
    fn name(&self) -> &str {
        "NetworkSecurity"
    }
}

fn is_internal_ip(url: &str) -> bool {
    // Check for private IP ranges
    url.contains("127.0.0")
        || url.contains("10.")
        || url.contains("192.168.")
        || url.contains("172.16.")
        || url.contains("localhost")
        || url.contains("0.0.0.0")
}

fn is_dangerous_scheme(url: &str) -> bool {
    url.starts_with("file://")
        || url.starts_with("gopher://")
        || url.starts_with("ftp://")
        || url.starts_with("dict://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_policy_evaluation() {
        struct MockAuditLogger;
        #[async_trait::async_trait]
        impl AuditLogger for MockAuditLogger {
            async fn log_policy_decision(&self, _record: AuditRecord) {}
            async fn log_violation(&self, _record: ViolationRecord) {}
            async fn log_behavior_anomaly(&self, _record: AnomalyRecord) {}
        }

        let engine = QuantumPolicyEngine::new(Arc::new(MockAuditLogger));

        let tool = ToolRequest {
            tool_name: "read_file".to_string(),
            tool_id: "test-123".to_string(),
            capabilities: vec![Capability::FileRead],
            parameters: HashMap::new(),
            target_resources: vec![ResourceTarget::File(PathBuf::from("/test.txt"))],
        };

        let context = ExecutionContext {
            scope: PolicyScope::Global,
            user_id: "user-123".to_string(),
            channel: "test".to_string(),
            channel_type: ChannelType::DirectMessage,
            message_id: "msg-123".to_string(),
            timestamp: SystemTime::now(),
            source_ip: None,
            is_admin: false,
            auth_method: AuthMethod::Token,
            session_duration: Duration::from_secs(0),
            previous_violations: 0,
        };

        let result = engine.evaluate(&tool, &context).await.unwrap();
        assert_eq!(result.decision, PolicyDecision::Allow);
    }
}
