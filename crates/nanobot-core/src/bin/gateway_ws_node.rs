use nanobot_core::agent::{AgentMessage, StreamChunk, TerminalKind};
use nanobot_core::gateway::{Gateway, GatewayConfig};
use nanobot_core::persistence::PersistenceManager;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(port_raw) = args.next() else {
        return Err(anyhow::anyhow!("usage: gateway_ws_node <port>"));
    };
    let port: u16 = port_raw
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid port '{}': must be numeric", port_raw))?;
    let sleep_before_terminal_ms = std::env::var("NANOBOT_TEST_GATEWAY_SLEEP_BEFORE_TERMINAL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let persistence = std::env::var("NANOBOT_TEST_GATEWAY_DB_PATH")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|path| {
            let manager = PersistenceManager::new(PathBuf::from(path));
            manager.init()?;
            Ok::<_, anyhow::Error>(Arc::new(manager))
        })
        .transpose()?;

    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentMessage>(256);
    tokio::spawn(async move {
        while let Some(msg) = agent_rx.recv().await {
            if let Some(pm) = persistence.as_ref()
                && let Err(err) = pm.save_message_for_request(
                    &msg.session_id,
                    "user",
                    &msg.request_id,
                    &msg.content,
                )
            {
                tracing::warn!("gateway-ws-node failed to persist user message: {}", err);
            }

            if msg.content.contains("slow-test") {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            if sleep_before_terminal_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(sleep_before_terminal_ms)).await;
            }

            let can_emit_terminal = nanobot_core::distributed::terminal_dedupe_store()
                .try_mark_terminal(&msg.session_id, &msg.request_id)
                .await;
            if !can_emit_terminal {
                nanobot_core::metrics::GLOBAL_METRICS
                    .increment_counter("llm_terminal_duplicate_total", 1);
                continue;
            }

            let allowed = nanobot_core::distributed::allow_provider_request("openai").await;
            if allowed {
                if let Some(pm) = persistence.as_ref()
                    && let Err(err) = pm.save_message_for_request(
                        &msg.session_id,
                        "assistant",
                        &msg.request_id,
                        "gateway-ws-node-ok",
                    )
                {
                    tracing::warn!("gateway-ws-node failed to persist assistant message: {}", err);
                }
                let _ = msg
                    .response_tx
                    .send(StreamChunk::TextDelta("gateway-ws-node-ok".to_string()))
                    .await;
                let _ = msg
                    .response_tx
                    .send(StreamChunk::Done {
                        request_id: msg.request_id,
                        kind: TerminalKind::SuccessDone,
                    })
                    .await;
            } else {
                let _ = msg
                    .response_tx
                    .send(StreamChunk::Done {
                        request_id: msg.request_id,
                        kind: TerminalKind::ErrorDone {
                            code: "provider_rate_limited_global".to_string(),
                            reason: "global provider rate limit reached".to_string(),
                        },
                    })
                    .await;
            }
        }
    });

    let gateway = Gateway::new(
        GatewayConfig {
            port,
            bind_host: "127.0.0.1".to_string(),
        },
        agent_tx,
        Arc::new(tokio::sync::Mutex::new(
            nanobot_core::tools::ConfirmationService::new(),
        )),
    );
    gateway.start().await
}
