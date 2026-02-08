/// E2E Integration Test: Multi-Agent Supervision
/// 
/// This test verifies that the Supervisor correctly isolates child agent failures
/// and applies supervision policies appropriately.

use nanobot_core::agent::supervisor::{Supervisor, SupervisionPolicy, ChildConfig};
use nanobot_core::agent::AgentMessage;
use tokio::time::{sleep, Duration};
use tokio::sync::mpsc;

#[tokio::test]
async fn test_multi_agent_isolation_one_for_one() {
    let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);
    
    // Spawn 3 child agents
    let child1 = supervisor.spawn_child(
        ChildConfig::default(),
        |mut rx| {
            tokio::spawn(async move {
                while let Some(_msg) = rx.recv().await {
                    // Child 1 stays alive
                }
            })
        }
    ).await.unwrap();
    
    let child2 = supervisor.spawn_child(
        ChildConfig::default(),
        |_rx| {
            tokio::spawn(async move {
                // Child 2 exits immediately (simulates crash)
            })
        }
    ).await.unwrap();
    
    let child3 = supervisor.spawn_child(
        ChildConfig::default(),
        |mut rx| {
            tokio::spawn(async move {
                while let Some(_msg) = rx.recv().await {
                    // Child 3 stays alive
                }
            })
        }
    ).await.unwrap();
    
    assert_eq!(supervisor.child_count(), 3, "Should have 3 children initially");
    println!("✅ Spawned 3 child agents");
    
    // Wait for child2 to finish
    sleep(Duration::from_millis(100)).await;
    
    // Apply supervision
    supervisor.supervise().await;
    
    // OneForOne policy: only child2 should be removed
    assert_eq!(supervisor.child_count(), 2, "Should have 2 children after supervision");
    
    let remaining = supervisor.list_children();
    assert!(remaining.contains(&child1), "Child 1 should still exist");
    assert!(!remaining.contains(&child2), "Child 2 should be removed");
    assert!(remaining.contains(&child3), "Child 3 should still exist");
    
    println!("✅ OneForOne policy: Middle child removed, others unaffected");
    
    // Verify routing to remaining children still works
    let msg1 = AgentMessage {
        session_id: "test-session-1".to_string(),
        content: "Test message".to_string(),
        response_tx: mpsc::channel(10).0,
    };
    
    assert!(supervisor.send_to_child(&child1, msg1).await.is_ok(), 
           "Should route to child1");
    
    println!("✅ Routing still works after child removal");
}

#[tokio::test]
async fn test_agent_auto_restart_with_factory() {
    let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);
    
    // Spawn child with factory (enables restart)
    let child_id = supervisor.spawn_child_with_factory(
        ChildConfig {
            max_restarts: 3,
            ..Default::default()
        },
        |_rx| {
            tokio::spawn(async move {
                // Exit immediately to trigger restart
            })
        }
    ).await.unwrap();
    
    assert_eq!(supervisor.child_count(), 1);
    println!("✅ Spawned child with factory");
    
    // Wait for child to finish
    sleep(Duration::from_millis(100)).await;
    
    // Apply supervision (should restart)
    supervisor.supervise().await;
    
    // Child should have been restarted (count stays at 1)
    assert_eq!(supervisor.child_count(), 1, "Child should be restarted");
    println!("✅ Child auto-restarted successfully");
}

#[tokio::test]
async fn test_max_restart_limit() {
    let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);
    
    // Spawn child that always crashes, with max_restarts = 2
    let child_id = supervisor.spawn_child_with_factory(
        ChildConfig {
            max_restarts: 2,
            ..Default::default()
        },
        |_rx| {
            tokio::spawn(async move {
                // Always exit immediately
            })
        }
    ).await.unwrap();
    
    println!("✅ Spawned crashy child with max_restarts=2");
    
    // Cycle 1: Initial crash, restart 1
    sleep(Duration::from_millis(50)).await;
    supervisor.supervise().await;
    assert_eq!(supervisor.child_count(), 1, "Should restart (attempt 1)");
    
    // Cycle 2: Crash again, restart 2
    sleep(Duration::from_millis(50)).await;
    supervisor.supervise().await;
    assert_eq!(supervisor.child_count(), 1, "Should restart (attempt 2)");
    
    // Cycle 3: Crash again, exceed limit, no restart
    sleep(Duration::from_millis(50)).await;
    supervisor.supervise().await;
    assert_eq!(supervisor.child_count(), 0, "Should NOT restart (exceeded limit)");
    
    println!("✅ Max restart limit enforced correctly");
}

#[tokio::test]
async fn test_all_for_one_policy() {
    let mut supervisor = Supervisor::new(SupervisionPolicy::AllForOne);
    
    // Spawn 3 children
    for _ in 0..2 {
        supervisor.spawn_child(
            ChildConfig::default(),
            |mut rx| {
                tokio::spawn(async move {
                    while let Some(_msg) = rx.recv().await {}
                })
            }
        ).await.unwrap();
    }
    
    // One child that crashes
    supervisor.spawn_child(
        ChildConfig::default(),
        |_rx| {
            tokio::spawn(async move {
                // Crashes immediately
            })
        }
    ).await.unwrap();
    
    assert_eq!(supervisor.child_count(), 3);
    println!("✅ Spawned 3 children (AllForOne policy)");
    
    sleep(Duration::from_millis(100)).await;
    supervisor.supervise().await;
    
    // AllForOne: all children should be killed
    assert_eq!(supervisor.child_count(), 0, "All children should be killed");
    println!("✅ AllForOne policy: All children killed on single failure");
}
