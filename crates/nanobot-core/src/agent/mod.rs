use crate::antigravity::AntigravityClient;
use crate::context::ContextTree;
use crate::events::AgentEvent;
use anyhow::Result;
use futures::StreamExt;
use rig::OneOrMany;
use rig::client::CompletionClient;
use rig::completion::message::{AssistantContent, Text, UserContent};
use rig::completion::{CompletionModel, CompletionRequest, Message, Document};
use rig::streaming::StreamedAssistantContent;
use serde_json::json;
use tokio::sync::mpsc;

pub mod personality;
pub mod supervisor;
use std::path::PathBuf;

// Define message types for internal communication
#[derive(Debug)]
pub enum StreamChunk {
    TextDelta(String),
    Thinking(String),
    ToolCall(String),
    ToolResult(String),
    Done,
}

#[derive(Debug)]
pub struct AgentMessage {
    pub session_id: String,
    pub tenant_id: String, // Added for Multi-tenancy
    pub content: String,
    pub response_tx: mpsc::Sender<StreamChunk>,
}

use crate::config;
// AntigravityClient kept for initialization
use futures::stream::Stream;
use rig::providers::openai;
use std::pin::Pin;

pub enum AgentProvider {
    Antigravity(crate::antigravity::AntigravityCompletionModel),
    Google(crate::google::GoogleCompletionModel),
    OpenAI(openai::CompletionModel),
    Meta(crate::llm::meta_provider::MetaCompletionModel),
}

impl AgentProvider {
    pub async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<
                        Item = Result<StreamedAssistantContent<String>, rig::completion::CompletionError>,
                    > + Send,
            >,
        >,
        rig::completion::CompletionError,
    > {
        match self {
            AgentProvider::Antigravity(m) => {
                let stream = m.stream(request).await?;
                // Map AntigravityStreamingResponse to String
                let mapped = stream.map(|res| {
                    res.map(|content| {
                        match content {
                            StreamedAssistantContent::Text(t) => StreamedAssistantContent::Text(t),
                            StreamedAssistantContent::ToolCall(t) => {
                                StreamedAssistantContent::ToolCall(t)
                            }
                            // Map Final(R) to Final(String)
                            StreamedAssistantContent::Final(f) => {
                                StreamedAssistantContent::Final(f.content)
                            }
                            // Fallback for others to avoid complex imports
                            _ => StreamedAssistantContent::text(""),
                        }
                    })
                });
                Ok(Box::pin(mapped))
            }
            AgentProvider::Google(m) => {
                 let stream = m.stream(request).await?;
                 let mapped = stream.map(|res| {
                     res.map(|content| {
                         match content {
                             StreamedAssistantContent::Text(t) => StreamedAssistantContent::Text(t),
                             StreamedAssistantContent::Final(f) => StreamedAssistantContent::Final(f.content),
                             _ => StreamedAssistantContent::text(""),
                         }
                     })
                 });
                 Ok(Box::pin(mapped))
            }
            AgentProvider::OpenAI(m) => {
                let stream = m.stream(request).await?;
                // Map OpenAI response (R) to String (empty or debug)
                let mapped = stream.map(|res| {
                    res.map(|content| {
                        match content {
                            StreamedAssistantContent::Text(t) => StreamedAssistantContent::Text(t),
                            StreamedAssistantContent::ToolCall(t) => {
                                StreamedAssistantContent::ToolCall(t)
                            }
                            StreamedAssistantContent::Final(_) => {
                                StreamedAssistantContent::Final(String::new())
                            } // Discard Final object
                            _ => StreamedAssistantContent::text(""),
                        }
                    })
                });
                Ok(Box::pin(mapped))
            }
            AgentProvider::Meta(_m) => {
                Err(rig::completion::CompletionError::ProviderError(
                    "MetaProvider streaming not yet implemented".to_string()
                ))
            }
        }
    }
}

/// Simple heuristic token estimator (char/4)
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

fn estimate_message_tokens(msg: &Message) -> usize {
    match msg {
        Message::User { content } => {
            content.iter().map(|c| match c {
                UserContent::Text(t) => estimate_tokens(&t.text),
                _ => 0, // Ignore images for simple estimation
            }).sum()
        }
        Message::Assistant { content, .. } => {
            content.iter().map(|c| match c {
                AssistantContent::Text(t) => estimate_tokens(&t.text),
                _ => 0,
            }).sum()
        }
    }
}

fn prune_tool_outputs(chat_history: &mut Vec<Message>, max_chars: usize) {
    let head = max_chars / 2;
    let tail = max_chars.saturating_sub(head);

    for msg in chat_history.iter_mut() {
        if let Message::User { content } = msg {
            for part in content.iter_mut() {
                if let UserContent::Text(text) = part {
                    if text.text.starts_with("Tool '") && text.text.contains("Output:") {
                        if text.text.len() > max_chars {
                            let head_part = &text.text[..head.min(text.text.len())];
                            let tail_part = &text.text[text.text.len().saturating_sub(tail)..];
                            text.text = format!(
                                "{}\n... [tool output truncated] ...\n{}",
                                head_part,
                                tail_part
                            );
                        }
                    }
                }
            }
        }
    }
}

