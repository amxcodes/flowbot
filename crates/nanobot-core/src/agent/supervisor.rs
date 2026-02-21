use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Supervisor manages child agent lifecycle with configurable supervision policies
pub struct Supervisor {
    children: HashMap<AgentId, ChildHandle>,
    policy: SupervisionPolicy,
    next_agent_id: u64,
}

pub type AgentId = String;

/// Supervision strategies for handling child failures
#[derive(Debug, Clone, Copy)]
pub enum SupervisionPolicy {
    /// Restart only the failed child
    OneForOne,
    /// Restart all children if any one fails
    AllForOne,
}

/// Factory function that creates a new agent task
pub type AgentFactory =
    Arc<dyn Fn(mpsc::Receiver<super::AgentMessage>) -> JoinHandle<()> + Send + Sync>;

struct ChildHandle {
    task: JoinHandle<()>,
    sender: mpsc::Sender<super::AgentMessage>,
    config: ChildConfig,
    restart_count: usize,
    factory: Option<AgentFactory>,
}

#[derive(Debug, Clone)]
pub struct ChildConfig {
    pub session_prefix: String,
    pub max_loops: usize,
    pub max_restarts: usize,
}

impl Default for ChildConfig {
    fn default() -> Self {
        Self {
            session_prefix: "child".to_string(),
            max_loops: 5,
            max_restarts: 3, // Restart up to 3 times
        }
    }
}

impl Supervisor {
    pub fn new(policy: SupervisionPolicy) -> Self {
        Self {
            children: HashMap::new(),
            policy,
            next_agent_id: 1,
        }
    }

    /// Spawn a new child agent with a factory function for restarts
    pub async fn spawn_child_with_factory<F>(
        &mut self,
        config: ChildConfig,
        factory: F,
    ) -> Result<AgentId>
    where
        F: Fn(mpsc::Receiver<super::AgentMessage>) -> JoinHandle<()> + Send + Sync + 'static,
    {
        let agent_id = self.generate_agent_id();
        let (tx, rx) = mpsc::channel(100);
        let factory_arc: AgentFactory = Arc::new(factory);

        let task = (factory_arc.as_ref())(rx);

        let handle = ChildHandle {
            task,
            sender: tx,
            config,
            restart_count: 0,
            factory: Some(factory_arc),
        };

        self.children.insert(agent_id.clone(), handle);
        tracing::info!("Spawned child agent: {}", agent_id);

        Ok(agent_id)
    }

    /// Spawn a new child agent (legacy method without restart capability)
    pub async fn spawn_child(
        &mut self,
        config: ChildConfig,
        agent_fn: impl FnOnce(mpsc::Receiver<super::AgentMessage>) -> tokio::task::JoinHandle<()>
        + Send
        + 'static,
    ) -> Result<AgentId> {
        let agent_id = self.generate_agent_id();
        let (tx, rx) = mpsc::channel(100);

        let task = agent_fn(rx);

        let handle = ChildHandle {
            task,
            sender: tx,
            config,
            restart_count: 0,
            factory: None,
        };

        self.children.insert(agent_id.clone(), handle);
        tracing::info!("Spawned child agent: {}", agent_id);

        Ok(agent_id)
    }

