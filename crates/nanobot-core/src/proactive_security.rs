//! Fixed proactive security system - Production Ready
//!
//! Simplified version that compiles and works

use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Security check result
#[derive(Debug, Clone)]
pub enum SecurityCheck {
    Safe,
    Unsafe(Vec<SecurityViolation>),
}

#[derive(Debug, Clone)]
pub struct SecurityViolation {
    pub category: ViolationCategory,
    pub description: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationCategory {
    PathTraversal,
    CommandInjection,
    PromptInjection,
    Ssrf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    High,
    Critical,
}

/// Fixed proactive security scanner
pub struct ProactiveSecurity {
    /// Known malicious patterns
    attack_patterns: Arc<RwLock<Vec<AttackPattern>>>,

    /// Blocked IPs/domains
    blocked_entities: Arc<RwLock<HashSet<String>>>,
}

#[derive(Debug, Clone)]
pub struct AttackPattern {
    pub name: String,
    pub category: ViolationCategory,
    pub pattern: regex::Regex,
    pub severity: Severity,
}

impl ProactiveSecurity {
    pub fn new() -> Self {
        let patterns = vec![
            AttackPattern {
                name: "path_traversal".to_string(),
                category: ViolationCategory::PathTraversal,
                pattern: regex::Regex::new(r"\.\./|\.\.\\").unwrap(),
                severity: Severity::High,
            },
            AttackPattern {
                name: "null_byte".to_string(),
                category: ViolationCategory::PathTraversal,
                pattern: regex::Regex::new(r"%00").unwrap(),
                severity: Severity::Critical,
            },
            AttackPattern {
                name: "command_injection".to_string(),
                category: ViolationCategory::CommandInjection,
                pattern: regex::Regex::new(r"[;&|`]").unwrap(),
                severity: Severity::Critical,
            },
            AttackPattern {
                name: "prompt_injection".to_string(),
                category: ViolationCategory::PromptInjection,
                pattern: regex::RegexBuilder::new(
                    r"ignore previous|disregard|forget your instructions",
                )
                .case_insensitive(true)
                .build()
                .unwrap(),
                severity: Severity::High,
            },
            AttackPattern {
                name: "ssrf_internal".to_string(),
                category: ViolationCategory::Ssrf,
                pattern: regex::Regex::new(r"https?://(127\.0\.0\.1|localhost|0\.0\.0\.0)")
                    .unwrap(),
                severity: Severity::High,
            },
        ];

        Self {
            attack_patterns: Arc::new(RwLock::new(patterns)),
            blocked_entities: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Scan input for attack patterns
    pub async fn scan_input(&self, input: &str) -> Vec<SecurityViolation> {
        let mut violations = Vec::new();
        let patterns = self.attack_patterns.read().await;

        for pattern in patterns.iter() {
            if pattern.pattern.is_match(input) {
                violations.push(SecurityViolation {
                    category: pattern.category,
                    description: format!("Pattern '{}' matched", pattern.name),
                    severity: pattern.severity,
                });
            }
        }

        violations
    }

    /// Verify file path is safe
    pub async fn verify_path(&self, path: &Path, base_dir: &Path) -> SecurityCheck {
        let mut violations = Vec::new();

        // Check for path traversal in string representation
        let path_str = path.to_string_lossy();
        if path_str.contains("../") || path_str.contains("..\\") {
            violations.push(SecurityViolation {
                category: ViolationCategory::PathTraversal,
                description: "Path contains parent directory reference".to_string(),
                severity: Severity::High,
            });
        }

        // Enforce workspace boundary via canonicalized path comparison.
        // For non-existent targets (e.g., write operations), canonicalize parent and
        // reconstruct candidate path.
        let base_canon = match base_dir.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                if base_dir.is_absolute() {
                    base_dir.to_path_buf()
                } else {
                    match std::env::current_dir() {
                        Ok(cwd) => cwd.join(base_dir),
                        Err(_) => base_dir.to_path_buf(),
                    }
                }
            }
        };

        let is_absolute = path.is_absolute();
        let candidate_abs = if is_absolute {
            path.to_path_buf()
        } else {
            base_canon.join(path)
        };

        let candidate_canon = if candidate_abs.exists() {
            match candidate_abs.canonicalize() {
                Ok(p) => p,
                Err(_) => candidate_abs.clone(),
            }
        } else {
            match candidate_abs.parent() {
                Some(parent) => match parent.canonicalize() {
                    Ok(parent_canon) => {
                        if let Some(name) = candidate_abs.file_name() {
                            parent_canon.join(name)
                        } else {
                            parent_canon
                        }
                    }
                    Err(_) => candidate_abs.clone(),
                },
                None => candidate_abs.clone(),
            }
        };

        if !is_absolute && !candidate_canon.starts_with(&base_canon) {
            violations.push(SecurityViolation {
                category: ViolationCategory::PathTraversal,
                description: format!(
                    "Path escapes workspace boundary: {} (base: {})",
                    candidate_canon.display(),
                    base_canon.display()
                ),
                severity: Severity::Critical,
            });
        }

        if violations.is_empty() {
            SecurityCheck::Safe
        } else {
            SecurityCheck::Unsafe(violations)
        }
    }

