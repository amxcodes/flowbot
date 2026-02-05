use anyhow::{Result, anyhow};
use rig::agent::AgentBuilder;
use rig::completion::{
    AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
    Message, Usage, GetTokenUsage,
};
use rig::completion::message::{ToolResultContent, UserContent};
use rig::OneOrMany;
use serde::{Deserialize, Serialize};
use futures::stream::StreamExt;
use eventsource_stream::Eventsource;
use rig::streaming::RawStreamingChoice;
use crate::tools::definitions::{get_tool_declarations, to_gemini_tools};
use crate::token_manager::TokenManager;
use std::sync::Arc;

pub struct AntigravityClient {
    base_urls: Vec<String>,
    token_manager: Option<Arc<TokenManager>>,
    api_key: Option<String>,
}

impl AntigravityClient {
    pub fn new(base_urls: Vec<String>, token_manager: Option<Arc<TokenManager>>, api_key: Option<String>) -> Self {
        Self {
            base_urls,
            token_manager,
            api_key,
        }
    }

    pub async fn from_env() -> Result<Self> {
        let config = crate::config::Config::load().ok();
        let provider_config = config.as_ref().and_then(|cfg| cfg.providers.antigravity.clone());

        let api_key = provider_config
            .as_ref()
            .and_then(|provider| {
                let key = provider.api_key.trim().to_string();
                if key.is_empty() { None } else { Some(key) }
            });

        // Initialize TokenManager
        let manager = Arc::new(TokenManager::new("antigravity"));
        let _ = manager.load_from_store().await; // Load existing token if any
        
        // Check if we have a valid token (even expired, as manager handles refresh)
        // We attempt to get a token to verify setup, but don't fail immediately if refreshing needed later?
        // Actually, just checking if we loaded something is enough to know we are in OAuth mode
        let has_oauth = manager.get_token().await.is_ok(); // This attempts refresh if expired
        
        let token_manager = if has_oauth { Some(manager) } else { None };

        let base_urls = build_antigravity_base_urls(provider_config, token_manager.is_some(), api_key.is_some());

        if token_manager.is_none() && api_key.is_none() {
            Err(anyhow!("No Antigravity token or API key found"))
        } else {
            eprintln!("DEBUG: oauth_mode: {}, api_key present: {}", token_manager.is_some(), api_key.is_some());
            Ok(Self::new(base_urls, token_manager, api_key))
        }
    }

    pub fn agent(&self, model: &str) -> AgentBuilder<AntigravityCompletionModel> {
        AgentBuilder::new(self.completion_model(model))
    }

    pub fn completion_model(&self, model: &str) -> AntigravityCompletionModel {
        AntigravityCompletionModel {
            base_urls: self.base_urls.clone(),
            _model: model.to_string(),
            token_manager: self.token_manager.clone(),
            api_key: self.api_key.clone(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Clone)]
pub struct AntigravityCompletionModel {
    base_urls: Vec<String>,
    _model: String,
    token_manager: Option<Arc<TokenManager>>,
    api_key: Option<String>,
    client: reqwest::Client,
}

// Gemini API request format
#[derive(Debug, Serialize, Clone)]
struct GeminiRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    system_instruction: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct WrappedGeminiRequest<'a> {
    request: &'a GeminiRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(rename = "requestType")]
    request_type: Option<String>,
    #[serde(rename = "userAgent")]
    user_agent: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Content {
    role: String,
    parts: Vec<Part>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "functionCall")]
    function_call: Option<FunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "functionResponse")]
    function_response: Option<FunctionResponse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FunctionResponse {
    name: String,
    response: FunctionResponseContent,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FunctionResponseContent {
    content: String, // Tool output is always string for now
}

#[derive(Debug, Serialize, Clone)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingConfig")]
    thinking_config: Option<ThinkingConfig>,
}

#[derive(Debug, Serialize, Clone)]
struct ThinkingConfig {
    #[serde(rename = "includeThoughts")]
    include_thoughts: bool,
}

// Gemini API response format
#[derive(Debug, Deserialize, Serialize)]
struct GeminiResponse {
    candidates: Option<Vec<Candidate>>,
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Candidate {
    content: Option<Content>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ApiError {
    code: i32,
    message: String,
    status: String,
}

// OpenAI-compatible API request format
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

// OpenAI-compatible API response format
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Option<Vec<ChatChoice>>,
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AntigravityStreamingResponse {
    pub content: String,
}

impl GetTokenUsage for AntigravityStreamingResponse {
    fn token_usage(&self) -> Option<Usage> {
        None
    }
}

impl CompletionModel for AntigravityCompletionModel {
    type Response = String;
    type StreamingResponse = AntigravityStreamingResponse;
    type Client = AntigravityClient;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self {
            base_urls: client.base_urls.clone(),
            _model: model.into(),
            token_manager: client.token_manager.clone(),
            api_key: client.api_key.clone(),
            client: reqwest::Client::new(),
        }
    }



    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        // Use OpenAI format only for generativelanguage.googleapis.com/openai endpoint
        // cloudcode-pa endpoints require Gemini native format
        let use_openai_format = self.base_urls.iter().any(|url| 
            url.contains("generativelanguage.googleapis.com") && url.contains("/openai")
        );

        if use_openai_format {
            // Use OpenAI-compatible format
            let mut messages = Vec::new();

            // Add system message if preamble exists
            if let Some(preamble) = request.preamble.as_ref() {
                messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: preamble.clone(),
                });
            }

            // Convert chat history to messages
            let mut message_stack = Vec::new();
            if let Some(docs) = request.normalized_documents() {
                message_stack.push(docs);
            }
            message_stack.extend(request.chat_history.iter().cloned());

            for msg in message_stack {
                if let Some(chat_msg) = message_to_chat_message(msg) {
                    messages.push(chat_msg);
                }
            }

            if messages.is_empty() {
                return Err(CompletionError::RequestError(
                    "Antigravity request contained no messages".to_string().into(),
                ));
            }

            let chat_request = ChatRequest {
                model: self._model.clone(),
                messages,
                temperature: request.temperature.map(|value| value as f32),
                max_tokens: request.max_tokens.map(|value| value.min(u64::from(u32::MAX)) as u32),
            };

            let api_response = send_chat_with_fallback(
                &self.client,
                &self.base_urls,
                &self.token_manager,
                &self.api_key,
                &chat_request,
            )
            .await?;

            if let Some(error) = api_response.error {
                return Err(CompletionError::ProviderError(format!("API Error {}: {}", error.code, error.message)));
            }

