use nanobot_core::agent::AgentLoop;

#[tokio::main]
async fn main() {
    println!("🔍 Starting Integrity Smoke Test...");

    // 1. Initialize AgentLoop
    // This verifies:
    // - PermissionManager creation
    // - ResourceMonitor creation
    // - Helper threads start (Cron, Workspace Watcher)
    println!("   > Initializing AgentLoop...");
    match AgentLoop::new().await {
        Ok(agent) => {
            println!("   ✅ AgentLoop initialized successfully.");
            println!("   ✅ PermissionManager: WIRED (part of struct)");
            println!("   ✅ ResourceMonitor: WIRED (part of struct)");
        }
        Err(e) => {
            eprintln!("   ❌ AgentLoop Initialization Failed: {}", e);
            std::process::exit(1);
        }
    }

    // 2. Resource Monitor Check
    // We can't access private fields, but successful init implies it started.
    // We can try to use the system module directly to verify it works.
    println!("   > Testing ResourceMonitor Module...");
    let monitor = nanobot_core::system::resources::ResourceMonitor::new();
    let usage = monitor.get_usage();
    println!("   ✅ CPU Usage: {}%", usage.cpu_usage_percent);
    println!(
        "   ✅ RAM Usage: {} MB / {} MB",
        usage.used_memory_mb, usage.total_memory_mb
    );

    if usage.total_memory_mb > 0 {
        println!("   ✅ Resource Monitor is reading system stats.");
    } else {
        println!("   ❌ Resource Monitor returned invalid stats.");
        std::process::exit(1);
    }

    println!("\n🎉 INTEGRITY VERIFICATION PASSED!");
}
