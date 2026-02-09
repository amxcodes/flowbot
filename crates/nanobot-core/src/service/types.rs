use serde::{Deserialize, Serialize};

/// Service runtime status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRuntime {
    pub status: ServiceStatus,
    pub pid: Option<u32>,
    pub uptime_seconds: Option<u64>,
    pub last_exit_code: Option<i32>,
    pub last_exit_reason: Option<String>,
}

/// Service status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Unknown,
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceStatus::Running => write!(f, "running"),
            ServiceStatus::Stopped => write!(f, "stopped"),
            ServiceStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// JSON response for service operations
#[derive(Debug, Serialize)]
pub struct ServiceResponse {
    pub ok: bool,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceInfo>,
}

#[derive(Debug, Serialize)]
pub struct ServiceInfo {
    pub label: String,
    pub loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<ServiceRuntime>,
}
