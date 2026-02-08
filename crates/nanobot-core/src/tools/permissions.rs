use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Security profile defining what a tool/skill can do
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityProfile {
    pub name: String,
    pub allow_commands: bool,
    pub allow_execution: bool,
    pub allow_file_write: bool,
    pub allow_file_delete: bool,
    pub filesystem_scope: Option<PathBuf>,
    pub network_allowlist: Vec<String>,
}

impl SecurityProfile {
    /// Trust profile - Full access, no restrictions
    pub fn trust() -> Self {
        Self {
            name: "trust".to_string(),
            allow_commands: true,
            allow_execution: true,
            allow_file_write: true,
            allow_file_delete: true,
            filesystem_scope: None, // No restriction
            network_allowlist: vec!["*".to_string()], // All networks
        }
    }

    /// Standard profile - Intelligent defaults, ask for dangerous operations
    pub fn standard(workspace: PathBuf) -> Self {
        Self {
            name: "standard".to_string(),
            allow_commands: false, // Requires confirmation
            allow_execution: false, // Requires confirmation
            allow_file_write: true,
            allow_file_delete: false, // Requires confirmation
            filesystem_scope: Some(workspace),
            network_allowlist: vec![], // Requires confirmation
        }
    }

    /// Strict profile - Read-only, minimal access
    pub fn strict(workspace: PathBuf) -> Self {
        Self {
            name: "strict".to_string(),
            allow_commands: false,
            allow_execution: false,
            allow_file_write: false,
            allow_file_delete: false,
            filesystem_scope: Some(workspace),
            network_allowlist: vec![],
        }
    }
}

/// Permission decision for a requested action
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask, // Requires user confirmation
}

/// Type of operation being requested
#[derive(Debug, Clone)]
pub enum Operation {
    ReadFile(PathBuf),
    WriteFile(PathBuf),
    DeleteFile(PathBuf),
    ExecuteCommand(String),
    NetworkRequest(String),
}

/// Manages security permissions based on active profile
pub struct PermissionManager {
    profile: SecurityProfile,
    session_cache: std::collections::HashMap<String, bool>,
}

impl PermissionManager {
    pub fn new(profile: SecurityProfile) -> Self {
        Self {
            profile,
            session_cache: std::collections::HashMap::new(),
        }
    }

    /// Create from profile name
    pub fn from_profile_name(name: &str, workspace: PathBuf) -> Result<Self> {
        let profile = match name.to_lowercase().as_str() {
            "trust" => SecurityProfile::trust(),
            "standard" => SecurityProfile::standard(workspace),
            "strict" => SecurityProfile::strict(workspace),
            _ => return Err(anyhow!("Unknown security profile: {}", name)),
        };
        Ok(Self::new(profile))
    }

    /// Check if an operation is permitted
    pub fn check_permission(&self, operation: &Operation) -> PermissionDecision {
        match operation {
            Operation::ReadFile(path) => {
                if let Some(scope) = &self.profile.filesystem_scope {
                    if !path.starts_with(scope) {
                        return PermissionDecision::Deny;
                    }
                }
                PermissionDecision::Allow
            }

            Operation::WriteFile(path) => {
                if let Some(scope) = &self.profile.filesystem_scope {
                    if !path.starts_with(scope) {
                        return PermissionDecision::Deny;
                    }
                }

                if self.profile.allow_file_write {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Ask
                }
            }

            Operation::DeleteFile(path) => {
                if let Some(scope) = &self.profile.filesystem_scope {
                    if !path.starts_with(scope) {
                        return PermissionDecision::Deny;
                    }
                }

                if self.profile.allow_file_delete {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Ask
                }
            }

            Operation::ExecuteCommand(_cmd) => {
                if self.profile.allow_execution {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Ask
                }
            }

            Operation::NetworkRequest(url) => {
                if self.profile.network_allowlist.contains(&"*".to_string()) {
                    return PermissionDecision::Allow;
                }

                if self.profile.network_allowlist.iter().any(|allowed| url.contains(allowed)) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Ask
                }
            }
        }
    }

    /// Cache a user decision for the session
    pub fn cache_decision(&mut self, operation_key: String, allowed: bool) {
        self.session_cache.insert(operation_key, allowed);
    }

    /// Check if a decision was cached
    pub fn get_cached_decision(&self, operation_key: &str) -> Option<bool> {
        self.session_cache.get(operation_key).copied()
    }

    /// Get current profile name
    pub fn profile_name(&self) -> &str {
        &self.profile.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trust_profile() {
        let manager = PermissionManager::new(SecurityProfile::trust());
        
        assert_eq!(
            manager.check_permission(&Operation::ExecuteCommand("rm -rf /".to_string())),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn test_standard_profile() {
        let workspace = PathBuf::from("/workspace");
        let manager = PermissionManager::new(SecurityProfile::standard(workspace.clone()));

        // Read should be allowed
        assert_eq!(
            manager.check_permission(&Operation::ReadFile(workspace.join("file.txt"))),
            PermissionDecision::Allow
        );

        // Write should be allowed in workspace
        assert_eq!(
            manager.check_permission(&Operation::WriteFile(workspace.join("file.txt"))),
            PermissionDecision::Allow
        );

        // Delete should require confirmation
        assert_eq!(
            manager.check_permission(&Operation::DeleteFile(workspace.join("file.txt"))),
            PermissionDecision::Ask
        );

        // Command execution should require confirmation
        assert_eq!(
            manager.check_permission(&Operation::ExecuteCommand("npm install".to_string())),
            PermissionDecision::Ask
        );
    }

    #[test]
    fn test_strict_profile() {
        let workspace = PathBuf::from("/workspace");
        let manager = PermissionManager::new(SecurityProfile::strict(workspace.clone()));

        // Read should be allowed in workspace
        assert_eq!(
            manager.check_permission(&Operation::ReadFile(workspace.join("file.txt"))),
            PermissionDecision::Allow
        );

        // Write should require confirmation
        assert_eq!(
            manager.check_permission(&Operation::WriteFile(workspace.join("file.txt"))),
            PermissionDecision::Ask
        );

        // Delete should require confirmation
        assert_eq!(
            manager.check_permission(&Operation::DeleteFile(workspace.join("file.txt"))),
            PermissionDecision::Ask
        );
    }

    #[test]
    fn test_filesystem_scope() {
        let workspace = PathBuf::from("/workspace");
        let manager = PermissionManager::new(SecurityProfile::standard(workspace.clone()));

        // Outside workspace should be denied
        assert_eq!(
            manager.check_permission(&Operation::ReadFile(PathBuf::from("/etc/passwd"))),
            PermissionDecision::Deny
        );
    }
}