    /// Kill a specific child agent
    pub async fn kill_child(&mut self, agent_id: &AgentId) -> Result<()> {
        if let Some(handle) = self.children.remove(agent_id) {
            handle.task.abort();
            tracing::info!("Killed child agent: {}", agent_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Agent not found: {}", agent_id))
        }
    }

    /// Send a message to a specific child agent
    pub async fn send_to_child(&self, agent_id: &AgentId, msg: super::AgentMessage) -> Result<()> {
        if let Some(handle) = self.children.get(agent_id) {
            handle.sender.send(msg).await?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Agent not found: {}", agent_id))
        }
    }

    /// Check for finished children and apply supervision policy
    pub async fn supervise(&mut self) {
        let mut finished = Vec::new();

        for (agent_id, handle) in &self.children {
            if handle.task.is_finished() {
                finished.push((
                    agent_id.clone(),
                    handle.config.clone(),
                    handle.restart_count,
                    handle.factory.clone(),
                ));
            }
        }

        if !finished.is_empty() {
            match self.policy {
                SupervisionPolicy::OneForOne => {
                    for (agent_id, config, restart_count, factory) in finished {
                        tracing::warn!(
                            "Child agent {} finished (restart count: {})",
                            agent_id,
                            restart_count
                        );
                        self.children.remove(&agent_id);

                        // Auto-restart if under limit AND factory is available
                        if restart_count < config.max_restarts {
                            if let Some(factory_fn) = factory {
                                tracing::info!(
                                    "Restarting child agent: {} (attempt {})",
                                    agent_id,
                                    restart_count + 1
                                );

                                // Create new channel for restarted child
                                let (tx, rx) = mpsc::channel(100);
                                let task = (factory_fn.as_ref())(rx);

                                let handle = ChildHandle {
                                    task,
                                    sender: tx,
                                    config: config.clone(),
                                    restart_count: restart_count + 1,
                                    factory: Some(factory_fn),
                                };

                                self.children.insert(agent_id.clone(), handle);
                                tracing::info!("Successfully restarted child agent: {}", agent_id);
                            } else {
                                tracing::warn!(
                                    "Cannot restart {}: no factory stored (use spawn_child_with_factory)",
                                    agent_id
                                );
                            }
                        } else {
                            tracing::warn!(
                                "Child agent {} exceeded max restarts ({}), not restarting",
                                agent_id,
                                config.max_restarts
                            );
                        }
                    }
                }
                SupervisionPolicy::AllForOne => {
                    if !finished.is_empty() {
                        tracing::warn!(
                            "Child failure detected, killing all children (AllForOne policy)"
                        );
                        self.kill_all().await;
                        // All-for-one restart intentionally disabled unless all child factories are registered.
                    }
                }
            }
        }
    }

    /// Kill all child agents
    pub async fn kill_all(&mut self) {
        for (_id, handle) in self.children.drain() {
            handle.task.abort();
        }
        tracing::info!("Killed all child agents");
    }

    /// Get count of active children
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// List all active child agent IDs
    pub fn list_children(&self) -> Vec<AgentId> {
        self.children.keys().cloned().collect()
    }

    fn generate_agent_id(&mut self) -> AgentId {
        let id = format!("agent-{}", self.next_agent_id);
        self.next_agent_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn test_spawn_child() {
        let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);

        let agent_id = supervisor
            .spawn_child(ChildConfig::default(), |mut rx| {
                tokio::spawn(async move {
                    while let Some(_msg) = rx.recv().await {
                        // Process messages
                    }
                })
            })
            .await
            .unwrap();

        assert_eq!(supervisor.child_count(), 1);
        assert!(supervisor.list_children().contains(&agent_id));
    }

    #[tokio::test]
    async fn test_kill_child() {
        let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);

        let agent_id = supervisor
            .spawn_child(ChildConfig::default(), |mut rx| {
                tokio::spawn(async move { while let Some(_msg) = rx.recv().await {} })
            })
            .await
            .unwrap();

        supervisor.kill_child(&agent_id).await.unwrap();
        assert_eq!(supervisor.child_count(), 0);
    }

    #[tokio::test]
    async fn test_kill_all() {
        let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);

        for _ in 0..3 {
            supervisor
                .spawn_child(ChildConfig::default(), |mut rx| {
                    tokio::spawn(async move { while let Some(_msg) = rx.recv().await {} })
                })
                .await
                .unwrap();
        }

        assert_eq!(supervisor.child_count(), 3);
        supervisor.kill_all().await;
        assert_eq!(supervisor.child_count(), 0);
    }

    #[tokio::test]
    async fn test_supervision_one_for_one() {
        let mut supervisor = Supervisor::new(SupervisionPolicy::OneForOne);

        // Spawn child that exits immediately
        supervisor
            .spawn_child(ChildConfig::default(), |_rx| {
                tokio::spawn(async move {
                    // Exit immediately
                })
            })
            .await
            .unwrap();

        // Spawn child that stays alive
        supervisor
            .spawn_child(ChildConfig::default(), |mut rx| {
                tokio::spawn(async move { while let Some(_msg) = rx.recv().await {} })
            })
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;
        supervisor.supervise().await;

        // OneForOne: only finished child removed, other stays
        assert_eq!(supervisor.child_count(), 1);
    }
}
