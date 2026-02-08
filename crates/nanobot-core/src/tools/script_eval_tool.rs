use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use super::definitions::Tool;
use crate::script::ScriptEngine;

/// Tool for evaluating Rhai scripts
pub struct ScriptEvalTool;

#[async_trait]
impl Tool for ScriptEvalTool {
    fn name(&self) -> &str {
        "script_eval"
    }

    fn description(&self) -> &str {
        "Evaluate a Rhai script for data transformation or logic execution"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "name": "script_eval",
            "description": "Execute a Rhai script safely with sandboxing",
            "parameters": {
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "The Rhai script code to execute"
                    },
                    "function": {
                        "type": "string",
                        "description": "Optional: specific function to call after script loads"
                    },
                    "args": {
                        "type": "array",
                        "description": "Optional: arguments to pass to the function",
                        "items": {"type": "string"}
                    }
                },
                "required": ["script"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let script = args["script"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing 'script' field"))?;

        // Create and compile the script engine
        let mut engine = ScriptEngine::new(script)?;

        // If a function is specified, call it
        if let Some(function_name) = args["function"].as_str() {
            // Check if function exists
            if !engine.has_function(function_name) {
                return Err(anyhow::anyhow!(
                    "Function '{}' not found in script",
                    function_name
                ));
            }

            // Get arguments if provided
            if let Some(args_array) = args["args"].as_array() {
                let rhai_args: Vec<rhai::Dynamic> = args_array
                    .iter()
                    .map(|v| rhai::Dynamic::from(v.as_str().unwrap_or("").to_string()))
                    .collect();

                engine.call_function(function_name, rhai_args)
            } else {
                // Call with no arguments
                engine.call_function(function_name, vec![])
            }
        } else {
            // No function specified, evaluate the script as an expression
            engine.eval(script)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_script_eval_expression() -> Result<()> {
        let tool = ScriptEvalTool;
        let args = serde_json::json!({
            "script": "let x = 10; let y = 20; x + y"
        });

        let result = tool.execute(args).await?;
        assert_eq!(result, "30");
        Ok(())
    }

    #[tokio::test]
    async fn test_script_eval_function() -> Result<()> {
        let tool = ScriptEvalTool;
        let args = serde_json::json!({
            "script": "fn greet(name) { 'Hello, ' + name + '!' }",
            "function": "greet",
            "args": ["World"]
        });

        let result = tool.execute(args).await?;
        assert_eq!(result, "Hello, World!");
        Ok(())
    }

    #[tokio::test]
    async fn test_script_eval_safety() -> Result<()> {
        let tool = ScriptEvalTool;
        // This should fail due to loop restriction
        let args = serde_json::json!({
            "script": "for i in 0..100 { print(i); }"
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
        Ok(())
    }
}
