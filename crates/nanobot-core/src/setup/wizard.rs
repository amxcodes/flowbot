use anyhow::Result;
use console::style;
use dialoguer::{Confirm, Input, MultiSelect, Select, theme::ColorfulTheme};
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

/// Result of the wizard, indicating what actions to take next
pub struct SetupResult {
    pub should_run_oauth: bool,
    pub oauth_provider: Option<String>,
    pub should_install_service: bool,
    pub should_hatch_tui: bool,
    pub should_start_gateway: bool,
    pub workspace_dir: PathBuf,
}

pub async fn interactive_setup(opts: SetupOptions) -> Result<SetupResult> {
    print_welcome_banner();
    
    // Security warning (like OpenClaw)
    print_security_warning();
    
    let understood = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("I understand this is powerful and inherently risky. Continue?")
        .default(true)
        .interact()?;
    
    if !understood {
        println!("{}", style("Setup cancelled.").yellow());
        return Ok(SetupResult {
            should_run_oauth: false,
            oauth_provider: None,
            should_install_service: false,
            should_hatch_tui: false,
            should_start_gateway: false,
            workspace_dir: PathBuf::from("."),
        });
    }

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
    let channel_options = vec!["Telegram", "Discord", "Slack"];
    let channel_indices = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Enable messaging channels? (Space to select, Enter to confirm)")
        .items(&channel_options)
        .interact()?;

    let channels: Vec<String> = channel_indices
        .into_iter()
        .map(|i| channel_options[i].to_string())
        .collect();

    // 5. Provider Selection (new section like OpenClaw)
    println!();
    println!("{}", style("Model/Auth Provider Configuration").bold().cyan());
    let provider_options = vec!["Google Antigravity (OAuth)", "OpenAI", "OpenRouter", "Skip for now"];
    let provider_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select your LLM provider")
        .items(&provider_options)
        .default(0)
        .interact()?;
    
    let (should_run_oauth, oauth_provider) = match provider_idx {
        0 => (true, Some("antigravity".to_string())),
        1 | 2 | 3 => (false, None),
        _ => (false, None),
    };

    // 6. Service Installation Prompt
    println!();
    let should_install_service = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Install as system service (24/7 operation)?")
        .default(true)
        .interact()?;

    // 7. Confirm setup
    println!();
    println!("{}", style("Setup Summary:").bold().cyan());
    println!("  Agent: {} {}", agent_emoji, agent_name);
    println!("  Vibe: {}", vibe_options[vibe_idx]);
    println!("  User: {}", user_name);
    println!("  Timezone: {}", timezone);
    println!(
        "  Channels: {}",
        if channels.is_empty() {
            "None".to_string()
        } else {
            channels.join(", ")
        }
    );
    println!("  Provider: {}", provider_options[provider_idx]);
    println!("  Service: {}", if should_install_service { "Yes" } else { "No" });
    println!();

    let confirm = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Create workspace with these settings?")
        .default(true)
        .interact()?;

    if !confirm {
        println!("Setup cancelled.");
        return Ok(SetupResult {
            should_run_oauth: false,
            oauth_provider: None,
            should_install_service: false,
            should_hatch_tui: false,
            should_start_gateway: false,
            workspace_dir: PathBuf::from("."),
        });
    }

    // 8. Create workspace
    let workspace_dir = opts.workspace_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
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

    // 9. Channel-specific setup
    if channels.contains(&"Telegram".to_string()) {
        println!();
        super::telegram::run_telegram_setup_wizard().await?;
    }
    
    if channels.contains(&"Discord".to_string()) {
        println!();
        super::discord::run_discord_setup_wizard().await?;
    }
    
    if channels.contains(&"Slack".to_string()) {
        println!();
        super::slack::run_slack_setup_wizard().await?;
    }

    println!();
    println!("{}", style("✅ Workspace created successfully!").green().bold());
    
    // 10. Show pairing system explanation
    if !channels.is_empty() {
        use super::channel_instructions::{print_pairing_explanation, print_channel_status};
        
        println!();
        print_pairing_explanation();
        
        // Show channels status
        let mut channel_statuses = vec![];
        for channel in &channels {
            channel_statuses.push((channel.clone(), "✓ configured".to_string()));
        }
        print_channel_status(&channel_statuses);
    }
    
    // 11. Auto-gateway option
    let should_start_gateway = if !channels.is_empty() {
        println!();
        println!("{}", style("Gateway Launch").bold().cyan());
        println!("The gateway connects all your messaging channels to the bot.");
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Start gateway now? (keeps channels active)")
            .default(true)
            .interact()?
    } else {
        false
    };
    
    // 12. Prompt for TUI hatch
    let should_hatch_tui = if !should_start_gateway {
        println!();
        println!("{}", style("Start TUI (best option!)").bold());
        println!("This is the defining action that makes your agent YOU.");
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Hatch in TUI now? (recommended)")
            .default(true)
            .interact()?
    } else {
        false
    };

    Ok(SetupResult {
        should_run_oauth,
        oauth_provider,
        should_install_service,
        should_hatch_tui,
        should_start_gateway,
        workspace_dir,
    })
}

fn print_welcome_banner() {
    println!();
    println!(
        "{}",
        style("╔═══════════════════════════════════════════╗").cyan()
    );
    println!(
        "{}",
        style("║   🤖 Welcome to Nanobot Setup!           ║").cyan()
    );
    println!(
        "{}",
        style("╚═══════════════════════════════════════════╝").cyan()
    );
    println!();
    println!("Let's personalize your AI assistant!");
    println!();
}

fn print_security_warning() {
    println!();
    println!("{}", style("⚠️  Security Warning").bold().yellow());
    println!();
    println!("Nanobot is a powerful AI agent that can:");
    println!("  • Read and write files");
    println!("  • Execute system commands (if tools are enabled)");
    println!("  • Access APIs and networks");
    println!();
    println!("A bad prompt can trick it into doing unsafe things.");
    println!();
    println!("{}", style("Recommended baseline:").bold());
    println!("  - Use pairing/allowlists for channel security");
    println!("  - Keep secrets out of the workspace");
    println!("  - Use the strongest available model");
    println!();
}

fn print_completion_message(workspace_dir: &std::path::Path) {
    println!();
    println!(
        "{}",
        style("✨ ═══════════════════════════════════════════ ✨").green()
    );
    println!("{}", style("   Setup Complete!").bold().green());
    println!(
        "{}",
        style("✨ ═══════════════════════════════════════════ ✨").green()
    );
    println!();
    println!("Workspace: {}", style(workspace_dir.display()).cyan());
    println!();
    println!("{}", style("Try these commands:").bold());
    println!(
        "  {} - Start chatting in terminal",
        style("flowbot chat").cyan()
    );
    println!("  {} - Start API server", style("flowbot server").cyan());
    println!(
        "  {} - Launch Telegram bot",
        style("flowbot gateway telegram").cyan()
    );
    println!();
    println!("{}", style("Edit your personality:").bold());
    println!(
        "  {} - Change your voice/tone",
        style("flowbot workspace:edit soul").cyan()
    );
    println!(
        "  {} - Update your identity",
        style("flowbot workspace:edit identity").cyan()
    );
    println!();
    println!("📚 Your personality files:");
    println!("   • SOUL.md - Your voice and personality");
    println!("   • IDENTITY.md - Your name, emoji, and vibe");
    println!("   • USER.md - Info about your human");
    println!();
}
