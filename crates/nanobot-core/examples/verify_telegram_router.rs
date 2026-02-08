
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use nanobot_core::gateway::router::{MessageRouter, RoutingStrategy};
    
    println!("🧪 Testing Telegram Router Integration...");
    
    // 1. Create Router
    let mut router = MessageRouter::new(RoutingStrategy::RoundRobin { 
        agents: vec!["agent-1".to_string()], 
        next_idx: 0 
    });
    
    // 2. Simulate a Telegram session key
    let telegram_session = "telegram:123456789:user:987654321"; // channel:id:user:id
    
    // 3. Ask router where to send it
    let agent_id = router.route_session(telegram_session).await;
    println!("Router sent session '{}' to agent '{}'", telegram_session, agent_id);
    
    // 4. Verify
    assert_eq!(agent_id, "agent-1", "Router should assign an agent to the Telegram session");
    
    println!("✅ Router correctly handles Telegram session keys (Generic Logic)");
    println!("⚠️ NOTE: This test confirms the Router *logic* works, but the 'TelegramBot' struct itself isn't stored in the Router for OUTBOUND dispatch.");
    
    Ok(())
}
