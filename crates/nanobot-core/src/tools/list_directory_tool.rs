use super::filesystem::ListDirArgs;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Tool for listing directory contents
pub struct ListDirectoryTool;

#[async_trait]
impl super::definitions::Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and subdirectories in a directory"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "list_directory",
            "description": "List files and subdirectories in a directory",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the directory to list"
                    }
                },
                "required": ["path"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?
            .to_string();

        let list_args = ListDirArgs {
            path,
            max_depth: Some(1),
        };
        let files = super::filesystem::list_directory(list_args).await?;
        Ok(serde_json::to_string_pretty(&files)?)
    }
}
