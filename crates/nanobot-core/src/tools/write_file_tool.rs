use super::filesystem::{WriteFileArgs, write_file};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Tool for writing content to files
pub struct WriteFileTool;

#[async_trait]
impl super::definitions::Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file on the filesystem"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "write_file",
            "description": "Write content to a file on the filesystem",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path where the file should be written"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?
            .to_string();

        let content = args["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?
            .to_string();

        let write_args = WriteFileArgs {
            path,
            content,
            overwrite: args["overwrite"].as_bool().unwrap_or(false),
        };
        write_file(write_args).await
    }

    fn validate_args(
        &self,
        args: &Value,
        policy: &super::policy::ToolPolicy,
    ) -> Result<(), super::policy::PolicyViolation> {
        let path = args["path"].as_str().ok_or_else(|| {
            super::policy::PolicyViolation::PathNotAllowed {
                path: std::path::PathBuf::from(""),
                operation: "write".to_string(),
            }
        })?;

        policy.check_write_path(path)
    }
}
