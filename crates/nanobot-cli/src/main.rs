use anyhow::Result;
use clap::{Parser, Subcommand};
use rig::client::ProviderClient;
use rig::providers::openai;
use tokio::sync::mpsc;
use std::path::PathBuf;
use uuid::Uuid;

// Import modules from nanobot-core
use nanobot_core::{
    AgentLoop, // Re-exported at root
    config, oauth, doctor, gateway, security,
    persistence::PersistenceManager,
};

// Alias the moved telegram module to maintain compatibility
use nanobot_core::gateway::telegram_adapter as telegram;

// Local modules
mod service;
mod web;

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
    /// Service management (install/start/stop daemon)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Memory Management
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Run agent from manifest file
    Run {
        /// Path to agent manifest (agent.toml)
        agent: PathBuf,
    },
    /// Start admin API server
    Admin {
        /// Port to listen on (default: 3000)
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
    /// Connect to admin API console (REPL)
    Console {
        /// Admin API port (default: 3000)
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
    /// Development mode with auto-rebuild on file changes
    Dev {
        /// Optional agent manifest to run
        agent: Option<PathBuf>,
        /// Port for admin API (optional)
        #[arg(short, long)]
        port: Option<u16>,
    },
    /// Start WebChat UI server
    WebChat {
        /// Port to listen on (default: 8080)
        #[arg(short, long, default_value = "8080")]
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
enum SecurityAction {
    /// Audit system security (file permissions, config, known risks)
    Audit,
}

#[derive(Subcommand, Debug)]
enum ServiceAction {
    /// Install service (systemd/Task Scheduler)
    Install {
        /// Force reinstall even if already installed
        #[arg(long)]
        force: bool,
        /// Output JSON for automation
        #[arg(long)]
        json: bool,
    },
    /// Uninstall service
    Uninstall {
        /// Output JSON for automation
        #[arg(long)]
        json: bool,
    },
    /// Start the daemon
    Start {
        /// Output JSON for automation
        #[arg(long)]
        json: bool,
    },
    /// Stop the daemon
    Stop {
        /// Output JSON for automation
        #[arg(long)]
        json: bool,
    },
    /// Restart the daemon
    Restart {
        /// Output JSON for automation
        #[arg(long)]
        json: bool,
    },
    /// Show daemon status
    Status {
        /// Output JSON for automation
        #[arg(long)]
        json: bool,
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
                nanobot_core::setup::telegram::run_telegram_setup_wizard().await?;
                return Ok(());
            }

            if wizard {
                let opts = nanobot_core::setup::SetupOptions {
                    workspace_dir: workspace.map(PathBuf::from),
                    skip_wizard: false,
                };
                let result = nanobot_core::setup::run_setup_wizard(opts).await?;
                
                // Chain OAuth if requested
                if result.should_run_oauth {
                    if let Some(provider) = result.oauth_provider {
                        println!();
                        println!("{}", console::style("Running OAuth login...").bold().cyan());
                        run_oauth_login(&provider).await?;
                    }
                }
                
                // Chain service installation if requested
                if result.should_install_service {
                    println!();
                    println!("{}", console::style("Installing system service...").bold().cyan());
                    if let Err(e) = service::install_service().await {
                        println!("{}", console::style(format!("⚠️  Service installation failed: {}", e)).yellow());
                        println!("You can install it later with: nanobot service install");
                    } else {
                        println!("{}", console::style("✅ Service installed!").green().bold());
                    }
                }
                
                // Chain TUI hatch if requested
                if result.should_hatch_tui {
                    println!();
                    println!("{}", console::style("🚀 Hatching into TUI...").bold().cyan());
                    println!();
                    run_rich_tui_chat().await?;
                } else if result.should_start_gateway {
                    // Launch gateway instead of TUI
                    println!();
                    println!("{}", console::style("🚀 Starting Gateway...").bold().cyan());
                    println!("Gateway will keep all your channels connected.");
                    println!("Press Ctrl+C to stop.");
                    println!();
                    
                    // Run gateway (this is a blocking call)
                    run_telegram_gateway().await?;
                }
            } else {
                // ... basic setup
                let opts = nanobot_core::setup::SetupOptions {
                    workspace_dir: workspace.map(PathBuf::from),
                    skip_wizard: true,
                };
                nanobot_core::setup::basic_setup(opts).await?;
            }
        }
        Commands::Workspace { command } => {
            use nanobot_core::setup::workspace_mgmt;
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
            if !tui {
                eprintln!("Legacy chat mode is deprecated. Using Rich TUI.");
            }
            run_rich_tui_chat(provider).await?;
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
            // Initialize file-based logging
            if let Err(e) = nanobot_core::logging::init_file_logging() {
                eprintln!("⚠️  Failed to initialize logging: {}", e);
                eprintln!("   Continuing without file logging...");
            } else {
                println!("📝 File logging initialized at ~/.nanobot/logs/");
            }
            
            // Clean up old logs (keep last 5 files)
            if let Err(e) = nanobot_core::logging::cleanup_old_logs(5) {
                tracing::warn!("Failed to cleanup old logs: {:?}", e);
            }
            
            // Setup shutdown coordinator
            let mut coordinator = nanobot_core::shutdown::ShutdownCoordinator::new();
            
            // Start health check server in background
            let health_port = 8081; // Default health check port
            let health_handle = tokio::spawn(async move {
                if let Err(e) = nanobot_core::health::start_health_server(health_port).await {
                    tracing::error!("Health server error: {:?}", e);
                }
            });
            coordinator.add_handle(health_handle);
            
            // Setup shutdown signal handler
            let shutdown_handle = tokio::spawn(async move {
                if let Err(e) = nanobot_core::shutdown::setup_shutdown_handler().await {
                    tracing::error!("Shutdown handler error: {:?}", e);
                }
            });
            
            tracing::info!("Starting Nanobot Gateway");
            tracing::info!("Health check available at http://localhost:{}/health", health_port);
            tracing::info!("Press Ctrl+C to shutdown gracefully");
            
            // Run the Telegram gateway with shutdown check
            let gateway_handle = tokio::spawn(async move {
                if let Err(e) = run_telegram_gateway().await {
                    tracing::error!("Gateway error: {:?}", e);
                }
            });
            coordinator.add_handle(gateway_handle);
            
            // Wait for shutdown signal
            shutdown_handle.await?;
            
            // Gracefully shutdown all tasks (10 second timeout)
            coordinator.shutdown(10).await;
            
            tracing::info!("Nanobot shutdown complete");
        }
        Commands::Pairing { action } => {
            // Initialize pairing database
            nanobot_core::pairing::init_database().await?;
            
            match action {
                PairingAction::List { channel } => {
                    let chan = channel.as_deref().unwrap_or("all");
                    let requests = nanobot_core::pairing::get_pending_requests(chan).await?;
                    
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
                    match nanobot_core::pairing::approve(&channel, &code).await {
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
                    match nanobot_core::pairing::reject(&channel, &code).await {
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
             let pm = PersistenceManager::new(db_path.clone());
             pm.init()?;

            // Create a dummy event channel (CLI doesn't listen to events)
            let (cron_event_tx, _cron_event_rx) = tokio::sync::mpsc::channel(100);
            
            let scheduler = nanobot_core::cron::CronScheduler::new(db_path.clone(), cron_event_tx).await?;
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
                                 nanobot_core::cron::Schedule::Cron{expr, ..} => expr,
                                 nanobot_core::cron::Schedule::Every{every_ms, ..} => format!("Every {}ms", every_ms),
                                 nanobot_core::cron::Schedule::At{at_ms} => format!("At {}", at_ms),
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
                    use nanobot_core::cron::{Schedule, Payload, SessionTarget, CronJob};
                    
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
                    use nanobot_core::cron::run_log;
                    
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
        Commands::Memory { action } => {
            use nanobot_core::memory::MemoryManager;
            
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
                        println!("💡 Run 'nanobot memory reindex' to create and populate the memory store.");
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
                    let provider = nanobot_core::memory::EmbeddingProvider::local()?;
                    let manager = MemoryManager::new(db_path.clone(), provider);
                    
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
                            
                            if let Err(e) = manager.add_document(&content, metadata, None).await {
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
        Commands::Run { agent } => {
            use nanobot_core::config::agent_loader::AgentLoader;
            
            println!("🚀 Loading agent from manifest: {}", agent.display());
            
            let manifest = AgentLoader::load(&agent)?;
            AgentLoader::validate(&manifest)?;
            
            println!("✅ Manifest valid!");
            AgentLoader::info(&manifest);
            
            println!("\n⚠️  Full agent runtime not yet implemented.");
            println!("   Coming soon: Channel setup, tool registration, and agent execution.");
        }
        Commands::Admin { port } => {
            println!("🔧 Starting Admin API server on port {}...", port);
            nanobot_core::server::start_admin_server(port).await?;
        }
        Commands::Console { port } => {
            use nanobot_core::console::ConsoleREPL;
            
            println!("🎮 Starting interactive console...");
            let mut repl = ConsoleREPL::new(port);
            repl.run().await?;
        }
        Commands::Dev { agent, port } => {
            println!("🔥 Starting development mode with auto-rebuild...");
            println!("   Press Ctrl+C to stop");
            
            // Check if cargo-watch is installed
            let check = std::process::Command::new("cargo")
                .args(["watch", "--version"])
                .output();
            
            if check.is_err() || !check.unwrap().status.success() {
                eprintln!("❌ cargo-watch is not installed!");
                eprintln!("   Install it with: cargo install cargo-watch");
                return Err(anyhow::anyhow!("cargo-watch not found"));
            }
            
            // Build the watch command
            let mut args = vec!["watch", "-x", "run", "--"];
            
            if let Some(ref agent_path) = agent {
                args.push("run");
                args.push(agent_path.to_str().unwrap());
            }
            
            let port_string;
            if let Some(admin_port) = port {
                args.push("admin");
                args.push("--port");
                port_string = admin_port.to_string();
                args.push(&port_string);
            }
            
            println!("   Running: cargo {}", args.join(" "));
            
            let status = std::process::Command::new("cargo")
                .args(&args)
                .status()?;
            
            if !status.success() {
                return Err(anyhow::anyhow!("cargo watch failed"));
            }
        }
        Commands::WebChat { port } => {
            use nanobot_core::agent::AgentLoop;
            
            println!("🌐 Starting WebChat UI server on port {}...", port);
            println!("⚙️  Initializing agent...");
            
            // Create agent loop with all features
            let agent = AgentLoop::new().await?;
            let (agent_tx, agent_rx) = tokio::sync::mpsc::channel(100);
            
            // Spawn agent loop in background
            tokio::spawn(async move {
                agent.run(agent_rx).await;
            });
            
            // Run web server with agent connection
            web::run_server(port, agent_tx).await?;
        }
        Commands::Service { action } => {
            use service::{ServiceManager, ServiceResponse, ServiceInfo};
            
            let manager = ServiceManager::new();
            
            // Helper to output JSON or human-readable
            let output_response = |response: ServiceResponse, json: bool| {
                if json {
                    println!("{}", serde_json::to_string_pretty(&response).unwrap());
                } else if !response.ok {
                    if let Some(error) = response.error {
                        eprintln!("❌ {}", error);
                    }
                } else if let Some(message) = response.message {
                    println!("{}", message);
                }
            };
            
            match action {
                ServiceAction::Install { force, json } => {
                    // Check if already installed
                    if !force && manager.is_installed() {
                        let response = ServiceResponse {
                            ok: true,
                            action: "install".to_string(),
                            result: Some("already-installed".to_string()),
                            message: Some("✅ Service already installed. Use --force to reinstall.".to_string()),
                            error: None,
                            service: Some(ServiceInfo {
                                label: if cfg!(target_os = "linux") { "systemd" } else { "Task Scheduler" }.to_string(),
                                loaded: true,
                                runtime: None,
                            }),
                        };
                        output_response(response, json);
                        return Ok(());
                    }
                    
                    if !json {
                        println!("🔧 Installing service...");
                    }
                    
                    match manager.install() {
                        Ok(()) => {
                            let response = ServiceResponse {
                                ok: true,
                                action: "install".to_string(),
                                result: Some("installed".to_string()),
                                message: if !json { Some("✅ Service installed successfully!".to_string()) } else { None },
                                error: None,
                                service: Some(ServiceInfo {
                                    label: if cfg!(target_os = "linux") { "systemd" } else { "Task Scheduler" }.to_string(),
                                    loaded: true,
                                    runtime: None,
                                }),
                            };
                            output_response(response, json);
                        }
                        Err(e) => {
                            let response = ServiceResponse {
                                ok: false,
                                action: "install".to_string(),
                                result: None,
                                message: None,
                                error: Some(format!("Installation failed: {}", e)),
                                service: None,
                            };
                            output_response(response, json);
                            std::process::exit(1);
                        }
                    }
                }
                ServiceAction::Uninstall { json } => {
                    if !json {
                        println!("🗑️  Uninstalling service...");
                    }
                    
                    match manager.uninstall() {
                        Ok(()) => {
                            let response = ServiceResponse {
                                ok: true,
                                action: "uninstall".to_string(),
                                result: Some("uninstalled".to_string()),
                                message: if !json { Some("✅ Service uninstalled successfully".to_string()) } else { None },
                                error: None,
                                service: None,
                            };
                            output_response(response, json);
                        }
                        Err(e) => {
                            let response = ServiceResponse {
                                ok: false,
                                action: "uninstall".to_string(),
                                result: None,
                                message: None,
                                error: Some(format!("{}", e)),
                                service: None,
                            };
                            output_response(response, json);
                        }
                    }
                }
                ServiceAction::Start { json } => {
                    if !json {
                        println!("▶️  Starting service...");
                    }
                    
                    match manager.start() {
                        Ok(()) => {
                            let response = ServiceResponse {
                                ok: true,
                                action: "start".to_string(),
                                result: Some("started".to_string()),
                                message: if !json { Some("✅ Service started".to_string()) } else { None },
                                error: None,
                                service: None,
                            };
                            output_response(response, json);
                        }
                        Err(e) => {
                            let response = ServiceResponse {
                                ok: false,
                                action: "start".to_string(),
                                result: None,
                                message: None,
                                error: Some(format!("{}", e)),
                                service: None,
                            };
                            output_response(response, json);
                        }
                    }
                }
                ServiceAction::Stop { json } => {
                    if !json {
                        println!("⏹️  Stopping service...");
                    }
                    
                    match manager.stop() {
                        Ok(()) => {
                            let response = ServiceResponse {
                                ok: true,
                                action: "stop".to_string(),
                                result: Some("stopped".to_string()),
                                message: if !json { Some("✅ Service stopped".to_string()) } else { None },
                                error: None,
                                service: None,
                            };
                            output_response(response, json);
                        }
                        Err(e) => {
                            let response = ServiceResponse {
                                ok: false,
                                action: "stop".to_string(),
                                result: None,
                                message: None,
                                error: Some(format!("{}", e)),
                                service: None,
                            };
                            output_response(response, json);
                        }
                    }
                }
                ServiceAction::Restart { json } => {
                    if !json {
                        println!("🔄 Restarting service...");
                    }
                    
                    match manager.restart() {
                        Ok(()) => {
                            let response = ServiceResponse {
                                ok: true,
                                action: "restart".to_string(),
                                result: Some("restarted".to_string()),
                                message: if !json { Some("✅ Service restarted".to_string()) } else { None },
                                error: None,
                                service: None,
                            };
                            output_response(response, json);
                        }
                        Err(e) => {
                            let response = ServiceResponse {
                                ok: false,
                                action: "restart".to_string(),
                                result: None,
                                message: None,
                                error: Some(format!("{}", e)),
                                service: None,
                            };
                            output_response(response, json);
                        }
                    }
                }
                ServiceAction::Status { json } => {
                    match manager.status() {
                        Ok(runtime) => {
                            let response = ServiceResponse {
                                ok: true,
                                action: "status".to_string(),
                                result: Some(runtime.status.to_string()),
                                message: if !json {
                                    let mut msg = format!("📊 Service Status\n   Status: {}", runtime.status);
                                    if let Some(pid) = runtime.pid {
                                        msg.push_str(&format!("\n   PID: {}", pid));
                                    }
                                    if let Some(uptime) = runtime.uptime_seconds {
                                        msg.push_str(&format!("\n   Uptime: {}s", uptime));
                                    }
                                    Some(msg)
                                } else {
                                    None
                                },
                                error: None,
                                service: Some(ServiceInfo {
                                    label: if cfg!(target_os = "linux") { "systemd" } else { "Task Scheduler" }.to_string(),
                                    loaded: runtime.status == service::ServiceStatus::Running,
                                    runtime: Some(runtime),
                                }),
                            };
                            output_response(response, json);
                        }
                        Err(e) => {
                            let response = ServiceResponse {
                                ok: false,
                                action: "status".to_string(),
                                result: None,
                                message: None,
                                error: Some(format!("{}", e)),
                                service: None,
                            };
                            output_response(response, json);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_rich_tui_chat(_provider: Option<String>) -> Result<()> {
    // Initialize TUI Logger
    nanobot_cli::ui::logger::LoggerService::init()?;
    
    // Start TUI Manager
    let mut tui = nanobot_cli::ui::tui::TuiManager::new()?;
    tui.start()?;

    // Initialize AgentLoop
    // Note: This ignores the 'provider' argument and uses config.toml for now, matching run_cli_agent behavior.
    // Future improvement: Add override support to AgentLoop::new()
    let agent_loop = nanobot_core::agent::AgentLoop::new().await?;
    
    // Create channels
    let (agent_tx, agent_rx) = tokio::sync::mpsc::channel(100);
    
    // Spawn AgentLoop
    tokio::spawn(async move {
        agent_loop.run(agent_rx).await;
    });

    let session_id = Uuid::new_v4().to_string();

    log::info!("Starting session: {}", session_id);
    log::info!("AgentLoop active. Type your message and press Enter. Ctrl+C to quit.");

    run_rich_loop(tui, agent_tx, &session_id).await
}

async fn run_rich_loop(
    mut tui: nanobot_cli::ui::tui::TuiManager,
    agent_tx: tokio::sync::mpsc::Sender<nanobot_core::agent::AgentMessage>,
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
                
                // Create response channel for this interaction
                let (response_tx, mut response_rx) = tokio::sync::mpsc::channel(100);
                
                let msg = nanobot_core::agent::AgentMessage {
                    session_id: session_id.to_string(),
                    tenant_id: "default".to_string(), // CLI/TUI uses default tenant
                    content: input,
                    response_tx,
                };
                
                if let Err(e) = agent_tx.send(msg).await {
                    log::error!("Failed to send message to agent: {}", e);
                    break;
                }
                
                let mut full_response = String::new();
                let mut thinking_buffer = String::new();
                
                while let Some(chunk) = response_rx.recv().await {
                    match chunk {
                        nanobot_core::agent::StreamChunk::Thinking(text) => {
                            thinking_buffer.push_str(&text);
                        }
                        nanobot_core::agent::StreamChunk::TextDelta(text) => {
                            if !thinking_buffer.is_empty() {
                                log::info!("💭 Thought: {}", thinking_buffer);
                                thinking_buffer.clear();
                            }
                            full_response.push_str(&text);
                        }
                        nanobot_core::agent::StreamChunk::ToolCall(name) => {
                            if !thinking_buffer.is_empty() {
                                log::info!("💭 Thought: {}", thinking_buffer);
                                thinking_buffer.clear();
                            }
                            log::info!("🔧 Tool: {}", name);
                        }
                        nanobot_core::agent::StreamChunk::ToolResult(res) => {
                            let display_res: String = res.chars().take(100).collect();
                            let suffix = if res.len() > 100 { "..." } else { "" };
                            log::info!("✓ Result: {}{}", display_res, suffix);
                        }
                        nanobot_core::agent::StreamChunk::Done => {
                            if !thinking_buffer.is_empty() {
                                log::info!("💭 Thought: {}", thinking_buffer);
                                thinking_buffer.clear();
                            }
                            break;
                        }
                    }
                }
                
                log::info!("Bot: {}", full_response);
            }
            None => break, // Quit
        }
    }
    
    tui.stop()?;
    Ok(())
}
async fn run_cli_agent(message: &str, _provider: Option<String>, _model: Option<String>) -> Result<()> {
    // Initialize AgentLoop (now CLI gets RAG + Cron + Personality!)
    let agent_loop = nanobot_core::agent::AgentLoop::new().await?;
    
    // Create channels
    let (agent_tx, agent_rx) = tokio::sync::mpsc::channel(100);
    let (response_tx, mut response_rx) = tokio::sync::mpsc::channel(100);
    
    // Spawn AgentLoop
    tokio::spawn(async move {
        agent_loop.run(agent_rx).await;
    });
    
    // Send message to AgentLoop
    let agent_msg = nanobot_core::agent::AgentMessage {
        session_id: format!("cli:{}", uuid::Uuid::new_v4()),
        tenant_id: "default".to_string(), // CLI uses default tenant
        content: message.to_string(),
        response_tx,
    };
    
    agent_tx.send(agent_msg).await?;
    
    // Collect and print streaming response
    let mut first_chunk = true;
    while let Some(chunk) = response_rx.recv().await {
        match chunk {
            nanobot_core::agent::StreamChunk::Thinking(text) => {
                print!("\x1b[90m{}\x1b[0m", text);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
            nanobot_core::agent::StreamChunk::TextDelta(text) => {
                if first_chunk {
                    print!("\n"); // Add newline before first response
                    first_chunk = false;
                }
                print!("{}", text);
                std::io::Write::flush(&mut std::io::stdout())?;
            }
            nanobot_core::agent::StreamChunk::ToolCall(call) => {
                eprintln!("\n🔧 Tool: {}", call);
            }
            nanobot_core::agent::StreamChunk::ToolResult(result) => {
                eprintln!("✓ Result: {}", result.chars().take(100).collect::<String>());
            }
            nanobot_core::agent::StreamChunk::Done => break,
        }
    }
    
    println!("\n"); // Final newline
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

#[allow(dead_code)]
fn get_openai_like_client(provider_name: &str, config: &config::Config) -> Result<openai::Client> {
    match provider_name {
        "openrouter" => {
            // Use OpenRouter API key
            if let Some(ref or_config) = config.providers.openrouter {
                if let Some(api_key) = &or_config.api_key {
                    if !api_key.is_empty() && !api_key.starts_with("sk-or-v1-...") {
                        unsafe {
                            std::env::set_var("OPENAI_API_KEY", api_key);
                            std::env::set_var("OPENAI_API_BASE", "https://openrouter.ai/api/v1");
                        }
                    } else {
                        return Err(anyhow::anyhow!(
                            "OpenRouter API key not configured. Add your key to config.toml"
                        ));
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
                if let Some(api_key) = &openai_config.api_key {
                    if !api_key.is_empty() {
                        // Use API key
                        unsafe {
                            std::env::set_var("OPENAI_API_KEY", api_key);
                            std::env::set_var("OPENAI_API_BASE", "https://api.openai.com/v1");
                        }
                    } 
                } else {
                    // Try OAuth token (ChatGPT Plus subscription)
                    let tokens = nanobot_core::oauth::OAuthTokens::load()?;
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
    use nanobot_core::config::OAuthTokens; // Changed from crate::config

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
    let config = nanobot_core::config::Config::load().ok();
    
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
             Run 'nanobot setup --telegram' to configure it interactively.\n\
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

    // Initialize AgentLoop (now Telegram gets RAG + Cron + Personality!)
    println!("🧠 Initializing AgentLoop with full features...");
    let agent_loop = nanobot_core::agent::AgentLoop::new().await?;
    
    // Create channel for AgentMessages
    let (agent_tx, agent_rx) = tokio::sync::mpsc::channel(100);

    // Initialize Gateway Registry
    let registry = std::sync::Arc::new(nanobot_core::gateway::registry::ChannelRegistry::new());

    // Spawn AgentLoop
    tokio::spawn(async move {
        println!("🤖 AgentLoop started for Telegram");
        agent_loop.run(agent_rx).await;
    });

    // Create Telegram bot (now sends to AgentLoop AND listens to Registry)
    let bot = telegram::TelegramBot::new(telegram_config, agent_tx, registry.clone());

    println!("✅ Telegram bot started with AgentLoop + Actor Registry!");
    println!("📱 Features enabled: RAG, Cron, Personality, Tool Loop, Bidirectional Messaging\n");

    // Run bot (Dual-task architecture for robust Actor behavior)
    tokio::select! {
        result = bot.run() => {
            if let Err(e) = result {
                eprintln!("❌ Telegram bot error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("🛑 Shutting down...");
        }
    }

    Ok(())
}