    /// Verify network URL is safe
    pub async fn verify_url(&self, url: &str) -> SecurityCheck {
        let mut violations = Vec::new();

        // Parse URL
        if let Ok(parsed) = reqwest::Url::parse(url) {
            // Check host
            if let Some(host) = parsed.host_str() {
                // Check if it's an IP
                if let Ok(ip) = host.parse::<IpAddr>()
                    && is_private_ip(&ip)
                {
                    violations.push(SecurityViolation {
                        category: ViolationCategory::Ssrf,
                        description: format!("Private IP address: {}", ip),
                        severity: Severity::High,
                    });
                }

                // Check against blocked list
                let blocked = self.blocked_entities.read().await;
                if blocked.contains(host) {
                    violations.push(SecurityViolation {
                        category: ViolationCategory::Ssrf,
                        description: format!("Host is blocked: {}", host),
                        severity: Severity::Critical,
                    });
                }
            }
        }

        // Check for attack patterns in URL
        let pattern_violations = self.scan_input(url).await;
        violations.extend(pattern_violations);

        if violations.is_empty() {
            SecurityCheck::Safe
        } else {
            SecurityCheck::Unsafe(violations)
        }
    }

    /// Verify command is safe
    pub async fn verify_command(
        &self,
        command: &str,
        allowed_commands: &HashSet<String>,
    ) -> SecurityCheck {
        let mut violations = Vec::new();

        // Check for injection patterns
        let pattern_violations = self.scan_input(command).await;
        violations.extend(
            pattern_violations
                .into_iter()
                .filter(|v| matches!(v.category, ViolationCategory::CommandInjection)),
        );

        // Extract base command
        let base_cmd = command.split_whitespace().next().unwrap_or(command);

        // Check if in allowlist
        if !allowed_commands.contains(base_cmd) {
            violations.push(SecurityViolation {
                category: ViolationCategory::CommandInjection,
                description: format!("Command '{}' not in allowlist", base_cmd),
                severity: Severity::High,
            });
        }

        if violations.is_empty() {
            SecurityCheck::Safe
        } else {
            SecurityCheck::Unsafe(violations)
        }
    }

}

impl Default for ProactiveSecurity {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if IP is private
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local(),
        IpAddr::V6(ipv6) => ipv6.is_loopback(),
    }
}

// Global proactive security instance
lazy_static::lazy_static! {
    pub static ref PROACTIVE_SECURITY: ProactiveSecurity = ProactiveSecurity::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_scan_input() {
        let security = ProactiveSecurity::new();
        let violations = security.scan_input("../../../etc/passwd").await;
        assert!(!violations.is_empty());
    }

    #[tokio::test]
    async fn test_verify_path() {
        let security = ProactiveSecurity::new();
        let base = Path::new("/tmp/workspace");

        match security.verify_path(Path::new("file.txt"), base).await {
            SecurityCheck::Safe => (),
            SecurityCheck::Unsafe(_) => panic!("Safe path rejected"),
        }
    }
}