            let response_text = api_response
                .choices
                .as_ref()
                .and_then(|choices| choices.first())
                .map(|choice| choice.message.content.clone());

            let Some(text) = response_text else {
                return Err(CompletionError::ResponseError(
                    "No content in response".to_string(),
                ));
            };

            Ok(CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text(text.clone())),
                usage: Usage::new(),
                raw_response: text,
            })
        } else {
            // Use Gemini native format
            let mut contents = Vec::new();
            let system_instruction = request.preamble.as_ref().map(|preamble| Content {
                role: "user".to_string(),
                parts: vec![Part { 
                    text: Some(preamble.clone()), 
                    function_call: None, 
                    function_response: None 
                }],
            });

            let mut message_stack = Vec::new();
            if let Some(docs) = request.normalized_documents() {
                message_stack.push(docs);
            }
            message_stack.extend(request.chat_history.iter().cloned());

            for msg in message_stack {
                if let Some(content) = message_to_content(msg) {
                    contents.push(content);
                }
            }

            if contents.is_empty() {
                return Err(CompletionError::RequestError(
                    "Antigravity request contained no messages".to_string().into(),
                ));
            }

            let generation_config = Some(GenerationConfig {
                temperature: request.temperature.map(|value| value as f32),
                max_output_tokens: request
                    .max_tokens
                    .map(|value| value.min(u64::from(u32::MAX)) as u32),
                thinking_config: Some(ThinkingConfig { include_thoughts: false }),
            });

            let request_body = GeminiRequest {
                contents,
                system_instruction,
                generation_config,
                tools: None, // Step 2: Added optional tools field (backward compatible)
            };

            let api_response = send_with_fallback(
                &self.client,
                &self.base_urls,
                &self.token_manager,
                &self.api_key,
                &request_body,
                &self._model, // Pass model name
            )
            .await?;

            if let Some(error) = api_response.error {
                return Err(CompletionError::ProviderError(format!("API Error {}: {}", error.code, error.message)));
            }

            let response_text = api_response
                .candidates
                .as_ref()
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.content.as_ref())
                .and_then(|content| content.parts.first())
                .and_then(|part| part.text.clone());

            let Some(text) = response_text else {
                return Err(CompletionError::ResponseError(
                    "No content in response".to_string(),
                ));
            };

            Ok(CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text(text.clone())),
                usage: Usage::new(),
                raw_response: text,
            })
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<rig::streaming::StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        // Use Gemini native format logic (simplified for streaming - assumes native)
        // Streaming usually requires connecting to Gemini endpoint directly.
        
        let mut contents = Vec::new();
        let system_instruction = request.preamble.as_ref().map(|preamble| Content {
            role: "user".to_string(),
            parts: vec![Part { 
                text: Some(preamble.clone()), 
                function_call: None, 
                function_response: None 
            }],
        });

        let mut message_stack = Vec::new();
        if let Some(docs) = request.normalized_documents() {
            message_stack.push(docs);
        }
        message_stack.extend(request.chat_history.iter().cloned());

        for msg in message_stack {
            if let Some(content) = message_to_content(msg) {
                contents.push(content);
            }
        }

        if contents.is_empty() {
             // If no history, maybe just user prompt? Rig usually puts user prompt in request.prompt?
             // Actually request.prompt is not in CompletionRequest? 
             // Rig's CompletionRequest has `trace_id`, `preamble`, `chat_history`, `documents`, `temperature`, `max_tokens`, `tools`, `tool_choice`.
             // Wait, where is the current user message?
             // It's usually appended to chat_history by the Agent before calling completion.
        }

        let generation_config = Some(GenerationConfig {
            temperature: request.temperature.map(|value| value as f32),
            max_output_tokens: request
                .max_tokens
                .map(|value| value.min(u64::from(u32::MAX)) as u32),
            thinking_config: Some(ThinkingConfig { include_thoughts: false }),
        });


        // Step 3: Enable tool definitions
        let tool_declarations = get_tool_declarations();
        eprintln!("DEBUG: Enabling {} tools for streaming request", tool_declarations.len());
        let gemini_tools = to_gemini_tools(tool_declarations);
        
        let request_body = GeminiRequest {
            contents,
            system_instruction,
            generation_config,
            tools: Some(gemini_tools), // Step 3: Tools enabled!
        };

        // Call streaming helper
        let stream = stream_with_fallback(
            &self.client,
            &self.base_urls,
            &self.token_manager,
            &self.api_key,
            &request_body,
            &self._model,
        ).await?;

        Ok(rig::streaming::StreamingCompletionResponse::stream(stream))
    }
}


// Streaming Helper (Duplicates send_with_fallback logic but for SSE)

