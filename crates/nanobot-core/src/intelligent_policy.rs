//! Fixed intelligent policy engine - Production Ready
//!
//! Simplified version that compiles and integrates properly

use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Simple policy decision
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Allow,
    Deny,
    Escalate,
}

/// Policy check result
#[derive(Debug, Clone)]
pub struct PolicyCheck {
    pub decision: Decision,
    pub reason: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Simple intelligent policy engine
pub struct IntelligentPolicy {
    /// Denied tools
    denied_tools: Arc<RwLock<HashSet<String>>>,

    /// Tools requiring approval
    approval_tools: Arc<RwLock<HashSet<String>>>,

    /// Recent decisions cache
    decision_cache: Arc<DashMap<String, (Decision, RiskLevel, Instant)>>,

    /// User behavior tracking
    user_behavior: Arc<DashMap<String, UserBehavior>>,
}

#[derive(Debug, Clone)]
struct UserBehavior {
    tool_usage_count: HashSet<String>,
    last_activity: Instant,
    suspicious_count: u32,
}

impl IntelligentPolicy {
    pub fn new() -> Self {
        Self {
            denied_tools: Arc::new(RwLock::new(HashSet::new())),
            approval_tools: Arc::new(RwLock::new(HashSet::new())),
            decision_cache: Arc::new(DashMap::new()),
            user_behavior: Arc::new(DashMap::new()),
        }
    }

    /// Check if tool execution is allowed
    pub async fn check_tool(&self, tool_name: &str, user_id: &str) -> PolicyCheck {
        // Check cache first
        let cache_key = format!("{}:{}", user_id, tool_name);
        if let Some(cached) = self.decision_cache.get(&cache_key) {
            let (decision, risk_level, timestamp) = cached.value();
            if timestamp.elapsed() < Duration::from_secs(60) {
                return PolicyCheck {
                    decision: *decision,
                    reason: "Cached decision".to_string(),
                    risk_level: *risk_level,
                };
            }
        }

        // Check if tool is denied
        let denied = self.denied_tools.read().await;
        if denied.contains(tool_name) {
            return PolicyCheck {
                decision: Decision::Deny,
                reason: format!("Tool '{}' is denied by policy", tool_name),
                risk_level: RiskLevel::High,
            };
        }
        drop(denied);

        // Check if tool requires approval
        let approval = self.approval_tools.read().await;
        let needs_approval = approval.contains(tool_name);
        drop(approval);

        // Check user behavior
        let risk_level = self.assess_risk(tool_name, user_id).await;

        let high_risk_tool = is_high_risk_tool(tool_name);
        let decision = match (risk_level, needs_approval, high_risk_tool) {
            (RiskLevel::Critical, _, _) => Decision::Deny,
            (_, true, _) => Decision::Escalate,
            (RiskLevel::High, _, true) => Decision::Escalate,
            (RiskLevel::Medium, _, true) => Decision::Escalate,
            _ => Decision::Allow,
        };

        let reason = match decision {
            Decision::Allow => "Tool execution permitted".to_string(),
            Decision::Deny => "High risk detected".to_string(),
            Decision::Escalate => "Requires user approval".to_string(),
        };

        // Cache decision
        self.decision_cache
            .insert(cache_key, (decision, risk_level, Instant::now()));

        // Update user behavior
        self.update_behavior(user_id, tool_name).await;

        PolicyCheck {
            decision,
            reason,
            risk_level,
        }
    }

    /// Assess risk based on user behavior
    async fn assess_risk(&self, tool_name: &str, user_id: &str) -> RiskLevel {
        if let Some(behavior) = self.user_behavior.get(user_id) {
            let behavior = behavior.value();

            // Check for suspicious patterns
            if behavior.suspicious_count > 5 {
                return RiskLevel::Critical;
            }

            if behavior.suspicious_count >= 3 {
                return RiskLevel::High;
            }

            // Check if tool is new for this user
            if !behavior.tool_usage_count.contains(tool_name) {
                return RiskLevel::Medium;
            }
        }

        RiskLevel::Low
    }

    /// Update user behavior
    async fn update_behavior(&self, user_id: &str, tool_name: &str) {
        self.user_behavior
            .entry(user_id.to_string())
            .and_modify(|behavior| {
                behavior.tool_usage_count.insert(tool_name.to_string());
                behavior.last_activity = Instant::now();
            })
            .or_insert(UserBehavior {
                tool_usage_count: {
                    let mut set = HashSet::new();
                    set.insert(tool_name.to_string());
                    set
                },
                last_activity: Instant::now(),
                suspicious_count: 0,
            });
    }

    /// Add tool to deny list
    #[cfg(test)]
    pub async fn deny_tool(&self, tool_name: &str) {
        self.denied_tools
            .write()
            .await
            .insert(tool_name.to_string());
    }

    /// Add tool to approval list
    #[cfg(test)]
    pub async fn require_approval(&self, tool_name: &str) {
        self.approval_tools
            .write()
            .await
            .insert(tool_name.to_string());
    }

    /// Mark suspicious activity
    pub async fn mark_suspicious(&self, user_id: &str) {
        self.user_behavior
            .entry(user_id.to_string())
            .and_modify(|behavior| {
                behavior.suspicious_count += 1;
                behavior.last_activity = Instant::now();
            })
            .or_insert(UserBehavior {
                tool_usage_count: HashSet::new(),
                last_activity: Instant::now(),
                suspicious_count: 1,
            });
    }
}

impl Default for IntelligentPolicy {
    fn default() -> Self {
        Self::new()
    }
}

// Global intelligent policy instance
lazy_static::lazy_static! {
    pub static ref INTELLIGENT_POLICY: IntelligentPolicy = IntelligentPolicy::new();
}

fn is_high_risk_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "run_command"
            | "spawn_process"
            | "bash"
            | "exec"
            | "apply_patch"
            | "write_file"
            | "edit_file"
            | "skill"
            | "browser_navigate"
            | "browser_click"
            | "browser_type"
            | "browser_evaluate"
            | "browser_pdf"
            | "browser_screenshot"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_policy_allow() {
        let policy = IntelligentPolicy::new();
        let result = policy.check_tool("read_file", "user1").await;
        assert_eq!(result.decision, Decision::Allow);
    }

    #[tokio::test]
    async fn test_policy_deny() {
        let policy = IntelligentPolicy::new();
        policy.deny_tool("dangerous_tool").await;
        let result = policy.check_tool("dangerous_tool", "user1").await;
        assert_eq!(result.decision, Decision::Deny);
    }

    #[tokio::test]
    async fn test_mark_suspicious_initializes_user_state() {
        let policy = IntelligentPolicy::new();
        let user = "new-user";

        policy.mark_suspicious(user).await;

        let result = policy.check_tool("read_file", user).await;
        assert!(matches!(
            result.risk_level,
            RiskLevel::Medium | RiskLevel::Low
        ));

        let behavior = policy
            .user_behavior
            .get(user)
            .expect("user state should exist");
        assert_eq!(behavior.suspicious_count, 1);
    }
}
