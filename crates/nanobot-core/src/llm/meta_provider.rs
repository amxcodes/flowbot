use crate::antigravity::AntigravityCompletionModel;
use crate::llm::config::LLMConfig;
use rig::OneOrMany;
use rig::client::CompletionClient;
use rig::completion::{
    AssistantContent, CompletionError, CompletionModel, CompletionRequest, CompletionResponse,
    Usage,
};
use rig::providers::openai::CompletionModel as OpenAIModel;
use rig::streaming::{RawStreamingChoice, StreamingCompletionResponse};
use serde::{Deserialize, Serialize};

/// Unified response type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaResponse {
    pub content: String,
}

#[derive(Clone)]
pub enum MetaInnerModel {
    Antigravity(AntigravityCompletionModel),
    OpenAI(OpenAIModel),
    Anthropic(rig::providers::anthropic::completion::CompletionModel),
}

pub struct MetaClient {
    pub config: LLMConfig,
    pub antigravity_client: Option<crate::antigravity::AntigravityClient>,
}

impl MetaClient {
    pub async fn new(config: LLMConfig) -> anyhow::Result<Self> {
        let antigravity_client = if config
            .antigravity
            .as_ref()
            .map(|c| c.enabled)
            .unwrap_or(false)
        {
            Some(crate::antigravity::AntigravityClient::from_env().await?)
        } else {
            None
        };

        Ok(Self {
            config,
            antigravity_client,
        })
    }
}

#[derive(Clone)]
pub struct MetaCompletionModel {
    pub chain: Vec<MetaInnerModel>,
    pub current_index: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub model_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaStreamingResponse {
    pub content: String,
}

impl rig::completion::GetTokenUsage for MetaStreamingResponse {
    fn token_usage(&self) -> Option<Usage> {
        None
    }
}

fn extract_text_from_choice(choice: &OneOrMany<AssistantContent>) -> String {
    match choice.first() {
        AssistantContent::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

impl CompletionModel for MetaCompletionModel {
    type Response = MetaResponse;
    type StreamingResponse = MetaStreamingResponse;
    type Client = MetaClient;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        let model_str = model.into();
        let mut chain = Vec::new();

        for provider_name in &client.config.failover_chain {
            match provider_name.as_str() {
                "antigravity" => {
                    if let Some(ag_client) = &client.antigravity_client {
                        let m = AntigravityCompletionModel::make(ag_client, &model_str);
                        chain.push(MetaInnerModel::Antigravity(m));
                    }
                }
                "openai" => {
                    if let Some(openai_cfg) = &client.config.openai
                        && openai_cfg.enabled
                    {
                        // Create OpenAI client and model lazily
                        if let Ok(oa_client) =
                            rig::providers::openai::Client::new(&openai_cfg.api_key)
                        {
                            let m = oa_client.completions_api().completion_model(&model_str);
                            chain.push(MetaInnerModel::OpenAI(m));
                        }
                    }
                }
                "anthropic" => {
                    if let Some(anthropic_cfg) = &client.config.anthropic
                        && anthropic_cfg.enabled
                    {
                        // Create Anthropic client and model
                        if let Ok(anthropic_client) =
                            rig::providers::anthropic::Client::new(&anthropic_cfg.api_key)
                        {
                            let m = anthropic_client.completion_model(&model_str);
                            chain.push(MetaInnerModel::Anthropic(m));
                        }
                    }
                }
                "openrouter" | "moonshot" | "qwen" => {
                    // These providers use OpenAI-compatible APIs but require custom base URLs.
                    // Rig 0.29 doesn't expose a clean way to set custom base URLs via the high-level API.
                    // For now, users should configure these via the OpenAI provider with appropriate base_url.
                    tracing::warn!(
                        "Provider '{}' requires custom base URL configuration. Please use 'openai' provider with appropriate base_url in config.",
                        provider_name
                    );
                }
                _ => {}
            }
        }

        Self {
            chain,
            current_index: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            model_name: model_str,
        }
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let start_idx = self
            .current_index
            .load(std::sync::atomic::Ordering::Relaxed);
        let chain_len = self.chain.len();

        let mut errors = Vec::new();

        if chain_len == 0 {
            return Err(CompletionError::ProviderError(
                "No providers configured/enabled in failover chain".to_string(),
            ));
        }

        for i in 0..chain_len {
            let idx = (start_idx + i) % chain_len;
            let provider = &self.chain[idx];

            let req_clone = request.clone();

            // Dispatch to the appropriate provider
            let result = match provider {
                MetaInnerModel::Antigravity(m) => m
                    .completion(req_clone)
                    .await
                    .map(|resp| (extract_text_from_choice(&resp.choice), resp.usage))
                    .map_err(|e| CompletionError::ProviderError(e.to_string())),
                MetaInnerModel::OpenAI(m) => m
                    .completion(req_clone)
                    .await
                    .map(|resp| (extract_text_from_choice(&resp.choice), resp.usage))
                    .map_err(|e| CompletionError::ProviderError(e.to_string())),
                MetaInnerModel::Anthropic(m) => m
                    .completion(req_clone)
                    .await
                    .map(|resp| (extract_text_from_choice(&resp.choice), resp.usage))
                    .map_err(|e| CompletionError::ProviderError(e.to_string())),
            };

            match result {
                Ok((content, usage)) => {
                    if idx != start_idx {
                        self.current_index
                            .store(idx, std::sync::atomic::Ordering::Relaxed);
                        tracing::info!("MetaProvider: Switched to provider index {}", idx);
                    }

                    return Ok(CompletionResponse {
                        choice: OneOrMany::one(AssistantContent::text(content.clone())),
                        usage,
                        raw_response: MetaResponse { content },
                    });
                }
                Err(e) => {
                    tracing::warn!("MetaProvider: Provider at index {} failed: {}", idx, e);
                    errors.push(format!("Provider {}: {}", idx, e));
                }
            }
        }

        Err(CompletionError::ProviderError(format!(
            "All providers failed: {:?}",
            errors
        )))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<rig::streaming::StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>
    {
        // First, try to get a completion from the failover chain
        // This respects the failover logic and updates the current provider index
        let completion = self.completion(request).await?;
        let content = extract_text_from_choice(&completion.choice);

        // Create faux streaming chunks from the completed content
        // This is a temporary solution until we can properly implement
        // native streaming with failover support across different provider types.
        //
        // Future enhancement: implement true native streaming that:
        // 1. Attempts streaming from each provider in the failover chain
        // 2. Falls back to next provider if streaming fails to start
        // 3. Properly handles type mapping between different provider streaming responses
        let chunks = vec![
            Ok(RawStreamingChoice::Message(content.clone())),
            Ok(RawStreamingChoice::FinalResponse(MetaStreamingResponse {
                content,
            })),
        ];

        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(chunks),
        )))
    }
}
