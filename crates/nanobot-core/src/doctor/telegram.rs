use crate::config::Config;
use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

pub async fn check_telegram(config: &Result<Config, anyhow::Error>) -> bool {
    print!("Checking Telegram...      ");

    // 1. Resolve Token
    let token = if let Ok(cfg) = config {
        if let Some(ref tg) = cfg.providers.telegram {
            Some(tg.bot_token.clone())
        } else {
            None
        }
    } else {
        None
    };

    // Fallback to Env
    let final_token = if let Some(t) = token {
        t
    } else {
        std::env::var("TELEGRAM_BOT_TOKEN")
            .or_else(|_| std::env::var("NANOBOT_TELEGRAM_TOKEN"))
            .unwrap_or_default()
    };

    if final_token.is_empty() {
        println!("{}", "⚪ Skipped (Not Configured)".dim());
        return true; // Not an error if just not using it
    }

    // 2. Verify Token with API
    let client = Client::new();
    match client
        .get(format!("https://api.telegram.org/bot{}/getMe", final_token))
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<Value>().await {
                    Ok(json) => {
                        if let Some(result) = json.get("result") {
                            let username = result
                                .get("username")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown");
                            println!("{} (@{})", "✅ OK".green(), username);

                            // Check allowed users warning
                            check_allowed_users_config();
                            return true;
                        }
                    }
                    Err(_) => {
                        println!("{}", "⚠️  API Error (Invalid JSON)".yellow());
                    }
                }
            } else {
                println!("{}", "❌ Invalid Token".red());
                println!("  ℹ️  The configured token was rejected by Telegram API.");
                if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
                    println!("  ℹ️  Using token from TELEGRAM_BOT_TOKEN environment variable.");
                } else {
                    println!("  ℹ️  Using token from config.toml.");
                }
                return false;
            }
        }
        Err(e) => {
            println!("{}", "❌ Network Error".red());
            println!("  ⚠️  Could not reach Telegram API: {}", e);
            return false;
        }
    }

    false
}

fn check_allowed_users_config() {
    // Audit for legacy allowed users vs pairing system
    if std::env::var("TELEGRAM_ALLOWED_USERS").is_ok() {
        println!("      ⚠️  Legacy TELEGRAM_ALLOWED_USERS detected.");
        println!("         Flowbot now uses a database-backed pairing system.");
        println!("         This env var is only used as a fallback/initial seed.");
    }
}

// Helper trait to match doctor's coloring (if not sharing common one yet)
trait Colorize {
    fn green(self) -> String;
    fn red(self) -> String;
    fn yellow(self) -> String;
    fn dim(self) -> String;
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
    fn dim(self) -> String {
        format!("\x1b[2m{}\x1b[0m", self)
    }
}
