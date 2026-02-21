use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::events::AgentEvent;

/// Session types for multi-agent orchestration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionType {
    Main,
    Isolated,
}

/// Cleanup policy for isolated sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CleanupPolicy {
    Keep,
    Delete,
}

/// Agent session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    pub session_type: SessionType,
    pub parent_session_id: Option<String>,
    pub cleanup_policy: CleanupPolicy,
    pub created_at: u64,
    #[serde(default)]
    pub initial_prompt: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Task delegation for subagents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTask {
    pub id: String,
    pub session_id: String,
    pub task: String,
    pub status: TaskStatus,
    pub result: Option<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(default)]
    pub retry_backoff_ms: u64,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Retrying,
    Paused,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentOptions {
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub timeout_seconds: u64,
}

impl Default for SubagentOptions {
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_backoff_ms: 1000,
            timeout_seconds: 120,
        }
    }
}

/// Multi-agent session manager
#[derive(Clone)]
pub struct AgentManager {
    sessions: Arc<Mutex<HashMap<String, AgentSession>>>,
    tasks: Arc<Mutex<HashMap<String, SessionTask>>>,
    /// Subagent hierarchy registry (parent_id -> Vec<child_id>)
    hierarchy: Arc<Mutex<HashMap<String, Vec<String>>>>,
    cancelled_sessions: Arc<Mutex<HashSet<String>>>,
    paused_sessions: Arc<Mutex<HashSet<String>>>,
    event_tx: Arc<Mutex<Option<mpsc::Sender<AgentEvent>>>>,
}

