use anyhow::Result;
use dotenv::dotenv;
use nanobot_core::gateway::registry::ChannelRegistry;
use nanobot_core::gateway::slack_adapter::{SlackBot, SlackConfig};
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    // Load config from environment
    let bot_token = env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKEN must be set");
    let channel_id = env::var("SLACK_CHANNEL_ID").expect("SLACK_CHANNEL_ID must be set");
    let app_token = env::var("SLACK_APP_TOKEN").ok(); // Optional

    println!("🤖 Initializing Slack Bot Verification...");

    // 1. Setup channels
    let (agent_tx, _agent_rx) = mpsc::channel(100);
    let registry = Arc::new(ChannelRegistry::new());

    // 2. Create Adapter
    let config = SlackConfig {
        bot_token,
        app_token,
    };

    let bot = SlackBot::new(config, agent_tx, registry.clone());
    
    // 3. Register and Run (in background)
    let bot_handle = tokio::spawn(async move {
        if let Err(e) = bot.run().await {
            eprintln!("Slack Bot Error: {:?}", e);
        }
    });

    // Give it a moment to register
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // 4. Send Verification Message via Registry
    println!("📨 Sending test message to channel: {}", channel_id);
    match registry.send("slack", &channel_id, "🔔 Nanobot Integration Verification: **Slack Adapter Online** [HTTP Mode]").await {
        Ok(_) => println!("✅ Message sent successfully!"),
        Err(e) => eprintln!("❌ Failed to send message: {:?}", e),
    }

    // 5. Cleanup
    // In a real app, we'd wait for signal. Here we just exit.
    bot_handle.abort();
    Ok(())
}
