use nanobot_core::config::BrowserConfig;
use nanobot_core::browser::BrowserClient;
use rusqlite::{Connection, Result};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Starting Validation...\n");

    // 1. Validate FTS5
    println!("🧪 Testing SQLite FTS5 support...");
    match test_fts5() {
        Ok(_) => println!("✅ FTS5 is ENABLED and working."),
        Err(e) => println!("❌ FTS5 failed: {}", e),
    }

    println!("\n--------------------------------\n");

    // 2. Validate Docker Browser (Dry Run)
    println!("🧪 Testing Docker Browser Configuration...");
    let config = BrowserConfig {
        headless: true,
        user_data_dir: None,
        proxy: None,
        use_docker: true,
        docker_image: "zenika/alpine-chrome:with-puppeteer".to_string(),
        docker_port: 9222,
    };

    println!("checking Docker availability...");
    let status = std::process::Command::new("docker").arg("--version").output();
    if let Ok(output) = status {
        if output.status.success() {
             println!("✅ Docker is available: {}", String::from_utf8_lossy(&output.stdout).trim());
             
             // We won't actually launch it here to avoid hanging / heavy resource usage in this simple check,
             // but we can verify the client struct creation.
             let _client = BrowserClient::new(config);
             println!("✅ BrowserClient instantiated with Docker config.");
        } else {
             println!("⚠️ Docker command failed. Is Docker Desktop running?");
        }
    } else {
        println!("⚠️ Docker not found in PATH.");
    }

    Ok(())
}

fn test_fts5() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute(
        "CREATE VIRTUAL TABLE test_fts USING fts5(content);",
        (),
    )?;
    conn.execute(
        "INSERT INTO test_fts (content) VALUES ('nano bot is awesome');",
        (),
    )?;
    
    let mut stmt = conn.prepare("SELECT * FROM test_fts WHERE test_fts MATCH 'nano';")?;
    let mut rows = stmt.query([])?;
    
    if let Some(_row) = rows.next()? {
        Ok(())
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}
