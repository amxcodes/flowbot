use super::confirmation::{ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

/// WebSocket/Gateway confirmation adapter
/// Sends confirmation requests as JSON events through WebSocket
pub struct GatewayConfirmationAdapter {
    /// Channel to send confirmation requests to the gateway
    request_tx: mpsc::Sender<GatewayConfirmationEvent>,
    /// Channel to receive responses from the gateway (wrapped in Mutex for interior mutability)
    response_rx: Mutex<mpsc::Receiver<GatewayConfirmationEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GatewayConfirmationEvent {
    #[serde(rename = "confirmation_request")]
    Request {
        id: String,
        tool_name: String,
        operation: String,
        args: String,
        risk_level: String,
    },
    #[serde(rename = "confirmation_response")]
    Response {
        id: String,
        allowed: bool,
        remember: bool,
    },
}

impl GatewayConfirmationAdapter {
    pub fn new(
        request_tx: mpsc::Sender<GatewayConfirmationEvent>,
        response_rx: mpsc::Receiver<GatewayConfirmationEvent>,
    ) -> Self {
        Self {
            request_tx,
            response_rx: Mutex::new(response_rx),
        }
    }
}

#[async_trait]
impl ConfirmationAdapter for GatewayConfirmationAdapter {
    async fn request_confirmation(&self, request: &ConfirmationRequest) -> Result<ConfirmationResponse> {
        // Send request event to gateway
        let event = GatewayConfirmationEvent::Request {
            id: request.id.clone(),
            tool_name: request.tool_name.clone(),
            operation: request.operation.clone(),
            args: request.args.clone(),
            risk_level: format!("{:?}", request.risk_level),
        };

        self.request_tx.send(event).await?;

        // Wait for response with timeout
        let timeout = request.timeout.unwrap_or(std::time::Duration::from_secs(300));
        
        match tokio::time::timeout(timeout, async {
            loop {
                let mut rx = self.response_rx.lock().await;
                if let Some(event) = rx.recv().await {
                    if let GatewayConfirmationEvent::Response { id, allowed, remember } = event {
                        if id == request.id {
                            return Ok(ConfirmationResponse {
                                id,
                                allowed,
                                remember,
                            });
                        }
                    }
                }
            }
        })
        .await
        {
            Ok(response) => response,
            Err(_) => {
                // Timeout - deny by default
                tracing::warn!("Gateway confirmation timeout for request {}", request.id);
                Ok(ConfirmationResponse {
                    id: request.id.clone(),
                    allowed: false,
                    remember: false,
                })
            }
        }
    }

    fn name(&self) -> &str {
        "Gateway"
    }

    async fn is_available(&self) -> bool {
        // Check if channels are open (basic availability check)
        !self.request_tx.is_closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gateway_adapter_creation() {
        let (tx, _rx) = mpsc::channel(10);
        let (_tx2, rx2) = mpsc::channel(10);
        let adapter = GatewayConfirmationAdapter::new(tx, rx2);
        assert_eq!(adapter.name(), "Gateway");
        assert!(adapter.is_available().await);
    }

    #[test]
    fn test_event_serialization() {
        let event = GatewayConfirmationEvent::Request {
            id: "test123".to_string(),
            tool_name: "run_command".to_string(),
            operation: "ExecuteCommand".to_string(),
            args: "npm install".to_string(),
            risk_level: "High".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("confirmation_request"));
        assert!(json.contains("test123"));
    }
}
