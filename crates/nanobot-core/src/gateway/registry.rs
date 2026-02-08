use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use anyhow::Result;
use crate::gateway::adapter::ChannelMessage;

/// Registry for all active channel adapters (Actors)
/// Allows any component to lookup a channel's inbox and send messages to it.
#[derive(Clone)]
pub struct ChannelRegistry {
    /// Map of channel_name -> inbox_sender
    pub adapters: Arc<RwLock<HashMap<String, mpsc::Sender<ChannelMessage>>>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a channel adapter's inbox
    pub async fn register(&self, name: &str, sender: mpsc::Sender<ChannelMessage>) {
        let mut map = self.adapters.write().await;
        map.insert(name.to_lowercase(), sender);
        tracing::info!("🔌 Registered channel adapter: {}", name);
    }

    /// Get a handle to a channel's inbox
    pub async fn get(&self, name: &str) -> Option<mpsc::Sender<ChannelMessage>> {
        let map = self.adapters.read().await;
        map.get(&name.to_lowercase()).cloned()
    }

    /// Helper: Directly send a message to a channel if it exists
    pub async fn send(&self, channel_name: &str, message: ChannelMessage) -> Result<()> {
        if let Some(sender) = self.get(channel_name).await {
            sender.send(message).await.map_err(|_| anyhow::anyhow!("Channel {} closed", channel_name))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Channel {} not found", channel_name))
        }
    }
    
    /// List all registered channels
    pub async fn list_channels(&self) -> Vec<String> {
        let map = self.adapters.read().await;
        map.keys().cloned().collect()
    }
}
