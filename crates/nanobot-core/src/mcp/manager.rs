// MCP Manager for handling multiple MCP servers
use super::client::McpClient;
use super::types::*;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct McpManager {
    clients: Arc<RwLock<HashMap<String, Arc<McpClient>>>>,
    configs: Arc<RwLock<HashMap<String, McpServerConfig>>>,
    tools_cache: Arc<RwLock<Vec<(String, McpTool)>>>, // (server_name, tool)
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            configs: Arc::new(RwLock::new(HashMap::new())),
            tools_cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn add_server(&self, config: McpServerConfig) -> Result<()> {
        let name = config.name.clone();
        
        // Store config
        {
            let mut configs = self.configs.write().await;
            configs.insert(name.clone(), config.clone());
        }

        tracing::info!("🔌 Connecting to MCP server: {}", name);
        
        let client = match McpClient::new(config).await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::error!("Failed to connect to MCP server {}: {}", name, e);
                return Err(e);
            }
        };
        
        // Fetch tools from this server
        match client.list_tools().await {
            Ok(tools) => {
                tracing::info!("📦 MCP server '{}' provides {} tools", name, tools.len());
                
                // Update cache
                let mut cache = self.tools_cache.write().await;
                // Remove old tools from this server
                cache.retain(|(s, _)| s != &name);
                // Add new tools
                for tool in tools {
                    cache.push((name.clone(), tool));
                }
            },
            Err(e) => {
                tracing::error!("Failed to list tools from MCP server {}: {}", name, e);
            }
        }
        
        // Store client
        let mut clients = self.clients.write().await;
        clients.insert(name, client);
        
        Ok(())
    }

    pub fn start_health_check(&self) {
        let clients = self.clients.clone();
        let configs = self.configs.clone();
        let manager_tools_cache = self.tools_cache.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                
                let configs_map = configs.read().await.clone();
                
                for (name, config) in configs_map {
                    let should_restart = {
                        let clients_guard = clients.read().await;
                        if let Some(client) = clients_guard.get(&name) {
                            !client.is_alive().await
                        } else {
                            true // Not connected, try to connect
                        }
                    };

                    if should_restart {
                        tracing::warn!("🔄 Restarting MCP server: {}", name);
                        
                        // We need to reimplement add_server logic here to avoid cloning self
                        // or make add_server not take &self but be a static method / logic?
                        // Or just duplicate logic for now since it's simple.
                        
                        match McpClient::new(config.clone()).await {
                            Ok(c) => {
                                let client = Arc::new(c);
                                // Fetch tools
                                if let Ok(tools) = client.list_tools().await {
                                    tracing::info!("📦 MCP server '{}' re-connected, {} tools", name, tools.len());
                                    let mut cache = manager_tools_cache.write().await;
                                    cache.retain(|(s, _)| s != &name);
                                    for tool in tools {
                                        cache.push((name.clone(), tool));
                                    }
                                }
                                
                                let mut clients_guard = clients.write().await;
                                clients_guard.insert(name, client);
                            },
                            Err(e) => {
                                tracing::error!("Failed to restart MCP server {}: {}", name, e);
                            }
                        }
                    }
                }
            }
        });
    }

    pub async fn get_all_tools(&self) -> Vec<(String, McpTool)> {
        let cache = self.tools_cache.read().await;
        cache.clone()
    }

    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult> {
        let clients = self.clients.read().await;
        
        let client = clients
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not found", server_name))?;
        
        client.call_tool(tool_name, arguments).await
    }

    pub async fn server_count(&self) -> usize {
        let clients = self.clients.read().await;
        clients.len()
    }

    pub async fn tool_count(&self) -> usize {
        let cache = self.tools_cache.read().await;
        cache.len()
    }
}
