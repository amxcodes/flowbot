pub mod admin_token;
pub mod audit;
pub mod secrets;
pub mod session_secrets;
pub mod setup;
pub mod web_password;

// New advanced security modules
pub mod content_security;
pub mod quantum_policy;

// Re-export new security types
pub use quantum_policy::{
    Capability, ExecutionContext, InheritanceMode, NetworkPolicy, PolicyDecision, PolicyEvaluation,
    PolicyScope, QuantumPolicyEngine, ResourceLimits, RiskLevel, ScopePolicy, ToolRequest,
};

pub use audit::{
    AuditConfig, AuditRiskLevel, ExecSecurityMode, FindingSeverity, SecurityAuditReport,
    SecurityAuditor, SecurityCategory, SecurityFinding,
};

pub use content_security::{
    AnalysisContext, AnalysisResult, ContentSecurityAnalyzer, Threat, ThreatSeverity, ThreatType,
    TrustLevel,
};

pub use admin_token::{clear_admin_token, read_admin_token, write_admin_token};
pub use secrets::SecretManager;
pub use session_secrets::{
    SessionSecrets, get_or_create_session_secrets, read_session_secrets, write_session_secrets,
};
pub use setup::{run_setup_wizard, setup_master_password_if_missing, verify_password};
pub use web_password::{clear_web_password, read_web_password, write_web_password};

pub fn secure_eq(lhs: &str, rhs: &str) -> bool {
    let left = lhs.as_bytes();
    let right = rhs.as_bytes();

    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());

    for idx in 0..max_len {
        let l = *left.get(idx).unwrap_or(&0);
        let r = *right.get(idx).unwrap_or(&0);
        diff |= (l ^ r) as usize;
    }

    diff == 0
}

pub fn read_primary_password() -> Option<String> {
    if let Ok(p) = std::env::var("NANOBOT_MASTER_PASSWORD")
        && !p.trim().is_empty()
    {
        return Some(p);
    }

    if let Ok(p) = std::env::var("NANOBOT_WEB_PASSWORD")
        && !p.trim().is_empty()
    {
        return Some(p);
    }

    read_web_password()
        .ok()
        .flatten()
        .filter(|p| !p.trim().is_empty())
}

pub fn read_admin_auth_secret() -> Option<String> {
    std::env::var("NANOBOT_ADMIN_TOKEN")
        .ok()
        .or_else(|| read_admin_token().ok().flatten())
        .or_else(read_primary_password)
        .filter(|s| !s.trim().is_empty())
}

pub fn verify_admin_rotation_secret(candidate: &str) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }

    if let Ok(env_admin) = std::env::var("NANOBOT_ADMIN_TOKEN")
        && !env_admin.trim().is_empty()
        && secure_eq(candidate, &env_admin)
    {
        return true;
    }

    if let Ok(Some(file_admin)) = read_admin_token()
        && !file_admin.trim().is_empty()
        && secure_eq(candidate, &file_admin)
    {
        return true;
    }

    if let Some(primary) = read_primary_password()
        && !primary.trim().is_empty()
        && secure_eq(candidate, &primary)
    {
        return true;
    }

    false
}

pub fn enforce_runtime_security_baseline() -> anyhow::Result<()> {
    let env = std::env::var("NANOBOT_ENV")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "development".to_string());

    if production_mode_guard_enabled() {
        enforce_production_mode_guard(&env)?;
    }

    if env != "production" && env != "prod" {
        return Ok(());
    }

    let ws_required = std::env::var("NANOBOT_GATEWAY_REQUIRE_TOKEN")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true);

    let insecure_override = std::env::var("NANOBOT_ALLOW_INSECURE_WS")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    if !ws_required || insecure_override {
        anyhow::bail!(
            "Refusing to start in production with insecure websocket auth. Ensure NANOBOT_GATEWAY_REQUIRE_TOKEN=true and NANOBOT_ALLOW_INSECURE_WS is unset."
        );
    }

    let allow_antigravity_hardcoded = unsafe_antigravity_override_enabled();
    let antigravity_active = match crate::config::Config::load() {
        Ok(config) => {
            config.default_provider.eq_ignore_ascii_case("antigravity")
                || config
                    .llm
                    .as_ref()
                    .map(|c| {
                        c.failover_chain
                            .iter()
                            .any(|p| p.eq_ignore_ascii_case("antigravity"))
                    })
                    .unwrap_or(false)
        }
        Err(_) => true,
    };

    if antigravity_active && !allow_antigravity_hardcoded {
        anyhow::bail!(
            "Refusing to start in production with antigravity hardcoded credentials path active. Set BOTH NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY=1 and NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM=I_UNDERSTAND_THIS_IS_INSECURE to override (unsafe)."
        );
    }

    if antigravity_active {
        if let Some(ttl_minutes) = std::env::var("NANOBOT_UNSAFE_OVERRIDE_TTL_MIN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0)
        {
            let issued_at = std::env::var("NANOBOT_UNSAFE_OVERRIDE_ISSUED_AT_EPOCH")
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "NANOBOT_UNSAFE_OVERRIDE_TTL_MIN is set but NANOBOT_UNSAFE_OVERRIDE_ISSUED_AT_EPOCH is missing or invalid"
                    )
                })?;

            let now = chrono::Utc::now().timestamp();
            let age_seconds = now.saturating_sub(issued_at);
            let max_age_seconds = ttl_minutes.saturating_mul(60);
            if age_seconds > max_age_seconds {
                anyhow::bail!(
                    "Unsafe antigravity override expired (age={}s, max={}s)",
                    age_seconds,
                    max_age_seconds
                );
            }
        }

        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown-host".to_string());
        tracing::error!(
            env = %env,
            hostname = %hostname,
            version = env!("CARGO_PKG_VERSION"),
            "UNSAFE override active: allowing antigravity hardcoded credential path in production"
        );
        set_unsafe_override_metric(1.0);
    } else {
        set_unsafe_override_metric(0.0);
    }

    Ok(())
}

