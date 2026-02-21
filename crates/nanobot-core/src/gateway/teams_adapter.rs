use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::adapter::{ChannelAdapter, ChannelMessage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamsConfig {
    pub webhook_url: String,
}

pub struct TeamsAdapter {
    config: TeamsConfig,
}

impl TeamsAdapter {
    pub fn new(config: TeamsConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ChannelAdapter for TeamsAdapter {
    async fn send_message(&self, _user_id: &str, content: &str) -> Result<()> {
        let payload = serde_json::json!({"text": content});
        let client = reqwest::Client::new();
        client
            .post(&self.config.webhook_url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn send_stream_chunk(&self, user_id: &str, chunk: &str) -> Result<()> {
        self.send_message(user_id, chunk).await
    }

    fn channel_name(&self) -> &str {
        "teams"
    }
}

pub async fn run_outbound_loop(
    config: TeamsConfig,
    mut inbox: mpsc::Receiver<ChannelMessage>,
) -> Result<()> {
    let adapter = TeamsAdapter::new(config);
    while let Some(msg) = inbox.recv().await {
        let _ = adapter.send_message(&msg.user_id, &msg.content).await;
    }
    Ok(())
}
