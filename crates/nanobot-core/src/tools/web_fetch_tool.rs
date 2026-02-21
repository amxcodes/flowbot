use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use super::definitions::Tool;
use super::fetch::{WebFetchArgs, web_fetch};

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Download and extract content from a URL"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "web_fetch",
            "description": "Download and extract content from a URL",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch"
                    },
                    "extract_mode": {
                        "type": "string",
                        "description": "Extraction mode (optional)"
                    }
                },
                "required": ["url"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let fetch_args = WebFetchArgs {
            url: args["url"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' field"))?
                .to_string(),
            extract_mode: args["extract_mode"].as_str().map(|s| s.to_string()),
        };
        web_fetch(fetch_args).await
    }
}