fn is_tool_output_msg(msg: &Message) -> bool {
    match msg {
        Message::User { content } => content.iter().any(|part| match part {
            UserContent::Text(text) => {
                text.text.starts_with("Tool '") && text.text.contains("Output:")
            }
            _ => false,
        }),
        _ => false,
    }
}

pub struct AgentLoop {
    provider: std::sync::Arc<tokio::sync::RwLock<AgentProvider>>,
    config: config::Config,
    key_indices: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, usize>>>,
    context_tree: std::sync::Arc<ContextTree>,
    skill_loader: Option<std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>>,
    cron_scheduler: crate::cron::CronScheduler,
    agent_manager: std::sync::Arc<crate::gateway::agent_manager::AgentManager>,
    memory_manager: std::sync::Arc<crate::memory::MemoryManager>,
    #[allow(dead_code)]
    mcp_manager: Option<std::sync::Arc<crate::mcp::McpManager>>,
    #[allow(dead_code)]
    workspace_watcher: Option<crate::memory::WorkspaceWatcher>,
    personality: Option<personality::PersonalityContext>,
    system_prompt_override: Option<String>,
    agent_event_rx: Option<tokio::sync::mpsc::Receiver<AgentEvent>>,
    last_interaction:
        std::sync::Arc<tokio::sync::Mutex<Option<(String, mpsc::Sender<StreamChunk>)>>>,
    session_senders: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, mpsc::Sender<StreamChunk>>>>,
    permission_manager: std::sync::Arc<tokio::sync::Mutex<crate::tools::PermissionManager>>,
    tool_policy: std::sync::Arc<crate::tools::ToolPolicy>,
    confirmation_service: std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    resource_monitor: std::sync::Arc<crate::system::resources::ResourceMonitor>,
    persistence: std::sync::Arc<crate::persistence::PersistenceManager>,
    #[cfg(feature = "browser")]
    browser_client: Option<crate::browser::BrowserClient>,
    active_tasks: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>>,
}

