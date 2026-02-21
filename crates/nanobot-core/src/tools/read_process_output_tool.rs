use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use super::definitions::Tool;
use super::process::{PidArgs, read_process_output};

pub struct ReadProcessOutputTool;

#[async_trait]
impl Tool for ReadProcessOutputTool {
    fn name(&self) -> &str {
        "read_process_output"
    }

    fn description(&self) -> &str {
        "Read output from a background process by PID"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "read_process_output",
            "description": "Read output from a background process",
            "parameters": {
                "type": "object",
                "properties": {
                    "pid": {
                        "type": ["integer", "string"],
                        "description": "Process ID"
                    }
                },
                "required": ["pid"]
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

        let pid_args = PidArgs { pid };
        read_process_output(pid_args).await
    }
}
