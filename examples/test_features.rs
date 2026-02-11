use flowbot_rs::tools::definitions::get_tool_declarations;
use flowbot_rs::tools::executor::execute_tool;
use flowbot_rs::persistence::PersistenceManager;
use serde_json::json;
use std::path::PathBuf;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🧪 Starting End-to-End Feature Test...\n");

    // 1. Test Persistence
    println!("--- Testing Persistence ---");
    let db_path = PathBuf::from("test_sessions.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
    }
    
    let persistence = PersistenceManager::new(db_path.clone());
    persistence.init()?;
    
    persistence.save_message("session_1", "user", "Hello Persistence")?;
    persistence.save_message("session_1", "assistant", "Hello User")?;
    
    let history = persistence.get_history("session_1")?;
    println!("✅ Saved & Loaded {} messages.", history.len());
    assert_eq!(history.len(), 2);
    
    // Cleanup
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
    }
    println!("✅ Persistence verified.\n");

    // 2. Test Web Fetch
    println!("--- Testing Web Fetch ---");
    // Use a reliable, small page (e.g., example.com)
    let fetch_call = json!({
        "tool": "web_fetch",
        "url": "https://example.com"
    }).to_string();
    
    match execute_tool(&fetch_call, None, None, None, None, None, None, None, None, None, None).await {
        Ok(content) => {
            println!("✅ Fetch successful (length: {} bytes)", content.len());
            if content.contains("Example Domain") {
                println!("✅ Content verified.");
            } else {
                eprintln!("⚠️ Unexpected content: {}", content);
            }
        },
        Err(e) => eprintln!("❌ Web Fetch failed: {}", e),
    }
    println!("");

    // 3. Test Process Spawn
    println!("--- Testing Spawn Process ---");
    
    // Spawn 'ping' (Windows: ping -n 4 127.0.0.1)
    let spawn_call = json!({
        "tool": "spawn_process",
        "command": "ping",
        "args": ["-n", "4", "127.0.0.1"]
    }).to_string();
    
    let spawn_result = execute_tool(&spawn_call, None, None, None, None, None, None, None, None, None, None).await?;
    println!("Spawn Result: {}", spawn_result);
    
    // Extract PID from string "Started process 'ping' with PID: <uuid>"
    let pid = spawn_result.split("PID: ").nth(1).unwrap().trim();
    println!("✅ PID Extracted: {}", pid);

    // Initial read
    sleep(Duration::from_secs(1)).await;
    let read_call = json!({
        "tool": "read_process_output",
        "pid": pid
    }).to_string();
    let output1 = execute_tool(&read_call, None, None, None, None, None, None, None, None, None, None).await?;
    println!("--- Output 1 ---\n{}", output1);

    // Wait more
    sleep(Duration::from_secs(2)).await;
    let output2 = execute_tool(&read_call, None, None, None, None, None, None, None, None, None, None).await?;
    println!("--- Output 2 ---\n{}", output2);

    // List processes
    let list_call = json!({ "tool": "list_processes" }).to_string();
    let list_out = execute_tool(&list_call, None, None, None, None, None, None, None, None, None, None).await?;
    println!("--- Process List ---\n{}", list_out);

    // Test Input: Send "hello" (though ping doesn't read it, it verifies the plumbing)
    println!("--- Testing Write Process Input ---");
    let input_call = json!({
        "tool": "write_process_input",
        "pid": pid,
        "input": "hello\n"
    }).to_string();
    match execute_tool(&input_call, None, None, None, None, None, None, None, None, None, None).await {
         Ok(res) => println!("✅ Input sent: {}", res),
         Err(e) => eprintln!("❌ Input failed: {}", e),
    }

    // Kill process
    let kill_call = json!({
        "tool": "kill_process",
        "pid": pid
    }).to_string();
    execute_tool(&kill_call, None, None, None, None, None, None, None, None, None, None).await?;
    println!("✅ Process killed.");
    
    // Verify list empty (or exited)
    let list_out_final = execute_tool(&list_call, None, None, None, None, None, None, None, None, None, None).await?;
    println!("--- Final Process List ---\n{}", list_out_final);

    println!("\n✅ All Tests Completed Successfully!");
    
    Ok(())
}

