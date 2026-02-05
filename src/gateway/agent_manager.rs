use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// Multi-agent session manager
#[derive(Clone)]
pub struct AgentManager {
    sessions: Arc<Mutex<HashMap<String, AgentSession>>>,
    tasks: Arc<Mutex<HashMap<String, SessionTask>>>,
    /// Subagent hierarchy registry (parent_id -> Vec<child_id>)
    hierarchy: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl AgentManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            hierarchy: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start background cleanup task
    pub fn start_cleanup_task(&self) {
        let manager = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await; // Run every 5 minutes
                if let Err(e) = manager.cleanup_sessions().await {
                    eprintln!("Error in agent cleanup task: {}", e);
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
        let path = std::path::Path::new(&home).join(".nanobot").join("agents.json");
        
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
        let path = std::path::Path::new(&home).join(".nanobot").join("agents.json");
        
        if path.exists() {
            let json = tokio::fs::read_to_string(path).await?;
            let registry: AgentRegistry = serde_json::from_str(&json)?;
            
            let mut sessions = self.sessions.lock().await;
            *sessions = registry.sessions;
            
            let mut tasks = self.tasks.lock().await;
            *tasks = registry.tasks;
        }
        Ok(())
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        session_type: SessionType,
        parent_session_id: Option<String>,
        cleanup_policy: CleanupPolicy,
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
    ) -> Result<(AgentSession, SessionTask)> {
        // Create isolated session
        let session = self
            .create_session(
                SessionType::Isolated,
                Some(parent_session_id.clone()),
                cleanup_policy,
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

        Ok((session, task_obj))
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
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = status;
            task.result = result;
            if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
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

    /// Get all child session IDs for a parent
    pub async fn get_children(&self, parent_session_id: &str) -> Vec<String> {
        let hierarchy = self.hierarchy.lock().await;
        hierarchy.get(parent_session_id).cloned().unwrap_or_default()
    }
    
    /// Get task by session ID  
    pub async fn get_task_by_session(&self, session_id: &str) -> Option<SessionTask> {
        let tasks = self.tasks.lock().await;
        tasks.values()
            .find(|t| t.session_id == session_id)
            .cloned()
    }
    
    /// Get all tasks for a session
    pub async fn get_session_tasks(&self, session_id: &str) -> Vec<SessionTask> {
        let tasks = self.tasks.lock().await;
        tasks.values()
            .filter(|t| t.session_id == session_id)
            .cloned()
            .collect()
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
                .all(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Failed));

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
