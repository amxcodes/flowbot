use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::gateway::adapter::ChannelMessage;

use super::confirmation::{
    ConfirmationAdapter, ConfirmationRequest, ConfirmationResponse, RiskLevel,
};

#[derive(Debug, Clone)]
pub struct ChannelConfirmationResponse {
    pub request_id: String,
    pub allowed: bool,
}

pub struct ChannelConfirmationAdapter {
    channel: String,
    outbound_tx: mpsc::Sender<ChannelMessage>,
    response_rx: tokio::sync::Mutex<mpsc::Receiver<ChannelConfirmationResponse>>,
}

impl ChannelConfirmationAdapter {
    pub fn new(
        channel: String,
        outbound_tx: mpsc::Sender<ChannelMessage>,
        response_rx: mpsc::Receiver<ChannelConfirmationResponse>,
    ) -> Self {
        Self {
            channel,
            outbound_tx,
            response_rx: tokio::sync::Mutex::new(response_rx),
        }
    }

    fn format_risk_label(risk: RiskLevel) -> &'static str {
        match risk {
            RiskLevel::Low => "LOW",
            RiskLevel::Medium => "MEDIUM",
            RiskLevel::High => "HIGH",
            RiskLevel::Critical => "CRITICAL",
        }
    }

    fn split_channel(&self) -> (&str, &str) {
        if let Some((channel_name, user_id)) = self.channel.split_once(':') {
            (channel_name, user_id)
        } else {
            ("channel", self.channel.as_str())
        }
    }
}

#[async_trait]
impl ConfirmationAdapter for ChannelConfirmationAdapter {
    async fn request_confirmation(
        &self,
        request: &ConfirmationRequest,
    ) -> Result<ConfirmationResponse> {
        let (channel_name, user_id) = self.split_channel();
        let message = format!(
            "Security confirmation required.\n\nTool: {}\nRisk: {}\nOperation: {}\n\nArgs:\n{}\n\nApprove: /allow {}\nDeny: /deny {}",
            request.tool_name,
            Self::format_risk_label(request.risk_level),
            request.operation,
            request.args,
            request.id,
            request.id
        );

        let outbound = ChannelMessage::new(
            format!("{}:{}", channel_name, user_id),
            channel_name.to_string(),
            message,
        );

        self.outbound_tx.send(outbound).await?;

        let timeout = request
            .timeout
            .unwrap_or(std::time::Duration::from_secs(300));
        let response = tokio::time::timeout(timeout, async {
            loop {
                let mut rx = self.response_rx.lock().await;
                if let Some(event) = rx.recv().await
                    && event.request_id == request.id
                {
                    return Ok(ConfirmationResponse {
                        id: event.request_id,
                        allowed: event.allowed,
                        remember: false,
                    });
                }
            }
        })
        .await;

        match response {
            Ok(result) => result,
            Err(_) => Ok(ConfirmationResponse {
                id: request.id.clone(),
                allowed: false,
                remember: false,
            }),
        }
    }

    fn name(&self) -> &str {
        "Channel"
    }

    fn channel(&self) -> Option<&str> {
        Some(&self.channel)
    }
}
