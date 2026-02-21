#![cfg(feature = "distributed-redis")]

use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

static PROCESS_SOAK_ENV_LOCK: once_cell::sync::Lazy<Mutex<()>> =
    once_cell::sync::Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone)]
struct TerminalResult {
    request_id: String,
    status: String,
    code: Option<String>,
}

#[derive(Debug, Clone)]
struct ObservedStreamResult {
    terminal: Option<TerminalResult>,
    text_delta_count: usize,
}

fn ci_env_enabled() -> bool {
    std::env::var("CI")
        .ok()
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn pick_free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn spawn_gateway_node(port: u16, sleep_before_terminal_ms: Option<u64>) -> anyhow::Result<Child> {
    let exe = std::env::var("CARGO_BIN_EXE_gateway_ws_node")
        .map_err(|_| anyhow::anyhow!("CARGO_BIN_EXE_gateway_ws_node missing"))?;
    let mut command = Command::new(exe);
    command.arg(port.to_string());
    if let Some(ms) = sleep_before_terminal_ms {
        command.env(
            "NANOBOT_TEST_GATEWAY_SLEEP_BEFORE_TERMINAL_MS",
            ms.to_string(),
        );
    }

    let child = command
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn gateway ws node: {}", e))?;

    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if tokio::time::Instant::now() > ready_deadline {
            return Err(anyhow::anyhow!(
                "gateway ws node on port {} did not become ready",
                port
            ));
        }
        let url = format!("ws://127.0.0.1:{}/ws", port);
        if connect_async(url).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(child)
}

async fn read_json_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> anyhow::Result<serde_json::Value> {
    use futures::StreamExt;

    let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .map_err(|_| anyhow::anyhow!("websocket frame timeout"))?
        .ok_or_else(|| anyhow::anyhow!("websocket closed"))??;
    let text = match frame {
        Message::Text(t) => t,
        other => {
            return Err(anyhow::anyhow!("expected text websocket frame, got {other:?}"));
        }
    };
    let json = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| anyhow::anyhow!("invalid websocket json frame: {}", e))?;
    Ok(json)
}

async fn read_json_frame_with_timeout(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    use futures::StreamExt;

    let frame = tokio::time::timeout(timeout, ws.next())
        .await
        .map_err(|_| anyhow::anyhow!("websocket frame timeout"))?
        .ok_or_else(|| anyhow::anyhow!("websocket closed"))??;
    let text = match frame {
        Message::Text(t) => t,
        other => {
            return Err(anyhow::anyhow!("expected text websocket frame, got {other:?}"));
        }
    };
    let json = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| anyhow::anyhow!("invalid websocket json frame: {}", e))?;
    Ok(json)
}

async fn send_compat_request_with_terminal_timeout(
    port: u16,
    request_id: &str,
    content: &str,
    terminal_timeout: Duration,
) -> anyhow::Result<TerminalResult> {
    use futures::SinkExt;

    let url = format!("ws://127.0.0.1:{}/ws", port);
    let (mut ws, _) = tokio::time::timeout(Duration::from_millis(500), connect_async(url))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout"))??;

    let _session_init = read_json_frame(&mut ws).await?;

    let connect_req_id = format!("connect-{}", request_id);
    let connect_req = json!({
        "type": "req",
        "id": connect_req_id,
        "method": "connect",
        "params": {}
    });
    tokio::time::timeout(Duration::from_millis(500), ws.send(Message::Text(connect_req.to_string())))
        .await
        .map_err(|_| anyhow::anyhow!("connect request send timeout"))??;

    let connect_res = read_json_frame(&mut ws).await?;
    let token = connect_res
        .get("payload")
        .and_then(|p| p.get("token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing compat token in connect response"))?
        .to_string();

    let req = json!({
        "type": "req",
        "id": request_id,
        "method": "agent/send/message",
        "params": {
            "token": token,
            "message": content
        }
    });
    tokio::time::timeout(Duration::from_millis(500), ws.send(Message::Text(req.to_string())))
        .await
        .map_err(|_| anyhow::anyhow!("message request send timeout"))??;

    let deadline = tokio::time::Instant::now() + terminal_timeout;
    while tokio::time::Instant::now() < deadline {
        let frame = read_json_frame_with_timeout(&mut ws, Duration::from_millis(150)).await?;
        if frame.get("type").and_then(|v| v.as_str()) == Some("event")
            && frame.get("event").and_then(|v| v.as_str()) == Some("agent.done")
        {
            let payload = frame
                .get("payload")
                .ok_or_else(|| anyhow::anyhow!("missing done payload"))?;
            let req_id = payload
                .get("request_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing done payload request_id"))?;
            if req_id != request_id {
                continue;
            }
            let status = payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let code = payload
                .get("code")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return Ok(TerminalResult {
                request_id: req_id.to_string(),
                status,
                code,
            });
        }
    }

    Err(anyhow::anyhow!(
        "did not receive compat agent.done for request_id {}",
        request_id
    ))
}

