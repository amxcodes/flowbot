use anyhow::Result;
use dialoguer::{theme::ColorfulTheme, Input, Select, MultiSelect, Confirm};
use console::style;
use std::path::PathBuf;

use super::{SetupOptions, templates::Personality, workspace_mgmt};

pub struct WizardData {
    pub agent_name: String,
    pub personality: Personality,
    pub user_name: String,
    pub timezone: String,
    pub channels: Vec<String>,
    pub agent_emoji: String,
}

pub async fn interactive_setup(opts: SetupOptions) -> Result<()> {
    print_welcome_banner();
    
    // 1. Agent identity
    let agent_name = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("What should I call myself?")
        .default("Flowbot".into())
        .interact()?;
    
    let vibe_options = vec![
        "Professional (formal, precise)",
        "Casual (friendly, relaxed)", 
        "Chaotic Good (helpful but quirky)",
        "Custom (I'll create my own SOUL.md)",
    ];
    
    let vibe_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What's my vibe?")
        .items(&vibe_options)
        .default(1)
        .interact()?;
    
    let personality = match vibe_idx {
        0 => Personality::Professional,
        1 => Personality::Casual,
        2 => Personality::ChaoticGood,
        3 => Personality::Custom,
        _ => Personality::Casual,
    };
    
    // 2. Agent emoji
    let emoji_options = vec!["🤖", "🦊", "🐙", "⚡", "🌟", "🔮", "🎯", "Custom"];
    let emoji_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Pick my emoji signature")
        .items(&emoji_options)
        .default(0)
        .interact()?;
    
    let agent_emoji = if emoji_idx == emoji_options.len() - 1 {
        Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter custom emoji")
            .default("🤖".into())
            .interact()?
    } else {
        emoji_options[emoji_idx].to_string()
    };
    
    // 3. User profile
    println!();
    println!("{}", style("Now let me learn about you!").bold());
    println!();
    
    let user_name = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("What should I call you?")
        .interact()?;
    
    let timezone = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("Your timezone (e.g., America/New_York)")
        .default("UTC".into())
        .interact()?;
    
    // 4. Channel selection
    println!();
    let channel_options = vec!["Telegram", "WebSocket", "Discord (coming soon)"];
    let channel_indices = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Enable messaging channels? (Space to select, Enter to confirm)")
        .items(&channel_options)
        .interact()?;
    
    let channels: Vec<String> = channel_indices
        .into_iter()
        .map(|i| channel_options[i].to_string())
        .collect();
    
    // 5. Confirm setup
    println!();
    println!("{}", style("Setup Summary:").bold().cyan());
    println!("  Agent: {} {}", agent_emoji, agent_name);
    println!("  Vibe: {}", vibe_options[vibe_idx]);
    println!("  User: {}", user_name);
    println!("  Timezone: {}", timezone);
    println!("  Channels: {}", if channels.is_empty() { "None".to_string() } else { channels.join(", ") });
    println!();
    
    let confirm = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Create workspace with these settings?")
        .default(true)
        .interact()?;
    
    if !confirm {
        println!("Setup cancelled.");
        return Ok(());
    }
    
    // 6. Create workspace
    let workspace_dir = opts.workspace_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".flowbot")
            .join("workspace")
    });
    
    println!();
    println!("{}", style("Creating workspace...").bold());
    
    let wizard_data = WizardData {
        agent_name,
        personality,
        user_name,
        timezone,
        channels: channels.clone(),
        agent_emoji,
    };
    
    workspace_mgmt::create_workspace(&workspace_dir, wizard_data).await?;
    
    if channels.contains(&"Telegram".to_string()) {
        println!();
        println!("{}", style("Running Telegram Setup...").bold().cyan());
        super::telegram::run_telegram_setup_wizard().await?;
    }
    
    print_completion_message(&workspace_dir);
    
    Ok(())
}

fn print_welcome_banner() {
    println!();
    println!("{}", style("╔═══════════════════════════════════════════╗").cyan());
    println!("{}", style("║   🤖 Welcome to Flowbot Setup!           ║").cyan());
    println!("{}", style("╚═══════════════════════════════════════════╝").cyan());
    println!();
    println!("Let's personalize your AI assistant!");
    println!();
}

fn print_completion_message(workspace_dir: &std::path::Path) {
    println!();
    println!("{}", style("✨ ═══════════════════════════════════════════ ✨").green());
    println!("{}", style("   Setup Complete!").bold().green());
    println!("{}", style("✨ ═══════════════════════════════════════════ ✨").green());
    println!();
    println!("Workspace: {}", style(workspace_dir.display()).cyan());
    println!();
    println!("{}", style("Try these commands:").bold());
    println!("  {} - Start chatting in terminal", style("flowbot chat").cyan());
    println!("  {} - Start API server", style("flowbot server").cyan());
    println!("  {} - Launch Telegram bot", style("flowbot gateway telegram").cyan());
    println!();
    println!("{}", style("Edit your personality:").bold());
    println!("  {} - Change your voice/tone", style("flowbot workspace:edit soul").cyan());
    println!("  {} - Update your identity", style("flowbot workspace:edit identity").cyan());
    println!();
    println!("📚 Your personality files:");
    println!("   • SOUL.md - Your voice and personality");
    println!("   • IDENTITY.md - Your name, emoji, and vibe");
    println!("   • USER.md - Info about your human");
    println!();
}
