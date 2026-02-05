use async_trait::async_trait;
use serde_json::{json, Value};
use anyhow::Result;
use super::cli_wrapper;
use super::commands::RunCommandArgs;

/// Tool for running system commands
pub struct RunCommandTool;

#[async_trait]
impl super::definitions::Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }
    
    fn description(&self) -> &str {
        "Execute a system command with smart output formatting"
    }
    
    fn schema(&self) -> Value {
        json!({
            "name": "run_command",
            "description": "Execute a system command with smart output formatting",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute"
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Command arguments"
                    },
                    "use_docker": {
                        "type": "boolean",
                        "description": "Execute command in Docker container"
                    }
                },
                "required": ["command"]
            }
        })
    }
    
    async fn execute(&self, args: Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?
            .to_string();
        
        let cmd_args = args["args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        
        let use_docker = args["use_docker"].as_bool().unwrap_or(false);
        
        let run_args = RunCommandArgs {
            command,
            args: cmd_args,
            use_docker,
            docker_image: None,
        };
        
        cli_wrapper::run_and_format(run_args).await
    }
    
    fn validate_args(&self, args: &Value, policy: &super::policy::ToolPolicy) -> Result<(), super::policy::PolicyViolation> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| super::policy::PolicyViolation::CommandNotAllowed(String::new()))?;
        
        policy.check_command_allowed(command)
    }
}
