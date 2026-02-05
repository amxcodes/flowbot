use anyhow::Result;
use clap::{Parser, Subcommand};
use rig::client::{ProviderClient, CompletionClient}; 
use rig::completion::Prompt;
use rig::providers::openai;
use tokio::sync::mpsc;
use flowbot_rs::agent::AgentLoop; // Changed from crate::agent
use flowbot_rs::persistence::PersistenceManager; // Changed from crate::persistence
use std::path::PathBuf;
use uuid::Uuid;

// Import modules from the library
use flowbot_rs::{config, oauth, tui, doctor, tools, antigravity, telegram, gateway};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Interactive setup wizard
    Setup {
        /// Run interactive wizard
        #[arg(long)]
        wizard: bool,
        /// Custom workspace directory
        #[arg(long)]
        workspace: Option<String>,
        /// Setup Telegram bot
        #[arg(long)]
        telegram: bool,
    },
    /// Manage workspace files
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommands,
    },
    /// Check system health and configuration
    Doctor,
    /// Start interactive TUI chat
    Chat {
        /// Provider to use (antigravity, openai, openrouter)
        #[arg(short, long)]
        provider: Option<String>,
    },
    /// Send a single message (CLI mode)
    Agent {
        /// The message to send
        #[arg(short, long)]
        message: String,
        /// Provider to use
        #[arg(short, long)]
        provider: Option<String>,
        /// Model to use (optional, overrides default)
        #[arg(long)]
        model: Option<String>,
    },
    /// Login to a provider (OAuth)
    Login {
        /// Provider to login to (antigravity, openai)
        provider: String,
    },
    /// Debug Sandbox Connectivity
    DebugSandbox,
    /// Run Telegram bot gateway
    Gateway,
    /// Manage pairing requests for secure channel access
    Pairing {
        #[command(subcommand)]
        action: PairingAction,
    },
    /// Manage scheduled tasks (Cron)
    Cron {
        #[command(subcommand)]
        action: CronCommands,
    },
    /// Start API Server (HTTP/WebSocket)
    Server {
        /// Port to listen on (default: 3000)
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum CronCommands {
    /// List all cron jobs
    List {
        /// Include disabled jobs
        #[arg(long)]
        all: bool,
    },
    /// Add a new cron job
    Add {
        /// Schedule expression (cron format)
        #[arg(long, short)]
        schedule: String,
        /// Message/Text to inject
        #[arg(long, short)]
        text: String,
        /// Job name (optional)
        #[arg(long, short)]
        name: Option<String>,
        /// Target isolated session (default: main system event)
        #[arg(long)]
        isolated: bool,
    },
    /// Remove a job
    Remove {
        /// Job ID
        id: String,
    },
    /// Get scheduler status
    Status,
    /// View job execution history
    Runs {
        /// Job ID
        job_id: String,
        /// Maximum number of runs to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Send wake event
    Wake {
        /// Message to inject
        #[arg(long, short)]
        text: String,
        /// Wake mode: now | next-heartbeat
        #[arg(long, default_value = "next-heartbeat")]
        mode: String,
    },
}

#[derive(Subcommand, Debug)]
enum PairingAction {
    /// List pending pairing requests
    List {
        /// Channel filter (telegram, discord, etc.)
        channel: Option<String>,
    },
    /// Approve a pairing request
    Approve {
        /// Channel (e.g., 'telegram')
        channel: String,
        /// 6-digit code
        code: String,
    },
    /// Reject a pairing request
    Reject {
        /// Channel (e.g., 'telegram')
        channel: String,
        /// 6-digit code
        code: String,
    },
}

#[derive(Subcommand, Debug)]
enum WorkspaceCommands {
    /// Edit workspace file (soul, identity, user, agents, tools)
    Edit {
        file: String,
    },
    /// Show workspace information
    Show,
    /// Reset workspace to defaults
    Reset,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Setup { wizard, workspace, telegram } => {
            if telegram {
                flowbot_rs::setup::telegram::run_telegram_setup_wizard().await?;
                return Ok(());
            }

            if wizard {
                let opts = flowbot_rs::setup::SetupOptions {
                    workspace_dir: workspace.map(PathBuf::from),
                    skip_wizard: false,
                };
                flowbot_rs::setup::run_setup_wizard(opts).await?;
            } else {
                // ... basic setup
                let opts = flowbot_rs::setup::SetupOptions {
                    workspace_dir: workspace.map(PathBuf::from),
                    skip_wizard: true,
                };
                flowbot_rs::setup::basic_setup(opts).await?;
            }
        }
        Commands::Workspace { command } => {
            use flowbot_rs::setup::workspace_mgmt;
            match command {
                WorkspaceCommands::Edit { file } => {
                    workspace_mgmt::edit_file(&file).await?;
                }
                WorkspaceCommands::Show => {
                    workspace_mgmt::show().await?;
                }
                WorkspaceCommands::Reset => {
                    workspace_mgmt::reset().await?;
                }
            }
        }
        Commands::Doctor => {
            doctor::run_doctor().await?;
        }
        Commands::Chat { provider } => {
            run_tui_chat(provider).await?;
        }
        Commands::Agent { message, provider, model } => {
            run_cli_agent(&message, provider, model).await?;
        }
        Commands::Login { provider } => {
            run_oauth_login(&provider).await?;
        }
        Commands::DebugSandbox => {
            run_debug_sandbox().await?;
        }
        Commands::Gateway => {
            run_telegram_gateway().await?;
        }
        Commands::Pairing { action } => {
            // Initialize pairing database
            flowbot_rs::pairing::init_database().await?;
            
            match action {
                PairingAction::List { channel } => {
                    let chan = channel.as_deref().unwrap_or("all");
                    let requests = flowbot_rs::pairing::get_pending_requests(chan).await?;
                    
                    if requests.is_empty() {
                        println!("\n📋 No pending pairing requests.\n");
                    } else {
                        println!("\n📋 Pending Pairing Requests:\n");
                        println!("{:<10} {:<15} {:<15} {:<8}", "Channel", "Username", "User ID", "Code");
                        println!("{}", "-".repeat(60));
                        for req in requests {
                            println!(
                                "{:<10} {:<15} {:<15} {:<8}",
                                req.channel,
                                req.username.as_deref().unwrap_or("(none)"),
                                req.user_id,
                                req.code
                            );
                        }
                        println!();
                    }
                }
                PairingAction::Approve { channel, code } => {
                    match flowbot_rs::pairing::approve(&channel, &code).await {
                        Ok(user_id) => {
                            println!("✅ Approved user {} on {}", user_id, channel);
                        }
                        Err(e) => {
                            eprintln!("❌ Failed to approve: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                PairingAction::Reject { channel, code } => {
                    match flowbot_rs::pairing::reject(&channel, &code).await {
                        Ok(()) => {
                            println!("❌ Rejected pairing code {}", code);
                        }
                        Err(e) => {
                            eprintln!("❌ Failed to reject: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
            }
        Commands::Cron { action } => {
            // Need to initialize persistence/DB path to pass to scheduler
            // Ideally we should use a shared init function, but duplicate for now
             let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
            if let Some(parent) = db_path.parent() {
                 tokio::fs::create_dir_all(parent).await?;
            }
            // Init minimal persistence for table creation if needed
             let persistence = PersistenceManager::new(db_path.clone());
             persistence.init()?;

            // Create a dummy event channel (CLI doesn't listen to events)
            let (cron_event_tx, _cron_event_rx) = tokio::sync::mpsc::channel(100);
            
            let scheduler = flowbot_rs::cron::CronScheduler::new(db_path.clone(), cron_event_tx).await?;
            // Note: We don't need to start() the scheduler just to list/add/remove jobs from DB, 
            // unless we want to run them *now*. But for CLI management, DB access is enough.

            match action {
                CronCommands::List { all } => {
                    let jobs = scheduler.list_jobs(all)?;
                    if jobs.is_empty() {
                         println!("No cron jobs found.");
                    } else {
                         println!("{:<36} {:<20} {:<20} {:<10}", "ID", "Name", "Schedule", "Enabled");
                         println!("{}", "-".repeat(90));
                         for job in jobs {
                             let schedule_desc = match job.schedule {
                                 flowbot_rs::cron::Schedule::Cron{expr, ..} => expr,
                                 flowbot_rs::cron::Schedule::Every{every_ms, ..} => format!("Every {}ms", every_ms),
                                 flowbot_rs::cron::Schedule::At{at_ms} => format!("At {}", at_ms),
                             };
                             println!("{:<36} {:<20} {:<20} {:<10}", 
                                job.id, 
                                job.name.unwrap_or_default(), 
                                schedule_desc, 
                                job.enabled
                             );
                         }
                    }
                }
                CronCommands::Add { schedule, text, name, isolated } => {
                    use flowbot_rs::cron::{Schedule, Payload, SessionTarget, CronJob};
                    
                    let schedule_obj = Schedule::Cron { expr: schedule, tz: None };
                    
                    let (payload, target) = if isolated {
                        (Payload::AgentTurn { 
                            message: text, 
                            model: None, 
                            thinking: None, 
                            timeout_seconds: None 
                        }, SessionTarget::Isolated)
                    } else {
                        (Payload::SystemEvent { text }, SessionTarget::Main)
                    };
                    
                    let job = CronJob::new(name, schedule_obj, payload, target);
                    let id = scheduler.add_job(job).await?;
                    println!("✅ Added cron job: {}", id);
                }
                CronCommands::Remove { id } => {
                    scheduler.remove_job(&id)?;
                    println!("🗑️ Removed cron job: {}", id);
                }
                CronCommands::Status => {
                    let status = scheduler.status().await?;
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                CronCommands::Runs { job_id, limit } => {
                    use flowbot_rs::cron::run_log;
                    
                    let log_path = run_log::resolve_run_log_path(&db_path, &job_id);
                    let entries = run_log::read_run_log(&log_path, limit)?;
                    
                    if entries.is_empty() {
                        println!("No execution history for job: {}", job_id);
                    } else {
                        println!("\n📊 Execution History for Job: {}\n", job_id);
                        for (i, entry) in entries.iter().enumerate() {
                            let emoji = match entry.status.as_deref() {
                                Some("ok") => "✅",
                                Some("error") => "❌",
                                Some("skipped") => "⏭️",
                                _ => "❓",
                            };
                            
                            let time = chrono::DateTime::from_timestamp_millis(entry.ts as i64)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| entry.ts.to_string());
                            
                            let duration = entry.duration_ms
                                .map(|ms| format!(" ({}ms)", ms))
                                .unwrap_or_default();
                            
                            println!("{}. {} {}{}", i + 1, emoji, time, duration);
                            
                            if let Some(err) = &entry.error {
                                println!("   ❌ {}", err);
                            }
                            if let Some(summary) = &entry.summary {
                                println!("   📝 {}", summary);
                            }
                        }
                        println!();
                    }
                }
                CronCommands::Wake { text, mode } => {
                    println!("⚠️ Wake command not yet implemented");
                    println!("   Mode: {}", mode);
                    println!("   Text: {}", text);
                    println!("   (This requires integration with the running agent loop)");
                }
            }
        }
        Commands::Server { port } => {
            // Channel for Gateway -> Agent communication
            let (agent_tx, agent_rx) = mpsc::channel(100);

            // Spawn Agent Loop
            tokio::spawn(async move {
                println!("🤖 Starting Agent Loop...");
                match AgentLoop::new().await {
                   Ok(agent) => {
                       agent.run(agent_rx).await;
                   }
                   Err(e) => {
                       eprintln!("🔥 Failed to start Agent Loop: {}", e);
                   }
                }
            });

            let config = gateway::GatewayConfig { port: port };
            let gateway = gateway::Gateway::new(config, agent_tx);
            gateway.start().await?;
        }
    }

    Ok(())
}

async fn run_tui_chat(provider: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    let provider_name = provider.unwrap_or(config.default_provider.clone());

    println!("Starting TUI chat with provider: {}", provider_name);
    println!("Loading...");

    let preamble = format!(
        "You are FlowBot, a helpful AI assistant with tool access.\n{}",
        tools::executor::get_tool_descriptions()
    );

    let mut ui = tui::ChatUI::new(provider_name.clone());

    match provider_name.as_str() {
        "antigravity" => {
            let client = antigravity::AntigravityClient::from_env().await?;
            let model_name = "gemini-2.5-flash"; 
            let agent = client
                .agent(model_name)
                .preamble(&preamble)
                .build();
            
             // Add welcome message
            ui.add_message(
                "system".to_string(),
                format!(
                    "Welcome to Nanobot! Using provider: {} ({})\n\nI have access to:\n- File operations (read, write, edit, list)\n- Web search\n- Command execution",
                    provider_name, model_name
                ),
            );

            // Init Persistence
            let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
            if let Some(parent) = db_path.parent() {
                 tokio::fs::create_dir_all(parent).await?;
            }
            let persistence = PersistenceManager::new(db_path);
            persistence.init()?;
            let session_id = Uuid::new_v4().to_string();

            run_chat_loop(agent, ui, &persistence, &session_id).await
        }
        "openai" | "openrouter" | _ => {
            let client = get_openai_like_client(&provider_name, &config)?;
            let model_name = match provider_name.as_str() {
                "openrouter" => "anthropic/claude-3.5-sonnet",
                "openai" => "gpt-4-turbo",
                _ => "gpt-4o",
            };
            
            let agent = client
                .agent(model_name)
                .preamble(&preamble)
                .build();

             // Add welcome message
             ui.add_message(
                "system".to_string(),
                format!(
                    "Welcome to Nanobot! Using provider: {} ({})\n\nI have access to:\n- File operations (read, write, edit, list)\n- Web search\n- Command execution",
                    provider_name, model_name
                ),
            );

            // Init Persistence
            let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
            if let Some(parent) = db_path.parent() {
                 tokio::fs::create_dir_all(parent).await?;
            }
            let persistence = PersistenceManager::new(db_path);
            persistence.init()?;
            let session_id = Uuid::new_v4().to_string();

            run_chat_loop(agent, ui, &persistence, &session_id).await
        }
    }
}

async fn run_chat_loop<P: Prompt>(agent: P, mut ui: tui::ChatUI, persistence: &PersistenceManager, session_id: &str) -> Result<()> {
    loop {
        match ui.run()? {
            Some(user_message) => {
                // Add user message to UI
                ui.add_message("user".to_string(), user_message.clone());
                
                // Save to DB
                if let Err(e) = persistence.save_message(session_id, "user", &user_message) {
                    eprintln!("Failed to save user message: {}", e);
                }

                // Agentic loop: Agent may use tools multiple times
                let mut conversation_context = user_message.clone();
                let max_iterations = 5;
                
                for _iteration in 0..max_iterations {
                    // Prompt the agent (the agent itself is stateless here, context is passed in prompt)
                    
                    let response = agent.prompt(&conversation_context).await
                        .map_err(|e| anyhow::anyhow!("Agent prompt failed: {}", e))?;

                    // Check if response is a tool call
                    if tools::executor::is_tool_call(&response) {
                        // Show tool call in UI
                        ui.add_message(
                            "system".to_string(),
                            format!("🔧 Using tool: {}", response.lines().next().unwrap_or(&response)),
                        );

                        // Execute tool
                        match tools::executor::execute_tool(&response, None, None, None).await {
                            Ok(result) => {
                                // Add tool result to UI
                                ui.add_message(
                                    "system".to_string(),
                                    format!("✓ Tool result:\n{}", result),
                                );

                                // Continue conversation with tool result
                                conversation_context = format!(
                                    "{}\n\nTool result:\n{}",
                                    conversation_context, result
                                );
                            }
                            Err(e) => {
                                // Show error
                                ui.add_message(
                                    "system".to_string(),
                                    format!("✗ Tool execution failed: {}", e),
                                );

                                // Tell agent about error
                                conversation_context = format!(
                                    "{}\n\nTool execution failed: {}",
                                    conversation_context, e
                                );
                            }
                        }
                    } else {
                        // Final response - add to UI and break
                        ui.add_message("assistant".to_string(), response.clone());
                        
                        // Save assistant response
                        if let Err(e) = persistence.save_message(session_id, "assistant", &response) {
                            eprintln!("Failed to save user message: {}", e);
                        }

                        break;
                    }
                }
            }
            None => {
                // User pressed Esc
                break;
            }
        }
    }
    Ok(())
}

async fn run_cli_agent(message: &str, provider: Option<String>, model: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    let provider_name = provider.unwrap_or(config.default_provider.clone());

    println!("User: {}", message);

    // Init Persistence
    let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
    if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
    }
    let persistence = PersistenceManager::new(db_path);
    persistence.init()?;
    let session_id = Uuid::new_v4().to_string();

    persistence.save_message(&session_id, "user", message).ok();

    match provider_name.as_str() {
        "antigravity" => {
            let client = antigravity::AntigravityClient::from_env().await?;
            let model_name = model.as_deref().unwrap_or("gemini-3-flash");
            let agent = client
                .agent(model_name)
                .preamble("You are a helpful AI assistant called Nanobot.")
                .build();
            let response = agent.prompt(message).await.map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("Nanobot: {}", response);
            persistence.save_message(&session_id, "assistant", &response).ok();
        }
        _ => {
            let client = get_openai_like_client(&provider_name, &config)?;
             // Use completion_model() -> agent logic if prompt() on client is not direct.
             // Client implements Prompt? No, agent does.
            let agent = client
                .agent("anthropic/claude-3-opus")
                .preamble("You are a helpful AI assistant called Nanobot.")
                .build();
            let response = agent.prompt(message).await.map_err(|e| anyhow::anyhow!("{}", e))?;
            println!("Nanobot: {}", response);
            persistence.save_message(&session_id, "assistant", &response).ok();
        }
    }

    Ok(())
}

async fn run_oauth_login(provider: &str) -> Result<()> {
    println!("🔐 Starting OAuth login for: {}", provider);
    
    let mut flow = oauth::OAuthFlow::new(provider);
    let auth_url = flow.get_auth_url()?;

    println!("\n📋 Step 1: Copy this URL and open it in your browser:");
    println!("{}", auth_url);
    println!("\n📋 Step 2: After logging in, copy the redirect URL from your browser");
    println!("           (It will look like: http://localhost:8080/callback?code=...)");
    print!("\n📥 Paste the redirect URL here: ");

    use std::io::{self, Write};
    io::stdout().flush()?;

    let mut redirect_url = String::new();
    io::stdin().read_line(&mut redirect_url)?;
    let redirect_url = redirect_url.trim();

    println!("\n⏳ Exchanging code for token...");
    flow.complete_flow(redirect_url).await?;

    println!("✅ Login successful! You can now use the '{}' provider.", provider);

    Ok(())
}

fn get_openai_like_client(provider_name: &str, config: &config::Config) -> Result<openai::Client> {
    match provider_name {
        "openrouter" => {
            // Use OpenRouter API key
            if let Some(ref or_config) = config.providers.openrouter {
                if !or_config.api_key.is_empty() && !or_config.api_key.starts_with("sk-or-v1-...") {
                    unsafe {
                        std::env::set_var("OPENAI_API_KEY", &or_config.api_key);
                        std::env::set_var("OPENAI_API_BASE", "https://openrouter.ai/api/v1");
                    }
                } else {
                    return Err(anyhow::anyhow!(
                        "OpenRouter API key not configured. Add your key to config.toml"
                    ));
                }
            }
        }
        "openai" => {
            // OpenAI - support both API key and OAuth
            if let Some(ref openai_config) = config.providers.openai {
                if !openai_config.api_key.is_empty() {
                    // Use API key
                    unsafe {
                        std::env::set_var("OPENAI_API_KEY", &openai_config.api_key);
                        std::env::set_var("OPENAI_API_BASE", "https://api.openai.com/v1");
                    }
                } else {
                    // Try OAuth token (ChatGPT Plus subscription)
                    let tokens = config::OAuthTokens::load()?;
                    if let Some(token) = tokens.get(provider_name) {
                        unsafe {
                            std::env::set_var("OPENAI_API_KEY", &token.access_token);
                            std::env::set_var("OPENAI_API_BASE", "https://api.openai.com/v1");
                        }
                    } else {
                        return Err(anyhow::anyhow!(
                            "OpenAI not configured. Either:\n  1. Add API key to config.toml (get from https://platform.openai.com/api-keys)\n  2. Run: nanobot-rs login openai (requires OAuth client ID)"
                        ));
                    }
                }
            }
        }
        "antigravity" => {
             return Err(anyhow::anyhow!("Antigravity should use the native client path, not OpenAI-compatible path."));
        }
        _ => {
            return Err(anyhow::anyhow!("Unknown provider: {}", provider_name));
        }
    }

    Ok(openai::Client::from_env())
}

async fn run_debug_sandbox() -> Result<()> {
    use reqwest::Client;
    use serde_json::json;
    use flowbot_rs::config::OAuthTokens; // Changed from crate::config

    println!("Debug Sandbox: Loading tokens...");
    let tokens = OAuthTokens::load()?;
    let token = tokens.get("antigravity").ok_or_else(|| anyhow::anyhow!("No antigravity token found. Run login first."))?.access_token.clone();

    let client = Client::new();
    let sandbox_url = "https://daily-cloudcode-pa.sandbox.googleapis.com";
    let url = format!("{}/v1internal:loadCodeAssist", sandbox_url);

    println!("Testing Sandbox URL: {}", url);

    let body = json!({
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        }
    });

    let res = client.post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "google-api-nodejs-client/9.15.1")
        .header("X-Goog-Api-Client", "gl-node/22.17.0")
        .json(&body)
        .send()
        .await?;

    println!("Status: {}", res.status());
    println!("Body: {}", res.text().await?);

    Ok(())
}

async fn run_telegram_gateway() -> Result<()> {
    println!("🤖 Starting Telegram Bot Gateway...\n");

    // Get Telegram token from Config or Environment
    let config = flowbot_rs::config::Config::load().ok();
    
    let telegram_token = if let Some(ref c) = config {
        if let Some(ref tg) = c.providers.telegram {
             tg.bot_token.clone()
        } else {
             // Fallback to env
             std::env::var("TELEGRAM_BOT_TOKEN")
                .or_else(|_| std::env::var("NANOBOT_TELEGRAM_TOKEN"))
                .unwrap_or_default()
        }
    } else {
         std::env::var("TELEGRAM_BOT_TOKEN")
            .or_else(|_| std::env::var("NANOBOT_TELEGRAM_TOKEN"))
            .unwrap_or_default()
    };

    if telegram_token.is_empty() {
        return Err(anyhow::anyhow!(
            "Telegram bot token not found.\n\
             Run 'flowbot setup --telegram' to configure it interactively.\n\
             Or set env var: export TELEGRAM_BOT_TOKEN=your_token_here"
        ));
    }

    // Get allowed users (Legacy Env Support - pairing system relies on DB)
    // We pass None to config if not set in env, let pairing system handle it
    let allowed_users: Option<Vec<i64>> = std::env::var("TELEGRAM_ALLOWED_USERS")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|id| id.trim().parse().ok())
                .collect()
        });

    if let Some(ref users) = allowed_users {
        println!("📋 Allowed users (Legacy): {:?}", users);
    } 

    let telegram_config = telegram::TelegramConfig {
        token: telegram_token,
        allowed_users,
    };

    // Create channels for communication
    let (agent_tx, telegram_rx) = tokio::sync::mpsc::channel(100);
    let (response_tx, response_rx) = tokio::sync::mpsc::channel(100);

    // Create Telegram bot
    let bot = telegram::TelegramBot::new(telegram_config, agent_tx, response_rx);

    // Create simple agent
    let agent = telegram::SimpleAgent::new(telegram_rx, response_tx);

    println!("✅ Telegram bot started!");
    println!("📱 Send a message to your bot to test it\n");

    // Run both concurrently
    tokio::select! {
        result = bot.run() => {
            if let Err(e) = result {
                eprintln!("❌ Telegram bot error: {}", e);
            }
        }
        _ = agent.run() => {
            println!("Agent loop stopped");
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\n👋 Shutting down gracefully...");
        }
    }

    Ok(())
}
