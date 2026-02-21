use crate::config::{Config, TelegramConfig};
use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, Input, theme::ColorfulTheme};
use reqwest::Client;
use serde_json::Value;

use super::channel_instructions::print_instruction_box;

fn parse_allowed_user_ids(raw: &str) -> Result<Option<Vec<i64>>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let mut ids = Vec::new();
    for piece in trimmed.split(',') {
        let token = piece.trim();
        if token.is_empty() {
            continue;
        }
        let parsed = token.parse::<i64>().map_err(|_| {
            anyhow::anyhow!(
                "Invalid user ID '{}' (use numeric Telegram IDs, comma-separated)",
                token
            )
        })?;
        ids.push(parsed);
    }

    if ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ids))
    }
}

pub async fn run_telegram_setup_wizard() -> Result<()> {
    println!();
    println!("{}", style("Telegram Bot Setup").bold().cyan());

    print_instruction_box(
        "Telegram Bot Token",
        &[
            "1) Open Telegram and chat with @BotFather",
            "2) Run /newbot (or /mybots to manage existing bots)",
            "3) Follow prompts to name your bot",
            "4) Copy the token (looks like 123456:ABC-DEF...)",
            "",
            "Tip: You can also set TELEGRAM_BOT_TOKEN in your .env file",
            "Docs: https://core.telegram.org/bots#how-do-i-create-a-bot",
        ],
    );

    let mut config = match Config::load() {
        Ok(c) => c,
        Err(_) => {
            println!(
                "{}",
                style("⚠️  Config file not found. Creating new config.").yellow()
            );
            // Create default config structure
            // In a real scenario we might want a default constructor
            return Err(anyhow::anyhow!(
                "Config file not found. Run 'nanobot setup' first to create the workspace and config."
            ));
        }
    };

    let token: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter your Telegram Bot Token")
        .validate_with(|input: &String| -> Result<(), &str> {
            if input.trim().is_empty() {
                Err("Token cannot be empty")
            } else if !input.contains(':') {
                Err("Invalid token format (should contain ':')")
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    println!("{}", style("Verifying token...").dim());

    let client = Client::new();
    let resp = client
        .get(format!("https://api.telegram.org/bot{}/getMe", token))
        .send()
        .await
        .context("Failed to connect to Telegram API")?;

    if !resp.status().is_success() {
        println!("{}", style("❌ Invalid token (API returned error)").red());
        return Ok(());
    }

    let json: Value = resp.json().await?;

    if let Some(result) = json.get("result") {
        let bot_user = result
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        let bot_name = result
            .get("first_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        println!("✅ {}", style("Token verified!").green());
        println!("   Bot Name: {}", style(bot_name).bold());
        println!("   Username: @{}", style(bot_user).cyan());
        println!();

        // Setup Pairing
        println!("{}", style("🔐 Security Configuration").bold());
        println!("Nanobot uses a secure Pairing System by default.");
        println!("Unauthorized users will be blocked and given a pairing code.");
        println!("You can approve them with `nanobot pairing approve telegram <CODE>`.");
        println!();

        let allowed_users = if Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Add trusted Telegram user IDs now? (optional)")
            .default(true)
            .interact()?
        {
            println!("Tip: get your numeric ID from @userinfobot");
            let raw_ids: String = Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter allowed user ID(s), comma-separated")
                .allow_empty(true)
                .interact_text()?;

            match parse_allowed_user_ids(&raw_ids) {
                Ok(v) => v,
                Err(e) => {
                    println!("{} {}", style("⚠️  Skipping allowlist:").yellow(), e);
                    println!(
                        "You can set it later in config.toml under providers.telegram.allowed_users"
                    );
                    None
                }
            }
        } else {
            None
        };

        if Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Save this configuration?")
            .default(true)
            .interact()?
        {
            // Update config
            if config.providers.telegram.is_none() {
                config.providers.telegram = Some(TelegramConfig {
                    bot_token: token.clone(),
                    allowed_users,
                });
            } else if let Some(ref mut tg) = config.providers.telegram {
                tg.bot_token = token.clone();
                tg.allowed_users = allowed_users;
            }

            config.save()?;
            println!("{}", style("✓ Configuration saved to config.toml").green());

            println!();
            println!("🚀 To start the bot:");
            println!("   {}", style("nanobot gateway").cyan());
        }
    } else {
        println!("{}", style("❌ Failed to parse Telegram response").red());
    }

    Ok(())
}
