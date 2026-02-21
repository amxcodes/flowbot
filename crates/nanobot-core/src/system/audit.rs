use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditDecision {
    Allow,
    Deny,
    Prompt,
    Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub decision: AuditDecision,
    pub tool_name: String,
    pub session_id: String,
    pub policy: String,
    pub args: serde_json::Value,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

pub struct AuditLogger {
    log_path: PathBuf,
}

impl AuditLogger {
    pub fn new(log_path: PathBuf) -> Self {
        Self { log_path }
    }

    pub fn log(&self, entry: AuditEntry) {
        let json = match serde_json::to_string(&entry) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize audit entry: {}", e);
                return;
            }
        };

        if let Some(parent) = self.log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to open audit log: {}", e);
                return;
            }
        };

        if let Err(e) = writeln!(file, "{}", json) {
            tracing::warn!("Failed to write audit log: {}", e);
        }
    }

    pub fn log_allow(
        &self,
        tool_name: &str,
        session_id: &str,
        policy: &str,
        args: serde_json::Value,
    ) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Allow,
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            policy: policy.to_string(),
            args,
            reason: None,
            provider: None,
            model: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        });
    }

    pub fn log_deny(
        &self,
        tool_name: &str,
        session_id: &str,
        policy: &str,
        args: serde_json::Value,
        reason: &str,
    ) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Deny,
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            policy: policy.to_string(),
            args,
            reason: Some(reason.to_string()),
            provider: None,
            model: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        });
    }

    pub fn log_prompt(
        &self,
        tool_name: &str,
        session_id: &str,
        policy: &str,
        args: serde_json::Value,
    ) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Prompt,
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            policy: policy.to_string(),
            args,
            reason: None,
            provider: None,
            model: None,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        });
    }

    pub fn log_usage(
        &self,
        session_id: &str,
        provider: &str,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Usage,
            tool_name: "llm".to_string(),
            session_id: session_id.to_string(),
            policy: "telemetry".to_string(),
            args: serde_json::json!({}),
            reason: None,
            provider: Some(provider.to_string()),
            model: Some(model.to_string()),
            prompt_tokens: Some(prompt_tokens),
            completion_tokens: Some(completion_tokens),
            total_tokens: Some(prompt_tokens.saturating_add(completion_tokens)),
        });
    }
}