impl AgentLoop {
    /// Create a provider instance based on config and key indices
    pub async fn create_provider(
        config: &config::Config, 
        indices: &std::collections::HashMap<String, usize>
    ) -> Result<AgentProvider> {
        // Priority 1: Check for LLM failover config
        if let Some(llm_config) = config.llm.clone() {
            tracing::info!("Using MetaProvider with failover chain: {:?}", llm_config.failover_chain);
            
            let meta_client = crate::llm::meta_provider::MetaClient::new(llm_config.clone()).await?;
            let model_name = "gemini-2.0-flash-001".to_string();
            
            return Ok(AgentProvider::Meta(
                crate::llm::meta_provider::MetaCompletionModel::make(&meta_client, model_name)
            ));
        }

        // Priority 2: Fall back to traditional default_provider
        let default = config.default_provider.as_str();
        let index = *indices.get(default).unwrap_or(&0);
        
        match default {
            "antigravity" => {
                let _ag_config = config.providers.antigravity.as_ref();
                
                // Resolution logic: 
                // Antigravity now strictly uses OAuth via TokenManager (handled inside client)
                // We REMOVE the unsafe env var injection for API keys here.
                
                let client = AntigravityClient::from_env().await?;
                Ok(AgentProvider::Antigravity(client.completion_model("gemini-2.0-flash-exp")))
            }
            "google" => {
                let google_config = config.providers.google.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Google provider not configured"))?;
                
                let api_key = if let Some(keys) = &google_config.api_keys {
                     if !keys.is_empty() {
                         keys[index % keys.len()].clone()
                     } else {
                         google_config.api_key.clone().unwrap_or_default()
                     }
                } else {
                    google_config.api_key.clone().unwrap_or_default()
                };

                if api_key.is_empty() {
                    return Err(anyhow::anyhow!("Google API key missing. Configure 'api_key' or 'api_keys' in [providers.google]"));
                }
                
                let client = reqwest::Client::new();
                let mut model = crate::google::GoogleCompletionModel::make(&client, "gemini-2.0-flash");
                model.api_key = api_key;
                
                Ok(AgentProvider::Google(model))
            }
            "openrouter" => {
                let or_config = config.providers.openrouter.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("OpenRouter not configured"))?;
                
                let api_key = if let Some(keys) = &or_config.api_keys {
                     if !keys.is_empty() {
                         keys[index % keys.len()].clone()
                     } else {
                         or_config.api_key.clone().unwrap_or_default()
                     }
                } else {
                    or_config.api_key.clone().unwrap_or_default()
                };

                if api_key.is_empty() {
                    return Err(anyhow::anyhow!("OpenRouter API key missing. Configure 'api_key' or 'api_keys' in config.toml"));
                }

                let client = openai::Client::new(&api_key)?;
                Ok(AgentProvider::OpenAI(
                    client.completions_api().completion_model("google/gemini-2.0-flash-001"),
                ))
            }
            "openai" | _ => {
                let oa_config = config.providers.openai.as_ref();
                let api_key = if let Some(c) = oa_config {
                    if let Some(keys) = &c.api_keys {
                        if !keys.is_empty() {
                            Some(keys[index % keys.len()].clone())
                        } else {
                            c.api_key.clone()
                        }
                    } else {
                        c.api_key.clone()
                    }
                } else {
                    None
                }
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .ok_or_else(|| anyhow::anyhow!("OpenAI API Key missing"))?;

                let client = openai::Client::new(&api_key)?;
                Ok(AgentProvider::OpenAI(client.completions_api().completion_model("gpt-4o")))
            }
        }
    }

    /// Rotate the current provider's API key
    async fn rotate_provider(&self) -> Result<()> {
        let mut key_indices = self.key_indices.lock().await;
        // Determine current provider name from config
        let provider_name = if self.config.llm.is_some() {
            "meta".to_string()
        } else {
             self.config.default_provider.clone()
        };
        
        // Increment index
        let entry = key_indices.entry(provider_name).or_insert(0);
        *entry += 1;
        tracing::info!("Rotating auth key for provider '{}' (index: {})", self.config.default_provider,entry);
        
        // Re-create provider
        let new_provider = Self::create_provider(&self.config, &key_indices).await?;
        
        let mut provider_guard = self.provider.write().await;
        *provider_guard = new_provider;
        
        Ok(())
    }

    pub async fn new() -> Result<Self> {
        let config = config::Config::load()?;

        #[cfg(not(feature = "browser"))]
        if config.browser.is_some() {
            tracing::warn!(
                "Browser config present but binary built without 'browser' feature"
            );
        }
        
        // Initial provider creation
        let indices_map = std::collections::HashMap::new();
        let provider = Self::create_provider(&config, &indices_map).await?;
        let provider = std::sync::Arc::new(tokio::sync::RwLock::new(provider));
        let key_indices = std::sync::Arc::new(tokio::sync::Mutex::new(indices_map));

        let db_path = PathBuf::from(".").join(".nanobot").join("context_tree.db");

        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let context_tree = std::sync::Arc::new(ContextTree::new(
            db_path.to_str().ok_or_else(|| anyhow::anyhow!("Invalid db path"))?
        )?);

        // Create channel for agent events (cron + subagent updates)
        let (agent_event_tx, agent_event_rx) = tokio::sync::mpsc::channel(100);

        // Initialize Cron Scheduler
        let cron_scheduler =
            crate::cron::CronScheduler::new(db_path.clone(), agent_event_tx.clone()).await?;
        cron_scheduler.start().await?;

        // Initialize Agent Manager
        let agent_manager = std::sync::Arc::new(crate::gateway::agent_manager::AgentManager::new());
        agent_manager.load_registry().await?; // Restore persistent state
        agent_manager.set_event_sender(agent_event_tx.clone()).await;
        let recovered = agent_manager.recover_sessions().await?;
        if recovered > 0 {
            tracing::info!("Recovered {} running subagent session(s)", recovered);
        }
        agent_manager.start_cleanup_task();

        // Initialize Memory Manager
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
        let memory_db_path = PathBuf::from(&home)
            .join(".nanobot")
            .join("memory")
            .join("index.json");

        // Create provider for memory (OpenAI or Local)
        let mem_provider = if let Ok(openai_key) = std::env::var("OPENAI_API_KEY") {
            crate::memory::EmbeddingProvider::openai(openai_key)
        } else {
            crate::memory::EmbeddingProvider::local()?
        };

        let memory_manager = std::sync::Arc::new(crate::memory::MemoryManager::new(
            memory_db_path,
            mem_provider,
        ));
        let _ = memory_manager.load_index(); // Best effort load

        // Initialize Workspace Watcher
        let workspace_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".flowbot")
            .join("workspace");

        // Check local override or use default
        let watch_path = if PathBuf::from(".").join("Cargo.toml").exists() {
            PathBuf::from(".")
                .canonicalize()
                .unwrap_or(PathBuf::from("."))
        } else {
            workspace_dir.clone()
        };

        let workspace_watcher = match crate::memory::WorkspaceWatcher::new(
            watch_path.clone(),
            memory_manager.clone(),
            Some("system".to_string()),
        ) {
            Ok(w) => {
                println!("👀 File watcher active on {:?}", watch_path);
                Some(w)
            }
            Err(e) => {
                eprintln!("⚠️ Failed to start file watcher: {}", e);
                None
            }
        };

        // Initialize Skills Loader
        let skills_path = workspace_dir.join("skills");
        let skill_loader = if skills_path.exists() {
            let mut loader = crate::skills::SkillLoader::new(skills_path.clone());
            match loader.scan() {
                Ok(_) => {
                    let skill_count = loader.skills().len();
                    if skill_count > 0 {
                        eprintln!("📦 Loaded {} skills from {:?}", skill_count, skills_path);
                    }
                    Some(std::sync::Arc::new(tokio::sync::Mutex::new(loader)))
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to load skills: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let personality = if workspace_dir.exists() {
            match personality::PersonalityContext::load(&workspace_dir).await {
                Ok(p) => {
                    eprintln!(
                        "✨ Loaded personality: {} {}",
                        p.agent_emoji(),
                        p.agent_name()
                    );
                    Some(p)
                }
                Err(e) => {
                    eprintln!("⚠️  WARNING: Could not load personality: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize Permission Manager with workspace scope
        let workspace_root = std::env::current_dir()?;
        let security_profile = crate::tools::permissions::SecurityProfile::standard(workspace_root);
        let permission_manager = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::tools::PermissionManager::new(security_profile)
        ));

        let tool_policy = std::sync::Arc::new(crate::tools::ToolPolicy::permissive());

        let confirmation_service = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));

        {
            let mut service = confirmation_service.lock().await;
            service.register_adapter(Box::new(crate::tools::cli_confirmation::CliConfirmationAdapter::new()));
        }

        // Initialize Resource Monitor
        let resource_monitor = std::sync::Arc::new(crate::system::resources::ResourceMonitor::new());
        resource_monitor.start_monitoring().await;

        // Initialize MCP Manager
        let mcp_manager = if let Some(mcp_config) = config.mcp.as_ref() {
            if mcp_config.enabled && !mcp_config.servers.is_empty() {
                let manager = std::sync::Arc::new(crate::mcp::McpManager::new());
                
                for server_config in &mcp_config.servers {
                    match manager.add_server(server_config.clone()).await {
                        Ok(_) => {},
                        Err(e) => {
                            eprintln!("⚠️  Failed to connect to MCP server '{}': {}", server_config.name, e);
                        }
                    }
                }
                
                let tool_count = manager.tool_count().await;
                if tool_count > 0 {
                    eprintln!("🔌 MCP: Loaded {} tools from {} servers", tool_count, manager.server_count().await);
                }
                
                // Start health check loop
                manager.start_health_check();
                
                Some(manager)
            } else {
                None
            }
        } else {
            None
        };

        // Initialize Browser Client
        #[cfg(feature = "browser")]
        let browser_client = if let Some(browser_config) = config.browser {
            Some(crate::browser::BrowserClient::new(browser_config))
        } else {
            None
        };

        let persistence_db_path = db_path.clone();
        let persistence = std::sync::Arc::new(crate::persistence::PersistenceManager::new(
            persistence_db_path,
        ));
        persistence.init()?;

        Ok(Self {
            provider,
            config,
            key_indices,
            context_tree,
            skill_loader,
            cron_scheduler,
            agent_manager,
            memory_manager,
            mcp_manager,
            workspace_watcher,
            personality,
            system_prompt_override: None,
            agent_event_rx: Some(agent_event_rx),
            last_interaction: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            session_senders: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            permission_manager,
            tool_policy,
            confirmation_service,
            resource_monitor,
            persistence,
            #[cfg(feature = "browser")]
            browser_client,
            active_tasks: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        })
    }

    pub fn confirmation_service(
        &self,
    ) -> std::sync::Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>> {
        self.confirmation_service.clone()
    }

    pub fn set_system_prompt_override(&mut self, prompt: Option<String>) {
        self.system_prompt_override = prompt;
    }

    pub fn set_tool_policy(&mut self, policy: crate::tools::ToolPolicy) {
        self.tool_policy = std::sync::Arc::new(policy);
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<AgentMessage>) {
        // Take ownership of agent_event_rx
        let agent_event_rx = self.agent_event_rx.take();

        // Wrap self in Arc to share with the cron handler task
        let agent = std::sync::Arc::new(self);

        // Spawn agent event handler task
        if let Some(mut event_rx) = agent_event_rx {
            let agent_inner = agent.clone();
            tokio::spawn(async move {
                println!("🕐 Agent event handler started");
                while let Some(event) = event_rx.recv().await {
                    match event {
                        AgentEvent::SystemEvent { job_id, text } => {
                            let source = job_id.clone().unwrap_or_else(|| "unknown".to_string());
                            println!("📅 [AgentEvent] SystemEvent from {}: {}", source, text);

                            // Try to inject into last active session
                            let interaction = {
                                let last = agent_inner.last_interaction.lock().await;
                                last.clone()
                            };

                            if let Some((session_id, response_tx)) = interaction {
                                println!("💉 Injecting SystemEvent into session {}", session_id);
                                let msg = AgentMessage {
                                    session_id,
                                    tenant_id: "default".to_string(), // Scheduler runs as system/default
                                    content: format!("(System Event) {}", text),
                                    response_tx,
                                };
                                agent_inner.process_streaming(msg).await;
                            } else {
                                println!(
                                    "⚠️ No active interaction found for SystemEvent injection."
                                );
                            }
                        }
                        AgentEvent::AgentTurn {
                            job_id,
                            message,
                            model,
                            ..
                        } => {
                            let source = job_id.clone().unwrap_or_else(|| "unknown".to_string());
                            println!(
                                "📅 [AgentEvent] AgentTurn from {}: {} (model: {:?})",
                                source, message, model
                            );

                            // Spawn task to execute isolated agent
                            let agent_mgr = agent_inner.agent_manager.clone();
                            let last_int = agent_inner.last_interaction.clone();
                            let job_id_clone = job_id.clone().unwrap_or_else(|| "unknown".to_string());
                            let message_clone = message.clone();

                            tokio::spawn(async move {
                                println!(
                                    "🔄 [Cron] Executing isolated agent for job {}...",
                                    job_id_clone
                                );

                                // Create a minimal CronJob for isolated execution
                                // In a full implementation, we'd fetch the job from DB to get isolation config
                                // For now, we use a simple inline job
                                let job = crate::cron::CronJob {
                                    id: job_id_clone.clone(),
                                    name: Some(format!("Agent Turn: {}", job_id_clone)),
                                    enabled: true,
                                    schedule: crate::cron::Schedule::At { at_ms: 0 },
                                    payload: crate::cron::Payload::AgentTurn {
                                        message: message_clone.clone(),
                                        model: model.clone(),
                                        thinking: None,
                                        timeout_seconds: Some(120),
                                    },
                                    session_target: crate::cron::SessionTarget::Main,
                                    wake_mode: crate::cron::WakeMode::default(),
                                    isolation: None, // TODO: Fetch from DB to support post-to-main
                                    delete_after_run: false,
                                    created_at: 0,
                                };

                                // Execute isolated agent turn
                                match crate::cron::isolated_agent::run_isolated_agent_turn(
                                    &job,
                                    &agent_mgr,
                                    message_clone.clone(),
                                )
                                .await
                                {
                                    Ok(result) => {
                                        println!(
                                            "✅ [Cron] {} Isolated agent completed: {:?}",
                                            job_id_clone, result.status
                                        );

                                        // Post-to-main feedback
                                        if let Some(output) = result.output_text {
                                            let feedback_msg =
                                                format!("[Cron Job: {}]: {}", job_id_clone, output);

                                            // Try to inject into last active session
                                            let interaction = {
                                                let last = last_int.lock().await;
                                                last.clone()
                                            };

                                            if let Some((session_id, response_tx)) = interaction {
                                                println!(
                                                    "💉 [Cron] Injecting result into session {}",
                                                    session_id
                                                );
                                                let _ = response_tx
                                                    .send(crate::agent::StreamChunk::TextDelta(
                                                        format!("\n\n{}\n\n", feedback_msg),
                                                    ))
                                                    .await;
                                            } else {
                                                println!(
                                                    "⚠️ [Cron] No active session for feedback injection"
                                                );
                                            }
                                        }

                                        if let Some(error) = result.error {
                                            eprintln!("❌ [Cron] Agent turn error: {}", error);
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "❌ [Cron] Failed to execute isolated agent: {}",
                                            e
                                        );
                                    }
                                }
                            });
                        }
                        AgentEvent::SessionMessage { session_id, text } => {
                            let tx = {
                                let senders = agent_inner.session_senders.lock().await;
                                senders.get(&session_id).cloned()
                            };

                            if let Some(response_tx) = tx {
                                let _ = response_tx
                                    .send(crate::agent::StreamChunk::TextDelta(format!(
                                        "\n\n{}\n\n",
                                        text
                                    )))
                                    .await;
                                if let Err(e) = agent_inner.save_message(&session_id, "assistant", &text) {
                                    eprintln!("Failed to persist injected message: {}", e);
                                }
                                if let Err(e) = agent_inner.persistence.save_message(&session_id, "assistant", &text) {
                                    eprintln!("Failed to persist injected message to history: {}", e);
                                }
                            } else {
                                println!(
                                    "⚠️ No active session sender found for SessionMessage injection."
                                );
                            }
                        }
                    }
                }
                println!("🕐 Agent event handler stopped");
            });
        }

        // Main agent message loop
        while let Some(msg) = rx.recv().await {
            let session_id = msg.session_id.clone();
            let agent_clone = agent.clone();

            // Cancel existing task for this session if any regarding interruptibility
            {
                let mut tasks = agent.active_tasks.lock().await;
                if let Some(handle) = tasks.remove(&session_id) {
                    tracing::info!("⚠️ Interrupting active task for session {}", session_id);
                    handle.abort();
                }
            }

            // Spawn new task
            let session_id_clone = session_id.clone();
            let task = tokio::spawn(async move {
                let session_id = session_id_clone;
                // Update last interaction tracking
                {
                    let mut last = agent_clone.last_interaction.lock().await;
                    *last = Some((msg.session_id.clone(), msg.response_tx.clone()));
                }

                // Track active session sender for targeted injections
                {
                    let mut senders = agent_clone.session_senders.lock().await;
                    senders.insert(msg.session_id.clone(), msg.response_tx.clone());
                }

                agent_clone.process_streaming(msg).await;

                // Cleanup task from map upon completion
                let mut tasks = agent_clone.active_tasks.lock().await;
                tasks.remove(&session_id);

                let mut senders = agent_clone.session_senders.lock().await;
                senders.remove(&session_id);
            });

            // Store handle
            let mut tasks = agent.active_tasks.lock().await;
            tasks.insert(session_id, task.abort_handle());
        }
    }

    // Agent turn loop - Process one message with streaming
    #[tracing::instrument(skip(self, msg), fields(session_id = %msg.session_id))]
    async fn process_streaming(&self, msg: AgentMessage) {
        // Get adaptive configuration based on current resources
        let adaptive_config = self.resource_monitor.get_adaptive_config();
        let resource_level = self.resource_monitor.get_resource_level();
        
        // Warn user if resources are constrained
        if resource_level != crate::system::resources::ResourceLevel::High {
            let level_str = match resource_level {
                crate::system::resources::ResourceLevel::Low => "LOW (Throttled)",
                crate::system::resources::ResourceLevel::Medium => "MEDIUM (Limited)",
                crate::system::resources::ResourceLevel::High => unreachable!(),
            };
            let _ = msg.response_tx.send(StreamChunk::TextDelta(
                format!("⚠️ Resource Mode: {} | Context: {} msgs | RAG: {} docs | Tokens: {}\n\n",
                    level_str,
                    adaptive_config.context_history_limit,
                    adaptive_config.rag_doc_count,
                    adaptive_config.max_tokens
                )
            )).await;
        }

        if let Err(e) = self.save_message(&msg.session_id, "user", &msg.content) {
            eprintln!("Failed to save user message: {}", e);
        }

        if let Err(e) = self.persistence.save_message(&msg.session_id, "user", &msg.content) {
            eprintln!("Failed to persist user message: {}", e);
        }

        let mut chat_history = self.get_conversation_history(&msg.session_id);

        // Apply adaptive context history limit
        // Apply adaptive context history limit (Count-based)
        if chat_history.len() > adaptive_config.context_history_limit {
            let skip = chat_history.len() - adaptive_config.context_history_limit;
            chat_history = chat_history.into_iter().skip(skip).collect();
        }

        prune_tool_outputs(&mut chat_history, 4000);

        // Apply Token Limit (Hybrid) - Heuristic pruning + summary fallback
        let token_limit = self.config.context_token_limit;
        let total_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();

        if total_tokens > token_limit {
            tracing::warn!(
                "⚠️ Context exceeding token limit (~{} > {}). Pruning...",
                total_tokens,
                token_limit
            );

            let large_threshold = std::cmp::max(token_limit / 4, 800);
            let mut items: Vec<(Message, usize, bool, bool)> = chat_history
                .into_iter()
                .map(|msg| {
                    let tokens = estimate_message_tokens(&msg);
                    let is_tool = is_tool_output_msg(&msg);
                    let is_large = tokens > large_threshold;
                    (msg, tokens, is_tool, is_large)
                })
                .collect();

            let mut total = total_tokens;
            let mut dropped: Vec<Message> = Vec::new();
            let mut kept: Vec<(Message, usize)> = Vec::new();

            for (msg, tokens, is_tool, is_large) in items.drain(..) {
                if total > token_limit && (is_tool || is_large) {
                    total = total.saturating_sub(tokens);
                    dropped.push(msg);
                } else {
                    kept.push((msg, tokens));
                }
            }

            if total > token_limit {
                let mut remaining: Vec<Message> = Vec::new();
                for (msg, tokens) in kept.into_iter() {
                    if total > token_limit {
                        total = total.saturating_sub(tokens);
                        dropped.push(msg);
                    } else {
                        remaining.push(msg);
                    }
                }
                chat_history = remaining;
            } else {
                chat_history = kept.into_iter().map(|(msg, _)| msg).collect();
            }

            if !dropped.is_empty() {
                match self.summarize_messages(&dropped).await {
                    Ok(summary) => {
                        chat_history.insert(
                            0,
                            Message::User {
                                content: OneOrMany::one(UserContent::Text(Text {
                                    text: format!("(Context Summary)\n{}", summary),
                                })),
                            },
                        );
                        tracing::info!("✅ Compressed {} messages into summary", dropped.len());
                    }
                    Err(e) => {
                        tracing::error!("Failed to summarize: {}. Falling back to pruning only.", e);
                    }
                }
            }
        }

        chat_history.push(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: msg.content.clone(),
            })),
        });

        let max_loops = 5;
        let mut loop_count = 0;

        loop {
            loop_count += 1;
            if loop_count > max_loops {
                let _ = msg
                    .response_tx
                    .send(StreamChunk::TextDelta(
                        "\n[System: Max tool loops reached]".to_string(),
                    ))
                    .await;
                break;
            }

            let system_msg = if let Some(ref override_prompt) = self.system_prompt_override {
                format!(
                    "{}\n\n# Available Tools\n{}",
                    override_prompt,
                    crate::tools::executor::get_tool_descriptions()
                )
            } else if let Some(ref personality) = self.personality {
                format!(
                    "{}\n\n# Available Tools\n{}",
                    personality.to_preamble(),
                    crate::tools::executor::get_tool_descriptions()
                )
            } else {
                format!(
                    "You are Flowbot, a helpful AI assistant.\n\n# Available Tools\n{}",
                    crate::tools::executor::get_tool_descriptions()
                )
            };

            // Auto-RAG: Retrieve relevant context from Memory (with adaptive limit)
            let context_docs = match self.memory_manager.search(&msg.content, adaptive_config.rag_doc_count, Some(&msg.tenant_id)).await {
                Ok(results) => {
                    if !results.is_empty() {
                         tracing::info!("📚 RAG: Found {} relevant documents", results.len());
                    }
                    results.into_iter().map(|(_score, entry)| {
                        Document {
                            id: entry.id,
                            text: entry.content,
                            additional_props: entry.metadata,
                        }
                    }).collect()
                }
                Err(e) => {
                    tracing::error!("RAG Search failed: {}", e);
                    vec![]
                }
            };

            let request = CompletionRequest {
                chat_history: OneOrMany::many(chat_history.clone())
                    .expect("Chat history should convert to OneOrMany::Many"),
                preamble: Some(system_msg),
                max_tokens: Some(adaptive_config.max_tokens),  // ADAPTIVE
                temperature: Some(0.7),
                tools: vec![],
                tool_choice: None,
                documents: context_docs,
                additional_params: Some(json!({})),
            };

            // Auth Rotation / Retry Loop
            let mut retry_count = 0;
            let max_retries = 3;
            
            let stream = loop {
                let provider_guard = self.provider.read().await;
                match provider_guard.stream(request.clone()).await {
                    Ok(s) => break Ok(s),
                    Err(e) => {
                         drop(provider_guard); // Drop read lock to allow rotation
                         let err_str = e.to_string();
                         // Check for 429 (Too Many Requests) or Quota errors
                         if (err_str.contains("429") || err_str.contains("Quota") || err_str.contains("Rate limit")) && retry_count < max_retries {
                             retry_count += 1;
                             tracing::warn!("⚠️ Provider rate limit hit: {}. Rotating key (attempt {}/{})", err_str, retry_count, max_retries);
                             if let Err(rot_err) = self.rotate_provider().await {
                                 tracing::error!("Failed to rotate provider: {}", rot_err);
                                 // Don't break immediately, maybe next retry works? 
                                 // But if rotation failed, likely no more keys.
                             }
                             continue;
                         }
                         break Err(e);
                    }
                }
            };

            let mut stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = msg
                        .response_tx
                        .send(StreamChunk::TextDelta(format!("Error: {}", e)))
                        .await;
                    break;
                }
            };

            let mut tool_calls = Vec::new();
            let mut current_text = String::new();
            let assistant_message_id = match self.persistence.start_message(&msg.session_id, "assistant") {
                Ok(id) => Some(id),
                Err(e) => {
                    eprintln!("Failed to start assistant persistence: {}", e);
                    None
                }
            };

            let mut thinking = false;

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(chunk) => {
                        match chunk {
                            StreamedAssistantContent::Text(text) => {
                                let content = text.text.clone();
                                current_text.push_str(&content);

                                if let Some(message_id) = assistant_message_id {
                                    let _ = self.persistence.append_message_content(message_id, &content);
                                }
                                
                                // Simple parser for <think> blocks
                                // Note: detailed split-tag handling omitted for brevity, assumes tags arrive mostly intact
                                let mut remaining = content.as_str();
                                
                                while !remaining.is_empty() {
                                    if !thinking {
                                        if let Some(start_idx) = remaining.find("<think>") {
                                            if start_idx > 0 {
                                                let pre = &remaining[0..start_idx];
                                                let _ = msg.response_tx.send(StreamChunk::TextDelta(pre.to_string())).await;
                                            }
                                            thinking = true;
                                            remaining = &remaining[start_idx + 7..];
                                        } else {
                                            // No start tag, normal text
                                            let _ = msg.response_tx.send(StreamChunk::TextDelta(remaining.to_string())).await;
                                            break;
                                        }
                                    } else {
                                        if let Some(end_idx) = remaining.find("</think>") {
                                            let think_content = &remaining[0..end_idx];
                                            let _ = msg.response_tx.send(StreamChunk::Thinking(think_content.to_string())).await;
                                            thinking = false;
                                            remaining = &remaining[end_idx + 8..];
                                        } else {
                                            // No end tag, all thinking
                                            let _ = msg.response_tx.send(StreamChunk::Thinking(remaining.to_string())).await;
                                            break;
                                        }
                                    }
                                }
                            }
                            StreamedAssistantContent::ToolCall(tool) => {
                                // Accumulate tool calls
                                tool_calls.push(tool);
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        eprintln!("Stream error: {}", e);
                    }
                }
            }

            // Save assistant response including tool calls
            if !current_text.is_empty() {
                let _ = self.save_message(&msg.session_id, "assistant", &current_text);
                chat_history.push(Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(Text {
                        text: current_text.clone(),
                    })),
                });
            }

            if tool_calls.is_empty() {
                let _ = msg.response_tx.send(StreamChunk::Done).await;
                break;
            }

            // Execute Tools
            for tool_call in tool_calls {
                let _ = msg
                    .response_tx
                    .send(StreamChunk::ToolCall(tool_call.function.name.clone()))
                    .await;

                // Parse args to check valid JSON, but we need to merge with tool name for executor
                let mut args_value = tool_call.function.arguments.clone();

                // Add "tool" field to args for legacy executor compatibility
                if let Some(obj) = args_value.as_object_mut() {
                    obj.insert("tool".to_string(), json!(tool_call.function.name));
                }

                let tool_input_str = serde_json::to_string(&args_value).unwrap_or_default();

                let result_str = match tool_call.function.name.as_str() {
                    "cron" => {
                        match crate::tools::cron::execute_cron_tool(
                            &self.cron_scheduler,
                            &args_value,
                        )
                        .await
                        {
                            Ok(res) => res,
                            Err(e) => format!("Error: {}", e),
                        }
                    }
                    _ => {
                        // Fallback to general executor
                        match crate::tools::executor::execute_tool(
                            &tool_input_str,
                            Some(&self.cron_scheduler),
                            Some(&self.agent_manager),
                            Some(&self.memory_manager),
                            Some(&*self.permission_manager),
                            Some(&self.tool_policy),
                            Some(&*self.confirmation_service),
                            self.skill_loader.as_ref(),
                            #[cfg(feature = "browser")]
                            self.browser_client.as_ref(),
                            Some(&msg.tenant_id),
                            self.mcp_manager.as_ref(), // Pass MCP manager
                        )
                        .await
                        {
                            Ok(res) => res,
                            Err(e) => format!("Error: {}", e),
                        }
                    }
                };

                let _ = msg
                    .response_tx
                    .send(StreamChunk::ToolResult(result_str.clone()))
                    .await;

                // Add tool result to history for next loop
                // Note: Rig implementation specific, usually need specific ToolResult message
                // For now, we append user message with Tool Result as a simple pattern if Rig types are restrictive,
                // or use proper ToolResult content if available.
                // Let's use User message with "Tool Output" for broad compatibility.
                chat_history.push(Message::User {
                    content: OneOrMany::one(UserContent::Text(Text {
                        text: format!("Tool '{}' Output: {}", tool_call.function.name, result_str),
                    })),
                });
            }
            // Loop automatically continues with updated history
        }
    }

    fn save_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        self.context_tree.with_transaction(|tx| {
            let parent_id = crate::context::tree::ContextTree::get_active_leaf_tx(tx, session_id)?;
            crate::context::tree::ContextTree::add_message_in_tx(
                tx,
                session_id,
                role,
                content,
                parent_id,
                None,
            )?;
            crate::persistence::PersistenceManager::save_message_tx(
                tx,
                session_id,
                role,
                content,
            )?;
            Ok(())
        })
        ?;

        const MAX_NODES: usize = 2000;
        const KEEP_RECENT: usize = 1500;
        if let Ok(count) = self.context_tree.count_session_nodes(session_id) {
            if count > MAX_NODES {
                if let Ok(removed) = self.context_tree.prune_session(session_id, MAX_NODES, KEEP_RECENT) {
                    tracing::info!(
                        "Context prune removed {} nodes for session {}",
                        removed,
                        session_id
                    );
                }
            }
        }

        Ok(())
    }

    // Helper to summarize a list of messages
    async fn summarize_messages(&self, msgs: &[Message]) -> Result<String> {
        let text_content: String = msgs.iter().map(|m| {
            match m {
                Message::User { content } => {
                    content.iter().map(|c| match c {
                        UserContent::Text(t) => format!("User: {}\n", t.text),
                        _ => "User: [Media]\n".to_string(),
                    }).collect::<String>()
                }
                Message::Assistant { content, .. } => {
                     content.iter().map(|c| match c {
                        AssistantContent::Text(t) => format!("Assistant: {}\n", t.text),
                        _ => "Assistant: [Media]\n".to_string(),
                    }).collect::<String>()
                }
            }
        }).collect();

        if text_content.is_empty() {
            return Ok("No history.".to_string());
        }

        let prompt = format!(
            "Summarize the following conversation history, retaining key facts, user preferences, and important context, while removing conversational filler:\n\n{}", 
            text_content
        );

        let request = CompletionRequest {
            chat_history: OneOrMany::one(Message::User { content: OneOrMany::one(UserContent::text(prompt)) }),
            preamble: Some("You are a helpful summarizer.".to_string()),
            max_tokens: Some(500),
            temperature: Some(0.3),
            tools: vec![],
            tool_choice: None,
            documents: vec![],
            additional_params: None,
        };

        // We use the same provider
        let provider_guard = self.provider.read().await;
        let stream = provider_guard.stream(request).await?;
        
        let mut summary = String::new();
        let mut s = stream;
        while let Some(chunk_res) = s.next().await {
            if let Ok(chunk) = chunk_res {
                if let StreamedAssistantContent::Text(t) = chunk {
                    summary.push_str(&t.text);
                }
            }
        }
        
        Ok(summary)
    }

    fn get_conversation_history(&self, session_id: &str) -> Vec<Message> {
        // Get active leaf and reconstruct trace
        match self.context_tree.get_active_leaf(session_id) {
            Ok(Some(leaf_id)) => {
                // Get trace from root to leaf
                match self.context_tree.get_trace(&leaf_id) {
                    Ok(nodes) => {
                        // Convert ContextNode to rig::Message
                        nodes.iter().map(|node| {
                            match node.role.as_str() {
                                "user" => Message::User {
                                    content: OneOrMany::one(UserContent::Text(Text {
                                        text: node.content.clone(),
                                    })),
                                },
                                "assistant" => Message::Assistant {
                                    id: None,
                                    content: OneOrMany::one(AssistantContent::Text(Text {
                                        text: node.content.clone(),
                                    })),
                                },
                                _ => Message::User {
                                    content: OneOrMany::one(UserContent::Text(Text {
                                        text: node.content.clone(),
                                    })),
                                },
                            }
                        }).collect()
                    }
                    Err(e) => {
                        eprintln!("Failed to load conversation trace: {}", e);
                        Vec::new()
                    }
                }
            }
            Ok(None) => {
                // No history for this session yet
                Vec::new()
            }
            Err(e) => {
                eprintln!("Failed to get active leaf: {}", e);
                Vec::new()
            }
        }
    }
}
