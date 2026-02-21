use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LLMConfig {
    /// Global retry settings
    pub max_retries: Option<u32>,
    pub retry_delay_ms: Option<u64>,

    /// Failover chain: ordered list of provider names to try
    #[serde(default = "default_failover_chain")]
    pub failover_chain: Vec<String>,

    /// Provider-specific configurations
    pub antigravity: Option<AntigravityConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub openai: Option<OpenAIConfig>,
    pub openrouter: Option<OpenRouterConfig>,
    pub moonshot: Option<MoonshotConfig>,
    pub qwen: Option<QwenConfig>,
}

fn default_failover_chain() -> Vec<String> {
    vec![
        "antigravity".to_string(),
        "anthropic".to_string(),
        "openai".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityConfig {
    pub enabled: bool,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicConfig {
    pub enabled: bool,
    pub api_key: String,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIConfig {
    pub enabled: bool,
    pub api_key: String,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterConfig {
    pub enabled: bool,
    pub api_key: String,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoonshotConfig {
    pub enabled: bool,
    pub api_key: String,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QwenConfig {
    pub enabled: bool,
    pub api_key: String,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
}
