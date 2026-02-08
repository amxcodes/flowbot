
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Note: nanobot_core is the library name in code
    use std::sync::Arc;
    use tokio::sync::mpsc;
    
    println!("🧪 Testing Gateway Registry Integration...");
    
    // 1. Create Registry
    let registry = Arc::new(nanobot_core::gateway::registry::ChannelRegistry::new());
    
    // 2. Create Dummy Inbox Channel
    let (inbox_tx, mut inbox_rx) = mpsc::channel::<nanobot_core::gateway::adapter::ChannelMessage>(100);
    
    // 3. Register it manually
    registry.register("telegram", inbox_tx).await;
    
    // 4. Verify Registration
    let lookup = registry.get("telegram").await;
    assert!(lookup.is_some(), "Registry should contain 'telegram'");
    println!("✅ Registry lookup successful");
    
    // 5. Simulate Outbound Message
    let msg = nanobot_core::gateway::adapter::ChannelMessage::new(
        "user123".to_string(),
        "telegram".to_string(),
        "Hello from Registry!".to_string()
    );
    
    println!("📤 Sending message via Registry...");
    registry.send("telegram", msg.clone()).await?;
    
    // 6. Verify Receipt
    let received = inbox_rx.try_recv();
    assert!(received.is_ok(), "Inbox should receive message");
    let received_msg = received.unwrap();
    assert_eq!(received_msg.content, "Hello from Registry!");
    println!("✅ Message received in Inbox channel");
    
    println!("🎉 Architecture Verification Passed: Registry <-> Adapter wiring is solid.");
    Ok(())
}
