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
                eprintln!("Failed to serialize audit entry: {}", e);
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
                eprintln!("Failed to open audit log: {}", e);
                return;
            }
        };

        if let Err(e) = writeln!(file, "{}", json) {
            eprintln!("Failed to write audit log: {}", e);
        }
    }

    pub fn log_allow(&self, tool_name: &str, session_id: &str, policy: &str, args: serde_json::Value) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Allow,
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            policy: policy.to_string(),
            args,
            reason: None,
        });
    }

    pub fn log_deny(&self, tool_name: &str, session_id: &str, policy: &str, args: serde_json::Value, reason: &str) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Deny,
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            policy: policy.to_string(),
            args,
            reason: Some(reason.to_string()),
        });
    }

    pub fn log_prompt(&self, tool_name: &str, session_id: &str, policy: &str, args: serde_json::Value) {
        self.log(AuditEntry {
            timestamp: Utc::now(),
            decision: AuditDecision::Prompt,
            tool_name: tool_name.to_string(),
            session_id: session_id.to_string(),
            policy: policy.to_string(),
            args,
            reason: None,
        });
    }
}
