use anyhow::{Result, Context};
use console::style;
use dialoguer::{Input, theme::ColorfulTheme};

use super::channel_instructions::print_instruction_box;

/// Run Discord setup wizard with detailed instructions
pub async fn run_discord_setup_wizard() -> Result<()> {
    print_instruction_box(
        "Discord Bot Setup",
        &[
            "1) Go to https://discord.com/developers/applications",
            "2) Click 'New Application', name your bot",
            "3) Go to 'Bot' section → Reset Token → Copy token",
            "4) Copy 'Application ID' from General Information",
            "5) Enable 'MESSAGE CONTENT INTENT' under Bot → Privileged Gateway Intents",
            "6) Invite bot: OAuth2 → URL Generator → Select 'bot' → Permissions → Copy URL",
            "",
            "Tip: You can also set DISCORD_TOKEN and DISCORD_APP_ID in .env",
            "Docs: https://discord.com/developers/docs/intro",
        ],
    );

    // Get bot token
    let bot_token: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter Discord bot token")
        .interact_text()?;

    // Get application ID
    let app_id: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter Discord application ID")
        .interact_text()?;

    // Basic validation
    if bot_token.is_empty() || !bot_token.contains('.') {
        anyhow::bail!("Invalid Discord token format");
    }

    if app_id.is_empty() || !app_id.chars().all(|c| c.is_numeric()) {
        anyhow::bail!("Invalid Discord application ID (should be numeric)");
    }

    // Store in environment
    std::env::set_var("DISCORD_TOKEN", &bot_token);
    std::env::set_var("DISCORD_APP_ID", &app_id);

    println!();
    println!("{}", style("✅ Discord configured successfully!").green().bold());
    println!("{}", style(format!("   Application ID: {}", app_id)).cyan());
    
    Ok(())
}
