// Stub for sessions module

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub created_at: u64,
    pub last_active_at: u64,
}

pub struct SessionManager {
    sessions: RwLock<HashMap<String, SessionInfo>>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    pub async fn create_session(&self) -> SessionInfo {
        let id = uuid::Uuid::new_v4().to_string();
        let now = current_timestamp();
        let info = SessionInfo {
            id: id.clone(),
            created_at: now,
            last_active_at: now,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(id.clone(), info.clone());
        info
    }

    pub async fn get_session(&self, session_id: &str) -> Option<SessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    pub async fn touch_session(&self, session_id: &str) -> Option<SessionInfo> {
        let mut sessions = self.sessions.write().await;
        if let Some(mut info) = sessions.get(session_id).cloned() {
            info.last_active_at = current_timestamp();
            sessions.insert(session_id.to_string(), info.clone());
            Some(info)
        } else {
            None
        }
    }

    pub async fn remove_session(&self, session_id: &str) -> Option<SessionInfo> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id)
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
