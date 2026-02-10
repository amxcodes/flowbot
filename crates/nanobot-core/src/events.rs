#[derive(Debug, Clone)]
pub enum AgentEvent {
    SystemEvent {
        job_id: Option<String>,
        text: String,
    },
    AgentTurn {
        job_id: Option<String>,
        message: String,
        model: Option<String>,
        thinking: Option<String>,
        timeout_seconds: Option<u64>,
    },
    SessionMessage {
        session_id: String,
        text: String,
    },
}
