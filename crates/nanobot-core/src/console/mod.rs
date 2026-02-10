use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use std::io::{self, Write};

/// Interactive REPL console for debugging nanobot
pub struct ConsoleREPL {
    admin_url: String,
    http_client: Client,
    history: Vec<String>,
}

impl ConsoleREPL {
    pub fn new(port: u16) -> Self {
        Self {
            admin_url: format!("http://localhost:{}", port),
            http_client: Client::new(),
            history: Vec::new(),
        }
    }
    
    pub async fn run(&mut self) -> Result<()> {
        println!("🎮 Nanobot Console REPL");
        println!("   Connected to: {}", self.admin_url);
        println!("   Type /help for available commands\n");
        
        loop {
            print!("nanobot> ");
            io::stdout().flush()?;
            
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            
            let cmd = input.trim();
            
            if cmd.is_empty() {
                continue;
            }
            
            if cmd == "/exit" || cmd == "/quit" {
                println!("👋 Goodbye!");
                break;
            }
            
            self.history.push(cmd.to_string());
            
            match self.execute(cmd).await {
                Ok(output) => println!("{}\n", output),
                Err(e) => eprintln!("❌ Error: {}\n", e),
            }
        }
        
        Ok(())
    }
    
    async fn execute(&self, cmd: &str) -> Result<String> {
        match cmd {
            "/help" => Ok(self.show_help()),
            "/state" => self.fetch_state().await,
            "/tools" => self.fetch_tools().await,
            "/health" => self.fetch_health().await,
            "/history" => Ok(self.show_history()),
            cmd if cmd.starts_with("/eval ") => self.eval_tool(cmd).await,
            _ => Ok(format!("Unknown command: {}. Type /help for available commands.", cmd)),
        }
    }
    
    fn show_help(&self) -> String {
        r#"Available Commands:
  /help            - Show this help message
  /state           - Show server status (uptime, agents, tools)
  /tools           - List all registered tools
  /health          - Check server health
  /eval <tool>     - Execute a tool via admin API
  /history         - Show command history
  /exit, /quit     - Exit the console
        "#.to_string()
    }
    
    async fn fetch_state(&self) -> Result<String> {
        let url = format!("{}/state", self.admin_url);
        let response: Value = self.http_client.get(&url).send().await?.json().await?;
        Ok(format!("📊 Server State:\n{}", serde_json::to_string_pretty(&response)?))
    }
    
    async fn fetch_tools(&self) -> Result<String> {
        let url = format!("{}/tools", self.admin_url);
        let response: Value = self.http_client.get(&url).send().await?.json().await?;
        Ok(format!("🔧 Registered Tools:\n{}", serde_json::to_string_pretty(&response)?))
    }
    
    async fn fetch_health(&self) -> Result<String> {
        let url = format!("{}/health", self.admin_url);
        let response = self.http_client.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;
        
        if status.is_success() {
            Ok(format!("✅ Server is healthy: {}", body))
        } else {
            Ok(format!("⚠️  Server returned status {}: {}", status, body))
        }
    }
    
    async fn eval_tool(&self, cmd: &str) -> Result<String> {
        let rest = cmd.trim_start_matches("/eval").trim();
        if rest.is_empty() {
            return Ok("Usage: /eval <tool> {json-args}".to_string());
        }

        let mut parts = rest.splitn(2, ' ');
        let tool = parts.next().unwrap_or("");
        if tool.is_empty() {
            return Ok("Usage: /eval <tool> {json-args}".to_string());
        }

        let args = if let Some(raw) = parts.next() {
            if raw.trim().is_empty() {
                None
            } else {
                Some(serde_json::from_str::<Value>(raw)?)
            }
        } else {
            None
        };

        let url = format!("{}/eval", self.admin_url);
        let payload = serde_json::json!({
            "tool": tool,
            "args": args,
        });

        let mut request = self.http_client.post(&url).json(&payload);
        let token = std::env::var("NANOBOT_ADMIN_TOKEN")
            .ok()
            .or_else(|| crate::security::read_admin_token().ok().flatten());
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response: Value = request.send().await?.json().await?;
        let output = response
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("(no output)");
        Ok(output.to_string())
    }
    
    fn show_history(&self) -> String {
        if self.history.is_empty() {
            "No command history yet.".to_string()
        } else {
            let mut output = "Command History:\n".to_string();
            for (i, cmd) in self.history.iter().enumerate() {
                output.push_str(&format!("  {}: {}\n", i + 1, cmd));
            }
            output
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_help_command() {
        let repl = ConsoleREPL::new(3000);
        let help = repl.show_help();
        assert!(help.contains("/help"));
        assert!(help.contains("/state"));
    }
}
