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
use flowbot_rs::{config, oauth, tui, doctor, tools, antigravity, telegram, gateway, security}; // Added security

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
        /// Enable Rich Terminal UI
        #[arg(long)]
        tui: bool,
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
    /// Security Auditing Tools
    Security {
        #[command(subcommand)]
        action: SecurityAction,
    },
    /// Memory Management
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Manage Skills (extensible tools/plugins)
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
    /// Start WebChat UI (embedded web interface)
    WebChat {
        /// Port to listen on (default: 3030)
        #[arg(short, long, default_value = "3030")]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum MemoryAction {
    /// Show memory statistics (indexed files, vector count, DB size)
    Status,
    /// Wipe all vectors from the memory store
    Clean {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Re-index all workspace files
    Reindex {
        /// Workspace directory to index
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum SkillsAction {
    /// List all available skills
    List {
        /// Show all skills including disabled ones
        #[arg(long)]
        all: bool,
    },
    /// Enable a skill
    Enable {
        /// Skill name
        name: String,
    },
    /// Disable a skill
    Disable {
        /// Skill name
        name: String,
    },
    /// Create a new skill scaffold
    Create {
        /// Skill name
        name: String,
        /// Category: automation, integration, productivity, custom
        #[arg(long, default_value = "custom")]
        category: String,
    },
}

#[derive(Subcommand, Debug)]
enum SecurityAction {
    /// Audit system security (file permissions, config, known risks)
    Audit,
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
    human_panic::setup_panic!();
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
        Commands::Chat { provider, tui } => {
            if tui {
                run_rich_tui_chat(provider).await?;
            } else {
                run_legacy_chat(provider).await?;
            }
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
        Commands::WebChat { port } => {
            println!("🚀 Starting WebChat UI on port {}...", port);
            flowbot_rs::web::run_server(port).await?;
        }
        Commands::Security { action } => {
            match action {
                SecurityAction::Audit => {
                    let mut auditor = security::audit::SecurityAuditor::new();
                    println!("🔎 Running Security Audit...");
                    if let Err(e) = auditor.run_all_checks() {
                        eprintln!("❌ Error running checks: {}", e);
                    }
                    auditor.print_report();
                }
            }
        }
        Commands::Skills { action } => {
            use flowbot_rs::skills::SkillLoader;
            
            let workspace_dir = std::env::current_dir()?;
            let mut loader = SkillLoader::new(workspace_dir);
            loader.scan()?;
            
            match action {
                SkillsAction::List { all } => {
                    let skills: Vec<_> = if *all {
                        loader.skills().values().collect()
                    } else {
                        loader.enabled_skills().collect()
                   };
                    
                    if skills.is_empty() {
                        println!("\n📦 No skills found.");
                        println!("💡 Create a new skill with: nanobot skills create <name>\n");
                    } else {
                        println!("\n📦 Available Skills:\n");
                        println!("{:<20} {:<15} {:<10} {}", "Name", "Category", "Status", "Tools");
                        println!("{}", "-".repeat(70));
                        
                        for skill in skills {
                            let enabled_mark = if skill.enabled { "✓" } else { "✗" };
                            let tools_count = skill.tools.len();
                            println!(
                                "{:<20} {:<15} {:<10} {}",
                                skill.name,
                                skill.category,
                                format!("{} {}", enabled_mark, skill.status),
                                format!("{} tools", tools_count)
                            );
                        }
                        println!();
                    }
                }
                SkillsAction::Enable { name } => {
                    loader.enable_skill(name)?;
                    println!("✓ Enabled skill: {}", name);
                }
                SkillsAction::Disable { name } => {
                    loader.disable_skill(name)?;
                    println!("✓ Disabled skill: {}", name);
                }
                SkillsAction::Create { name, category } => {
                    let skills_dir = std::env::current_dir()?.join("skills");
                    tokio::fs::create_dir_all(&skills_dir).await?;
                    
                    let skill_dir = skills_dir.join(name);
                    tokio::fs::create_dir_all(&skill_dir).await?;
                    
                    let skill_template = format!(r#"---
name: {}
description: "A custom skill"
category: {}
status: active
---

# {} Skill

Brief description of what this skill does.

## Tools Provided

- `{}_tool`: Description of the tool

## Configuration

```toml
[skills.{}]
enabled = true
```

## Usage Examples

```
> Use {}_tool to do something
✓ Done!
```

## Implementation Notes

Add any notes about how this skill works.
"#, name, category, name, name, name, name);
                    
                    tokio::fs::write(skill_dir.join("SKILL.md"), skill_template).await?;
                    
                    println!("✓ Created new skill: {}", name);
                    println!("📝 Edit: {}/SKILL.md", skill_dir.display());
                }
            }
        }
        Commands::Memory { action } => {
            use flowbot_rs::memory::MemoryManager;
            
            let db_path = PathBuf::from(".").join(".nanobot").join("memory.db");
            if let Some(parent) = db_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            
            match action {
                MemoryAction::Status => {
                    println!("📊 Memory Status\n");
                    
                    // Check if DB exists
                    if !db_path.exists() {
                        println!("⚠️  Memory database not found at: {}", db_path.display());
                        println!("💡 Run 'flowbot memory reindex' to create and populate the memory store.");
                        return Ok(());
                    }
                    
                    // Get DB file size
                    let db_size = std::fs::metadata(&db_path)?.len();
                    let db_size_mb = db_size as f64 / (1024.0 * 1024.0);
                    
                    println!("📁 Database: {}", db_path.display());
                    println!("💾 Size: {:.2} MB", db_size_mb);
                    
                    // TODO: Query MemoryManager for vector count
                    // This would require adding a stats() method to MemoryManager
                    println!("\n💡 For detailed stats, use the MemoryManager API");
                }
                MemoryAction::Clean { force } => {
                    if !force {
                        print!("⚠️  This will delete all indexed vectors. Continue? (y/N): ");
                        use std::io::{self, Write};
                        io::stdout().flush()?;
                        
                        let mut response = String::new();
                        io::stdin().read_line(&mut response)?;
                        
                        if response.trim().to_lowercase() != "y" {
                            println!("❌ Cancelled.");
                            return Ok(());
                        }
                    }
                    
                    println!("🧹 Cleaning memory store...");
                    
                    if db_path.exists() {
                        std::fs::remove_file(&db_path)?;
                        println!("✅ Memory store cleaned.");
                    } else {
                        println!("ℹ️  Memory store does not exist.");
                    }
                }
                MemoryAction::Reindex { workspace } => {
                    println!("🔄 Re-indexing workspace...");
                    
                    let workspace_path = workspace
                        .map(PathBuf::from)
                        .unwrap_or_else(|| std::env::current_dir().unwrap());
                    
                    println!("📂 Workspace: {}", workspace_path.display());
                    
                    // Initialize memory manager
                    let provider = flowbot_rs::memory::EmbeddingProvider::local()?;
                    let mut manager = MemoryManager::new(db_path.clone(), provider);
                    
                    // Simple scan: find all .rs, .md, .txt files
                    println!("🔍 Scanning files...");
                    let mut files = Vec::new();
                    for entry in walkdir::WalkDir::new(&workspace_path)
                        .max_depth(5)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        if entry.file_type().is_file() {
                            if let Some(ext) = entry.path().extension() {
                                let ext_str = ext.to_str().unwrap_or("");
                                if ["rs", "md", "txt", "toml", "json"].contains(&ext_str) {
                                    files.push(entry.path().to_path_buf());
                                }
                            }
                        }
                    }
                    
                    println!("📄 Found {} files", files.len());
                    
                    // Batch process
                    for file in files {
                        if let Ok(content) = tokio::fs::read_to_string(&file).await {
                            let mut metadata = std::collections::HashMap::new();
                            metadata.insert("path".to_string(), file.display().to_string());
                            
                            if let Err(e) = manager.add_document(&content, metadata).await {
                                eprintln!("⚠️  Failed to index {}: {}", file.display(), e);
                            } else {
                                println!("✓ {}", file.display());
                            }
                        }
                    }
                    
                    println!("\n✅ Re-indexing complete!");
                }
            }
        }
    }

    Ok(())
}

async fn run_rich_tui_chat(provider: Option<String>) -> Result<()> {
    // Initialize TUI Logger
    flowbot_rs::ui::logger::LoggerService::init()?;
    
    // Start TUI Manager
    let mut tui = flowbot_rs::ui::tui::TuiManager::new()?;
    tui.start()?;

    // Load Config and Provider
    let config = config::Config::load()?;
    let provider_name = provider.unwrap_or(config.default_provider.clone());
    let preamble = format!(
        "You are FlowBot, a helpful AI assistant with tool access.\n{}",
        tools::executor::get_tool_descriptions()
    );

    // Initialize Persistence
    let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");
    if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
    }
    let persistence = PersistenceManager::new(db_path);
    persistence.init()?;
    let session_id = Uuid::new_v4().to_string();

    log::info!("Starting session: {}", session_id);
    log::info!("Provider: {}", provider_name);
    log::info!("Type your message and press Enter. Ctrl+C to quit.");

    // Setup Agent
    // Note: We need to box the agent to handle different types if we wanted to be generic,
    // but for now let's just use the same match logic as legacy.
    
    match provider_name.as_str() {
        "antigravity" => {
            let client = antigravity::AntigravityClient::from_env().await?;
            let agent = client.agent("gemini-2.5-flash").preamble(&preamble).build();
            run_rich_loop(tui, agent, &persistence, &memory_manager, &session_id).await
        }
        _ => {
            let client = get_openai_like_client(&provider_name, &config)?;
            let model_name = match provider_name.as_str() {
                "openrouter" => "anthropic/claude-3.5-sonnet",
                "openai" => "gpt-4-turbo",
                _ => "gpt-4o",
            };
            let agent = client.agent(model_name).preamble(&preamble).build();
            run_rich_loop(tui, agent, &persistence, &memory_manager, &session_id).await
        }
    }
}

async fn run_rich_loop<P: Prompt>(
    mut tui: flowbot_rs::ui::tui::TuiManager,
    agent: P,
    persistence: &PersistenceManager,
    memory_manager: &flowbot_rs::memory::vector_store::MemoryManager,
    session_id: &str,
) -> Result<()> {
    loop {
        // Wait for input
        let input_opt = tui.wait_for_input().await?;
        
        match input_opt {
            Some(input) => {
                if input.trim().is_empty() { continue; }
                if input.trim() == "/quit" { break; }
                
                log::info!("> {}", input);
                persistence.save_message(session_id, "user", &input).ok();
                
                // RAG Retrieval
                let mut full_input = input.clone();
                let results = memory_manager.search(&input, 3).await.unwrap_or_default();
                if !results.is_empty() {
                    let context_str = results.iter()
                        .map(|(score, entry)| format!("- {} (similarity: {:.2})", entry.content, score))
                        .collect::<Vec<_>>()
                        .join("\n");
                    
                    full_input = format!("Context from memory:\n{}\n\nUser Query: {}", context_str, input);
                    log::info!("🧠 Retrieved {} memory items", results.len());
                }
                
                // Prompt Agent
                // TODO: Move to background task for non-blocking UI
                match agent.prompt(&full_input).await {
                    Ok(response) => {
                        log::info!("Bot: {}", response);
                        persistence.save_message(session_id, "assistant", &response).ok();
                        
                        // Save to Long Term Memory
                        let mut metadata = std::collections::HashMap::new();
                        metadata.insert("role".to_string(), "user".to_string());
                        metadata.insert("session_id".to_string(), session_id.to_string());
                        metadata.insert("type".to_string(), "conversation".to_string());
                        
                        let memory_content = format!("User: {}\nAssistant: {}", input, response);
                        if let Err(e) = memory_manager.add_document(&memory_content, metadata).await {
                             log::error!("Failed to save memory: {}", e);
                        }

                        // Handle tool calls if any
                        if tools::executor::is_tool_call(&response) {
                              log::info!("🔧 Executing tool...");
                              match tools::executor::execute_tool(&response, None, None, None).await {
                                 Ok(res) => log::info!("✓ Result: {}", res),
                                 Err(e) => log::error!("✗ Error: {}", e),
                              }
                         }
                    }
                    Err(e) => {
                        log::error!("Error: {}", e);
                    }
                }
            }
            None => break, // Quit
        }
    }
    
    tui.stop()?;
    Ok(())
}
}

async fn run_legacy_chat(provider: Option<String>) -> Result<()> {
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
