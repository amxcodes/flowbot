// Browser tool definitions for LLM
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserNavigateArgs {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserClickArgs {
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTypeArgs {
    pub selector: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserExecuteJsArgs {
    pub script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserScreenshotArgs {
    #[serde(default)]
    pub selector: Option<String>,
}

/// Get all browser tool definitions for LLM
pub fn get_browser_tools() -> Vec<serde_json::Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "browser_navigate",
                "description": "Navigate to a URL in the headless browser. Opens a new page and loads the specified URL.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to navigate to (must include protocol: https:// or http://)"
                        }
                    },
                    "required": ["url"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_click",
                "description": "Click an element on the current page using a CSS selector.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": {
                            "type": "string",
                            "description": "CSS selector for the element to click (e.g., 'button#submit', '.login-btn')"
                        }
                    },
                    "required": ["selector"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_type",
                "description": "Type text into an input field using a CSS selector.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": {
                            "type": "string",
                            "description": "CSS selector for the input element (e.g., 'input[name=\"email\"]')"
                        },
                        "text": {
                            "type": "string",
                            "description": "The text to type into the field"
                        }
                    },
                    "required": ["selector", "text"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_screenshot",
                "description": "Take a screenshot of the current page or a specific element. Returns base64-encoded PNG.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "selector": {
                            "type": "string",
                            "description": "Optional CSS selector for a specific element. If omitted, captures the entire page."
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_execute_js",
                "description": "Execute JavaScript code in the browser context and return the result.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "script": {
                            "type": "string",
                            "description": "JavaScript code to execute (e.g., 'document.title', 'document.querySelector(\"h1\").textContent')"
                        }
                    },
                    "required": ["script"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_get_html",
                "description": "Get the full HTML content of the current page.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        }),
    ]
}
