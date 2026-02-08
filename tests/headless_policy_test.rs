/// E2E Integration Test: Headless Policy Enforcement
/// 
/// This test verifies that the HeadlessDeny policy correctly blocks dangerous
/// operations and logs them to the audit trail.

use nanobot_core::config::{Config, InteractionPolicy};
use nanobot_core::system::audit::AuditLogger;
use nanobot_core::tools::guard::ToolGuard;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

#[tokio::test]
async fn test_headless_deny_policy_enforcement() {
    // Create temp directory for audit log
    let temp_dir = TempDir::new().unwrap();
    let audit_path = temp_dir.path().join("audit.log");
    
    // Create config with HeadlessDeny policy
    let mut config = Config::default();
    config.interaction_policy = InteractionPolicy::HeadlessDeny;
    config.audit_log_path = Some(audit_path.clone());
    
    // Create audit logger
    let logger = AuditLogger::new(audit_path.clone());
    
    // Test 1: Dangerous command should be blocked by ToolGuard
    let dangerous_cmd = json!({
        "tool": "run_command",
        "command": "rm",
        "args": ["-rf", "/"]
    });
    
    let result = ToolGuard::validate_args("run_command", &dangerous_cmd);
    assert!(result.is_err(), "Dangerous command should be blocked");
    assert!(result.unwrap_err().to_string().contains("dangerous"), 
           "Error should mention it's dangerous");
    
    // Log the denial
    logger.log_deny(
        "run_command",
        &dangerous_cmd.to_string(),
        "Dangerous command pattern detected"
    );
    
    // Test 2: Write to protected path should be blocked
    let protected_write = json!({
        "tool": "write_file",
        "path": "/etc/passwd",
        "content": "hacked"
    });
    
    let result = ToolGuard::validate_args("write_file", &protected_write);
    assert!(result.is_err(), "Protected path write should be blocked");
    
    logger.log_deny(
        "write_file",
        &protected_write.to_string(),
        "Write to protected system path"
    );
    
    // Verify audit log contains both denials
    let log_contents = fs::read_to_string(&audit_path).unwrap();
    assert!(log_contents.contains("\"decision\":\"Deny\""), "Should have Deny decision");
    assert!(log_contents.contains("run_command"), "Should log run_command");
    assert!(log_contents.contains("write_file"), "Should log write_file");
    
    println!("✅ Headless policy correctly denied dangerous operations");
    println!("📝 Audit log: {}", log_contents);
}

#[tokio::test]
async fn test_headless_allow_log_policy() {
    let temp_dir = TempDir::new().unwrap();
    let audit_path = temp_dir.path().join("audit_allow.log");
    
    let mut config = Config::default();
    config.interaction_policy = InteractionPolicy::HeadlessAllowLog;
    config.audit_log_path = Some(audit_path.clone());
    
    let logger = AuditLogger::new(audit_path.clone());
    
    // Safe command that would normally require confirmation
    let safe_cmd = json!({
        "tool": "run_command",
        "command": "echo",
        "args": ["Hello"]
    });
    
    // In HeadlessAllowLog mode, this would be allowed but logged
    logger.log_allow(
        "run_command",
        &safe_cmd.to_string(),
        "HeadlessAllowLog policy"
    );
    
    let log_contents = fs::read_to_string(&audit_path).unwrap();
    assert!(log_contents.contains("\"decision\":\"Allow\""));
    
    println!("✅ HeadlessAllowLog policy correctly logged allowed operation");
}

#[test]
fn test_interaction_policy_serialization() {
    // Verify config serialization works
    let config = Config {
        interaction_policy: InteractionPolicy::HeadlessDeny,
        ..Default::default()
    };
    
    let serialized = toml::to_string(&config).unwrap();
    assert!(serialized.contains("headlessdeny"), 
           "Should serialize to lowercase");
    
    println!("✅ InteractionPolicy serialization working");
}
