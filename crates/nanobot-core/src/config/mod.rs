use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub mod agent_loader;
pub mod agent_manifest;
pub mod encrypted_storage;

pub use agent_loader::AgentLoader;
pub use agent_manifest::AgentManifest;
pub use encrypted_storage::EncryptedSecrets;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InteractionPolicy {
    Interactive,      // Prompt user for dangerous actions (default)
    HeadlessDeny,     // Auto-deny dangerous actions in non-interactive mode
    HeadlessAllowLog, // Auto-allow but log to audit file
}

impl Default for InteractionPolicy {
    fn default() -> Self {
        Self::Interactive
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub default_provider: String,
    pub providers: Providers,
    #[serde(default)]
    pub llm: Option<crate::llm::config::LLMConfig>,
    #[serde(default)]
    pub interaction_policy: InteractionPolicy,
    #[serde(default)]
    pub audit_log_path: Option<String>,
    #[serde(default)]
    pub mcp: Option<McpConfig>,
    #[serde(default)]
    pub browser: Option<BrowserConfig>,
    #[serde(default = "default_context_token_limit")]
    pub context_token_limit: usize,
    #[serde(default)]
    pub session: SessionConfig,
}

fn default_context_token_limit() -> usize {
    32_000
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DmScope {
    Main,
    PerPeer,
    PerChannelPeer,
}

impl Default for DmScope {
    fn default() -> Self {
        Self::Main
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub dm_scope: DmScope,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            dm_scope: DmScope::Main,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Providers {
    #[serde(default)]
    pub openrouter: Option<OpenRouterConfig>,
    #[serde(default)]
    pub antigravity: Option<AntigravityConfig>,
    #[serde(default)]
    pub openai: Option<OpenAIConfig>,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub teams: Option<TeamsConfig>,
    #[serde(default)]
    pub google_chat: Option<GoogleChatConfig>,
    #[serde(default)]
    pub google: Option<GoogleConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoogleConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramConfig {
    pub bot_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TeamsConfig {
    pub webhook_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoogleChatConfig {
    pub webhook_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub servers: Vec<crate::mcp::McpServerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BrowserConfig {
    #[serde(default = "default_headless")]
    pub headless: bool,
    #[serde(default)]
    pub user_data_dir: Option<String>,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub use_docker: bool,
    #[serde(default = "default_docker_image")]
    pub docker_image: String,
    #[serde(default = "default_docker_port")]
    pub docker_port: u16,
}

fn default_docker_image() -> String {
    "zenika/alpine-chrome:with-puppeteer".to_string()
}

fn default_docker_port() -> u16 {
    9222
}

fn default_headless() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenRouterConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_keys: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AntigravityConfig {
    #[serde(default)]
    pub api_key: Option<String>, // Google AI Studio API key
    #[serde(default)]
    pub api_keys: Option<Vec<String>>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub fallback_base_urls: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIConfig {
    #[serde(default)]
    pub api_key: Option<String>, // OpenAI API key
    #[serde(default)]
    pub api_keys: Option<Vec<String>>,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let config_path = Self::config_path();

        if !config_path.exists() {
            return Err(anyhow::anyhow!(
                "Config file not found at {:?}. Please create config.toml",
                config_path
            ));
        }

        let contents = fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn config_path() -> PathBuf {
        PathBuf::from("config.toml")
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::config_path();
        let contents = toml::to_string_pretty(self)?;
        fs::write(config_path, contents)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuthTokens {
    pub tokens: HashMap<String, ProviderToken>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

impl OAuthTokens {
    pub fn load() -> anyhow::Result<Self> {
        let token_path = Self::token_path();

        if !token_path.exists() {
            return Ok(Self {
                tokens: HashMap::new(),
            });
        }

        let contents = fs::read_to_string(&token_path)?;
        let tokens: OAuthTokens = serde_json::from_str(&contents)?;
        Ok(tokens)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let token_path = Self::token_path();

        if let Some(parent) = token_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = serde_json::to_string_pretty(self)?;
        fs::write(&token_path, contents)?;
        Ok(())
    }

    pub fn token_path() -> PathBuf {
        PathBuf::from(".").join(".nanobot").join("tokens.json")
    }

    pub fn get(&self, provider: &str) -> Option<&ProviderToken> {
        self.tokens.get(provider)
    }

    pub fn set(&mut self, provider: String, token: ProviderToken) {
        self.tokens.insert(provider, token);
    }
}
