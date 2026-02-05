// Gateway module - Telegram bot and multi-agent management

pub mod telegram;
pub mod streaming;
pub mod agent_manager;

// Re-export the Gateway struct for backward compatibility
pub use streaming::*;
