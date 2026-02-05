use async_trait::async_trait;
use serde_json::Value;
use anyhow::Result;

/// A standardized interface for agent tools.
/// This trait matches the OpenClaw plugin architecture, allowing tools to be
/// defined as self-contained units rather than part of a monolithic switch statement.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The unique name of the tool (e.g., "read_file", "run_command")
    fn name(&self) -> &str;
    
    /// A human-readable description of what the tool does
    fn description(&self) -> &str;
    
    /// The input schema for the tool (JSON Schema)
    fn schema(&self) -> Value;
    
    /// Execute the tool with the provided arguments
    async fn execute(&self, args: Value) -> Result<String>;
    
    /// Validate arguments against a policy (optional, default is no validation)
    fn validate_args(&self, args: &Value, policy: &super::policy::ToolPolicy) -> Result<(), super::policy::PolicyViolation> {
        // Default implementation: no validation
        let _ = (args, policy);  // Suppress unused warnings
        Ok(())
    }
}

/// Registry for managing available tools
pub struct ToolRegistry {
    tools: std::collections::HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }
    
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }
    
    pub fn get(&self, name: &str) -> Option<&Box<dyn Tool>> {
        self.tools.get(name)
    }
    
    pub fn list_tools(&self) -> Vec<Value> {
        self.tools.values().map(|t| t.schema()).collect()
    }
    
    /// Execute a tool with policy validation
    pub async fn execute_with_policy(
        &self,
        tool_name: &str,
        args: Value,
        policy: &super::policy::ToolPolicy,
    ) -> Result<String> {
        // Check if tool is allowed by policy
        policy.check_tool_allowed(tool_name)
            .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;
        
        // Get the tool
        let tool = self.get(tool_name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", tool_name))?;
        
        // Validate arguments against policy
        tool.validate_args(&args, policy)
            .map_err(|e| anyhow::anyhow!("Policy violation: {}", e))?;
        
        // Execute the tool
        tool.execute(args).await
    }
}

/// Backward compatibility: Get tool declarations for Antigravity
pub fn get_tool_declarations() -> Vec<Value> {
    vec![
        // File System Tools
        serde_json::json!({
            "name": "read_file",
            "description": "Read contents of a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path to the file"}
                },
                "required": ["path"]
            }
        }),
        serde_json::json!({
            "name": "write_file",
            "description": "Write content to a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path to the file"},
                    "content": {"type": "string", "description": "Content to write"}
                },
                "required": ["path", "content"]
            }
        }),
        serde_json::json!({
            "name": "list_directory",
            "description": "List files and subdirectories in a directory",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path to the directory"}
                },
                "required": ["path"]
            }
        }),
        // Web Tools
        serde_json::json!({
            "name": "web_search",
            "description": "Search the web using DuckDuckGo",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "num_results": {"type": "integer", "description": "Number of results (default: 5)"}
                },
                "required": ["query"]
            }
        }),
        // Command Execution
        serde_json::json!({
            "name": "run_command",
            "description": "Execute a system command",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command to execute"},
                    "args": {"type": "array", "items": {"type": "string"}, "description": "Command arguments"},
                    "use_docker": {"type": "boolean", "description": "Run in Docker"}
                },
                "required": ["command"]
            }
        }),
    ]
}

/// Backward compatibility: Convert tool declarations to Gemini format
pub fn to_gemini_tools(declarations: Vec<Value>) -> Value {
    // Gemini API expects: { "function_declarations": [...] }
    serde_json::json!({
        "function_declarations": declarations
    })
}

/// Global tool registry - lazily initialized
pub fn get_tool_registry() -> &'static ToolRegistry {
    use once_cell::sync::Lazy;
    static REGISTRY: Lazy<ToolRegistry> = Lazy::new(|| {
        let mut registry = ToolRegistry::new();
        
        // Register all implemented tools
        registry.register(Box::new(super::read_file_tool::ReadFileTool));
        registry.register(Box::new(super::write_file_tool::WriteFileTool));
        registry.register(Box::new(super::list_directory_tool::ListDirectoryTool));
        registry.register(Box::new(super::web_search_tool::WebSearchTool));
        registry.register(Box::new(super::run_command_tool::RunCommandTool));
        
        registry
    });
    
    &REGISTRY
}