fn parse_bool_env(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn production_mode_guard_enabled() -> bool {
    parse_bool_env("PRODUCTION_MODE")
}

fn configured_llm_queue_wait_ms() -> u64 {
    std::env::var("NANOBOT_LLM_QUEUE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or_else(|| {
            let provider = std::env::var("NANOBOT_PROVIDER")
                .ok()
                .or_else(|| std::env::var("NANOBOT_DEFAULT_PROVIDER").ok())
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_default();
            if provider == "antigravity" {
                1000
            } else {
                5000
            }
        })
}

fn production_queue_wait_ceiling_ms() -> u64 {
    std::env::var("NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(15000)
}

fn is_mock_provider_enabled() -> bool {
    parse_bool_env("NANOBOT_MOCK_PROVIDER")
}

fn adaptive_permits_enabled_for_guard() -> bool {
    std::env::var("NANOBOT_LLM_ADAPTIVE_PERMITS")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or_else(|| {
            let provider = std::env::var("NANOBOT_PROVIDER")
                .ok()
                .or_else(|| std::env::var("NANOBOT_DEFAULT_PROVIDER").ok())
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_default();
            provider == "antigravity"
        })
}

fn record_production_guard_failure(reason: &str) {
    if tokio::runtime::Handle::try_current().is_ok() {
        crate::metrics::GLOBAL_METRICS
            .increment_counter(&format!("production_guard_failed_total{{reason={}}}", reason), 1);
    }
    tracing::error!(reason = reason, "Production mode guard rejected startup");
}

fn fail_production_guard(reason: &'static str, detail: String) -> anyhow::Error {
    record_production_guard_failure(reason);
    anyhow::anyhow!("Production mode guard failed ({}): {}", reason, detail)
}

fn enforce_production_mode_guard(env: &str) -> anyhow::Result<()> {
    if env != "production" && env != "prod" {
        return Err(fail_production_guard(
            "not_production_env",
            "PRODUCTION_MODE=1 requires NANOBOT_ENV=production or NANOBOT_ENV=prod".to_string(),
        ));
    }

    let unsafe_flags = [
        "NANOBOT_ALLOW_DANGEROUS_COMMANDS",
        "NANOBOT_ALLOW_INSECURE_WS",
        "NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY",
        "NANOBOT_ALLOW_UNSANDBOXED_NODE_FALLBACK",
    ];
    let enabled_unsafe: Vec<&str> = unsafe_flags
        .into_iter()
        .filter(|key| parse_bool_env(key))
        .collect();
    if !enabled_unsafe.is_empty() {
        return Err(fail_production_guard(
            "unsafe_flags",
            format!("unsafe override flags enabled: {}", enabled_unsafe.join(", ")),
        ));
    }

    if is_mock_provider_enabled() {
        return Err(fail_production_guard(
            "mock_provider",
            "NANOBOT_MOCK_PROVIDER must be disabled in PRODUCTION_MODE".to_string(),
        ));
    }

    if !adaptive_permits_enabled_for_guard() {
        return Err(fail_production_guard(
            "adaptive_permits_disabled",
            "NANOBOT_LLM_ADAPTIVE_PERMITS must be enabled in PRODUCTION_MODE"
                .to_string(),
        ));
    }

    let queue_wait_ms = configured_llm_queue_wait_ms();
    let ceiling_ms = production_queue_wait_ceiling_ms();
    if queue_wait_ms > ceiling_ms {
        return Err(fail_production_guard(
            "queue_wait_ceiling",
            format!(
                "NANOBOT_LLM_QUEUE_WAIT_MS={} exceeds NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS={}",
                queue_wait_ms, ceiling_ms
            ),
        ));
    }

    if tokio::runtime::Handle::try_current().is_ok() {
        crate::metrics::GLOBAL_METRICS.set_gauge("production_mode_guard_active", 1.0);
    }
    Ok(())
}

fn unsafe_antigravity_override_enabled() -> bool {
    let allow = std::env::var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    let confirm = std::env::var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM")
        .ok()
        .map(|v| v.trim().to_string())
        .unwrap_or_default();

    allow && confirm == "I_UNDERSTAND_THIS_IS_INSECURE"
}

fn set_unsafe_override_metric(value: f64) {
    if tokio::runtime::Handle::try_current().is_ok() {
        crate::metrics::GLOBAL_METRICS.set_gauge("unsafe_overrides_active{type=antigravity}", value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static ENV_LOCK: once_cell::sync::Lazy<Mutex<()>> =
        once_cell::sync::Lazy::new(|| Mutex::new(()));

    fn clear_production_mode_test_overrides() {
        unsafe {
            env::remove_var("PRODUCTION_MODE");
            env::remove_var("NANOBOT_ALLOW_DANGEROUS_COMMANDS");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
            env::remove_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY");
            env::remove_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM");
            env::remove_var("NANOBOT_ALLOW_UNSANDBOXED_NODE_FALLBACK");
            env::remove_var("NANOBOT_MOCK_PROVIDER");
            env::remove_var("NANOBOT_LLM_ADAPTIVE_PERMITS");
            env::remove_var("NANOBOT_LLM_QUEUE_WAIT_MS");
            env::remove_var("NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS");
            env::remove_var("NANOBOT_CONFIG_PATH");
        }
    }

    #[test]
    fn production_refuses_insecure_ws_flags() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "false");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
        }
        assert!(enforce_runtime_security_baseline().is_err());

        unsafe {
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::set_var("NANOBOT_ALLOW_INSECURE_WS", "1");
        }
        assert!(enforce_runtime_security_baseline().is_err());

        unsafe {
            env::set_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY", "1");
            env::set_var(
                "NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM",
                "I_UNDERSTAND_THIS_IS_INSECURE",
            );
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
        }
        assert!(enforce_runtime_security_baseline().is_ok());
    }

    #[test]
    fn production_refuses_antigravity_with_single_override_var() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        let mut tmp = tempfile::NamedTempFile::new().expect("temp config");
        use std::io::Write;
        writeln!(&mut tmp, "default_provider = \"antigravity\"").expect("write config");
        tmp.flush().expect("flush config");

        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
            env::set_var("NANOBOT_CONFIG_PATH", tmp.path());
            env::set_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY", "1");
            env::remove_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM");
        }
        assert!(enforce_runtime_security_baseline().is_err());
    }

    #[test]
    fn production_refuses_antigravity_with_confirm_only() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        let mut tmp = tempfile::NamedTempFile::new().expect("temp config");
        use std::io::Write;
        writeln!(&mut tmp, "default_provider = \"antigravity\"").expect("write config");
        tmp.flush().expect("flush config");

        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
            env::set_var("NANOBOT_CONFIG_PATH", tmp.path());
            env::remove_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY");
            env::set_var(
                "NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM",
                "I_UNDERSTAND_THIS_IS_INSECURE",
            );
        }
        assert!(enforce_runtime_security_baseline().is_err());
    }

    #[test]
    fn production_refuses_antigravity_with_wrong_confirm_string() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        let mut tmp = tempfile::NamedTempFile::new().expect("temp config");
        use std::io::Write;
        writeln!(&mut tmp, "default_provider = \"antigravity\"").expect("write config");
        tmp.flush().expect("flush config");

        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
            env::set_var("NANOBOT_CONFIG_PATH", tmp.path());
            env::set_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY", "1");
            env::set_var(
                "NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM",
                "WRONG_CONFIRM_STRING",
            );
        }
        assert!(enforce_runtime_security_baseline().is_err());
    }

    #[test]
    fn production_allows_antigravity_with_two_key_unlock() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        let mut tmp = tempfile::NamedTempFile::new().expect("temp config");
        use std::io::Write;
        writeln!(&mut tmp, "default_provider = \"antigravity\"").expect("write config");
        tmp.flush().expect("flush config");

        unsafe {
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
            env::set_var("NANOBOT_CONFIG_PATH", tmp.path());
            env::set_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY", "1");
            env::set_var(
                "NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM",
                "I_UNDERSTAND_THIS_IS_INSECURE",
            );
        }
        assert!(enforce_runtime_security_baseline().is_ok());
    }

    #[test]
    fn production_mode_guard_requires_production_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        unsafe {
            env::set_var("PRODUCTION_MODE", "1");
            env::set_var("NANOBOT_ENV", "development");
        }

        let err = enforce_runtime_security_baseline().expect_err("guard should reject non-prod env");
        assert!(
            err.to_string().contains("not_production_env"),
            "unexpected error: {err}"
        );

        unsafe {
            env::remove_var("PRODUCTION_MODE");
        }
    }

    #[test]
    fn production_mode_guard_rejects_unsafe_flags_mock_and_queue() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        unsafe {
            env::set_var("PRODUCTION_MODE", "1");
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_ALLOW_DANGEROUS_COMMANDS", "1");
        }
        let err = enforce_runtime_security_baseline().expect_err("unsafe flag should be rejected");
        assert!(err.to_string().contains("unsafe_flags"), "unexpected error: {err}");

        unsafe {
            env::remove_var("NANOBOT_ALLOW_DANGEROUS_COMMANDS");
            env::set_var("NANOBOT_MOCK_PROVIDER", "1");
        }
        let err = enforce_runtime_security_baseline().expect_err("mock provider should be rejected");
        assert!(
            err.to_string().contains("mock_provider"),
            "unexpected error: {err}"
        );

        unsafe {
            env::remove_var("NANOBOT_MOCK_PROVIDER");
            env::set_var("NANOBOT_LLM_ADAPTIVE_PERMITS", "false");
        }
        let err = enforce_runtime_security_baseline()
            .expect_err("adaptive permits disabled should be rejected");
        assert!(
            err.to_string().contains("adaptive_permits_disabled"),
            "unexpected error: {err}"
        );

        unsafe {
            env::set_var("NANOBOT_LLM_ADAPTIVE_PERMITS", "true");
            env::set_var("NANOBOT_LLM_QUEUE_WAIT_MS", "30000");
            env::set_var("NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS", "10000");
        }
        let err =
            enforce_runtime_security_baseline().expect_err("queue wait ceiling should be rejected");
        assert!(
            err.to_string().contains("queue_wait_ceiling"),
            "unexpected error: {err}"
        );

        unsafe {
            env::remove_var("PRODUCTION_MODE");
            env::remove_var("NANOBOT_LLM_ADAPTIVE_PERMITS");
            env::remove_var("NANOBOT_LLM_QUEUE_WAIT_MS");
            env::remove_var("NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS");
        }
    }

    #[test]
    fn production_mode_guard_allows_safe_configuration() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_production_mode_test_overrides();

        let mut tmp = tempfile::NamedTempFile::new().expect("temp config");
        use std::io::Write;
        writeln!(&mut tmp, "default_provider = \"openai\"").expect("write config");
        writeln!(&mut tmp, "[providers.openai]").expect("write config");
        writeln!(&mut tmp, "api_key = \"\"").expect("write config");
        writeln!(&mut tmp, "[providers.antigravity]").expect("write config");
        writeln!(&mut tmp, "api_key = \"\"").expect("write config");
        tmp.flush().expect("flush config");

        unsafe {
            env::set_var("PRODUCTION_MODE", "1");
            env::set_var("NANOBOT_ENV", "production");
            env::set_var("NANOBOT_GATEWAY_REQUIRE_TOKEN", "true");
            env::remove_var("NANOBOT_ALLOW_INSECURE_WS");
            env::remove_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY");
            env::remove_var("NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM");
            env::remove_var("NANOBOT_MOCK_PROVIDER");
            env::set_var("NANOBOT_LLM_ADAPTIVE_PERMITS", "true");
            env::set_var("NANOBOT_LLM_QUEUE_WAIT_MS", "5000");
            env::set_var("NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS", "15000");
            env::set_var("NANOBOT_CONFIG_PATH", tmp.path());
        }

        if let Err(err) = enforce_runtime_security_baseline() {
            panic!("safe production mode config should pass: {err}");
        }

        unsafe {
            env::remove_var("PRODUCTION_MODE");
            env::remove_var("NANOBOT_LLM_ADAPTIVE_PERMITS");
            env::remove_var("NANOBOT_LLM_QUEUE_WAIT_MS");
            env::remove_var("NANOBOT_PROD_MAX_LLM_QUEUE_WAIT_MS");
            env::remove_var("NANOBOT_CONFIG_PATH");
        }
    }
}
