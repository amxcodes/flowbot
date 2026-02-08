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
    ToolCall(String),
    ToolResult(String),
    Done,
}

#[derive(Debug)]
pub struct AgentMessage {
    pub session_id: String,
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

pub struct AgentLoop {
    provider: AgentProvider,
    context_tree: std::sync::Arc<ContextTree>,
    skill_loader: Option<std::sync::Arc<tokio::sync::Mutex<crate::skills::SkillLoader>>>,
    cron_scheduler: crate::cron::CronScheduler,
    agent_manager: std::sync::Arc<crate::gateway::agent_manager::AgentManager>,
    memory_manager: std::sync::Arc<crate::memory::MemoryManager>,
    mcp_manager: Option<std::sync::Arc<crate::mcp::McpManager>>,
    workspace_watcher: Option<crate::memory::WorkspaceWatcher>,
    personality: Option<personality::PersonalityContext>,
    cron_event_rx: Option<tokio::sync::mpsc::Receiver<crate::cron::CronEvent>>,
    last_interaction:
        std::sync::Arc<tokio::sync::Mutex<Option<(String, mpsc::Sender<StreamChunk>)>>>,
    permission_manager: std::sync::Arc<tokio::sync::Mutex<crate::tools::PermissionManager>>,
    resource_monitor: std::sync::Arc<crate::system::resources::ResourceMonitor>,
    #[cfg(feature = "browser")]
    browser_client: Option<crate::browser::BrowserClient>,
}

impl AgentLoop {
    pub async fn new() -> Result<Self> {
        let config = config::Config::load()?;
        
        // Priority 1: Check for LLM failover config
        let provider = if let Some(llm_config) = config.llm.clone() {
            tracing::info!("Using MetaProvider with failover chain: {:?}", llm_config.failover_chain);
            
            let meta_client = crate::llm::meta_provider::MetaClient::new(llm_config.clone()).await?;
            // Use a default model name - providers will use their configured models
            let model_name = "gemini-2.0-flash-001".to_string();
            
            AgentProvider::Meta(
                crate::llm::meta_provider::MetaCompletionModel::make(&meta_client, model_name)
            )
        } else {
            // Priority 2: Fall back to traditional default_provider
            let default = config.default_provider.as_str();
            
            match default {
                "antigravity" => {
                    let client = AntigravityClient::from_env().await?;
                    AgentProvider::Antigravity(client.completion_model("gemini-2.0-flash-exp"))
                }
                "openrouter" => {
                    let api_key = config
                        .providers
                        .openrouter
                        .as_ref()
                        .map(|c| c.api_key.clone())
                        .ok_or_else(|| anyhow::anyhow!("OpenRouter API key not configured"))?;

                    let client = openai::Client::new(&api_key)?;
                    // Use completions_api to return CompletionModel instead of ResponsesCompletionModel
                    AgentProvider::OpenAI(
                        client
                            .completions_api()
                            .completion_model("google/gemini-2.0-flash-001"),
                    )
                }
                "openai" | _ => {
                    let api_key = std::env::var("OPENAI_API_KEY")
                        .or_else(|_| {
                            config
                                    .providers
                                    .openai
                                    .as_ref()
                                    .map(|c| c.api_key.clone())
                                    .ok_or(String::new())
                        })
                        .map_err(|_| anyhow::anyhow!("OpenAI API Key missing"))?;

                    let client = openai::Client::new(&api_key)?;
                    // Use completions_api to return CompletionModel
                    AgentProvider::OpenAI(client.completions_api().completion_model("gpt-4o"))
                }
            }
        };

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
            // Update last interaction tracking
            {
                let mut last = agent.last_interaction.lock().await;
                *last = Some((msg.session_id.clone(), msg.response_tx.clone()));
            }

            agent.process_streaming(msg).await;
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
        if chat_history.len() > adaptive_config.context_history_limit {
            let skip = chat_history.len() - adaptive_config.context_history_limit;
            chat_history = chat_history.into_iter().skip(skip).collect();
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
            let context_docs = match self.memory_manager.search(&msg.content, adaptive_config.rag_doc_count).await {
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

            let mut stream = match self.provider.stream(request).await {
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

            while let Some(chunk_res) = stream.next().await {
                match chunk_res {
                    Ok(chunk) => {
                        match chunk {
                            StreamedAssistantContent::Text(text) => {
                                current_text.push_str(&text.text);
                                let _ = msg
                                    .response_tx
                                    .send(StreamChunk::TextDelta(text.text))
                                    .await;
                            }
                            StreamedAssistantContent::ToolCall(tool) => {
                                // Rig streams tool calls, sometimes incrementally or fully.
                                // Assuming full tool call for now based on Rig's default behavior for most providers
                                // or we accumulate if needed. Rig 0.0.6 usually gives fully formed tool calls in the stream event if using high-level
                                // but with generic stream it might be different. Let's assume we collect them.
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
