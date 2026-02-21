use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A confirmation request sent to the user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationRequest {
    pub id: String,
    pub tool_name: String,
    pub operation: String,
    pub args: String,
    pub risk_level: RiskLevel,
    pub timeout: Option<Duration>,
    pub channel: Option<String>,
}

/// Risk level for an operation
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,      // Read operations
    Medium,   // Write operations, safe commands
    High,     // Delete operations, network requests
    Critical, // System commands, dangerous operations
}

/// User's response to a confirmation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationResponse {
    pub id: String,
    pub allowed: bool,
    pub remember: bool, // "Always allow this" checkbox
}

/// Trait for confirmation adapters (CLI, Telegram, WhatsApp, etc.)
#[async_trait]
pub trait ConfirmationAdapter: Send + Sync {
    /// Request confirmation from the user
    /// Returns true if approved, false if denied
    async fn request_confirmation(
        &self,
        request: &ConfirmationRequest,
    ) -> Result<ConfirmationResponse>;

    /// Get the adapter's name (for logging/debugging)
    fn name(&self) -> &str;

    /// Channel identifier this adapter serves (e.g., "telegram:123")
    fn channel(&self) -> Option<&str> {
        None
    }

    /// Check if the adapter is available/ready
    async fn is_available(&self) -> bool {
        true
    }
}

/// The confirmation service that manages adapters
pub struct ConfirmationService {
    adapters: Vec<Box<dyn ConfirmationAdapter>>,
    default_timeout: Duration,
    pending: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl ConfirmationService {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
            default_timeout: Duration::from_secs(300), // 5 minutes
            pending: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Register a new adapter
    pub fn register_adapter(&mut self, adapter: Box<dyn ConfirmationAdapter>) {
        self.adapters.push(adapter);
    }

    /// Request confirmation using the first available adapter
    pub async fn request_confirmation(
        &self,
        mut request: ConfirmationRequest,
    ) -> Result<ConfirmationResponse> {
        if request.id.is_empty() {
            request.id = uuid::Uuid::new_v4().to_string();
        }

        // Set default timeout if not specified
        if request.timeout.is_none() {
            request.timeout = Some(self.default_timeout);
        }

        {
            let mut pending = self.pending.lock().unwrap();
            pending.insert(request.id.clone(), std::time::Instant::now());
        }

        if let Some(channel) = request.channel.as_deref() {
            for adapter in &self.adapters {
                if adapter.channel() == Some(channel) && adapter.is_available().await {
                    tracing::info!("Requesting confirmation via adapter: {}", adapter.name());
                    let response = adapter.request_confirmation(&request).await;
                    return self.validate_response(&request, response);
                }
            }
        }

        // Fallback: Try any available adapter
        for adapter in &self.adapters {
            if adapter.is_available().await {
                tracing::info!("Requesting confirmation via adapter: {}", adapter.name());
                let response = adapter.request_confirmation(&request).await;
                return self.validate_response(&request, response);
            }
        }

        // No adapter available - deny by default (fail-safe)
        tracing::warn!("No confirmation adapter available, denying request");
        Ok(ConfirmationResponse {
            id: request.id,
            allowed: false,
            remember: false,
        })
    }

    fn validate_response(
        &self,
        request: &ConfirmationRequest,
        response: Result<ConfirmationResponse>,
    ) -> Result<ConfirmationResponse> {
        let response = response?;
        let mut pending = self.pending.lock().unwrap();

        if let Some(created_at) = pending.remove(&response.id) {
            let timeout = request.timeout.unwrap_or(self.default_timeout);
            if created_at.elapsed() > timeout {
                return Ok(ConfirmationResponse {
                    id: response.id,
                    allowed: false,
                    remember: false,
                });
            }

            Ok(response)
        } else {
            Ok(ConfirmationResponse {
                id: response.id,
                allowed: false,
                remember: false,
            })
        }
    }
}

impl Default for ConfirmationService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAdapter {
        name: String,
        response: bool,
    }

    #[async_trait]
    impl ConfirmationAdapter for MockAdapter {
        async fn request_confirmation(
            &self,
            request: &ConfirmationRequest,
        ) -> Result<ConfirmationResponse> {
            Ok(ConfirmationResponse {
                id: request.id.clone(),
                allowed: self.response,
                remember: false,
            })
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_confirmation_service() {
        let mut service = ConfirmationService::new();
        service.register_adapter(Box::new(MockAdapter {
            name: "test".to_string(),
            response: true,
        }));

        let request = ConfirmationRequest {
            id: "test_123".to_string(),
            tool_name: "run_command".to_string(),
            operation: "execute".to_string(),
            args: "npm install".to_string(),
            risk_level: RiskLevel::Medium,
            timeout: None,
            channel: None,
        };

        let response = service.request_confirmation(request).await.unwrap();
        assert!(response.allowed);
    }

    #[tokio::test]
    async fn test_no_adapter_denies() {
        let service = ConfirmationService::new();

        let request = ConfirmationRequest {
            id: "test_123".to_string(),
            tool_name: "run_command".to_string(),
            operation: "execute".to_string(),
            args: "rm -rf /".to_string(),
            risk_level: RiskLevel::Critical,
            timeout: None,
            channel: None,
        };

        let response = service.request_confirmation(request).await.unwrap();
        assert!(!response.allowed); // Should deny when no adapter available
    }
}
