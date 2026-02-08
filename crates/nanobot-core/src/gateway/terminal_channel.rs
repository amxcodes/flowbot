use anyhow::Result;
use async_trait::async_trait;
use super::channel::{Channel, IncomingEvent};
use tokio::sync::mpsc;
use std::io::{self, BufRead};

/// Terminal channel for stdin/stdout interaction
pub struct TerminalChannel {
    id: String,
}

impl TerminalChannel {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
        }
    }
}

#[async_trait]
impl Channel for TerminalChannel {
    fn id(&self) -> &str {
        &self.id
    }
    
    async fn start(&self, tx: mpsc::Sender<IncomingEvent>) -> Result<()> {
        println!("📟 Terminal channel started. Type your messages:");
        
        // Spawn blocking IO task
        tokio::task::spawn_blocking(move || {
            let stdin = io::stdin();
            let handle = stdin.lock();
            
            for line in handle.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("Terminal read error: {}", e);
                        break;
                    }
                };
                
                if line.trim().is_empty() {
                    continue;
                }
                
                let event = IncomingEvent {
                    channel_id: "terminal".to_string(),
                    user_id: "terminal_user".to_string(),
                    content: line.trim().to_string(),
                    metadata: Default::default(),
                };
                
                if let Err(e) = tx.blocking_send(event) {
                    eprintln!("Failed to send terminal event: {}", e);
                    break;
                }
            }
        });
        
        Ok(())
    }
    
    async fn send(&self, _target: &str, content: &str) -> Result<()> {
        println!("🤖: {}", content);
        Ok(())
    }
}
