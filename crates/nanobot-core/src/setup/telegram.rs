use crate::config::{Config, TelegramConfig};
use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, Input, theme::ColorfulTheme};
use reqwest::Client;
use serde_json::Value;

use super::channel_instructions::print_instruction_box;

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
                "Config file not found. Run 'flowbot setup' first to create the workspace and config."
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
        println!("Flowbot uses a secure Pairing System by default.");
        println!("Unauthorized users will be blocked and given a pairing code.");
        println!("You can approve them with `flowbot pairing approve telegram <CODE>`.");
        println!();

        // Ask for optional personal User ID for auto-approval?
        // Actually, let's keep it simple: just setup the token.
        // The pairing system handles the rest.

        if Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Save this configuration?")
            .default(true)
            .interact()?
        {
            // Update config
            if config.providers.telegram.is_none() {
                config.providers.telegram = Some(TelegramConfig {
                    bot_token: token.clone(),
                });
            } else {
                if let Some(ref mut tg) = config.providers.telegram {
                    tg.bot_token = token.clone();
                }
            }

            config.save()?;
            println!("{}", style("✓ Configuration saved to config.toml").green());

            println!();
            println!("🚀 To start the bot:");
            println!("   {}", style("flowbot gateway").cyan());
        }
    } else {
        println!("{}", style("❌ Failed to parse Telegram response").red());
    }

    Ok(())
}