impl AgentManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            hierarchy: Arc::new(Mutex::new(HashMap::new())),
            cancelled_sessions: Arc::new(Mutex::new(HashSet::new())),
            paused_sessions: Arc::new(Mutex::new(HashSet::new())),
            event_tx: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_event_sender(&self, sender: mpsc::Sender<AgentEvent>) {
        let mut tx = self.event_tx.lock().await;
        *tx = Some(sender);
    }

    pub async fn broadcast_to_session(&self, session_id: &str, text: String) -> Result<()> {
        let tx = {
            let guard = self.event_tx.lock().await;
            guard.clone()
        };

        if let Some(sender) = tx {
            sender
                .send(AgentEvent::SessionMessage {
                    session_id: session_id.to_string(),
                    text,
                })
                .await
                .map_err(|e| anyhow::anyhow!("Failed to send session message: {}", e))?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Agent event channel not configured"))
        }
    }

    pub async fn broadcast_to_parent(&self, child_session_id: &str, text: String) -> Result<()> {
        let parent_id = {
            let sessions = self.sessions.lock().await;
            sessions
                .get(child_session_id)
                .and_then(|s| s.parent_session_id.clone())
        };

        if let Some(parent_session_id) = parent_id {
            self.broadcast_to_session(&parent_session_id, text).await
        } else {
            Err(anyhow::anyhow!(
                "Parent session not found for {}",
                child_session_id
            ))
        }
    }

    /// Start background cleanup task
    pub fn start_cleanup_task(&self) {
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await; // Run every 5 minutes
                if let Err(e) = manager.cleanup_sessions().await {
                    tracing::warn!("Error in agent cleanup task: {}", e);
                }
            }
        });
    }

    /// Save registry to disk
    async fn save_registry(&self) -> Result<()> {
        let registry = AgentRegistry {
            sessions: self.sessions.lock().await.clone(),
            tasks: self.tasks.lock().await.clone(),
        };

        // Resolve path
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
        let path = std::path::Path::new(&home)
            .join(".nanobot")
            .join("agents.json");

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let json = serde_json::to_string_pretty(&registry)?;
        tokio::fs::write(path, json).await?;

        Ok(())
    }

    /// Load registry from disk
    pub async fn load_registry(&self) -> Result<()> {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
        let path = std::path::Path::new(&home)
            .join(".nanobot")
            .join("agents.json");

        if path.exists() {
            let json = tokio::fs::read_to_string(path).await?;
            let registry: AgentRegistry = serde_json::from_str(&json)?;

            let mut sessions = self.sessions.lock().await;
            *sessions = registry.sessions;

            let mut tasks = self.tasks.lock().await;
            *tasks = registry.tasks;

            // Rebuild hierarchy from restored sessions
            let mut hierarchy_map: HashMap<String, Vec<String>> = HashMap::new();
            for session in sessions.values() {
                if let Some(parent) = &session.parent_session_id {
                    hierarchy_map
                        .entry(parent.clone())
                        .or_default()
                        .push(session.id.clone());
                }
            }
            let mut hierarchy = self.hierarchy.lock().await;
            *hierarchy = hierarchy_map;
        }
        Ok(())
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        session_type: SessionType,
        parent_session_id: Option<String>,
        cleanup_policy: CleanupPolicy,
        initial_prompt: Option<String>,
        model: Option<String>,
    ) -> Result<AgentSession> {
        let session = AgentSession {
            id: Uuid::new_v4().to_string(),
            session_type,
            parent_session_id,
            cleanup_policy,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            initial_prompt,
            model,
        };

        {
            let mut sessions = self.sessions.lock().await;
            sessions.insert(session.id.clone(), session.clone());
        }
        self.save_registry().await?;

        Ok(session)
    }

    /// Spawn an isolated subagent session
    pub async fn spawn_subagent(
        &self,
        parent_session_id: String,
        task: String,
        _label: Option<String>,
        cleanup_policy: CleanupPolicy,
        model: Option<String>,
    ) -> Result<(AgentSession, SessionTask)> {
        self.spawn_subagent_with_options(
            parent_session_id,
            task,
            _label,
            cleanup_policy,
            model,
            SubagentOptions::default(),
        )
        .await
    }

    pub async fn spawn_subagent_with_options(
        &self,
        parent_session_id: String,
        task: String,
        _label: Option<String>,
        cleanup_policy: CleanupPolicy,
        model: Option<String>,
        options: SubagentOptions,
    ) -> Result<(AgentSession, SessionTask)> {
        // Create isolated session
        let session = self
            .create_session(
                SessionType::Isolated,
                Some(parent_session_id.clone()),
                cleanup_policy,
                Some(task.clone()),
                model,
            )
            .await?;

        // Create task
        let task_obj = SessionTask {
            id: Uuid::new_v4().to_string(),
            session_id: session.id.clone(),
            task,
            status: TaskStatus::Pending,
            result: None,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            completed_at: None,
            attempts: 0,
            max_retries: options.max_retries,
            retry_backoff_ms: options.retry_backoff_ms,
            timeout_seconds: options.timeout_seconds,
            last_error: None,
        };

        {
            let mut tasks = self.tasks.lock().await;
            tasks.insert(task_obj.id.clone(), task_obj.clone());
        }

        // Track hierarchy relationship
        {
            let mut hierarchy = self.hierarchy.lock().await;
            hierarchy
                .entry(parent_session_id.clone())
                .or_insert_with(Vec::new)
                .push(session.id.clone());
        }

        self.save_registry().await?;

        self.spawn_task_execution(session.clone(), task_obj.clone());

        Ok((session, task_obj))
    }

    fn spawn_task_execution(&self, session: AgentSession, task: SessionTask) {
        let manager = self.clone();
        tokio::spawn(async move {
            let _ = manager
                .broadcast_to_parent(
                    &session.id,
                    format!("[Subagent {}] Thinking...", session.id),
                )
                .await;

            let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
            let progress_manager = manager.clone();
            let progress_session_id = session.id.clone();
            let progress_handle = tokio::spawn(async move {
                let mut buffer = String::new();
                let mut last_flush = Instant::now();

                while let Some(chunk) = progress_rx.recv().await {
                    buffer.push_str(&chunk);
                    if buffer.len() >= 256 || last_flush.elapsed() >= Duration::from_secs(2) {
                        let snippet = crate::cron::isolated_agent::truncate_utf8(&buffer, 512);
                        if !snippet.is_empty() {
                            let _ = progress_manager
                                .broadcast_to_parent(
                                    &progress_session_id,
                                    format!(
                                        "[Subagent {}] Progress: {}",
                                        progress_session_id, snippet
                                    ),
                                )
                                .await;
                        }
                        buffer.clear();
                        last_flush = Instant::now();
                    }
                }

                if !buffer.is_empty() {
                    let snippet = crate::cron::isolated_agent::truncate_utf8(&buffer, 512);
                    let _ = progress_manager
                        .broadcast_to_parent(
                            &progress_session_id,
                            format!("[Subagent {}] Progress: {}", progress_session_id, snippet),
                        )
                        .await;
                }
            });

            let max_attempts = task.max_retries + 1;
            let timeout = Duration::from_secs(task.timeout_seconds.max(1));
            let mut attempt: u32 = 0;

            loop {
                attempt += 1;

                let cancelled = {
                    let cancelled = manager.cancelled_sessions.lock().await;
                    cancelled.contains(&session.id)
                };
                if cancelled {
                    let _ = manager
                        .update_task_status(
                            &task.id,
                            TaskStatus::Cancelled,
                            Some("Cancelled by parent".to_string()),
                        )
                        .await;
                    break;
                }

                // Cooperative pause gate (between attempts)
                loop {
                    let paused = {
                        let paused = manager.paused_sessions.lock().await;
                        paused.contains(&session.id)
                    };
                    if !paused {
                        break;
                    }

                    let _ = manager
                        .update_task_status_attempt(
                            &task.id,
                            TaskStatus::Paused,
                            attempt.saturating_sub(1),
                            None,
                            None,
                        )
                        .await;

                    let cancelled_while_paused = {
                        let cancelled = manager.cancelled_sessions.lock().await;
                        cancelled.contains(&session.id)
                    };
                    if cancelled_while_paused {
                        let _ = manager
                            .update_task_status(
                                &task.id,
                                TaskStatus::Cancelled,
                                Some("Cancelled by parent".to_string()),
                            )
                            .await;
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(250)).await;
                }

                let cancelled = {
                    let cancelled = manager.cancelled_sessions.lock().await;
                    cancelled.contains(&session.id)
                };
                if cancelled {
                    break;
                }

                let status = if attempt == 1 {
                    TaskStatus::Running
                } else {
                    TaskStatus::Retrying
                };
                let _ = manager
                    .update_task_status_attempt(&task.id, status, attempt, None, None)
                    .await;

                let exec_progress_tx = progress_tx.clone();
                let result = tokio::time::timeout(
                    timeout,
                    crate::cron::isolated_agent::execute_agent_message(
                        &session.id,
                        &task.task,
                        session.model.clone(),
                        Some(exec_progress_tx),
                    ),
                )
                .await;

                match result {
                    Ok(Ok(output)) => {
                        let summary = crate::cron::isolated_agent::truncate_utf8(&output, 2000);
                        let _ = manager
                            .update_task_status_attempt(
                                &task.id,
                                TaskStatus::Completed,
                                attempt,
                                Some(output),
                                None,
                            )
                            .await;
                        let _ = manager
                            .broadcast_to_parent(
                                &session.id,
                                format!("[Subagent {}] Completed\n{}", session.id, summary),
                            )
                            .await;
                        break;
                    }
                    Ok(Err(e)) => {
                        let err_text = e.to_string();
                        if attempt < max_attempts {
                            let _ = manager
                                .broadcast_to_parent(
                                    &session.id,
                                    format!(
                                        "[Subagent {}] Attempt {}/{} failed: {}. Retrying...",
                                        session.id, attempt, max_attempts, err_text
                                    ),
                                )
                                .await;
                            tokio::time::sleep(Duration::from_millis(task.retry_backoff_ms)).await;
                            continue;
                        }
                        let _ = manager
                            .update_task_status_attempt(
                                &task.id,
                                TaskStatus::Failed,
                                attempt,
                                Some(err_text.clone()),
                                Some(err_text.clone()),
                            )
                            .await;
                        let _ = manager
                            .broadcast_to_parent(
                                &session.id,
                                format!("[Subagent {}] Failed: {}", session.id, err_text),
                            )
                            .await;
                        break;
                    }
                    Err(_) => {
                        let err_text = format!("Timed out after {}s", timeout.as_secs());
                        if attempt < max_attempts {
                            let _ = manager
                                .broadcast_to_parent(
                                    &session.id,
                                    format!(
                                        "[Subagent {}] Attempt {}/{} timed out. Retrying...",
                                        session.id, attempt, max_attempts
                                    ),
                                )
                                .await;
                            tokio::time::sleep(Duration::from_millis(task.retry_backoff_ms)).await;
                            continue;
                        }
                        let _ = manager
                            .update_task_status_attempt(
                                &task.id,
                                TaskStatus::TimedOut,
                                attempt,
                                Some(err_text.clone()),
                                Some(err_text.clone()),
                            )
                            .await;
                        let _ = manager
                            .broadcast_to_parent(
                                &session.id,
                                format!("[Subagent {}] Timed out: {}", session.id, err_text),
                            )
                            .await;
                        break;
                    }
                }
            }

            drop(progress_tx);
            let _ = progress_handle.await;
        });
    }

    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<AgentSession> {
        let sessions = self.sessions.lock().await;
        sessions.values().cloned().collect()
    }

    /// Get session by ID
    pub async fn get_session(&self, session_id: &str) -> Option<AgentSession> {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).cloned()
    }

    /// Update task status
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        result: Option<String>,
    ) -> Result<()> {
        self.update_task_status_attempt(task_id, status, 0, result, None)
            .await
    }

    pub async fn update_task_status_attempt(
        &self,
        task_id: &str,
        status: TaskStatus,
        attempts: u32,
        result: Option<String>,
        last_error: Option<String>,
    ) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = status;
            if attempts > 0 {
                task.attempts = attempts;
            }
            if let Some(r) = result {
                task.result = Some(r);
            }
            if let Some(err) = last_error {
                task.last_error = Some(err);
            }
            if matches!(
                task.status,
                TaskStatus::Completed
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
                    | TaskStatus::TimedOut
            ) {
                task.completed_at = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                );
            }
            // Drop lock before saving
            drop(tasks);
            self.save_registry().await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found: {}", task_id))
        }
    }

    pub async fn cancel_session(&self, session_id: &str) -> Result<()> {
        {
            let mut cancelled = self.cancelled_sessions.lock().await;
            cancelled.insert(session_id.to_string());
        }

        let task_id = {
            let tasks = self.tasks.lock().await;
            tasks
                .values()
                .find(|t| t.session_id == session_id)
                .map(|t| t.id.clone())
        };

        if let Some(task_id) = task_id {
            self.update_task_status_attempt(
                &task_id,
                TaskStatus::Cancelled,
                0,
                Some("Cancelled by parent".to_string()),
                Some("Cancelled by parent".to_string()),
            )
            .await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("No task found for session {}", session_id))
        }
    }

    pub async fn pause_session(&self, session_id: &str) -> Result<()> {
        {
            let mut paused = self.paused_sessions.lock().await;
            paused.insert(session_id.to_string());
        }

        let task_id = {
            let tasks = self.tasks.lock().await;
            tasks
                .values()
                .find(|t| t.session_id == session_id)
                .map(|t| t.id.clone())
        };

        if let Some(task_id) = task_id {
            self.update_task_status_attempt(&task_id, TaskStatus::Paused, 0, None, None)
                .await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("No task found for session {}", session_id))
        }
    }

    pub async fn resume_session(&self, session_id: &str) -> Result<()> {
        {
            let mut paused = self.paused_sessions.lock().await;
            paused.remove(session_id);
        }

        let task_id = {
            let tasks = self.tasks.lock().await;
            tasks
                .values()
                .find(|t| t.session_id == session_id)
                .map(|t| t.id.clone())
        };

        if let Some(task_id) = task_id {
            self.update_task_status_attempt(&task_id, TaskStatus::Retrying, 0, None, None)
                .await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("No task found for session {}", session_id))
        }
    }

    /// Get all child session IDs for a parent
    pub async fn get_children(&self, parent_session_id: &str) -> Vec<String> {
        let hierarchy = self.hierarchy.lock().await;
        hierarchy
            .get(parent_session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get task by session ID  
    pub async fn get_task_by_session(&self, session_id: &str) -> Option<SessionTask> {
        let tasks = self.tasks.lock().await;
        tasks.values().find(|t| t.session_id == session_id).cloned()
    }

    pub async fn wait_for_task(&self, session_id: &str, timeout: Duration) -> Result<SessionTask> {
        let start = Instant::now();
        loop {
            if start.elapsed() > timeout {
                return Err(anyhow::anyhow!(
                    "Timeout waiting for session {}",
                    session_id
                ));
            }

            if let Some(task) = self.get_task_by_session(session_id).await {
                if matches!(
                    task.status,
                    TaskStatus::Completed
                        | TaskStatus::Failed
                        | TaskStatus::Cancelled
                        | TaskStatus::TimedOut
                ) {
                    return Ok(task);
                }
            } else {
                return Err(anyhow::anyhow!("No task found for session {}", session_id));
            }

            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Get all tasks for a session
    pub async fn get_session_tasks(&self, session_id: &str) -> Vec<SessionTask> {
        let tasks = self.tasks.lock().await;
        tasks
            .values()
            .filter(|t| t.session_id == session_id)
            .cloned()
            .collect()
    }

    pub async fn recover_sessions(&self) -> Result<usize> {
        let sessions = self.sessions.lock().await.clone();
        let tasks = self.tasks.lock().await.clone();

        let mut recovered = 0;

        for task in tasks.values() {
            if matches!(task.status, TaskStatus::Running | TaskStatus::Retrying)
                && let Some(session) = sessions.get(&task.session_id)
            {
                self.spawn_task_execution(session.clone(), task.clone());
                recovered += 1;
            }
        }

        Ok(recovered)
    }

    /// Cleanup completed isolated sessions based on policy
    pub async fn cleanup_sessions(&self) -> Result<usize> {
        let mut sessions = self.sessions.lock().await;
        let tasks = self.tasks.lock().await;

        let mut cleaned = 0;

        // Find sessions to cleanup
        let session_ids: Vec<String> = sessions
            .values()
            .filter(|s| {
                matches!(s.session_type, SessionType::Isolated)
                    && matches!(s.cleanup_policy, CleanupPolicy::Delete)
            })
            .map(|s| s.id.clone())
            .collect();

        for session_id in session_ids {
            // Check if all tasks for this session are completed
            let all_completed = tasks
                .values()
                .filter(|t| t.session_id == session_id)
                .all(|t| {
                    matches!(
                        t.status,
                        TaskStatus::Completed
                            | TaskStatus::Failed
                            | TaskStatus::Cancelled
                            | TaskStatus::TimedOut
                    )
                });

            if all_completed {
                sessions.remove(&session_id);
                cleaned += 1;
            }
        }

        if cleaned > 0 {
            // Drop locks before saving
            drop(sessions);
            drop(tasks);
            self.save_registry().await?;
        }

        Ok(cleaned)
    }
}

#[derive(Serialize, Deserialize)]
struct AgentRegistry {
    sessions: HashMap<String, AgentSession>,
    tasks: HashMap<String, SessionTask>,
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}
