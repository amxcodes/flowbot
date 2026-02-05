use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use headless_chrome::{Browser, LaunchOptions};
use std::sync::Arc;

use super::definitions::Tool;

pub struct BrowserTool;

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser_open"
    }

    fn description(&self) -> &str {
        "Open a URL in a headless browser and extract text content or take a screenshot"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "browser_open",
            "description": "Open a URL in a headless browser",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to visit"
                    },
                    "action": {
                        "type": "string",
                        "enum": ["extract_text", "screenshot", "html"],
                        "description": "Action to perform (default: extract_text)"
                    }
                },
                "required": ["url"]
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let url = args["url"].as_str().ok_or_else(|| anyhow::anyhow!("Missing 'url'"))?;
        let action = args["action"].as_str().unwrap_or("extract_text");

        // Launch options
        let options = LaunchOptions {
            headless: true,
            ..Default::default()
        };

        // Launch browser (this might block, ideally move to spawn_blocking if it was heavy, but it spawns a process)
        let browser = Browser::new(options)?;
        let tab = browser.new_tab()?;
        
        tab.navigate_to(url)?;
        tab.wait_until_navigated()?;

        match action {
            "screenshot" => {
                let png_data = tab.capture_screenshot(headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png, None, None, true)?;
                let base64_data = base64::encode(&png_data);
                // We return a truncated message + base64 data? 
                // Creating a data URI might be too large for context window.
                // Best to save to file and return path.
                let filename = format!("screenshot_{}.png", chrono::Utc::now().timestamp());
                let path = std::path::PathBuf::from(".").join(&filename);
                std::fs::write(&path, png_data)?;
                Ok(format!("Screenshot saved to {}", path.display()))
            }
            "html" => {
                let content = tab.get_content()?;
                Ok(content)
            }
            "extract_text" | _ => {
                // simple text extraction via DOM
                let element = tab.find_element("body")?;
                let text = element.get_inner_text()?;
                Ok(text)
            }
        }
    }
}
