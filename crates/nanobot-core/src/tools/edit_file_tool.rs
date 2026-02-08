use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use super::definitions::Tool;
use super::filesystem::{edit_file, EditFileArgs};

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Find and replace text in a file"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "edit_file",
            "description": "Find and replace text in a file",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Text to find and replace"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "Replacement text"
                    },
                    "all_occurrences": {
                        "type": "boolean",
                        "description": "Replace all occurrences (default: false)"
                    }
                },
                "required": ["path", "old_text", "new_text"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let edit_args = EditFileArgs {
            path: args["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' field"))?
                .to_string(),
            old_text: args["old_text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'old_text' field"))?
                .to_string(),
            new_text: args["new_text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'new_text' field"))?
                .to_string(),
            all_occurrences: args["all_occurrences"].as_bool().unwrap_or(false),
        };
        edit_file(edit_args).await
    }
}
