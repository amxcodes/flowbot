use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::definitions::Tool;
use super::process::{WriteInputArgs, write_process_input};

pub struct WriteProcessInputTool;

#[async_trait]
impl Tool for WriteProcessInputTool {
    fn name(&self) -> &str {
        "write_process_input"
    }

    fn description(&self) -> &str {
        "Send input to a running process stdin"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "write_process_input",
            "description": "Send text input to a running process (stdin)",
            "parameters": {
                "type": "object",
                "properties": {
                    "pid": {
                        "type": ["integer", "string"],
                        "description": "Process ID"
                    },
                    "input": {
                        "type": "string",
                        "description": "Input text to send"
                    }
                },
                "required": ["pid", "input"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let pid_val = &args["pid"];
        let pid = if let Some(n) = pid_val.as_u64() {
            n as u32
        } else if let Some(s) = pid_val.as_str() {
            s.parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid PID format"))?
        } else {
            return Err(anyhow::anyhow!("Missing 'pid' field"));
        };

        let input_args = WriteInputArgs {
            pid,
            input: args["input"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'input' field"))?
                .to_string(),
        };
        write_process_input(input_args).await
    }
}
