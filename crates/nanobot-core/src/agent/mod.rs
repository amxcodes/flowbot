use crate::antigravity::AntigravityClient;
use crate::persistence::PersistenceManager;
use anyhow::Result;
use futures::StreamExt;
use rig::OneOrMany;
use rig::client::CompletionClient;
use rig::completion::message::{AssistantContent, Text, UserContent};
use rig::completion::{CompletionModel, CompletionRequest, Message};
use rig::streaming::StreamedAssistantContent;
use serde_json::json;
use tokio::sync::mpsc;

pub mod personality;
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
}

impl AgentProvider {
    pub async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<
                        Item = Result<
                            StreamedAssistantContent<String>,
                            rig::completion::CompletionError,
                        >,
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
        }
    }
}

pub struct AgentLoop {
    provider: AgentProvider,
    persistence: PersistenceManager,
    cron_scheduler: crate::cron::CronScheduler,
    agent_manager: std::sync::Arc<crate::gateway::agent_manager::AgentManager>,
    memory_manager: std::sync::Arc<crate::memory::MemoryManager>,
    workspace_watcher: Option<crate::memory::WorkspaceWatcher>,
    personality: Option<personality::PersonalityContext>,
    cron_event_rx: Option<tokio::sync::mpsc::Receiver<crate::cron::CronEvent>>,
    last_interaction:
        std::sync::Arc<tokio::sync::Mutex<Option<(String, mpsc::Sender<StreamChunk>)>>>,
}

impl AgentLoop {
    pub async fn new() -> Result<Self> {
        let config = config::Config::load()?;
        let default = config.default_provider.as_str();

        let provider = match default {
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
        };

        let db_path = PathBuf::from(".").join(".nanobot").join("sessions.db");

        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let persistence = PersistenceManager::new(db_path.clone());
        persistence.init()?;

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

        Ok(Self {
            provider,
            persistence,
            cron_scheduler,
            agent_manager,
            memory_manager,
            workspace_watcher,
            personality,
            cron_event_rx: Some(cron_event_rx),
            last_interaction: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
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

    async fn process_streaming(&self, msg: AgentMessage) {
        if let Err(e) = self.save_message(&msg.session_id, "user", &msg.content) {
            eprintln!("Failed to save user message: {}", e);
        }

        let mut chat_history = match self.persistence.get_history(&msg.session_id) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to load history: {}", e);
                Vec::new()
            }
        };

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

            let request = CompletionRequest {
                chat_history: OneOrMany::many(chat_history.clone()).unwrap(),
                preamble: Some(system_msg),
                max_tokens: Some(4096),
                temperature: Some(0.7),
                tools: vec![],
                tool_choice: None,
                documents: vec![],
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
        self.persistence.save_message(session_id, role, content)
    }
}
