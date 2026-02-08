// MCP Client implementation
use super::types::*;
use anyhow::{anyhow, Context, Result};
// use serde_json::json; // Removed to avoid conflict
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

pub struct McpClient {
    name: String,
    process: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    pending_requests: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<McpResponse>>>>>,
    next_id: AtomicI64,
}

impl McpClient {
    pub async fn new(config: McpServerConfig) -> Result<Self> {
        tracing::info!("🚀 Starting MCP server: {}", config.name);
        
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()); // Capture stderr for logging

        // Add environment variables
        for (key, value) in &config.env {
            let expanded = if value.starts_with("${") && value.ends_with("}") {
                let var_name = &value[2..value.len() - 1];
                std::env::var(var_name).unwrap_or_else(|_| value.clone())
            } else {
                value.clone()
            };
            cmd.env(key, expanded);
        }

        let mut process = cmd.spawn().context(format!("Failed to spawn {}", config.name))?;

        let stdin = process.stdin.take().ok_or_else(|| anyhow!("Failed to get stdin"))?;
        let stdout = process.stdout.take().ok_or_else(|| anyhow!("Failed to get stdout"))?;
        let stderr = process.stderr.take().ok_or_else(|| anyhow!("Failed to get stderr"))?;

        let pending_requests: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<McpResponse>>>>> = Arc::new(Mutex::new(HashMap::new()));
        
        // Spawn stderr reader to log server logs
        let name_clone = config.name.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("[MCP {}] stderr: {}", name_clone, line);
            }
        });

        // Spawn stdout reader to handle responses
        let pending_clone = pending_requests.clone();
        let name_clone = config.name.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() { continue; }
                
                // Try parsing as generic JSON-RPC message
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(msg) => {
                        // Check if it's a response (has "id" and ("result" or "error"))
                        if let Some(id_val) = msg.get("id") {
                            if let Some(id) = id_val.as_i64() {
                                let is_response = msg.get("result").is_some() || msg.get("error").is_some();
                                if is_response {
                                    let mut pending = pending_clone.lock().await;
                                    if let Some(tx) = pending.remove(&id) {
                                        // Parse full response
                                        match serde_json::from_value::<McpResponse>(msg) {
                                            Ok(response) => { let _ = tx.send(Ok(response)); }
                                            Err(e) => { let _ = tx.send(Err(anyhow::anyhow!("Failed to parse response: {}", e))); }
                                        }
                                    }
                                } else {
                                    // Could be a server-initiated request with an ID? 
                                    // For now we assume requests from server need handling separately
                                    tracing::warn!("[MCP {}] Unhandled request from server: {}", name_clone, line);
                                }
                            }
                        } else {
                            // Notification (no id)
                            if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
                                if method == "notifications/message" {
                                    if let Some(params) = msg.get("params") {
                                         tracing::info!("[MCP {}] 🔔 {}", name_clone, params);
                                    }
                                } else {
                                     tracing::debug!("[MCP {}] Notification: {}", name_clone, method);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[MCP {}] Invalid JSON received: {} - Line: {}", name_clone, e, line);
                    }
                }
            }
            tracing::error!("[MCP {}] Stdout stream ended", name_clone);
        });

        let client = Self {
            name: config.name.clone(),
            process: Mutex::new(Some(process)),
            stdin: Mutex::new(Some(stdin)),
            pending_requests,
            next_id: AtomicI64::new(1),
        };

        // Initialize with timeout
        match timeout(Duration::from_secs(10), client.initialize()).await {
            Ok(Ok(_)) => Ok(client),
            Ok(Err(e)) => Err(anyhow!("Failed to initialize MCP server {}: {}", config.name, e)),
            Err(_) => Err(anyhow!("Timeout waiting for MCP server {} to initialize", config.name)),
        }
    }

    async fn initialize(&self) -> Result<()> {
        let request = McpRequest::new(
            self.next_id.fetch_add(1, Ordering::SeqCst),
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "roots": { "listChanged": true },
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "nanobot",
                    "version": "0.1.0"
                }
            })),
        );

        let _response = self.send_request(request).await?;
        
        // Send initialized notification
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        
        self.send_notification(&notification).await?;

        Ok(())
    }

    async fn send_notification(&self, notification: &serde_json::Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        if let Some(stdin) = stdin.as_mut() {
            let msg = serde_json::to_string(notification)? + "\n";
            stdin.write_all(msg.as_bytes()).await?;
            stdin.flush().await?;
            Ok(())
        } else {
            Err(anyhow!("MCP stdin not available"))
        }
    }

    async fn send_request(&self, request: McpRequest) -> Result<McpResponse> {
        let id = request.id.as_i64().ok_or_else(|| anyhow!("Request ID must be an integer"))?;
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, tx);
        }

        let mut stdin = self.stdin.lock().await;
        if let Some(stdin) = stdin.as_mut() {
            let msg = serde_json::to_string(&request)? + "\n";
            stdin.write_all(msg.as_bytes()).await?;
            stdin.flush().await?;
        } else {
            return Err(anyhow!("MCP stdin not available"));
        }
        drop(stdin); // Release lock while waiting

        // Wait for response with timeout
        match timeout(Duration::from_secs(60), rx).await {
            Ok(Ok(Ok(response))) => {
                if let Some(error) = response.error {
                    Err(anyhow!("MCP Error {}: {}", error.code, error.message))
                } else {
                    Ok(response)
                }
            }
            Ok(Ok(Err(e))) => Err(e), // Internal parsing error
            Ok(Err(_)) => Err(anyhow!("Response channel closed unexpectedly")),
            Err(_) => {
                // Remove pending request on timeout
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                Err(anyhow!("Request timed out"))
            }
        }
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let request = McpRequest::new(
            self.next_id.fetch_add(1, Ordering::SeqCst),
            "tools/list",
            None,
        );

        let response = self.send_request(request).await?;

        if let Some(result) = response.result {
            let tools: Vec<McpTool> = serde_json::from_value(
                result.get("tools").cloned().unwrap_or(serde_json::json!([]))
            )?;
            Ok(tools)
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> Result<ToolCallResult> {
        let request = McpRequest::new(
            self.next_id.fetch_add(1, Ordering::SeqCst),
            "tools/call",
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        );

        let response = self.send_request(request).await?;

        if let Some(result) = response.result {
            let tool_result: ToolCallResult = serde_json::from_value(result)?;
            Ok(tool_result)
        } else {
            Err(anyhow!("No result from tool call"))
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn is_alive(&self) -> bool {
        let mut process = self.process.lock().await;
        if let Some(child) = process.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => false, // Exited
                Ok(None) => true,     // Still running
                Err(_) => false,      // Error checking
            }
        } else {
            false
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.try_lock() {
            if let Some(mut child) = process.take() {
                // We're in Drop, so we can't await. 
                // We'll trust the OS to cleanup properly if we just start kill.
                let _ = child.start_kill(); 
            }
        }
    }
}
