use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Routes messages to appropriate agent instances based on routing strategy
pub struct MessageRouter {
    strategy: RoutingStrategy,
    /// Track which agent is serving which session
    session_mapping: Arc<RwLock<HashMap<String, String>>>,
}

#[derive(Debug, Clone)]
pub enum RoutingStrategy {
    /// Each user gets the same agent (sticky sessions)
    StickySession { default_agent: String },
    /// Distribute load across multiple agents
    RoundRobin { agents: Vec<String>, next_idx: usize },
    /// All messages go to one agent
    Static { agent_id: String },
}

impl MessageRouter {
    pub fn new(strategy: RoutingStrategy) -> Self {
        Self {
            strategy,
            session_mapping: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn new_sticky(default_agent: String) -> Self {
        Self::new(RoutingStrategy::StickySession { default_agent })
    }

    pub fn new_static(agent_id: String) -> Self {
        Self::new(RoutingStrategy::Static { agent_id })
    }

    /// Determine which agent should handle this session
    pub async fn route_session(&mut self, session_key: &str) -> String {
        let mapping = self.session_mapping.read().await;
        
        // Check if we already have a mapping
        if let Some(agent_id) = mapping.get(session_key) {
            return agent_id.clone();
        }
        drop(mapping);

        // No existing mapping, create one based on strategy
        let agent_id = match &mut self.strategy {
            RoutingStrategy::StickySession { default_agent } => default_agent.clone(),
            RoutingStrategy::Static { agent_id } => agent_id.clone(),
            RoutingStrategy::RoundRobin { agents, next_idx } => {
                if agents.is_empty() {
                    "default".to_string()
                } else {
                    let selected = agents[*next_idx % agents.len()].clone();
                    *next_idx = (*next_idx + 1) % agents.len();
                    selected
                }
            }
        };

        // Store the mapping
        let mut mapping = self.session_mapping.write().await;
        mapping.insert(session_key.to_string(), agent_id.clone());

        agent_id
    }

    /// Remove a session mapping (e.g., on timeout or completion)
    pub async fn release_session(&self, session_key: &str) {
        let mut mapping = self.session_mapping.write().await;
        mapping.remove(session_key);
    }

    /// Get statistics about current routing state
    pub async fn stats(&self) -> RouterStats {
        let mapping = self.session_mapping.read().await;
        let active_sessions = mapping.len();
        
        let agents_in_use: std::collections::HashSet<_> = mapping.values().cloned().collect();
        
        RouterStats {
            active_sessions,
            agents_in_use: agents_in_use.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouterStats {
    pub active_sessions: usize,
    pub agents_in_use: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sticky_session_routing() {
        let mut router = MessageRouter::new_sticky("agent-1".to_string());
        
        let agent1 = router.route_session("session-1").await;
        let agent2 = router.route_session("session-1").await;
        
        assert_eq!(agent1, agent2); // Same session gets same agent
        assert_eq!(agent1, "agent-1");
    }

    #[tokio::test]
    async fn test_static_routing() {
        let mut router = MessageRouter::new_static("static-agent".to_string());
        
        let agent1 = router.route_session("session-1").await;
        let agent2 = router.route_session("session-2").await;
        
        assert_eq!(agent1, "static-agent");
        assert_eq!(agent2, "static-agent");
    }

    #[tokio::test]
    async fn test_release_session() {
        let mut router = MessageRouter::new_sticky("agent-1".to_string());
        
        router.route_session("session-1").await;
        router.release_session("session-1").await;
        
        let stats = router.stats().await;
        assert_eq!(stats.active_sessions, 0);
    }

    #[tokio::test]
    async fn test_router_stats() {
        let mut router = MessageRouter::new_sticky("agent-1".to_string());
        
        router.route_session("session-1").await;
        router.route_session("session-2").await;
        
        let stats = router.stats().await;
        assert_eq!(stats.active_sessions, 2);
        assert_eq!(stats.agents_in_use, 1);
    }
}