async fn send_compat_request_observe_stream(
    port: u16,
    request_id: &str,
    content: &str,
    observe_for: Duration,
) -> anyhow::Result<ObservedStreamResult> {
    use futures::{SinkExt, StreamExt};

    let url = format!("ws://127.0.0.1:{}/ws", port);
    let (mut ws, _) = tokio::time::timeout(Duration::from_millis(500), connect_async(url))
        .await
        .map_err(|_| anyhow::anyhow!("connect timeout"))??;

    let _session_init = read_json_frame(&mut ws).await?;

    let connect_req_id = format!("connect-{}", request_id);
    let connect_req = json!({
        "type": "req",
        "id": connect_req_id,
        "method": "connect",
        "params": {}
    });
    tokio::time::timeout(
        Duration::from_millis(500),
        ws.send(Message::Text(connect_req.to_string())),
    )
    .await
    .map_err(|_| anyhow::anyhow!("connect request send timeout"))??;

    let connect_res = read_json_frame(&mut ws).await?;
    let token = connect_res
        .get("payload")
        .and_then(|p| p.get("token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing compat token in connect response"))?
        .to_string();

    let req = json!({
        "type": "req",
        "id": request_id,
        "method": "agent/send/message",
        "params": {
            "token": token,
            "message": content
        }
    });
    tokio::time::timeout(Duration::from_millis(500), ws.send(Message::Text(req.to_string())))
        .await
        .map_err(|_| anyhow::anyhow!("message request send timeout"))??;

    let mut text_delta_count = 0usize;
    let mut terminal: Option<TerminalResult> = None;
    let deadline = tokio::time::Instant::now() + observe_for;
    while tokio::time::Instant::now() < deadline {
        let next = tokio::time::timeout(Duration::from_millis(120), ws.next()).await;
        let frame = match next {
            Ok(Some(Ok(Message::Text(t)))) => t,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) => break,
            Ok(None) => break,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&frame) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if json.get("type").and_then(|v| v.as_str()) == Some("event")
            && json.get("event").and_then(|v| v.as_str()) == Some("agent.delta")
        {
            if json
                .get("payload")
                .and_then(|p| p.get("delta"))
                .and_then(|v| v.as_str())
                .is_some()
            {
                text_delta_count += 1;
            }
            continue;
        }
        if json.get("type").and_then(|v| v.as_str()) == Some("event")
            && json.get("event").and_then(|v| v.as_str()) == Some("agent.done")
        {
            let payload = match json.get("payload") {
                Some(v) => v,
                None => continue,
            };
            let req_id = match payload.get("request_id").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => continue,
            };
            if req_id != request_id {
                continue;
            }
            terminal = Some(TerminalResult {
                request_id: req_id.to_string(),
                status: payload
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                code: payload
                    .get("code")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
            break;
        }
    }

    Ok(ObservedStreamResult {
        terminal,
        text_delta_count,
    })
}

