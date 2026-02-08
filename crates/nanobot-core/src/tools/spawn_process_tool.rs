use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::definitions::Tool;
use super::process::{SpawnArgs, spawn_process};

pub struct SpawnProcessTool;

#[async_trait]
impl Tool for SpawnProcessTool {
    fn name(&self) -> &str {
        "spawn_process"
    }

    fn description(&self) -> &str {
        "Start a background process and get its PID"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "spawn_process",
            "description": "Start a background process",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to execute"
                    },
                    "args": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Command arguments"
                    }
                },
                "required": ["command"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let spawn_args = SpawnArgs {
            command: args["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'command' field"))?
                .to_string(),
            args: args["args"].as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            }),
        };
        spawn_process(spawn_args).await
    }
}
