use async_trait::async_trait;
use serde_json::{json, Value};
use anyhow::Result;
use super::filesystem::{read_file, ReadFileArgs};

/// Tool for reading file contents
pub struct ReadFileTool;

#[async_trait]
impl super::definitions::Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    
    fn description(&self) -> &str {
        "Read the contents of a file from the filesystem"
    }
    
    fn schema(&self) -> Value {
        json!({
            "name": "read_file",
            "description": "Read the contents of a file from the filesystem",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the file to read"
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
        
        let read_args = ReadFileArgs { path };
        Ok(read_file(read_args).await?)
    }
    
    fn validate_args(&self, args: &Value, policy: &super::policy::ToolPolicy) -> Result<(), super::policy::PolicyViolation> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| super::policy::PolicyViolation::PathNotAllowed {
                path: std::path::PathBuf::from(""),
                operation: "read".to_string(),
            })?;
        
        policy.check_read_path(path)
    }
}
