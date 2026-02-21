use anyhow::Result;
use console::style;
use dialoguer::{Input, theme::ColorfulTheme};

use super::channel_instructions::print_instruction_box;

/// Run Slack setup wizard with Socket Mode instructions
pub async fn run_slack_setup_wizard() -> Result<()> {
    print_instruction_box(
        "Slack Bot Setup (Socket Mode)",
        &[
            "1) Go to https://api.slack.com/apps → Create New App → From scratch",
            "2) Name your app and select workspace",
            "3) Go to 'Socket Mode' → Enable Socket Mode",
            "4) Generate App-Level Token:",
            "   - Basic Information → App-Level Tokens → Generate Token",
            "   - Add scope: connections:write",
            "   - Copy token (starts with xapp-)",
            "5) Get Bot Token:",
            "   - OAuth & Permissions → Install to Workspace",
            "   - Copy 'Bot User OAuth Token' (starts with xoxb-)",
            "6) Enable Event Subscriptions → Subscribe to bot events:",
            "   - message.im, message.channels, message.groups",
            "",
            "Tip: You can also set SLACK_BOT_TOKEN and SLACK_APP_TOKEN in .env",
            "Docs: https://api.slack.com/apis/connections/socket",
        ],
    );

    // Get bot token
    let bot_token: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter Slack Bot Token (xoxb-...)")
        .interact_text()?;

    // Get app token
    let app_token: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Enter Slack App Token (xapp-...)")
        .interact_text()?;

    // Validate token formats
    if !bot_token.starts_with("xoxb-") {
        anyhow::bail!("Invalid Slack bot token (should start with xoxb-)");
    }

    if !app_token.starts_with("xapp-") {
        anyhow::bail!("Invalid Slack app token (should start with xapp-)");
    }

    // Store in environment
    unsafe {
        std::env::set_var("SLACK_BOT_TOKEN", &bot_token);
        std::env::set_var("SLACK_APP_TOKEN", &app_token);
    }

    println!();
    println!(
        "{}",
        style("✅ Slack configured successfully!").green().bold()
    );
    println!(
        "{}",
        style("   Socket Mode enabled, ready to connect").cyan()
    );

    Ok(())
}
