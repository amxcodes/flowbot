use crate::config;
use crate::persistence::PersistenceManager;
use anyhow::Result;
// We'll add this for nice output, or use direct ANSI codes if keeping deps minimal
use std::path::PathBuf;

pub mod telegram; // Add this line

pub async fn run_doctor() -> Result<()> {
    println!("\n🩺 Running Flowbot Doctor...\n");

    let mut all_ok = true;

    // 1. Check Configuration
    print!("Checking Configuration... ");
    let config_result = config::Config::load();

    match &config_result {
        Ok(cfg) => {
            println!("{}", "✅ OK".green());

            // Check Provider Keys
            if let Some(ref gl) = cfg.providers.antigravity {
                 let no_key = gl.api_key.as_ref().map_or(true, |k| k.is_empty()) 
                     && gl.api_keys.as_ref().map_or(true, |ks| ks.is_empty());
                
                 if no_key {
                    println!("  ⚠️  Antigravity API key is empty.");
                    all_ok = false;
                 }
            }
            
            if let Some(ref oa) = cfg.providers.openai {
                let no_key = oa.api_key.as_ref().map_or(true, |k| k.is_empty())
                     && oa.api_keys.as_ref().map_or(true, |ks| ks.is_empty());

                if no_key {
                    // Check for tokens
                    if let Ok(tokens) = crate::config::OAuthTokens::load() {
                        if tokens.get("openai").is_some() {
                            println!("  ✅ OpenAI (OAuth token found)");
                        } else {
                            println!("  ⚠️  OpenAI key empty and no OAuth token.");
                        }
                    } else {
                        println!("  ⚠️  OpenAI API key is empty.");
                    }
                }
            }
        }
        Err(_) => {
            println!("{}", "❌ Missing or Invalid".red());
            println!("  ℹ️  Run 'flowbot setup' to create a configuration file.");
            all_ok = false;
        }
    }

    // 2. Check Database / Persistence
    print!("Checking Storage...       ");
    let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
    if let Some(parent) = db_path.parent()
        && !parent.exists()
    {
        // Try creating it to see if we have permissions
        match std::fs::create_dir_all(parent) {
            Ok(_) => {}
            Err(_) => {
                println!("{}", "❌ Permission Denied".red());
                println!("  ⚠️  Cannot create .nanobot directory.");
                all_ok = false;
            }
        }
    }

    match PersistenceManager::new(db_path.clone()).init() {
        Ok(_) => println!("{}", "✅ OK".green()),
        Err(e) => {
            println!("{}", "❌ Error".red());
            println!("  ⚠️  Database error: {}", e);
            all_ok = false;
        }
    }

    // 3. Check Internet Connectivity (Ping Google/Cloudflare)
    print!("Checking Connectivity...  ");
    match reqwest::get("https://www.google.com").await {
        Ok(_) => println!("{}", "✅ OK".green()),
        Err(_) => {
            println!("{}", "❌ Offline".red());
            println!("  ⚠️  Cannot reach the internet.");
            all_ok = false;
        }
    }

    // 4. Check Telegram
    if !telegram::check_telegram(&config_result).await {
        all_ok = false;
    }

    println!("\nSummary:");
    if all_ok {
        println!(
            "{}",
            "✨ All systems operational! You are ready to run Flowbot."
                .green()
                .bold()
        );
    } else {
        println!(
            "{}",
            "⚠️  Some issues detected. See above hints.".yellow().bold()
        );
    }

    Ok(())
}

// Simple color helper trait (if we don't want to add 'colored' dependency yet)
trait Colorize {
    fn green(self) -> String;
    fn red(self) -> String;
    fn yellow(self) -> String;
    fn bold(self) -> String;
}

impl Colorize for &str {
    fn green(self) -> String {
        format!("\x1b[32m{}\x1b[0m", self)
    }
    fn red(self) -> String {
        format!("\x1b[31m{}\x1b[0m", self)
    }
    fn yellow(self) -> String {
        format!("\x1b[33m{}\x1b[0m", self)
    }
    fn bold(self) -> String {
        format!("\x1b[1m{}\x1b[0m", self)
    }
}
