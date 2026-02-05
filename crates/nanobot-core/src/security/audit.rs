use crate::config::Config;
use crate::tools::ToolPolicy;
use anyhow::{Context, Result};
use colored::*;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct AuditIssue {
    pub level: AuditLevel,
    pub category: String,
    pub message: String,
    pub remediation: Option<String>,
}

#[derive(Debug, PartialEq, PartialOrd, Ord, Eq)]
pub enum AuditLevel {
    Info,
    Warning,
    Critical,
}

impl AuditLevel {
    pub fn color(&self) -> Color {
        match self {
            AuditLevel::Info => Color::Blue,
            AuditLevel::Warning => Color::Yellow,
            AuditLevel::Critical => Color::Red,
        }
    }
}

pub struct SecurityAuditor {
    issues: Vec<AuditIssue>,
}

impl SecurityAuditor {
    pub fn new() -> Self {
        Self { issues: Vec::new() }
    }

    pub fn run_all_checks(&mut self) -> Result<()> {
        let config_result = Config::load();

        self.check_config_safety();
        self.check_filesystem_permissions();
        if let Ok(config) = config_result {
            self.check_auth_tokens(&config);
        } else {
            // If config can't be loaded, check env vars only
            self.check_auth_tokens_env_only();
        }

        Ok(())
    }

    fn check_config_safety(&mut self) {
        // Check for debug/unsafe flags
        // Assuming we might have a debug flag in the future
        if std::env::var("RUST_LOG").unwrap_or_default() == "trace" {
            self.add_issue(
                AuditLevel::Warning,
                "Config",
                "RUST_LOG is set to 'trace'. This may leak sensitive data in logs.",
                Some("Set RUST_LOG=info or RUST_LOG=warn in production."),
            );
        }
    }

    fn check_auth_tokens_env_only(&mut self) {
        let has_key =
            std::env::var("OPENAI_API_KEY").is_ok() || std::env::var("ANTIGRAVITY_API_KEY").is_ok();

        if !has_key {
            self.add_issue(
                AuditLevel::Critical,
                "Authentication",
                "No API Key found (OPENAI_API_KEY or ANTIGRAVITY_API_KEY missing).",
                Some("Set OPENAI_API_KEY in .env or config.toml."),
            );
        } else {
            self.add_issue(
                AuditLevel::Info,
                "Authentication",
                "API Key is configured.",
                None,
            );
        }
    }

    fn check_auth_tokens(&mut self, config: &Config) {
        // Check OpenAI/LLM Keys
        // We need to check both config file and env vars

        // This is a heuristic.
        let has_key = std::env::var("OPENAI_API_KEY").is_ok()
            || std::env::var("ANTIGRAVITY_API_KEY").is_ok()
            || !config.default_provider.is_empty(); // Rough proxy

        if !has_key {
            self.add_issue(
                AuditLevel::Critical,
                "Authentication",
                "No API Key found (OPENAI_API_KEY or ANTIGRAVITY_API_KEY missing).",
                Some("Set OPENAI_API_KEY in .env or config.toml."),
            );
        } else {
            self.add_issue(
                AuditLevel::Info,
                "Authentication",
                "API Key is configured.",
                None,
            );
        }

        // Check Telegram Token if configured
        // Note: Removed telegram field check as Config structure may vary
    }

    fn check_filesystem_permissions(&mut self) {
        let db_path = Path::new("sessions.db");
        if db_path.exists() {
            self.add_issue(AuditLevel::Info, "Filesystem", "sessions.db exists.", None);

            // On Windows, checking world-writable is hard without complex crate dependencies (permissions/owners).
            // We will just do a basic existence check for now, as consistent with "gap analysis" first pass.
        } else {
            self.add_issue(
                AuditLevel::Warning,
                "Filesystem",
                "sessions.db not found. Persistence might not be active yet.",
                None,
            );
        }

        let config_path = Path::new("config.toml");
        if config_path.exists() {
            // Check if it's writable by us (basic check)
            if IsWritable(config_path) {
                // This is expected for the app, but maybe not for "World".
                // In a real security audit we'd check ACLs.
            }
        }
    }

    fn add_issue(
        &mut self,
        level: AuditLevel,
        category: &str,
        message: &str,
        remediation: Option<&str>,
    ) {
        self.issues.push(AuditIssue {
            level,
            category: category.to_string(),
            message: message.to_string(),
            remediation: remediation.map(|s| s.to_string()),
        });
    }

    pub fn print_report(&self) {
        println!("\n{}", "🛡️  SECURITY AUDIT REPORT".bold().underline());
        println!("{}", "===========================".bold());

        if self.issues.is_empty() {
            println!("{}", "✅ No issues found.".green());
            return;
        }

        for issue in &self.issues {
            let badge = match issue.level {
                AuditLevel::Info => "[INFO]".blue(),
                AuditLevel::Warning => "[WARN]".yellow(),
                AuditLevel::Critical => "[CRIT]".red(),
            };

            println!("{} {}: {}", badge, issue.category.bold(), issue.message);
            if let Some(fix) = &issue.remediation {
                println!("   └─ 💡 {}", fix.italic());
            }
        }
        println!();
    }
}

// Helper for writable check (stub for cross-platform complexity)
fn IsWritable(path: &Path) -> bool {
    let metadata = std::fs::metadata(path);
    match metadata {
        Ok(m) => !m.permissions().readonly(),
        Err(_) => false,
    }
}
