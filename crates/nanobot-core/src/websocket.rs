// Stub for websocket module
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct WebSocketServer {
    gateway: crate::gateway::Gateway,
}

impl Default for WebSocketServer {
    fn default() -> Self {
        let (tx, _rx) = mpsc::channel(1);
        let confirmation_service = Arc::new(tokio::sync::Mutex::new(
            crate::tools::ConfirmationService::new(),
        ));
        Self::new(3000, tx, confirmation_service)
    }
}

impl WebSocketServer {
    pub fn new(
        port: u16,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    ) -> Self {
        let config = crate::gateway::GatewayConfig { port };
        let gateway = crate::gateway::Gateway::new(config, agent_tx, confirmation_service);
        Self { gateway }
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        self.gateway.start().await
    }
}
