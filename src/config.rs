use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub default_provider: String,
    pub providers: Providers,
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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramConfig {
    pub bot_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenRouterConfig {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AntigravityConfig {
    #[serde(default)]
    pub api_key: String, // Google AI Studio API key
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub fallback_base_urls: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIConfig {
    #[serde(default)]
    pub api_key: String, // OpenAI API key (for API access, not Plus subscription)
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
        PathBuf::from(".")
            .join(".nanobot")
            .join("tokens.json")
    }

    pub fn get(&self, provider: &str) -> Option<&ProviderToken> {
        self.tokens.get(provider)
    }

    pub fn set(&mut self, provider: String, token: ProviderToken) {
        self.tokens.insert(provider, token);
    }
}
