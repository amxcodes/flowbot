use anyhow::Result;
use console::style;
use dialoguer::{Confirm, Input, MultiSelect, Select, Password, theme::ColorfulTheme};
use std::path::PathBuf;

use super::{SetupOptions, templates::Personality, workspace_mgmt};

pub struct WizardData {
    pub agent_name: String,
    pub agent_name_pending: bool,
    pub personality: Personality,
    pub personality_pending: bool,
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
    pub enable_browser: bool,
    pub browser_use_docker: bool,
    pub browser_docker_image: String,
    pub browser_docker_port: u16,
    pub dm_scope: crate::config::DmScope,
    pub should_start_webchat: bool,
    pub should_start_server: bool,
    pub web_port: u16,
    pub server_port: u16,
    pub teams_webhook: Option<String>,
    pub google_chat_webhook: Option<String>,
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
            enable_browser: false,
            browser_use_docker: false,
            browser_docker_image: "zenika/alpine-chrome:with-puppeteer".to_string(),
            browser_docker_port: 9222,
            dm_scope: crate::config::DmScope::Main,
            should_start_webchat: false,
            should_start_server: false,
            web_port: 3000,
            server_port: 3000,
            teams_webhook: None,
            google_chat_webhook: None,
            workspace_dir: PathBuf::from("."),
        });
    }

    // 0. Wizard mode
    let mode_options = vec!["Quick (Recommended)", "Advanced"];
    let mode_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose setup mode")
        .items(&mode_options)
        .default(0)
        .interact()?;
    let quick_mode = mode_idx == 0;

    // 1. Agent identity
    let raw_agent_name = Input::<String>::with_theme(&ColorfulTheme::default())
        .with_prompt("What should I call myself? (type 'skip' to set later in channel)")
        .default("Flowbot".into())
        .interact()?;
    let agent_name_pending = raw_agent_name.trim().eq_ignore_ascii_case("skip");
    let agent_name = if agent_name_pending {
        "Assistant".to_string()
    } else {
        raw_agent_name
    };

    let vibe_options = vec![
        "Professional (formal, precise)",
        "Casual (friendly, relaxed)",
        "Chaotic Good (helpful but quirky)",
        "Custom (I'll create my own SOUL.md)",
        "Skip for now (use Casual)",
    ];

    let vibe_idx = if quick_mode {
        1
    } else {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What's my vibe?")
            .items(&vibe_options)
            .default(1)
            .interact()?
    };

    let personality = match vibe_idx {
        0 => Personality::Professional,
        1 => Personality::Casual,
        2 => Personality::ChaoticGood,
        3 => Personality::Custom,
        4 => Personality::Casual,
        _ => Personality::Casual,
    };
    let personality_pending = vibe_idx == 4;

    // 2. Agent emoji
    let emoji_options = vec!["🤖", "🦊", "🐙", "⚡", "🌟", "🔮", "🎯", "Custom"];
    let emoji_idx = if quick_mode {
        0
    } else {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Pick my emoji signature")
            .items(&emoji_options)
            .default(0)
            .interact()?
    };

    let agent_emoji = if emoji_idx == emoji_options.len() - 1 && !quick_mode {
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

    let user_name = if quick_mode {
        "User".to_string()
    } else {
        Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("What should I call you?")
            .interact()?
    };

    let timezone = if quick_mode {
        "UTC".to_string()
    } else {
        Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Your timezone (e.g., America/New_York)")
            .default("UTC".into())
            .interact()?
    };

    // 4. Channel selection
    println!();
    let channel_options = vec![
        "Telegram",
        "Discord",
        "Slack",
        "Teams (Webhook)",
        "Google Chat (Webhook)",
        "Web Chat (Browser)",
        "WebSocket API",
    ];
    let channel_indices = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Enable messaging channels? (Space to select, Enter to confirm)")
        .items(&channel_options)
        .interact()?;

    let mut channels: Vec<String> = channel_indices
        .into_iter()
        .map(|i| channel_options[i].to_string())
        .collect();

    if quick_mode && channels.is_empty() {
        channels.push("Web Chat (Browser)".to_string());
    }

    // 4a. DM isolation
    println!();
    let dm_options = vec![
        "Main (one shared DM session)",
        "Per user (stronger privacy)",
        "Per channel + user (recommended for multi-user inboxes)",
    ];
    let dm_idx = if quick_mode {
        2
    } else {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("DM isolation mode")
            .items(&dm_options)
            .default(2)
            .interact()?
    };

    let dm_scope = match dm_idx {
        0 => crate::config::DmScope::Main,
        1 => crate::config::DmScope::PerPeer,
        _ => crate::config::DmScope::PerChannelPeer,
    };

    // 4b. Admin token setup
    println!();
    if quick_mode {
        let token = uuid::Uuid::new_v4().to_string();
        crate::security::write_admin_token(token.trim())?;
        let masked = if token.len() > 8 {
            format!("{}...{}", &token[..4], &token[token.len() - 4..])
        } else {
            "****".to_string()
        };
        println!("{} {}", style("✅ Admin token saved:").green().bold(), masked);
        println!("{}", style("You can change it later with: nanobot admin-token set").dim());
    } else {
        let set_admin_token = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Set admin token for /eval and admin actions?")
            .default(true)
            .interact()?;

        if set_admin_token {
            let token = Password::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter admin token")
                .with_confirmation("Confirm admin token", "Tokens do not match")
                .interact()?;

            if !token.trim().is_empty() {
                crate::security::write_admin_token(token.trim())?;
                println!("{}", style("✅ Admin token saved").green().bold());
            } else {
                println!("{}", style("⚠️  Admin token not set").yellow());
            }
        }
    }

    // 4c. Session secrets setup
    println!();
    if quick_mode {
        let _ = crate::security::get_or_create_session_secrets()?;
        println!("{}", style("✅ Session tokens configured").green().bold());
    } else {
        let set_session_secrets = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Generate secure session tokens for Web/Gateway?")
            .default(true)
            .interact()?;

        if set_session_secrets {
            let secrets = crate::security::get_or_create_session_secrets()?;
            println!("{}", style("✅ Session secrets saved").green().bold());
            println!(
                "{} {}",
                style("Gateway token secret:").dim(),
                &secrets.gateway_session_secret[..8.min(secrets.gateway_session_secret.len())]
            );
            println!(
                "{} {}",
                style("Web token secret:").dim(),
                &secrets.web_token_secret[..8.min(secrets.web_token_secret.len())]
            );
        }
    }

    // 4d. Web chat password setup
    if channels.contains(&"Web Chat (Browser)".to_string()) {
        println!();
        if quick_mode {
            let password = uuid::Uuid::new_v4().to_string();
            crate::security::write_web_password(&password)?;
            println!("{} {}", style("✅ Web chat password:").green().bold(), password);
            println!("{}", style("Save this password; you will need it to log in.").dim());
        } else {
            let set_web_password = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Set Web Chat login password?")
                .default(true)
                .interact()?;

            if set_web_password {
                let password = Password::with_theme(&ColorfulTheme::default())
                    .with_prompt("Enter Web Chat password")
                    .with_confirmation("Confirm password", "Passwords do not match")
                    .interact()?;

                if !password.trim().is_empty() {
                    crate::security::write_web_password(password.trim())?;
                    println!("{}", style("✅ Web chat password saved").green().bold());
                } else {
                    println!("{}", style("⚠️  Web chat password not set").yellow());
                }
            }
        }
    }

    // 4e. Webhook URLs for Teams / Google Chat
    let mut teams_webhook: Option<String> = None;
    let mut google_chat_webhook: Option<String> = None;

    if channels.contains(&"Teams (Webhook)".to_string()) {
        println!();
        let webhook = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Teams webhook URL")
            .interact()?;
        if !webhook.trim().is_empty() {
            teams_webhook = Some(webhook.trim().to_string());
        }
    }

    if channels.contains(&"Google Chat (Webhook)".to_string()) {
        println!();
        let webhook = Input::<String>::with_theme(&ColorfulTheme::default())
            .with_prompt("Google Chat webhook URL")
            .interact()?;
        if !webhook.trim().is_empty() {
            google_chat_webhook = Some(webhook.trim().to_string());
        }
    }

    // 4e. Ports
    let web_port = if channels.contains(&"Web Chat (Browser)".to_string()) {
        if quick_mode {
            3000
        } else {
            Input::<u16>::with_theme(&ColorfulTheme::default())
                .with_prompt("Web Chat port")
                .default(3000)
                .interact()?
        }
    } else {
        3000
    };

    let server_port = if channels.contains(&"WebSocket API".to_string()) {
        if quick_mode {
            3000
        } else {
            Input::<u16>::with_theme(&ColorfulTheme::default())
                .with_prompt("WebSocket API port")
                .default(3000)
                .interact()?
        }
    } else {
        3000
    };

    // 5. Provider Selection (new section like OpenClaw)
    println!();
    println!("{}", style("Model/Auth Provider Configuration").bold().cyan());
    let provider_options = vec!["Google Antigravity (OAuth)", "OpenAI", "OpenRouter", "Skip for now"];
    let provider_idx = if quick_mode {
        let use_oauth = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Connect Google Antigravity (recommended)?")
            .default(true)
            .interact()?;
        if use_oauth { 0 } else { 3 }
    } else {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Select your LLM provider")
            .items(&provider_options)
            .default(0)
            .interact()?
    };
    
    let (should_run_oauth, oauth_provider) = match provider_idx {
        0 => (true, Some("antigravity".to_string())),
        1 | 2 | 3 => (false, None),
        _ => (false, None),
    };

    // 6. Service Installation Prompt
    println!();
    let should_install_service = if quick_mode {
        true
    } else {
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Install as system service (24/7 operation)?")
            .default(true)
            .interact()?
    };

    // 7. Offline speech stack
    println!();
    let offline_options = vec![
        "Install now (choose Whisper/Sherpa models)",
        "Install later",
        "Skip",
    ];
    let offline_choice = if quick_mode {
        0
    } else {
        Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Offline speech setup")
            .items(&offline_options)
            .default(1)
            .interact()?
    };

    // 8. Confirm setup
    println!();
    println!("{}", style("Setup Summary:").bold().cyan());
    println!("  Agent: {} {}", agent_emoji, agent_name);
    if agent_name_pending {
        println!("  Agent name status: Pending (will ask in channel)");
    }
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
    println!("  DM isolation: {}", dm_options[dm_idx]);
    println!("  Offline speech: {}", offline_options[offline_choice]);
    println!("  Web Chat: {}", if channels.contains(&"Web Chat (Browser)".to_string()) { "Yes" } else { "No" });
    println!("  WebSocket API: {}", if channels.contains(&"WebSocket API".to_string()) { "Yes" } else { "No" });
    println!("  Teams webhook: {}", if channels.contains(&"Teams (Webhook)".to_string()) { "Yes" } else { "No" });
    println!("  Google Chat webhook: {}", if channels.contains(&"Google Chat (Webhook)".to_string()) { "Yes" } else { "No" });
    if channels.contains(&"Web Chat (Browser)".to_string()) {
        println!("  Web Chat port: {}", web_port);
    }
    if channels.contains(&"WebSocket API".to_string()) {
        println!("  WebSocket API port: {}", server_port);
    }
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
            enable_browser: false,
            browser_use_docker: false,
            browser_docker_image: "zenika/alpine-chrome:with-puppeteer".to_string(),
            browser_docker_port: 9222,
            dm_scope,
            should_start_webchat: false,
            should_start_server: false,
            web_port,
            server_port,
            teams_webhook,
            google_chat_webhook,
            workspace_dir: PathBuf::from("."),
        });
    }

    // 9. Create workspace
    let workspace_dir = opts.workspace_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
    });

    println!();
    println!("{}", style("Creating workspace...").bold());

    let wizard_data = WizardData {
        agent_name,
        agent_name_pending,
        personality,
        personality_pending,
        user_name,
        timezone,
        channels: channels.clone(),
        agent_emoji,
    };

    workspace_mgmt::create_workspace(&workspace_dir, wizard_data).await?;

    // 10. Channel-specific setup
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

    if offline_choice == 0 {
        println!();
        println!("{}", style("Installing offline speech components...").bold().cyan());
        if let Err(err) = super::offline_models::run_offline_models_installer().await {
            println!("{} {}", style("⚠️  Offline installer failed:").yellow(), err);
            println!("Run later with: {}", style("nanobot setup --offline-models").green());
        }
    } else if offline_choice == 1 {
        println!();
        println!(
            "{}",
            style("You can install offline speech models later with: nanobot setup --offline-models")
                .dim()
        );
    }

    // Quick start
    println!();
    println!("{}", style("Quick Start").bold().cyan());
    println!("  1) Start the bot gateway (keeps channels connected):");
    println!("     {}", style("nanobot gateway --channel all").green());
    if channels.contains(&"Web Chat (Browser)".to_string()) {
        println!("  2) Start Web Chat (browser UI):");
        println!("     {}", style(format!("nanobot webchat --port {}", web_port)).green());
        println!("     {}", style(format!("Open: http://localhost:{}", web_port)).dim());
        if let Some(ip) = get_local_ip() {
            println!("     {}", style(format!("Open on LAN: http://{}:{}", ip, web_port)).dim());
        }
    }
    println!("  3) Start WebSocket API:");
    println!("     {}", style(format!("nanobot server --port {}", server_port)).green());
    println!();
    println!("{}", style("Tip:").dim());
    println!("  If you installed the service, it already runs 24x7.");
    
    // 11. Show pairing system explanation
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
    
    // 12. Auto-gateway option
    let has_messaging_channels = channels.iter().any(|c| {
        c == "Telegram" || c == "Discord" || c == "Slack"
    });
    let should_start_gateway = if has_messaging_channels {
        if quick_mode {
            true
        } else {
            println!();
            println!("{}", style("Gateway Launch").bold().cyan());
            println!("The gateway connects all your messaging channels to the bot.");
            Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Start gateway now? (keeps channels active)")
                .default(true)
                .interact()?
        }
    } else {
        false
    };
    
    // 13. Prompt for TUI hatch
    let should_hatch_tui = if !should_start_gateway {
        println!();
        println!("{}", style("Start TUI (best option!)").bold());
        println!("This is the defining action that makes your agent YOU.");
        if quick_mode {
            true
        } else {
            Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Hatch in TUI now? (recommended)")
                .default(true)
                .interact()?
        }
    } else {
        false
    };

    // 14. Browser capability setup (non-technical defaults)
    println!();
    println!("{}", style("Browser Automation Setup").bold().cyan());
    let docker_available = command_exists("docker");
    let local_browser_available = has_local_browser();

    let docker_option = if docker_available {
        "Docker managed browser (Recommended for easiest setup)".to_string()
    } else {
        "Docker managed browser (not detected on this machine)".to_string()
    };

    let local_option = if local_browser_available {
        "Use locally installed Chrome/Chromium".to_string()
    } else {
        "Use locally installed Chrome/Chromium (not detected)".to_string()
    };

    let browser_options = vec![
        "Disable browser tools".to_string(),
        docker_option,
        local_option,
    ];

    let default_browser_idx = if quick_mode {
        if docker_available {
            1
        } else if local_browser_available {
            2
        } else {
            0
        }
    } else {
        if docker_available {
            1
        } else {
            0
        }
    };

    let browser_idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("How should browser automation run?")
        .items(&browser_options)
        .default(default_browser_idx)
        .interact()?;

    let (enable_browser, browser_use_docker) = match browser_idx {
        1 => (true, true),
        2 => (true, false),
        _ => (false, false),
    };

    let browser_docker_image = "zenika/alpine-chrome:with-puppeteer".to_string();
    let browser_docker_port = 9222;

    if enable_browser && browser_use_docker {
        println!(
            "{}",
            style("Browser tools enabled with Docker (managed headless Chrome).").green()
        );
        if !docker_available {
            println!(
                "{}",
                style("Docker was not detected. Install Docker or switch to local browser mode later.")
                    .yellow()
            );
        }
    } else if enable_browser {
        println!(
            "{}",
            style("Browser tools enabled with local Chrome/Chromium.").green()
        );
        if !local_browser_available {
            println!(
                "{}",
                style("No local Chrome/Chromium detected. Install one browser executable first.")
                    .yellow()
            );
        }
    } else {
        println!("{}", style("Browser tools disabled.").dim());
    }

    Ok(SetupResult {
        should_run_oauth,
        oauth_provider,
        should_install_service,
        should_hatch_tui,
        should_start_gateway,
        enable_browser,
        browser_use_docker,
        browser_docker_image,
        browser_docker_port,
        dm_scope,
        should_start_webchat: channels.contains(&"Web Chat (Browser)".to_string()),
        should_start_server: channels.contains(&"WebSocket API".to_string()),
        web_port,
        server_port,
        teams_webhook,
        google_chat_webhook,
        workspace_dir,
    })
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn has_local_browser() -> bool {
    let candidates = [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "chrome",
        "msedge",
    ];

    candidates.iter().any(|cmd| command_exists(cmd))
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

fn get_local_ip() -> Option<String> {
    use std::net::UdpSocket;

    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    if socket.connect("8.8.8.8:80").is_err() {
        return None;
    }
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
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

// Completion message helper removed (unused).
