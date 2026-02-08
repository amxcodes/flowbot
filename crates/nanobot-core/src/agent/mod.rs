use crate::antigravity::AntigravityClient;
use crate::context::ContextTree;
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
    cron_event_rx: Option<tokio::sync::mpsc::Receiver<crate::cron::CronEvent>>,
    last_interaction:
        std::sync::Arc<tokio::sync::Mutex<Option<(String, mpsc::Sender<StreamChunk>)>>>,
    permission_manager: std::sync::Arc<tokio::sync::Mutex<crate::tools::PermissionManager>>,
    resource_monitor: std::sync::Arc<crate::system::resources::ResourceMonitor>,
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
                let ag_config = config.providers.antigravity.as_ref();
                
                // Resolution logic: 
                // 1. Try to get key from rotation list (api_keys) using index
                // 2. Fallback to single api_key
                // 3. Fallback to env var GOOGLE_API_KEY (implicit in client, but we set it here for rotation)
                
                let key_to_use = if let Some(c) = ag_config {
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
                };

                if let Some(k) = key_to_use {
                     unsafe { std::env::set_var("GOOGLE_API_KEY", k); }
                }
                
                let client = AntigravityClient::from_env().await?;
                Ok(AgentProvider::Antigravity(client.completion_model("gemini-2.0-flash-exp")))
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

        // Create channel for cron events
        let (cron_event_tx, cron_event_rx) = tokio::sync::mpsc::channel(100);

        // Initialize Cron Scheduler
        let cron_scheduler =
            crate::cron::CronScheduler::new(db_path.clone(), cron_event_tx).await?;
        cron_scheduler.start().await?;

        // Initialize Agent Manager
        let agent_manager = std::sync::Arc::new(crate::gateway::agent_manager::AgentManager::new());
        agent_manager.load_registry().await?; // Restore persistent state
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
            None, // Default tenant
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
            cron_event_rx: Some(cron_event_rx),
            last_interaction: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            permission_manager,
            resource_monitor,
            #[cfg(feature = "browser")]
            browser_client,
            active_tasks: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        })
    }

    pub async fn run(mut self, mut rx: mpsc::Receiver<AgentMessage>) {
        // Take ownership of cron_event_rx
        let cron_event_rx = self.cron_event_rx.take();

        // Wrap self in Arc to share with the cron handler task
        let agent = std::sync::Arc::new(self);

        // Spawn cron event handler task
        if let Some(mut event_rx) = cron_event_rx {
            let agent_inner = agent.clone();
            tokio::spawn(async move {
                println!("🕐 Cron event handler started");
                while let Some(event) = event_rx.recv().await {
                    match event {
                        crate::cron::CronEvent::SystemEvent { job_id, text } => {
                            println!("📅 [Cron] SystemEvent from job {}: {}", job_id, text);

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
                        crate::cron::CronEvent::AgentTurn {
                            job_id,
                            message,
                            model,
                            ..
                        } => {
                            println!(
                                "📅 [Cron] AgentTurn from job {}: {} (model: {:?})",
                                job_id, message, model
                            );

                            // Spawn task to execute isolated agent
                            let agent_mgr = agent_inner.agent_manager.clone();
                            let last_int = agent_inner.last_interaction.clone();
                            let job_id_clone = job_id.clone();
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
                    }
                }
                println!("🕐 Cron event handler stopped");
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

                agent_clone.process_streaming(msg).await;

                // Cleanup task from map upon completion
                let mut tasks = agent_clone.active_tasks.lock().await;
                tasks.remove(&session_id);
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

        let mut chat_history = self.get_conversation_history(&msg.session_id);

        // Apply adaptive context history limit
        // Apply adaptive context history limit (Count-based)
        if chat_history.len() > adaptive_config.context_history_limit {
            let skip = chat_history.len() - adaptive_config.context_history_limit;
            chat_history = chat_history.into_iter().skip(skip).collect();
        }

        // Apply Token Limit (Heuristic) - Hard cap at 32k tokens to prevent overflow
        // Logic: Drop oldest messages until we fit
        let total_tokens: usize = chat_history.iter().map(estimate_message_tokens).sum();
        let token_limit = self.config.context_token_limit;
        
        if total_tokens > token_limit {
             tracing::warn!("⚠️ Context exceeding token limit (~{} > {}). Summarizing...", total_tokens, token_limit);
             
             // Strategy: Summarize the first 50% of history into a single system message
             let split_idx = chat_history.len() / 2;
             if split_idx > 0 {
                 let older_msgs = chat_history.drain(0..split_idx).collect::<Vec<_>>();
                 
                 // Create temporary provider for summarization (avoid deadlock by cloning provider beforehand if needed, 
                 // but here we can just use the read lock or a separate lightweight request)
                 // For simplicity in this step, we'll try to just perform a direct summarization if possible, 
                 // or just execute a truncation with a system note if summarization is too expensive inline.
                 //
                 // Better: Spawn a summarization task? No, we need it now. 
                 // Let's do a meaningful truncation -> "Summary: [Old context removed]" 
                 // But user asked for *Summarization*.
                 
                 // We will effectively collapse them into a single User message saying:
                 // "Here is a summary of the previous conversation: ..."
                 // To do this properly requires an LLM call. 
                 
                 // FOR NOW: We will implement the PLUMBING for it. 
                 // 1. Convert older_msgs to string
                 // 2. Call internal summarize (we need to implement `summarize` method on AgentLoop)
                 
                 match self.summarize_messages(&older_msgs).await {
                     Ok(summary) => {
                         // Insert summary as a new User message at the start
                         chat_history.insert(0, Message::User { 
                             content: OneOrMany::one(UserContent::Text(Text { text: format!("(Prior Verification Summary)\n{}", summary) })) 
                         });
                         tracing::info!("✅ Compressed {} messages into summary", older_msgs.len());
                     }
                     Err(e) => {
                         tracing::error!("Failed to summarize: {}. Falling back to truncation.", e);
                         // Fallback is already done by drain, just didn't insert summary.
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

            let system_msg = if let Some(ref personality) = self.personality {
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

            let mut thinking = false;

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(chunk) => {
                        match chunk {
                            StreamedAssistantContent::Text(text) => {
                                let content = text.text.clone();
                                current_text.push_str(&content);
                                
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
                            self.skill_loader.as_ref(),
                            #[cfg(feature = "browser")]
                            self.browser_client.as_ref(),
                            Some(&msg.tenant_id),
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
        // Get current active leaf as parent
        let parent_id = self.context_tree.get_active_leaf(session_id)?;
        
        // Add message to tree
        self.context_tree.add_message(
            session_id,
            role,
            content,
            parent_id,
            None, // model can be added later if needed
        )?;
        
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
