use anyhow::Result;
use async_trait::async_trait;

use crate::config::DmScope;

/// Unified interface for all communication channels
/// Each channel (Web, Slack, Discord, Telegram) implements this trait
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Send a complete message to a user
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()>;

    /// Send a streaming chunk to a user (for progressive responses)
    async fn send_stream_chunk(&self, user_id: &str, chunk: &str) -> Result<()>;

    /// Get the channel identifier (e.g., "slack", "discord", "web")
    fn channel_name(&self) -> &str;

    /// Get the platform-specific user identifier format
    fn format_user_id(&self, raw_id: &str) -> String {
        format!("{}:{}", self.channel_name(), raw_id)
    }
}

/// Represents a normalized message from any channel
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    pub user_id: String,
    pub channel_id: String,
    pub content: String,
    pub timestamp: i64,
    pub metadata: serde_json::Value,
}

impl ChannelMessage {
    pub fn new(user_id: String, channel_id: String, content: String) -> Self {
        Self {
            user_id,
            channel_id,
            content,
            timestamp: chrono::Utc::now().timestamp(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Generate a unique session key for routing
    pub fn session_key(&self) -> String {
        format!("{}:user:{}", self.channel_id, self.user_id)
    }
}

pub fn build_session_id(
    channel: &str,
    channel_id: &str,
    user_id: &str,
    dm_scope: DmScope,
    is_dm: bool,
) -> String {
    if !is_dm {
        return format!("{}:{}", channel, channel_id);
    }

    match dm_scope {
        DmScope::Main => format!("{}:main", channel),
        DmScope::PerPeer => format!("{}:dm:{}", channel, user_id),
        DmScope::PerChannelPeer => format!("{}:{}:dm:{}", channel, channel_id, user_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAdapter;

    #[async_trait]
    impl ChannelAdapter for MockAdapter {
        async fn send_message(&self, _user_id: &str, _content: &str) -> Result<()> {
            Ok(())
        }

        async fn send_stream_chunk(&self, _user_id: &str, _chunk: &str) -> Result<()> {
            Ok(())
        }

        fn channel_name(&self) -> &str {
            "mock"
        }
    }

    #[test]
    fn test_format_user_id() {
        let adapter = MockAdapter;
        assert_eq!(adapter.format_user_id("user123"), "mock:user123");
    }

    #[test]
    fn test_session_key() {
        let msg = ChannelMessage::new(
            "user123".to_string(),
            "slack".to_string(),
            "hello".to_string(),
        );
        assert_eq!(msg.session_key(), "slack:user:user123");
    }
}
