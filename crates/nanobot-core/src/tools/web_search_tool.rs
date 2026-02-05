use super::websearch::{WebSearchArgs, web_search};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Tool for web search
pub struct WebSearchTool;

#[async_trait]
impl super::definitions::Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information using DuckDuckGo"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "web_search",
            "description": "Search the web for information using DuckDuckGo",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Number of results to return (default: 5)"
                    }
                },
                "required": ["query"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' parameter"))?
            .to_string();

        let max_results = args["num_results"].as_u64().unwrap_or(5) as usize;

        let search_args = WebSearchArgs { query, max_results };
        let results = web_search(search_args).await?;
        Ok(serde_json::to_string_pretty(&results)?)
    }
}