async fn metrics_counter_value(port: u16, metric_name: &str) -> anyhow::Result<f64> {
    let url = format!("http://127.0.0.1:{}/metrics", port);
    let body = reqwest::get(url).await?.text().await?;
    let prefix = format!("{} ", metric_name);
    let value = body
        .lines()
        .find_map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|v| v.trim().parse::<f64>().ok())
        })
        .unwrap_or(0.0);
    Ok(value)
}

fn count_distinct_request_ids(db_path: &std::path::Path, role: &str) -> anyhow::Result<i64> {
    let conn = rusqlite::Connection::open(db_path)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT request_id) FROM messages WHERE role = ?1 AND request_id IS NOT NULL",
        rusqlite::params![role],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn count_commit_markers(db_path: &std::path::Path, role: &str) -> anyhow::Result<i64> {
    let conn = rusqlite::Connection::open(db_path)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM message_request_commits WHERE role = ?1",
        rusqlite::params![role],
        |row| row.get(0),
    )?;
    Ok(count)
}

async fn run_process_kill_rolling_restart_soak() {
    let redis_url = std::env::var("NANOBOT_REDIS_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(redis_url) = redis_url else {
        if ci_env_enabled() {
            panic!("CI requires NANOBOT_REDIS_URL for process rolling soak");
        }
        eprintln!("skipping process rolling soak: NANOBOT_REDIS_URL is not set");
        return;
    };

    let qps = std::env::var("NANOBOT_PROCESS_ROLLING_SOAK_QPS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(6);
    let total_waves = std::env::var("NANOBOT_PROCESS_ROLLING_SOAK_WAVES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 10)
        .unwrap_or(60);
    let requests_per_wave = std::env::var("NANOBOT_PROCESS_ROLLING_SOAK_REQUESTS_PER_WAVE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4);
    let node_a_terminal_sleep_ms = std::env::var("NANOBOT_PROCESS_ROLLING_SOAK_NODE_A_SLEEP_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(450);

    let down_start_wave = total_waves * 40 / 100;
    let restart_wave = total_waves * 60 / 100;

    let unique_prefix = format!(
        "nanobot-proc-soak-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    unsafe {
        std::env::set_var("NANOBOT_REDIS_URL", redis_url);
        std::env::set_var("NANOBOT_REDIS_KEY_PREFIX", &unique_prefix);
        std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
        std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", qps.to_string());
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "proc-rolling-cluster");
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS", "150");
    }

    let port_a = pick_free_port().expect("pick free port for node a");
    let port_b = pick_free_port().expect("pick free port for node b");
    let mut node_a = spawn_gateway_node(port_a, Some(node_a_terminal_sleep_ms))
        .await
        .expect("spawn node a");
    let mut node_b = spawn_gateway_node(port_b, None).await.expect("spawn node b");

    let started = std::time::Instant::now();
    let start_epoch_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut request_cursor: usize = 0;
    let mut terminal_counts: HashMap<String, usize> = HashMap::new();
    let mut total_requests = 0usize;
    let mut total_allowed = 0usize;
    let mut total_denied = 0usize;
    let mut malformed_terminals = 0usize;
    let mut retries_to_node_b = 0usize;
    let mut node_a_down_waves = 0usize;
    let mut node_a_restart_events = 0usize;
    let mut amplification_violations = 0usize;

    for wave in 0..total_waves {
        while std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_millis()
            > 120
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        if wave == down_start_wave {
            let _ = node_a.kill().await;
        }
        if wave == restart_wave {
            node_a = spawn_gateway_node(port_a, Some(node_a_terminal_sleep_ms))
                .await
                .expect("restart node a");
            node_a_restart_events += 1;
        }

        let node_a_active = wave < down_start_wave || wave >= restart_wave;
        if !node_a_active {
            node_a_down_waves += 1;
        }

        let mut tasks = Vec::with_capacity(requests_per_wave);
        for i in 0..requests_per_wave {
            let request_id = format!("proc-ws-req-{}", request_cursor);
            request_cursor += 1;
            total_requests += 1;

            let prefer_a = i % 2 == 0;
            let primary_port = if prefer_a { port_a } else { port_b };
            let fallback_port = port_b;
            let content = if i % 5 == 0 { "slow-test" } else { "hello" }.to_string();

            tasks.push(tokio::spawn(async move {
                let first = send_compat_request_with_terminal_timeout(
                    primary_port,
                    &request_id,
                    &content,
                    Duration::from_millis(250),
                )
                .await;
                match first {
                    Ok(result) => (request_id, result, false),
                    Err(_) => {
                        let second = send_compat_request_with_terminal_timeout(
                            fallback_port,
                            &request_id,
                            &content,
                            Duration::from_secs(2),
                        )
                        .await
                        .expect("fallback request should succeed");
                        (request_id, second, true)
                    }
                }
            }));
        }

        let mut wave_allowed = 0usize;
        for task in tasks {
            let (request_id, terminal, retried) = task.await.expect("wave task join should succeed");
            if retried {
                retries_to_node_b += 1;
            }

            *terminal_counts.entry(request_id).or_insert(0) += 1;
            match terminal.status.as_str() {
                "success_done" => {
                    total_allowed += 1;
                    wave_allowed += 1;
                }
                "error_done" => {
                    total_denied += 1;
                    if terminal.code.as_deref() != Some("provider_rate_limited_global") {
                        malformed_terminals += 1;
                    }
                }
                _ => malformed_terminals += 1,
            }
            if terminal.request_id.is_empty() {
                malformed_terminals += 1;
            }
        }

        if wave_allowed > qps + 1 {
            amplification_violations += 1;
        }
    }

    let _ = node_a.kill().await;
    let _ = node_b.kill().await;

    let mut duplicate_terminal_ids = 0usize;
    let mut stuck_requests = 0usize;
    for idx in 0..request_cursor {
        let request_id = format!("proc-ws-req-{}", idx);
        let count = terminal_counts.get(&request_id).copied().unwrap_or(0);
        if count == 0 {
            stuck_requests += 1;
        }
        if count > 1 {
            duplicate_terminal_ids += 1;
        }
    }
    let terminal_violations = stuck_requests + duplicate_terminal_ids;

    let elapsed = started.elapsed();
    let end_epoch_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let summary = json!({
        "schema": 1,
        "scenario": "process_kill_rolling_restart_soak",
        "status": "pass",
        "start_epoch_ms": start_epoch_ms,
        "end_epoch_ms": end_epoch_ms,
        "elapsed_ms": elapsed.as_millis() as u64,
        "total_waves": total_waves,
        "requests_per_wave": requests_per_wave,
        "qps_limit": qps,
        "node_a_down_waves": node_a_down_waves,
        "node_a_restart_events": node_a_restart_events,
        "total_requests": total_requests,
        "total_allowed": total_allowed,
        "total_denied": total_denied,
        "stuck_requests": stuck_requests,
        "duplicate_terminal_ids": duplicate_terminal_ids,
        "terminal_violations": terminal_violations,
        "malformed_terminals": malformed_terminals,
        "amplification_violation_count": amplification_violations,
        "retry_to_node_b_count": retries_to_node_b,
    });
    eprintln!("PROCESS_ROLLING_SOAK_SUMMARY {}", summary);
    eprintln!(
        "PROCESS_ROLLING_SOAK_VERDICT schema=1 ok=1 reasons=[] topology={{\"down_waves\":{},\"restart_events\":{}}}",
        node_a_down_waves,
        node_a_restart_events,
    );

    assert!(node_a_down_waves > 0, "node-a down phase should occur");
    assert_eq!(node_a_restart_events, 1, "exactly one restart expected");
    assert_eq!(
        total_requests,
        total_allowed + total_denied,
        "request accounting mismatch"
    );
    assert_eq!(stuck_requests, 0, "no request should be missing a terminal");
    assert_eq!(
        duplicate_terminal_ids, 0,
        "no request should receive duplicate terminals"
    );
    assert_eq!(
        malformed_terminals, 0,
        "all error terminals should carry provider_rate_limited_global code"
    );
    assert_eq!(
        amplification_violations, 0,
        "global limiter amplification violations detected"
    );

    unsafe {
        std::env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
        std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
        std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_REDIS_TIMEOUT_MS");
    }
}

#[tokio::test]
#[ignore = "nightly soak"]
async fn process_kill_rolling_restart_soak_nightly() {
    run_process_kill_rolling_restart_soak().await;
}

#[tokio::test]
async fn process_kill_rolling_restart_smoke_ci() {
    let _guard = PROCESS_SOAK_ENV_LOCK.lock().expect("env test lock");

    unsafe {
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_QPS", "4");
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_WAVES", "10");
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_REQUESTS_PER_WAVE", "3");
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_NODE_A_SLEEP_MS", "350");
    }

    run_process_kill_rolling_restart_soak().await;

    unsafe {
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_QPS");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_WAVES");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_REQUESTS_PER_WAVE");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_NODE_A_SLEEP_MS");
    }
}

#[tokio::test]
async fn process_kill_rolling_restart_persistence_smoke_ci() {
    let _guard = PROCESS_SOAK_ENV_LOCK.lock().expect("env test lock");

    let redis_url = std::env::var("NANOBOT_REDIS_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(_redis_url) = redis_url else {
        if ci_env_enabled() {
            panic!("CI requires NANOBOT_REDIS_URL for persistence rolling smoke");
        }
        eprintln!("skipping persistence rolling smoke: NANOBOT_REDIS_URL is not set");
        return;
    };

    let mut db_path = std::env::temp_dir();
    db_path.push(format!(
        "nanobot-proc-persistence-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    unsafe {
        std::env::set_var("NANOBOT_TEST_GATEWAY_DB_PATH", &db_path);
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_QPS", "100");
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_WAVES", "10");
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_REQUESTS_PER_WAVE", "2");
        std::env::set_var("NANOBOT_PROCESS_ROLLING_SOAK_NODE_A_SLEEP_MS", "300");
    }

    run_process_kill_rolling_restart_soak().await;

    let expected_requests = 20_i64;
    let users = count_distinct_request_ids(&db_path, "user").expect("count user ids");
    let assistants = count_distinct_request_ids(&db_path, "assistant").expect("count assistant ids");
    let user_markers = count_commit_markers(&db_path, "user").expect("count user markers");
    let assistant_markers =
        count_commit_markers(&db_path, "assistant").expect("count assistant markers");

    assert_eq!(
        users, expected_requests,
        "persistence smoke requires exactly one persisted user row per request_id"
    );
    assert_eq!(
        assistants, expected_requests,
        "persistence smoke requires exactly one persisted assistant row per request_id"
    );
    assert_eq!(
        user_markers, expected_requests,
        "persistence smoke requires one user commit marker per request_id"
    );
    assert_eq!(
        assistant_markers, expected_requests,
        "persistence smoke requires one assistant commit marker per request_id"
    );

    unsafe {
        std::env::remove_var("NANOBOT_TEST_GATEWAY_DB_PATH");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_QPS");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_WAVES");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_REQUESTS_PER_WAVE");
        std::env::remove_var("NANOBOT_PROCESS_ROLLING_SOAK_NODE_A_SLEEP_MS");
    }

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
#[ignore = "nightly soak"]
async fn process_ws_dual_emit_dedupe_race_nightly() {
    let redis_url = std::env::var("NANOBOT_REDIS_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(redis_url) = redis_url else {
        if ci_env_enabled() {
            panic!("CI requires NANOBOT_REDIS_URL for process dedupe race test");
        }
        eprintln!("skipping process dedupe race test: NANOBOT_REDIS_URL is not set");
        return;
    };

    let unique_prefix = format!(
        "nanobot-proc-race-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    unsafe {
        std::env::set_var("NANOBOT_REDIS_URL", redis_url);
        std::env::set_var("NANOBOT_REDIS_KEY_PREFIX", &unique_prefix);
        std::env::set_var("NANOBOT_DISTRIBUTED_STORE_BACKEND", "redis");
        std::env::set_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT", "100");
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_BACKEND", "redis");
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE", "closed");
        std::env::set_var("NANOBOT_PROVIDER_LIMITER_IDENTITY", "proc-race-cluster");
    }

    let port_a = pick_free_port().expect("pick free port for race node a");
    let port_b = pick_free_port().expect("pick free port for race node b");
    let mut node_a = spawn_gateway_node(port_a, Some(200))
        .await
        .expect("spawn race node a");
    let mut node_b = spawn_gateway_node(port_b, Some(200))
        .await
        .expect("spawn race node b");

    let duplicate_before_a = metrics_counter_value(port_a, "llm_terminal_duplicate_total")
        .await
        .unwrap_or(0.0);
    let duplicate_before_b = metrics_counter_value(port_b, "llm_terminal_duplicate_total")
        .await
        .unwrap_or(0.0);

    let shared_request_id = "dedupe-race-req-1".to_string();
    let a_task = tokio::spawn({
        let request_id = shared_request_id.clone();
        async move {
            send_compat_request_observe_stream(
                port_a,
                &request_id,
                "dedupe-race",
                Duration::from_millis(1200),
            )
            .await
        }
    });
    let b_task = tokio::spawn({
        let request_id = shared_request_id.clone();
        async move {
            send_compat_request_observe_stream(
                port_b,
                &request_id,
                "dedupe-race",
                Duration::from_millis(1200),
            )
            .await
        }
    });

    let a_res = a_task.await.expect("race task a join should succeed");
    let b_res = b_task.await.expect("race task b join should succeed");

    let a_observed = a_res.expect("race observation a should complete");
    let b_observed = b_res.expect("race observation b should complete");

    let wins = [a_observed.terminal.is_some(), b_observed.terminal.is_some()]
        .into_iter()
        .filter(|v| *v)
        .count();
    assert_eq!(wins, 1, "exactly one node should win dual emit dedupe race");

    let winner = match (&a_observed.terminal, &b_observed.terminal) {
        (Some(v), None) => v,
        (None, Some(v)) => v,
        _ => panic!("exactly one winner response expected"),
    };
    assert_eq!(winner.request_id, shared_request_id, "winner request_id mismatch");
    assert!(
        winner.status == "success_done" || winner.status == "error_done",
        "winner terminal status should be terminal"
    );

    let loser_text_delta_count = if a_observed.terminal.is_some() {
        b_observed.text_delta_count
    } else {
        a_observed.text_delta_count
    };
    assert_eq!(
        loser_text_delta_count, 0,
        "losing node must not emit text deltas before dedupe loss"
    );

    let duplicate_after_a = metrics_counter_value(port_a, "llm_terminal_duplicate_total")
        .await
        .unwrap_or(0.0);
    let duplicate_after_b = metrics_counter_value(port_b, "llm_terminal_duplicate_total")
        .await
        .unwrap_or(0.0);
    let duplicate_increase =
        (duplicate_after_a - duplicate_before_a).max(0.0) + (duplicate_after_b - duplicate_before_b).max(0.0);
    assert!(
        duplicate_increase >= 1.0,
        "dual emit race should increment llm_terminal_duplicate_total on losing node"
    );

    let summary = json!({
        "schema": 1,
        "scenario": "process_ws_dual_emit_dedupe_race",
        "status": "pass",
        "request_id": shared_request_id,
        "winner_count": wins,
        "duplicate_counter_increase": duplicate_increase,
        "loser_text_delta_count": loser_text_delta_count,
    });
    eprintln!("PROCESS_DEDUPE_RACE_SUMMARY {}", summary);
    eprintln!("PROCESS_DEDUPE_RACE_VERDICT schema=1 ok=1 reasons=[]");

    let _ = node_a.kill().await;
    let _ = node_b.kill().await;

    unsafe {
        std::env::remove_var("NANOBOT_REDIS_KEY_PREFIX");
        std::env::remove_var("NANOBOT_DISTRIBUTED_STORE_BACKEND");
        std::env::remove_var("NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_BACKEND");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_FAILURE_MODE");
        std::env::remove_var("NANOBOT_PROVIDER_LIMITER_IDENTITY");
    }
}
