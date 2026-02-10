/// Slack bot channel integration using Slack-Morphism (Socket Mode)
/// This implementation uses Socket Mode for robust, firewall-friendly connectivity
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use async_trait::async_trait;
use std::sync::Arc;
use super::adapter::{build_session_id, ChannelAdapter, ChannelMessage};
use super::registry::ChannelRegistry;

use slack_morphism::prelude::*;

/// Slack bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub bot_token: String,      // xoxb-...
    pub app_token: Option<String>,      // xapp-... (Required for Socket Mode)
    #[serde(default)]
    pub dm_scope: crate::config::DmScope,
}

/// Slack bot instance using Gateway Registry pattern
pub struct SlackBot {
    config: SlackConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    registry: Arc<ChannelRegistry>,
    client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    confirmation_txs: Arc<tokio::sync::Mutex<std::collections::HashMap<String, mpsc::Sender<crate::tools::ChannelConfirmationResponse>>>>,
    confirmation_ready: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}

impl SlackBot {
    /// Create a new Slack bot
    pub fn new(
        config: SlackConfig,
        agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
        registry: Arc<ChannelRegistry>,
        confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    ) -> Self {
        let client = Arc::new(SlackClient::new(SlackClientHyperConnector::new().unwrap()));
        
        Self {
            config,
            agent_tx,
            registry,
            client,
            confirmation_service,
            confirmation_txs: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            confirmation_ready: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    // Methods moved to SlackHandler


    /// Run the Slack Bot (Socket Mode)
    pub async fn run(self) -> Result<()> {
        let registry = self.registry.clone();

        // 1. Create Inbox and register with Gateway Registry
        let (inbox_tx, mut _inbox_rx) = mpsc::channel::<ChannelMessage>(100);
        registry.register("slack", inbox_tx.clone()).await;
        tracing::info!("✅ Slack adapter registered");

        // 2. Start Socket Mode Listener
        let app_token = self.config.app_token.clone()
            .ok_or_else(|| anyhow::anyhow!("Slack App Token (xapp-...) required for Socket Mode"))?;
        
        let _client = self.client.clone();
        let app_token_value: SlackApiTokenValue = app_token.into();
        let app_token = SlackApiToken::new(app_token_value);

        tracing::info!("📥 Slack Socket Mode connecting...");

        // Create Handler first
        let handler = Arc::new(SlackHandler {
            client: self.client.clone(),
            config: self.config.clone(),
            agent_tx: self.agent_tx.clone(),
            confirmation_service: self.confirmation_service.clone(),
            confirmation_txs: self.confirmation_txs.clone(),
            confirmation_ready: self.confirmation_ready.clone(),
            confirmation_outbound_tx: inbox_tx.clone(),
        });
        
        let listener_environment = Arc::new(
            SlackClientEventsListenerEnvironment::new(
                self.client.clone()
            )
            .with_user_state(handler.clone())
        );

        let listener = SlackClientSocketModeListener::new(
            &SlackClientSocketModeConfig::new(),
            listener_environment,
            SlackSocketModeListenerCallbacks::new()
                .with_push_events(move |event, _client, states| {
                    async move {
                        // Retrieve handler from user_state to avoid closure capture issues
                        let handler = {
                            let states = states.read().await;
                            states.get_user_state::<Arc<SlackHandler>>().unwrap().clone()
                        };

                        if let Err(e) = handler.handle_event(SlackPushEvent::EventCallback(event)).await {
                             tracing::error!("Slack event error: {:?}", e);
                        }
                        Ok(())
                    }
                })
        );
        
        // 3. Spawn Listener in background
        tokio::spawn(async move {
            if let Err(e) = listener.listen_for(&app_token).await {
                tracing::error!("Slack listener error: {:?}", e);
            }
        });

        // 4. Outbound Loop (Keep main task alive or just return if run is spawned?)
        // The Gateway usually awaits run(), so we should probably await the inbox loop here
        // or just return and let the gateway handle it.
        // But `run` consumes `self`, so we need to keep the inbox receiver alive.
        
        // Actually, the ChannelRegistry holds the Sender. The Inbox Rx needs to be processed.
        // We set up a specialized inbox handler in `run`?
        // Wait, `registry.register` takes `inbox_tx`.
        // Who reads `inbox_rx`?
        // In Discord adapter: `while let Some(msg) = inbox_rx.recv().await ...`
        // We need that here too.

        tracing::info!("📤 Slack Outbound Actor started");
        
        let client = self.client.clone();
        
        // Process outbound messages
        while let Some(msg) = _inbox_rx.recv().await {
             let channel_id = msg.user_id.replace("slack:", "");
             // We need a token for the session
             let token = SlackApiToken::new(self.config.bot_token.clone().into());
             let session = client.open_session(&token);

             let post_chat_req = SlackApiChatPostMessageRequest::new(
                channel_id.into(),
                SlackMessageContent::new().with_text(msg.content.into())
            );

            if let Err(e) = session.chat_post_message(&post_chat_req).await {
                tracing::error!("Failed to send Slack message: {:?}", e);
            }
        }
        
        Ok(())
    }
}

// Inner handler to support Arc cloning for callbacks
struct SlackHandler {
    client: Arc<SlackClient<SlackClientHyperHttpsConnector>>,
    config: SlackConfig,
    agent_tx: mpsc::Sender<crate::agent::AgentMessage>,
    confirmation_service: Arc<tokio::sync::Mutex<crate::tools::ConfirmationService>>,
    confirmation_txs: Arc<tokio::sync::Mutex<std::collections::HashMap<String, mpsc::Sender<crate::tools::ChannelConfirmationResponse>>>>,
    confirmation_ready: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
    confirmation_outbound_tx: mpsc::Sender<ChannelMessage>,
}

impl SlackHandler {
    async fn post_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let token: SlackApiToken = SlackApiToken::new(self.config.bot_token.clone().into());
        let session = self.client.open_session(&token);

        let post_chat_req = SlackApiChatPostMessageRequest::new(
            channel_id.into(),
            SlackMessageContent::new().with_text(content.into())
        );

        session.chat_post_message(&post_chat_req).await?;
        Ok(())
    }

    async fn handle_event(&self, event: SlackPushEvent) -> Result<()> {
         // Logic same as above, duplicated for now to ensure compile
         // In real refactor, move logic here.
         if let SlackPushEvent::EventCallback(callback) = event {
             match callback.event {
                 SlackEventCallbackBody::Message(msg_event) => {
                     // Filter bot messages
                     if msg_event.sender.bot_id.is_some() {
                         return Ok(());
                     }

                      let channel_id = msg_event
                          .origin
                          .channel
                          .as_ref()
                          .ok_or(anyhow::anyhow!("No channel ID"))?
                          .to_string();
                      let user_id = msg_event
                          .sender
                          .user
                          .as_ref()
                          .ok_or(anyhow::anyhow!("No user ID"))?
                          .to_string();
                     let text = msg_event
                         .content
                         .as_ref()
                         .and_then(|c| c.text.clone())
                         .unwrap_or_default();

                      if text.is_empty() { return Ok(()); }

                      self.ensure_confirmation_adapter(&channel_id).await;

                      if let Some((allowed, request_id)) = parse_confirmation_response(&text) {
                          let tx = {
                              let txs = self.confirmation_txs.lock().await;
                              txs.get(&channel_id).cloned()
                          };

                          if let Some(sender) = tx {
                              let _ = sender
                                  .send(crate::tools::ChannelConfirmationResponse { request_id, allowed })
                                  .await;
                              let _ = self.post_message(&channel_id, "Confirmation received.").await;
                          } else {
                              let _ = self.post_message(&channel_id, "No pending confirmation.").await;
                          }
                          return Ok(());
                      }

                      match crate::pairing::is_authorized("slack", &user_id).await {
                          Ok(authorized) => {
                              if !authorized {
                                  match crate::pairing::get_user_code("slack", &user_id).await {
                                      Ok(Some(code)) => {
                                          self.post_message(&channel_id, &format!("Pending authorization. Code: {}", code)).await?;
                                      }
                                      Ok(None) => {
                                          if let Ok(code) = crate::pairing::create_pairing_request("slack", user_id.clone(), None).await {
                                              self.post_message(&channel_id, &format!("Authorization Code: {}", code)).await?;
                                          }
                                      }
                                      _ => {}
                                  }
                                  return Ok(());
                              }
                          }
                          Err(_) => return Ok(()),
                      }

                      if let Some(token) = text.strip_prefix("/set_admin_token ") {
                          let token = token.trim();
                          if token.is_empty() {
                              let _ = self.post_message(&channel_id, "Token cannot be empty").await;
                              return Ok(());
                          }
                          if let Err(e) = crate::security::write_admin_token(token) {
                              let _ = self
                                  .post_message(&channel_id, &format!("Failed to save token: {}", e))
                                  .await;
                              return Ok(());
                          }
                          let _ = self.post_message(&channel_id, "Admin token saved").await;
                          return Ok(());
                      }
                     
                     // Forward to AgentLoop
                    let (response_tx, mut response_rx) = mpsc::channel(100);
                      let is_dm = is_slack_dm(&msg_event, &channel_id);
                      let session_id = build_session_id(
                          "slack",
                          &channel_id,
                          &user_id,
                          self.config.dm_scope,
                          is_dm,
                      );
                    let agent_msg = crate::agent::AgentMessage {
                        session_id: session_id.clone(),
                        tenant_id: session_id,
                        content: text,
                        response_tx,
                    };

                    if self.agent_tx.send(agent_msg).await.is_err() {
                        return Ok(());
                    }

                    // Collect streaming response
                    let mut full_response = String::new();
                    while let Some(chunk) = response_rx.recv().await {
                        match chunk {
                            crate::agent::StreamChunk::TextDelta(delta) => {
                                full_response.push_str(&delta);
                            }
                            _ => {}
                        }
                    }

                    if !full_response.is_empty() {
                        self.post_message(&channel_id, &full_response).await?;
                    }
                 }
                 _ => {}
             }
         }
         Ok(())
    }

    async fn ensure_confirmation_adapter(&self, channel_id: &str) {
        let mut ready = self.confirmation_ready.lock().await;
        if ready.contains(channel_id) {
            return;
        }

        let (response_tx, response_rx) = mpsc::channel(10);
        let channel = format!("slack:{}", channel_id);
        let adapter = crate::tools::channel_confirmation::ChannelConfirmationAdapter::new(
            channel,
            self.confirmation_outbound_tx.clone(),
            response_rx,
        );

        {
            let mut service = self.confirmation_service.lock().await;
            service.register_adapter(Box::new(adapter));
        }

        let mut txs = self.confirmation_txs.lock().await;
        txs.insert(channel_id.to_string(), response_tx);
        ready.insert(channel_id.to_string());
    }
}

fn is_slack_dm(event: &SlackMessageEvent, channel_id: &str) -> bool {
    if let Some(channel_type) = event.origin.channel_type.as_ref() {
        let label = format!("{:?}", channel_type).to_lowercase();
        if label == "im" || label == "mpim" {
            return true;
        }
        if label == "channel" || label == "group" {
            return false;
        }
    }

    channel_id.starts_with('D')
}

fn parse_confirmation_response(text: &str) -> Option<(bool, String)> {
    let trimmed = text.trim();
    let (allowed, rest) = if let Some(rest) = trimmed.strip_prefix("/allow ") {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix("/deny ") {
        (false, rest)
    } else {
        return None;
    };

    let request_id = rest.trim();
    if request_id.is_empty() {
        None
    } else {
        Some((allowed, request_id.to_string()))
    }
}

#[async_trait]
impl ChannelAdapter for SlackBot {
    async fn send_message(&self, user_id: &str, content: &str) -> Result<()> {
        // Implementation needed
         let channel_id = user_id.replace("slack:", "");
         let token: SlackApiToken = SlackApiToken::new(self.config.bot_token.clone().into());
         let session = self.client.open_session(&token);
         
         let post_chat_req = SlackApiChatPostMessageRequest::new(
            channel_id.into(),
            SlackMessageContent::new().with_text(content.into())
        );
        session.chat_post_message(&post_chat_req).await?;
        Ok(())
    }

    async fn send_stream_chunk(&self, _user_id: &str, _chunk: &str) -> Result<()> {
        Ok(())
    }

    fn channel_name(&self) -> &str {
        "slack"
    }

    fn format_user_id(&self, raw_id: &str) -> String {
        format!("slack:{}", raw_id)
    }
}
