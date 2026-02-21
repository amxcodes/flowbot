use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use super::definitions::Tool;
use super::process::{PidArgs, terminate_process};

pub struct KillProcessTool;

#[async_trait]
impl Tool for KillProcessTool {
    fn name(&self) -> &str {
        "kill_process"
    }

    fn description(&self) -> &str {
        "Terminate a background process by PID"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "kill_process",
            "description": "Terminate a background process",
            "parameters": {
                "type": "object",
                "properties": {
                    "pid": {
                        "type": ["integer", "string"],
                        "description": "Process ID to terminate"
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
        terminate_process(pid_args).await
    }
}
