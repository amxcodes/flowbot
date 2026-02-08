use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Incoming event from a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingEvent {
    pub channel_id: String,
    pub user_id: String,
    pub content: String,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Generic channel trait for multi-channel support
#[async_trait]
pub trait Channel: Send + Sync {
    /// Unique channel identifier (e.g., "telegram", "terminal", "slack")
    fn id(&self) -> &str;
    
    /// Start listening for events and send them to the gateway
    async fn start(&self, tx: mpsc::Sender<IncomingEvent>) -> Result<()>;
    
    /// Send a message to a specific target (user/channel)
    async fn send(&self, target: &str, content: &str) -> Result<()>;
    
    /// Stop the channel
    async fn stop(&self) -> Result<()> {
        Ok(()) // Default implementation: no-op
    }
}
