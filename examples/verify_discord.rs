use anyhow::Result;
use dotenv::dotenv;
use nanobot_core::gateway::registry::ChannelRegistry;
use nanobot_core::gateway::discord_adapter::{DiscordBot, DiscordConfig};
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    // Load config from environment
    let token = env::var("DISCORD_BOT_TOKEN").expect("DISCORD_BOT_TOKEN must be set");
    let channel_id = env::var("DISCORD_CHANNEL_ID").expect("DISCORD_CHANNEL_ID must be set");
    let application_id = env::var("DISCORD_APP_ID")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);

    println!("🤖 Initializing Discord Bot Verification...");

    // 1. Setup channels
    let (agent_tx, _agent_rx) = mpsc::channel(100);
    let registry = Arc::new(ChannelRegistry::new());

    // 2. Create Adapter
    let config = DiscordConfig {
        token,
        application_id,
    };

    let bot = DiscordBot::new(config, agent_tx, registry.clone());
    
    // 3. Register and Run (in background)
    let bot_handle = tokio::spawn(async move {
        if let Err(e) = bot.run().await {
            eprintln!("Discord Bot Error: {:?}", e);
        }
    });

    // Give it a moment to register
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 4. Send Verification Message via Registry
    println!("📨 Sending test message to channel: {}", channel_id);
    match registry.send("discord", &channel_id, "🔔 Nanobot Integration Verification: **Discord Adapter Online** [HTTP Mode]").await {
        Ok(_) => println!("✅ Message sent successfully!"),
        Err(e) => eprintln!("❌ Failed to send message: {:?}", e),
    }

    // Test message splitting (sending a long message)
    println!("📨 Testing message splitting (long message)...");
    let long_msg = "test ".repeat(500); // Should be > 2000 chars
    match registry.send("discord", &channel_id, &format!("📏 Long Message Test:\n{}", long_msg)).await {
        Ok(_) => println!("✅ Long message sent (split) successfully!"),
        Err(e) => eprintln!("❌ Failed to send long message: {:?}", e),
    }

    // 5. Cleanup
    bot_handle.abort();
    Ok(())
}