async fn stream_with_fallback(
    client: &reqwest::Client, 
    base_urls: &[String], 
    token_manager: &Option<Arc<TokenManager>>, 
    _api_key: &Option<String>,
    request_body: &GeminiRequest, 
    _model_name: &str
) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<RawStreamingChoice<AntigravityStreamingResponse>, CompletionError>> + Send>>, CompletionError> {
    
    // Resolve Token
    let auth_token = if let Some(tm) = token_manager {
        Some(tm.get_token().await.map_err(|e| CompletionError::ProviderError(e.to_string()))?)
    } else {
        None
    };
    
    let mut last_error = None;
    let max_retries = 3;

    // Find Production Base URL for Discovery/Activation
    let prod_base_url = base_urls.iter()
        .find(|u| u.contains("cloudcode-pa.googleapis.com") && !u.contains("sandbox"))
        .cloned()
        .unwrap_or_else(|| "https://cloudcode-pa.googleapis.com".to_string());
        
    let is_cloudcode = auth_token.is_some() || base_urls.iter().any(|u| u.contains("cloudcode-pa"));
    
    let mut resolved_project_id = "rising-fact-p41fc".to_string(); // Default shared pool
    let mut available_models: Vec<String> = Vec::new();

    if is_cloudcode
        && let Some(token) = &auth_token {
            eprintln!("DEBUG: Starting CloudCode Protocol Sequence...");
            
            // Step 1: Discovery
            // We use the Production URL (prod_base_url)
            eprintln!("DEBUG: Step 1 - Discovery on {}", prod_base_url);
            if let Some(pid) = load_project_id(client, &prod_base_url, token).await {
                resolved_project_id = pid;
                eprintln!("DEBUG: Discovered Project ID: {}", resolved_project_id);
            } else {
                eprintln!("DEBUG: Discovery failed, using fallback: {}", resolved_project_id);
            }
            
            // Step 2: Activation
            // Must run on Production URL with the Resolved Project ID
            eprintln!("DEBUG: Step 2 - Activation for {}", resolved_project_id);
            if let Some(models) = fetch_available_models(client, &prod_base_url, token, &resolved_project_id).await {
                available_models = models;
                eprintln!("DEBUG: Activation sequence complete. Models found: {}", available_models.len());
            } else {
                eprintln!("DEBUG: Activation sequence complete (No models returned).");
            }
        }

    let mut retry_count = 0;
    
    for _ in 0..max_retries {
         // Step 3: Chat URL Selection
         let url = if is_cloudcode {
             // CloudCode Mode: Use Production (since Activation confirmed it works there)
             format!("{}/v1internal:streamGenerateContent?alt=sse", prod_base_url.trim_end_matches('/'))
         } else {
             // API Key Mode: Use generic v1beta endpoint
             // Find a generative language URL or fallback
             let gen_url = base_urls.iter().find(|u| u.contains("generativelanguage")).unwrap_or(&base_urls[0]);
             let clean_base = gen_url.replace("/openai", "").replace("/v1beta", "");
             format!("{}/v1beta/models/gemini-1.5-flash:streamGenerateContent?alt=sse", 
                 clean_base.trim_end_matches('/'))
         };
         
         eprintln!("DEBUG: Step 3 - Queueing Projects for Completion");
         eprintln!("DEBUG: Trying Project: '{}' (Sequence 1/1)", resolved_project_id);
             
         // Prepare wrapped request
         let mut modified_request = request_body.clone(); 
         
         // REPLICATE DEEPMIND SIGNATURE MODE from send_with_fallback
         eprintln!("DEBUG: Using DeepMind Signature Mode for Chat: {}", resolved_project_id);
         
         // DYNAMIC MODEL SELECTION
         let mut model_to_use = "gemini-3-flash".to_string(); // Default Preference
         
         if !available_models.is_empty() {
             // OpenClaw Priority: gemini-3-flash > gemini-3-flash-preview > gemini-2.0-flash-exp > etc.
             let priorities = [
                 "gemini-3-flash", 
                 "gemini-3-flash-preview", 
                 "google-antigravity/gemini-2.0-flash-exp", 
                 "gemini-2.0-flash-exp",
                 "gemini-1.5-flash"
             ];
             
             if let Some(preferred) = priorities.iter().find(|&&p| available_models.contains(&p.to_string())) {
                 model_to_use = preferred.to_string();
             } else if let Some(first) = available_models.first() {
                 model_to_use = first.clone();
             }
         }
         eprintln!("DEBUG: Using Model: {}", model_to_use);
         
         // Generate Request ID: agent-TIMESTAMP-RANDOM (10 char random like OpenClaw)
         let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
         let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
         let random_suffix = format!("{:09x}", (nanos as u64) ^ (retry_count * 99999) ^ ((timestamp as u64) & 0xFFFFFF));
         let suffix_9 = if random_suffix.len() > 9 { &random_suffix[random_suffix.len()-9..] } else { &random_suffix };
         let req_id = format!("agent-{}-{}", timestamp, suffix_9);

         // INJECT IDENTITY PROMPT (The "Key") + ROLE ALTERNATION (The "Loop Breaker")
         // 1. Extract User Message
         let mut user_msg = String::new();
         if let Some(first_content) = modified_request.contents.first()
             && let Some(first_part) = first_content.parts.first() {
                 user_msg = first_part.text.clone().unwrap_or_default();
             }
         eprintln!("DEBUG: Extracted User Message for Role Alternation: {}", user_msg);

         // 2. Prepare Dummy Model Turn (To satisfy User -> Model alternation)
         modified_request.contents = vec![Content {
             role: "model".to_string(),
             parts: vec![Part { 
                 text: Some(" ".to_string()),
                 function_call: None,
                 function_response: None 
             }],
         }];

         // 3. Merge Identity + Override + User Message into System Instruction
         if let Some(sys_instr) = &mut modified_request.system_instruction {
               let identity_block = format!(r#"<identity>
You are FlowBot, a powerful agentic AI coding assistant designed by the Google DeepMind team working on Advanced Agentic Coding.
You are pair programming with a USER to solve their coding task.
</identity>

User: {}"#, user_msg);
             
             let parts = &mut sys_instr.parts;
             if !parts.is_empty() {
                 let first_part = &mut parts[0];
                  // Merge identity into the first part
                  let current_text = first_part.text.as_ref().cloned().unwrap_or_default();
                  first_part.text = Some(format!("{}\n\n{}", identity_block, current_text));
                  // Remove other parts to enforce Single-Part rule
                  parts.truncate(1);
             } else {
                 parts.push(Part { 
                     text: Some(identity_block.to_string()),
                     function_call: None,
                     function_response: None 
                 });
             }
         }
         
         let wrapped_request = serde_json::json!({
            "request": {
                "contents": modified_request.contents,
                "systemInstruction": modified_request.system_instruction,
                "generationConfig": modified_request.generation_config,
            },
            "project": resolved_project_id, 
            "model": model_to_use, 
            "requestType": "agent",
            "userAgent": "antigravity",
            "requestId": req_id
        });
        
        // DEBUG: Print full body to verify structure
        // if let Ok(json_body) = serde_json::to_string(&wrapped_request) {
        //    eprintln!("DEBUG: Request Payload: {}", json_body);
        // }
        
        let mut request_builder = client.post(&url).json(&wrapped_request);
        if let Some(token) = &auth_token {
             request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
        }
        request_builder = request_builder
            .header("User-Agent", "antigravity/1.99.0 linux/x64")
            .header("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1")
            .header("Accept", "text/event-stream")
            .header("Client-Metadata", "{\"ideType\":\"IDE_UNSPECIFIED\",\"platform\":\"PLATFORM_UNSPECIFIED\",\"pluginType\":\"GEMINI\"}");

        match request_builder.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    // Success! Convert to stream
                    let stream = resp.bytes_stream()
                        .eventsource()
                        .map(|event_res| {
                            match event_res {
                                Ok(event) => {
                                    let data = event.data;
                                    // eprintln!("DEBUG: Received SSE Chunk (len: {})", data.len());
                                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data) {
                                        // Standard Response
                                        if let Some(text) = json["candidates"][0]["content"]["parts"][0]["text"].as_str() {
                                            return Ok(RawStreamingChoice::Message(text.to_string()));
                                        }
                                        
                                        // Step 4: Check for tool calls
                                        if let Some(parts) = json["candidates"][0]["content"]["parts"].as_array() {
                                            for part in parts {
                                                if let Some(function_call) = part.get("functionCall") {
                                                    let tool_name = function_call["name"].as_str().unwrap_or("unknown");
                                                    let tool_args = &function_call["args"];
                                                    eprintln!("🔧 Tool Call: {} with args: {}", tool_name, tool_args);
                                                    if let Ok(json_str) = serde_json::to_string(function_call) {
                                                        return Ok(RawStreamingChoice::Message(format!("__TOOL_CALL__{}", json_str)));
                                                    }
                                                }
                                            }
                                        }
                                        
                                        // Nested Response (Antigravity Wrapper)
                                        if let Some(resp_obj) = json.get("response") {
                                            if let Some(text) = resp_obj["candidates"][0]["content"]["parts"][0]["text"].as_str() {
                                                 return Ok(RawStreamingChoice::Message(text.to_string()));
                                            }
                                            
                                            // Check wrapped tool calls
                                            if let Some(parts) = resp_obj["candidates"][0]["content"]["parts"].as_array() {
                                                for part in parts {
                                                    if let Some(function_call) = part.get("functionCall") {
                                                        let tool_name = function_call["name"].as_str().unwrap_or("unknown");
                                                        let tool_args = &function_call["args"];
                                                        eprintln!("🔧 Tool Call (wrapped): {} with args: {}", tool_name, tool_args);
                                                        if let Ok(json_str) = serde_json::to_string(function_call) {
                                                            return Ok(RawStreamingChoice::Message(format!("__TOOL_CALL__{}", json_str)));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Ok(RawStreamingChoice::Message("".to_string())) 
                                },
                                Err(e) => Err(CompletionError::ResponseError(e.to_string()))
                            }
                        });
                        
                    return Ok(Box::pin(stream));
                } else {
                     let status = resp.status();
                     let text = resp.text().await.unwrap_or_default();
                     eprintln!("DEBUG: Stream setup failed: {} - {}", status, text);
                     last_error = Some(CompletionError::ProviderError(format!("Stream setup failed: {}", status)));
                     
                     if status.as_u16() == 429 {
                         let backoff = (retry_count + 1) * 2000;
                         eprintln!("DEBUG: 429 Throttled. Backing off {}ms", backoff);
                         tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                     }
                }
            },
            Err(e) => {
                eprintln!("DEBUG: Stream connect error: {}", e);
                last_error = Some(CompletionError::ProviderError(e.to_string()));
            }
        }
        
        retry_count += 1;
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }

    Err(last_error.unwrap_or_else(|| CompletionError::ProviderError("Stream connection failed".into())))
}


fn build_antigravity_base_urls(
    provider: Option<crate::config::AntigravityConfig>,
    has_oauth_token: bool,
    has_api_key: bool,
) -> Vec<String> {
    let mut base_urls = Vec::new();

    if let Some(provider) = provider.as_ref() {
        if let Some(base_url) = provider.base_url.as_ref() {
            let trimmed = base_url.trim();
            if !trimmed.is_empty() {
                base_urls.push(trimmed.to_string());
            }
        }

        if let Some(fallbacks) = provider.fallback_base_urls.as_ref() {
            for url in fallbacks {
                let trimmed = url.trim();
                if !trimmed.is_empty() {
                    base_urls.push(trimmed.to_string());
                }
            }
        }
    }

    if !base_urls.is_empty() {
        return dedupe_urls(base_urls);
    }

    // API key: Use generativelanguage OpenAI-compatible endpoint (Prioritize this as it's more reliable)
    // OAuth: Use cloudcode-pa endpoints (requires internal Google permission)
    if has_api_key {
        base_urls.push("https://generativelanguage.googleapis.com/v1beta/openai".to_string());
    } else if has_oauth_token {
        base_urls.extend([
            // REVERT TO PRODUCTION for OpenClaw Identity Switch
            "https://cloudcode-pa.googleapis.com",
            // Fallback to Daily Sandbox (Separate Quota Pool?)
            "https://daily-cloudcode-pa.sandbox.googleapis.com",
        ].iter().map(|url| url.to_string()));
    }

    dedupe_urls(base_urls)
}

#[derive(Debug, Serialize)]
struct LoadCodeAssistRequest {
    metadata: CodeAssistMetadata,
}

#[derive(Debug, Serialize)]
struct CodeAssistMetadata {
    #[serde(rename = "ideType")]
    ide_type: String,
    platform: String,
    #[serde(rename = "pluginType")]
    plugin_type: String,
}

#[derive(Debug, Deserialize)]
struct LoadCodeAssistResponse {
    #[serde(rename = "cloudaicompanionProject")]
    cloudaicompanion_project: Option<String>,
}

async fn load_project_id(client: &reqwest::Client, base_url: &str, token: &str) -> Option<String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{}/v1internal:loadCodeAssist", base);
    
    let request_body = LoadCodeAssistRequest {
        metadata: CodeAssistMetadata {
            ide_type: "ANTIGRAVITY".to_string(), // Authenticate as Antigravity for higher quota?
            platform: "PLATFORM_UNSPECIFIED".to_string(),
            plugin_type: "GEMINI".to_string(),
        },
    };

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .header("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1")
        .header("Client-Metadata", serde_json::to_string(&request_body.metadata).unwrap_or_default())
        .json(&request_body)
        .send()
        .await;

    match response {
        Ok(resp) => {
            if !resp.status().is_success() {
                eprintln!("DEBUG: load_project_id failed with status: {}", resp.status());
                if let Ok(text) = resp.text().await {
                   eprintln!("DEBUG: load_project_id error body: {}", text);
                }
                return None;
            }
            
            match resp.json::<LoadCodeAssistResponse>().await {
                Ok(data) => {
                    eprintln!("DEBUG: Received Project ID: {:?}", data.cloudaicompanion_project);
                    data.cloudaicompanion_project
                },
                Err(e) => {
                    eprintln!("DEBUG: Failed to parse loadCodeAssist response: {}", e);
                    None
                }
            }
        },
        Err(e) => {
            eprintln!("DEBUG: load_project_id request failed: {}", e);
            None
        }
    }
}

/// The "Activation" step - warms up the project by fetching available models
/// This triggers Google's auto-enablement for trusted clients
async fn fetch_available_models(client: &reqwest::Client, base_url: &str, token: &str, project_id: &str) -> Option<Vec<String>> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{}/v1internal:fetchAvailableModels", base);
    
    eprintln!("DEBUG: Activation step (fetchAvailableModels) for project: {}", project_id);

    // OPENCLAW REPLICATION: Usage Handshake
    let handshake_url = format!("{}/v1internal:loadCodeAssist", base);
    let handshake_body = serde_json::json!({
        "metadata": {
            "ideType": "ANTIGRAVITY",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI"
        }
    });
    
    eprintln!("DEBUG: Sending Usage Handshake (loadCodeAssist + ANTIGRAVITY)...");
    let _ = client
        .post(&handshake_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .header("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1")
        .json(&handshake_body)
        .send()
        .await;

    let body = serde_json::json!({
        "project": project_id
    });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "antigravity")
        .header("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await;

    match response {
        Ok(resp) => {
            if resp.status().is_success() {
                 if let Ok(text) = resp.text().await
                     && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
                         && let Some(models_obj) = json.get("models").and_then(|m| m.as_object()) {
                             let model_ids: Vec<String> = models_obj.keys().cloned().collect();
                             eprintln!("DEBUG: Activation successful. Available models: {:?}", model_ids);
                             return Some(model_ids);
                         }
                 eprintln!("DEBUG: Activation successful but no models parsed.");
                 Some(vec![])
            } else {
                eprintln!("DEBUG: Activation Failed: {}", resp.status());
                None
            }
        },
        Err(e) => {
            eprintln!("DEBUG: Activation request failed: {}", e);
            None
        }
    }
}

fn dedupe_urls(urls: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for url in urls {
        if !unique.iter().any(|existing| existing == &url) {
            unique.push(url);
        }
    }
    unique
}



async fn send_with_fallback(
    client: &reqwest::Client,
    base_urls: &[String],
    token_manager: &Option<Arc<TokenManager>>,
    _api_key: &Option<String>,
    request_body: &GeminiRequest,
    _model_name: &str,
) -> Result<GeminiResponse, CompletionError> {
    // Resolve Token
    let auth_token = if let Some(tm) = token_manager {
        Some(tm.get_token().await.map_err(|e| CompletionError::ProviderError(e.to_string()))?)
    } else {
        None
    };
    let mut last_error: Option<CompletionError> = None;
    let mut attempts = Vec::new();

    for base_url in base_urls {
        for url in build_candidate_urls(base_url) {
            attempts.push(url.clone());
            let mut project_ids = Vec::new();
            if url.contains("cloudcode-pa") {
                if let Some(token) = &auth_token {
                     // LOOP-BREAKER: Use Discovered Project ID with Identity Prompt
                     
                     // Step 1: Handshake on Dynamic Base URL (Prod or Sandbox)
                     eprintln!("DEBUG: Step 1 - Handshake on {} (Provisioning)", base_url);
                     let discovered_id = load_project_id(client, base_url, token).await;
                     let project_to_use = discovered_id.unwrap_or_else(|| "rising-fact-p41fc".to_string());
                     eprintln!("DEBUG: Discovered project: {}", project_to_use);
                     
                     // Step 2: Activation on Same Base URL (gl-node Identity)
                     eprintln!("DEBUG: Step 2 - Activation call for Discovered Project (gl-node)");
                     if let Some(models) = fetch_available_models(client, base_url, token, &project_to_use).await {
                         // Collect available models given we are treating this as "CloudCode" mode
                         // We need to pass this down or use it here. 
                         // Since we are inside a loop over URLs, we can cache it for the inner loop.
                         // For now, we unfortunately have to resort to a slightly ugly hack to get it into the inner loop
                         // or we just re-fetch? No, re-fetch is bad.
                         // Actually, this block is setting up `project_ids`. We can't easily pass models to the inner loop 
                         // without changing the structure significantly.
                         
                         // SIMPLIFICATION: We will carry the preferred model in a variable if possible, 
                         // but given the structure (project_ids vector), it's hard.
                         // Let's just strictly prefer "gemini-3-flash" if we see it in the activation list.
                         // Or better: Just use "gemini-3-flash" as the default if we are in CloudCode mode.
                         eprintln!("DEBUG: Activation models found: {}", models.len());
                     }
                     
                     // Step 3: Completion
                     eprintln!("DEBUG: Step 3 - Queueing Projects for Completion");
                     project_ids.push(project_to_use);
                } else {
                     project_ids.push("rising-fact-p41fc".to_string()); // No token, try shared pool
                }
                project_ids.push("".to_string()); // Final fallback to environment default
            }

            for (i, project_id) in project_ids.iter().enumerate() {
                // Retry loop for 429/5xx
                let max_retries = 10;
                let mut retry_count = 0;
                let mut backoff_ms = 1000;

                eprintln!("DEBUG: Trying Project: '{}' (Sequence {}/{})", project_id, i + 1, project_ids.len());

                loop {
                    let pid_str = if project_id.is_empty() { None } else { Some(project_id.clone()) };
                    
                    let project_for_chat = pid_str.as_deref().unwrap_or("rising-fact-p41fc");
                    
                    // DYNAMIC SELECTION PARITY
                    // Since we can't easily pass the specific model list from the outer loop without major refactor,
                    // We will default to the Known Good Model "gemini-3-flash" if the user didn't specify one.
                    // This is safer than the old "google-antigravity/gemini-2.0-flash-exp" which definitely 404s.
                    let mut model_to_use = _model_name.to_string();
                    if model_to_use.is_empty() || model_to_use.contains("gemini-2.0-flash-exp") {
                        model_to_use = "gemini-3-flash".to_string();
                    }

                    eprintln!("DEBUG: Using DeepMind Signature Mode for Chat: {}", project_for_chat);
                    eprintln!("DEBUG: Using Model: {}", model_to_use);
                    
                    // Generate Request ID: agent-TIMESTAMP-RANDOM (10 char random like OpenClaw)
                    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
                    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
                    // Create 9-char alphanumeric suffix (exactly 9 chars per Loop-Breaker)
                    // Use a slightly different mask to ensure we get enough variability for 9 hex chars
                    let random_suffix = format!("{:09x}", (nanos as u64) ^ (retry_count as u64 * 99999) ^ ((timestamp as u64) & 0xFFFFFF));
                    // Take exactly the last 9 characters if longer, or pad if shorter (though format! handles padding)
                    let suffix_9 = if random_suffix.len() > 9 { &random_suffix[random_suffix.len()-9..] } else { &random_suffix };
                    let req_id = format!("agent-{}-{}", timestamp, suffix_9);

                    // INJECT IDENTITY PROMPT (The "Key") + ROLE ALTERNATION (The "Loop Breaker")
                    let mut modified_request = request_body.clone();
                    
                    // 1. Extract User Message
                    let mut user_msg = String::new();
                    if let Some(first_content) = modified_request.contents.first()
                        && let Some(first_part) = first_content.parts.first() {
                            user_msg = first_part.text.clone().unwrap_or_default();
                        }
                    eprintln!("DEBUG: Extracted User Message for Role Alternation: {}", user_msg);

                    // 2. Prepare Dummy Model Turn (To satisfy User -> Model alternation)
                    modified_request.contents = vec![Content {
                        role: "model".to_string(),
                        parts: vec![Part { 
                            text: Some(" ".to_string()),
                            function_call: None,
                            function_response: None 
                        }],
                    }];

                    // 3. Merge Identity + Override + User Message into System Instruction
                    if let Some(sys_instr) = &mut modified_request.system_instruction {
                          let identity_block = format!(r#"<identity>
You are FlowBot, a powerful agentic AI coding assistant designed by the Google DeepMind team working on Advanced Agentic Coding.
You are pair programming with a USER to solve their coding task.
</identity>

User: {}"#, user_msg);
                        // Prepend identity block if not already present
                        // Prepend identity block if not already present
                        // Prepend identity block if not already present, MERGED into single part
                        let parts = &mut sys_instr.parts;
                        if !parts.is_empty() {
                            let first_part = &mut parts[0];
                             // Merge identity into the first part
                             let current_text = first_part.text.as_ref().cloned().unwrap_or_default();
                             first_part.text = Some(format!("{}\n\n{}", identity_block, current_text));
                             // Remove other parts to enforce Single-Part rule
                             parts.truncate(1);
                        } else {
                            parts.push(Part { 
                                text: Some(identity_block.to_string()),
                                function_call: None,
                                function_response: None 
                            });
                        }
                    }

                    // Restore Wrapped Request Structure (Required for cloudcode-pa)
                    let wrapped_request = serde_json::json!({
                        "request": {
                            "contents": modified_request.contents,
                            "systemInstruction": modified_request.system_instruction,
                            "generationConfig": modified_request.generation_config,
                        },
                        "project": project_for_chat,
                        "model": model_to_use,
                        "requestType": "agent",
                        "userAgent": "antigravity",
                        "requestId": req_id
                    });
                    
                    // DEBUG: Print the actual JSON body
                    if let Ok(json_body) = serde_json::to_string(&wrapped_request) {
                        eprintln!("DEBUG: Request Payload: {}", json_body);
                    }

                    let mut request_builder = client.post(&url).json(&wrapped_request);

                    if let Some(token) = &auth_token {
                        request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
                    }
                    
                    // Add Billing Header for Chat (Matched OpenClaw Logic: Use if found)
                    // Add Billing Header for Chat - DISABLED
                    // Hypothesis: The header triggers strict checks. Let the Body-based Project ID handle it.
                    /*
                    if let Some(pid) = &pid_str {
                         if pid != "rising-fact-p41fc" {
                             request_builder = request_builder.header("x-goog-user-project", pid);
                         }
                    }
                    */

                    // OpenClaw Identity Match - EXACT SPEC
                    request_builder = request_builder.header("User-Agent", "antigravity/1.99.0 linux/x64");
                    request_builder = request_builder.header("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1");
                    request_builder = request_builder.header("Accept", "text/event-stream");
                    
                    // Add Client-Metadata (IDE_UNSPECIFIED per Spec)
                    let client_metadata = serde_json::json!({
                        "ideType": "IDE_UNSPECIFIED",
                        "platform": "PLATFORM_UNSPECIFIED",
                        "pluginType": "GEMINI"
                    });
                    request_builder = request_builder.header("Client-Metadata", client_metadata.to_string());

                    let response_result = request_builder.send().await;

                    let response = match response_result {
                        Ok(resp) => resp,
                        Err(e) => {
                             if retry_count >= max_retries {
                                 if i == project_ids.len() - 1 {
                                     last_error = Some(CompletionError::ProviderError(e.to_string()));
                                 }
                                 break; // Move to next project/url
                             }
                             retry_count += 1;
                             eprintln!("DEBUG: Got 429/Error. Retrying ({}/{}) on same project...", retry_count, max_retries);
                             eprintln!("DEBUG: Network error, retrying ({}/{})", retry_count, max_retries);
                             tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                             backoff_ms *= 2;
                             continue;
                        }
                    };

                    if !response.status().is_success() {
                        let status = response.status();
                        
                        // Retry on 429 or 5xx
                        // Retry on 429 or 5xx
                        if (status.as_u16() == 429 || status.as_u16() >= 500) && retry_count < max_retries {
                            let text = response.text().await.unwrap_or_default();
                            eprintln!("DEBUG: Got {} from project: {}. Body: {}", status, project_id, text);
                            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                            backoff_ms *= 2;
                            retry_count += 1;
                            continue;
                        }

                        // If 403, log the reason and break internal loop to try next project
                        if status.as_u16() == 403 && i < project_ids.len() - 1 {
                             let text = response.text().await.unwrap_or_default();
                             eprintln!("DEBUG: Got 403 from {}. Body: {}", project_id, text);
                             break;
                        }

                        let text = response.text().await.unwrap_or_default();
                        if should_stop_on_error(status.as_u16(), &text) {
                            return Err(CompletionError::ProviderError(format!("API Error {}: {}", status, text)));
                        }
                        
                        // If we are here, it's a non-retryable error or retries exhausted
                        last_error = Some(CompletionError::ProviderError(format!("API Error {}: {}", status, text)));
                        break;
                    }

                    // Success - Parse SSE Stream
                    // The response is Server-Sent Events (SSE) format, not plain JSON
                    let response_text = response.text().await.map_err(|e| CompletionError::ResponseError(format!("Failed to read response body: {}", e)))?;
                    
                    // DEBUG: Save response to file for inspection
                    if let Err(e) = std::fs::write("debug_sse_response.txt", &response_text) {
                        eprintln!("DEBUG: Failed to write response to file: {}", e);
                    } else {
                        eprintln!("DEBUG: SSE response saved to debug_sse_response.txt");
                    }
                    eprintln!("DEBUG: Response length: {} bytes", response_text.len());
                    
                    // Parse SSE format: each event is "data: {...}\n\n"
                    let mut aggregated_response: Option<GeminiResponse> = None;
                    let mut chunk_count = 0;
                    let mut all_text_parts: Vec<String> = Vec::new();
                    
                    for line in response_text.lines() {
                        let line = line.trim();
                        if line.starts_with("data:") {
                            chunk_count += 1;
                            let json_str = line.strip_prefix("data:").unwrap().trim();
                            
                            eprintln!("DEBUG: Processing chunk #{}: {} chars", chunk_count, json_str.len());
                            
                            // Skip the "[DONE]" marker
                            if json_str == "[DONE]" {
                                break;
                            }
                            
                            // Parse the JSON chunk
                            match serde_json::from_str::<serde_json::Value>(json_str) {
                                Ok(chunk) => {
                                    // Extract the "response" field if present
                                    if let Some(response_obj) = chunk.get("response") {
                                        // Extract text from candidates
                                        if let Some(candidates) = response_obj.get("candidates").and_then(|c| c.as_array()) {
                                            for candidate in candidates {
                                                if let Some(content) = candidate.get("content")
                                                    && let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                                                        for part in parts {
                                                            // Skip thought parts
                                                            if part.get("thought").and_then(|t| t.as_bool()).unwrap_or(false) {
                                                                continue;
                                                            }
                                                            // Skip signature parts  
                                                            if part.get("thoughtSignature").is_some() {
                                                                continue;
                                                            }
                                                            // Collect text
                                                            if let Some(text) = part.get("text").and_then(|t| t.as_str())
                                                                && !text.is_empty() {
                                                                    all_text_parts.push(text.to_string());
                                                                    eprintln!("DEBUG: Extracted text: {}", text);
                                                                }
                                                        }
                                                    }
                                            }
                                        }
                                        
                                        // Try to deserialize as GeminiResponse (keep last one for metadata)
                                        match serde_json::from_value::<GeminiResponse>(response_obj.clone()) {
                                            Ok(parsed_response) => {
                                                aggregated_response = Some(parsed_response);
                                            },
                                            Err(e) => {
                                                eprintln!("DEBUG: Failed to parse SSE chunk as GeminiResponse: {}.", e);
                                            }
                                        }
                                    }
                                },
                                Err(e) => {
                                    eprintln!("DEBUG: Failed to parse SSE JSON: {}. Line: {}", e, json_str);
                                }
                            }
                        }
                    }
                    
                    eprintln!("DEBUG: Total chunks processed: {}", chunk_count);
                    eprintln!("DEBUG: Total text parts collected: {}", all_text_parts.len());
                    
                    // Combine all text parts and update the response
                    if let Some(mut final_response) = aggregated_response {
                        let combined_text = all_text_parts.join("");
                        eprintln!("DEBUG: Combined text length: {} chars", combined_text.len());
                        
                        // Update the response with combined text
                        if let Some(candidates) = final_response.candidates.as_mut()
                            && let Some(candidate) = candidates.first_mut()
                                && let Some(content) = candidate.content.as_mut() {
                                    content.parts = vec![Part { 
                                        text: Some(combined_text),
                                        function_call: None,
                                        function_response: None 
                                    }];
                                }
                        
                        return Ok(final_response);
                    } else {
                        return Err(CompletionError::ResponseError("No valid response chunks found in SSE stream".to_string()));
                    }
                } // End Retry Loop
                
                // If we broke out of loop without returning, we continue to next project/url
                
            }

        }
    }

    let attempts_list = if attempts.is_empty() {
        "<none>".to_string()
    } else {
        attempts.join(", ")
    };

    let base_error = match last_error {
        Some(CompletionError::ProviderError(message)) => message,
        Some(other) => other.to_string(),
        None => format!(
            "No Antigravity base URL configured. Attempts: {}",
            attempts_list
        ),
    };

    Err(CompletionError::ProviderError(format!(
        "{} (attempted: {})",
        base_error, attempts_list
    )))
}

async fn send_chat_with_fallback(
    client: &reqwest::Client,
    base_urls: &[String],
    token_manager: &Option<Arc<TokenManager>>,
    api_key: &Option<String>,
    request_body: &ChatRequest,
) -> Result<ChatResponse, CompletionError> {
    // Resolve Token
    let auth_token = if let Some(tm) = token_manager {
        Some(tm.get_token().await.map_err(|e| CompletionError::ProviderError(e.to_string()))?)
    } else {
        None
    };
    let mut last_error = None;
    let mut attempts = Vec::new();

    for base_url in base_urls {
        for url in build_candidate_urls(base_url) {
            attempts.push(url.clone());
            
            let mut project_ids = Vec::new();
            if url.contains("cloudcode-pa") {
                if let Some(token) = &auth_token {
                     eprintln!("DEBUG: Handshake start for {}", base_url);
                     let pid = load_project_id(client, base_url, token).await;
                     if let Some(p) = pid {
                         project_ids.push(p);
                     }
                     // Always add fallback
                     project_ids.push("rising-fact-p41fc".to_string());
                }
            } else {
                project_ids.push("".to_string()); // Dummy for non-cloudcode
            }

            // Loop through projects (or just once if not cloudcode)
            for (i, project_id) in project_ids.iter().enumerate() {
                // Retry loop for 429/5xx (Chat)
                let max_retries = 3;
                let mut retry_count = 0;
                let mut backoff_ms = 1000;

                loop {
                    let mut request_builder = client.post(&url).json(request_body);
                    
                    if let Some(token) = &auth_token {
                        request_builder = request_builder.header("Authorization", format!("Bearer {}", token));
                    }
                    if let Some(api_key) = api_key {
                        request_builder = request_builder.header("x-goog-api-key", api_key);
                    }
                    
                    if !project_id.is_empty() {
                        // Header removed per Deep Magic
                        // request_builder = request_builder.header("x-goog-user-project", project_id);
                    }

                    request_builder = request_builder.header("User-Agent", "antigravity/1.99.0 linux/x64");
                    request_builder = request_builder.header("X-Goog-Api-Client", "google-cloud-sdk vscode_cloudshelleditor/0.1");
                    request_builder = request_builder.header("Client-Metadata", r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#);
                    request_builder = request_builder.header("Accept", "text/event-stream");

                    let response_result = request_builder.send().await;
                    
                    let response = match response_result {
                        Ok(resp) => resp,
                        Err(e) => {
                             if retry_count >= max_retries {
                                 if i == project_ids.len() - 1 {
                                     last_error = Some(CompletionError::ProviderError(e.to_string()));
                                 }
                                 break;
                             }
                             retry_count += 1;
                             eprintln!("DEBUG: Network error, retrying ({}/{})", retry_count, max_retries);
                             tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                             backoff_ms *= 2;
                             continue;
                        }
                    };

                    if !response.status().is_success() {
                        let status = response.status();
                        // Retry on 429 or 5xx
                        if (status.as_u16() == 429 || status.as_u16() >= 500) && retry_count < max_retries {
                            eprintln!("DEBUG: Got {}, retrying ({}/{}) for project: {}", status, retry_count + 1, max_retries, project_id);
                            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                            backoff_ms *= 2;
                            retry_count += 1;
                            continue;
                        }

                        if status.as_u16() == 403 && i < project_ids.len() - 1 {
                            eprintln!("DEBUG: Got 403 from {}, trying fallback...", project_id);
                            break;
                        }

                        let text = response.text().await.unwrap_or_default();
                        if should_stop_on_error(status.as_u16(), &text) {
                             return Err(CompletionError::ProviderError(format!("API Error {}: {}", status, text)));
                        }
                        last_error = Some(CompletionError::ProviderError(format!("API Error {}: {}", status, text)));
                        break;
                    }

                    // Success
                    let api_response: ChatResponse = response.json().await.map_err(|e| CompletionError::ResponseError(e.to_string()))?;
                    return Ok(api_response);
                } // End Retry Loop
                
                // If broken out, continue to next project/url
            }
        }
    }

    let attempts_list = if attempts.is_empty() {
        "<none>".to_string()
    } else {
        attempts.join(", ")
    };

    let base_error = match last_error {
        Some(CompletionError::ProviderError(message)) => message,
        Some(other) => other.to_string(),
        None => format!(
            "No Antigravity base URL configured. Attempts: {}",
            attempts_list
        ),
    };

    Err(CompletionError::ProviderError(format!(
        "{} (attempted: {})",
        base_error, attempts_list
    )))
}

fn should_stop_on_error(status: u16, body: &str) -> bool {
    if status == 401 || status == 403 {
        let body_lower = body.to_lowercase();
        if body_lower.contains("license") {
            return true;
        }
        if body_lower.contains("iam_permission_denied") || body_lower.contains("permission_denied") {
            return true;
        }
    }

    false
}

fn build_candidate_urls(base_url: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    let mut urls = Vec::new();

    // For Antigravity Cloud Code - use Gemini-native endpoint (NOT OpenAI format)
    if base.contains("cloudcode-pa.googleapis.com") || base.contains("cloudcode-pa.sandbox.googleapis.com") {
        // Spec says Chat uses Sandbox: daily-cloudcode-pa.sandbox.googleapis.com
        // We will try BOTH Prod and Sandbox URLs if the base matches cloudcode-pa
        if base.contains("daily-cloudcode-pa") {
             urls.push(format!("{}/v1internal:streamGenerateContent?alt=sse", base));
        } else {
             // If Prod, push Prod first, then Sandbox fallback?
             // Actually, the spec says "Discovery on Prod, Chat on Sandbox".
             // We'll push Sandbox URL *explicitly* to be safe.
             urls.push("https://daily-cloudcode-pa.sandbox.googleapis.com/v1internal:streamGenerateContent?alt=sse".to_string());
             // Also keep Prod just in case
             urls.push(format!("{}/v1internal:streamGenerateContent?alt=sse", base));
        }
    } else if base.contains("/openai") {
        // For standard OpenAI-compatible endpoints (e.g., Google AI Studio)
        urls.push(format!("{}/chat/completions", base));
    } else {
        // Fallback: try Gemini-native first, then OpenAI-compatible
        urls.push(format!("{}/v1internal:streamGenerateContent?alt=sse", base));
        urls.push(format!("{}/v1beta/openai/chat/completions", base));
    }

    dedupe_urls(urls)
}

fn message_to_content(message: Message) -> Option<Content> {
    match message {
        Message::User { content, .. } => {
            let mut parts = Vec::new();
            let mut has_tool_result = false;
            
            for item in content.iter() {
                match item {
                    rig::completion::message::UserContent::Text(text) => {
                        parts.push(Part {
                            text: Some(text.text.clone()),
                            function_call: None,
                            function_response: None,
                        });
                    }
                    rig::completion::message::UserContent::ToolResult(result) => {
                        has_tool_result = true;
                        // Extract text content from tool result
                        let mut content_str = String::new();
                        for c in result.content.iter() {
                            if let rig::completion::message::ToolResultContent::Text(t) = c {
                                content_str.push_str(&t.text);
                            }
                        }
                        
                        parts.push(Part {
                            text: None, 
                            function_call: None,
                            function_response: Some(FunctionResponse {
                                name: result.call_id.clone().unwrap_or("unknown".to_string()),
                                response: FunctionResponseContent {
                                    content: content_str,
                                },
                            }),
                        });
                    }
                    _ => {}
                }
            }
            
            if parts.is_empty() { 
                None 
            } else { 
                let role = if has_tool_result { "function".to_string() } else { "user".to_string() };
                Some(Content { role, parts }) 
            }
        },
        Message::Assistant { content, .. } => {
             let mut parts = Vec::new();
             for item in content.iter() {
                 match item {
                     rig::completion::message::AssistantContent::Text(text) => {
                         parts.push(Part { 
                             text: Some(text.text.clone()), 
                             function_call: None, 
                             function_response: None 
                         });
                     },
                     rig::completion::message::AssistantContent::ToolCall(call) => {
                         let args_json = call.additional_params.as_ref()
                             .and_then(|p| p.get("arguments"))
                             .cloned()
                             .unwrap_or(serde_json::json!({}));

                         parts.push(Part {
                             text: None,
                             function_call: Some(FunctionCall {
                                 name: call.signature.clone().unwrap_or("unknown".to_string()),
                                 args: args_json,
                             }),
                             function_response: None,
                         });
                     },
                     _ => {}
                 }
             }
             
             if parts.is_empty() { None } else { Some(Content { role: "model".to_string(), parts }) }
        },
    }
}

fn message_to_chat_message(message: Message) -> Option<ChatMessage> {
    match message {
        Message::User { content } => user_content_to_text(&content).map(|text| ChatMessage {
            role: "user".to_string(),
            content: text,
        }),
        Message::Assistant { content, .. } => assistant_content_to_text(&content).map(|text| ChatMessage {
            role: "assistant".to_string(),
            content: text,
        }),
    }
}

fn user_content_to_text(content: &OneOrMany<UserContent>) -> Option<String> {
    let mut segments = Vec::new();
    for item in content.iter() {
        match item {
            UserContent::Text(text) => segments.push(text.text.clone()),
            UserContent::ToolResult(result) => {
                for result_content in result.content.iter() {
                    if let ToolResultContent::Text(text) = result_content {
                        segments.push(text.text.clone());
                    }
            }
            }
            _ => {}
        }
    }

    if segments.is_empty() {
        None
    } else {
        Some(segments.join("\n"))
    }
}

fn assistant_content_to_text(content: &OneOrMany<AssistantContent>) -> Option<String> {
    let mut segments = Vec::new();
    for item in content.iter() {
        if let AssistantContent::Text(text) = item {
            segments.push(text.text.clone());
        }
    }

    if segments.is_empty() {
        None
    } else {
        Some(segments.join("\n"))
    }
}
