use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub slack: Option<SlackConfig>,
    #[serde(default)]
    pub discord: Option<DiscordConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoogleConfig {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_keys: Option<Vec<String>>,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub oauth_client_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    #[serde(default)]
    pub allowed_users: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlackConfig {
    pub bot_token: String,
    #[serde(default)]
    pub app_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscordConfig {
    pub token: String,
    pub app_id: String,
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
        if let Ok(custom) = std::env::var("NANOBOT_CONFIG_PATH") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }

        let cwd_path = PathBuf::from("config.toml");
        if cwd_path.exists() {
            return cwd_path;
        }

        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
            .join("config.toml")
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let config_path = Self::config_path();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
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
    fn secret_manager_from_env() -> anyhow::Result<crate::security::SecretManager> {
        let password = if let Some(password) = crate::security::read_primary_password() {
            password
        } else {
            let session = crate::security::get_or_create_session_secrets()?;
            format!("nanobot-session-sealed:{}", session.gateway_session_secret)
        };

        let salt = crate::security::SecretManager::load_or_create_salt()?;
        crate::security::SecretManager::new(&password, &salt)
    }

    pub fn load() -> anyhow::Result<Self> {
        let manager = Self::secret_manager_from_env()?;

        if let Ok(secrets) = crate::config::encrypted_storage::EncryptedSecrets::load(&manager) {
            let tokens = secrets
                .tokens
                .into_iter()
                .map(|(provider, token)| {
                    (
                        provider,
                        ProviderToken {
                            access_token: token.access_token,
                            refresh_token: token.refresh_token,
                            expires_at: token.expires_at,
                        },
                    )
                })
                .collect();

            return Ok(Self { tokens });
        }

        let token_path = Self::token_path();

        if !token_path.exists() {
            if let Some(legacy_tokens) = Self::load_legacy_bridge_tokens()? {
                return Ok(legacy_tokens);
            }
            return Ok(Self {
                tokens: HashMap::new(),
            });
        }

        let contents = fs::read_to_string(&token_path)?;
        let tokens: OAuthTokens = serde_json::from_str(&contents)?;

        let mut secrets =
            crate::config::encrypted_storage::EncryptedSecrets::load(&manager).unwrap_or_default();
        secrets.tokens = tokens
            .tokens
            .iter()
            .map(|(provider, token)| {
                (
                    provider.clone(),
                    crate::config::encrypted_storage::ProviderToken {
                        access_token: token.access_token.clone(),
                        refresh_token: token.refresh_token.clone(),
                        expires_at: token.expires_at,
                    },
                )
            })
            .collect();
        secrets.save(&manager)?;

        let _ = fs::remove_file(&token_path);

        Ok(tokens)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let manager = Self::secret_manager_from_env()?;
        let mut secrets =
            crate::config::encrypted_storage::EncryptedSecrets::load(&manager).unwrap_or_default();
        secrets.tokens = self
            .tokens
            .iter()
            .map(|(provider, token)| {
                (
                    provider.clone(),
                    crate::config::encrypted_storage::ProviderToken {
                        access_token: token.access_token.clone(),
                        refresh_token: token.refresh_token.clone(),
                        expires_at: token.expires_at,
                    },
                )
            })
            .collect();
        secrets.save(&manager)?;

        let token_path = Self::token_path();
        if token_path.exists() {
            let _ = fs::remove_file(&token_path);
        }

        let mirror_legacy_tokens = std::env::var("NANOBOT_ENABLE_LEGACY_TOKEN_MIRROR")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false);

        if mirror_legacy_tokens {
            let _ = self.sync_legacy_bridge_tokens();
        }
        Ok(())
    }

    pub fn token_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
            .join("tokens.json")
    }

    pub fn get(&self, provider: &str) -> Option<&ProviderToken> {
        self.tokens.get(provider)
    }

    pub fn set(&mut self, provider: String, token: ProviderToken) {
        self.tokens.insert(provider, token);
    }

    fn legacy_auth_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".openclaw").join("auth"))
    }

    fn legacy_tokens_path() -> Option<PathBuf> {
        Self::legacy_auth_dir().map(|d| d.join("tokens.json"))
    }

    fn load_legacy_bridge_tokens() -> anyhow::Result<Option<Self>> {
        let Some(tokens_path) = Self::legacy_tokens_path() else {
            return Ok(None);
        };
        if !tokens_path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(tokens_path)?;
        let tokens: OAuthTokens = serde_json::from_str(&contents)?;
        Ok(Some(tokens))
    }

    fn sync_legacy_bridge_tokens(&self) -> anyhow::Result<()> {
        let Some(auth_dir) = Self::legacy_auth_dir() else {
            return Ok(());
        };

        self.sync_legacy_bridge_tokens_in_dir(&auth_dir)
    }

    fn sync_legacy_bridge_tokens_in_dir(&self, auth_dir: &Path) -> anyhow::Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let parent_dir = auth_dir
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Legacy auth dir must have a parent: {:?}", auth_dir))?;

        fs::create_dir_all(parent_dir)?;

        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let staged_dir = parent_dir.join(format!("auth.nanobot.stage.{}", nanos));
        let backup_dir = parent_dir.join(format!("auth.nanobot.backup.{}", nanos));

        fs::create_dir_all(&staged_dir)?;

        let tokens_bytes = serde_json::to_vec_pretty(self)?;
        fs::write(staged_dir.join("tokens.json"), tokens_bytes)?;

        let mut providers: Vec<_> = self.tokens.iter().collect();
        providers.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (provider, token) in providers {
            let provider_bytes = serde_json::to_vec_pretty(token)?;
            fs::write(
                staged_dir.join(format!("{}.json", provider)),
                provider_bytes,
            )?;
        }

        let mut had_original_dir = false;
        if auth_dir.exists() {
            fs::rename(auth_dir, &backup_dir).map_err(|e| {
                let _ = fs::remove_dir_all(&staged_dir);
                anyhow::anyhow!(
                    "Failed to prepare legacy auth mirror swap (rename to backup): {}",
                    e
                )
            })?;
            had_original_dir = true;
        }

        if let Err(e) = fs::rename(&staged_dir, auth_dir) {
            if had_original_dir {
                let _ = fs::rename(&backup_dir, auth_dir);
            }
            let _ = fs::remove_dir_all(&staged_dir);
            return Err(anyhow::anyhow!(
                "Failed to commit legacy auth mirror swap: {}",
                e
            ));
        }

        if had_original_dir {
            let _ = fs::remove_dir_all(&backup_dir);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sync_swaps_full_auth_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let auth_dir = dir.path().join("auth");
        fs::create_dir_all(&auth_dir).expect("create auth dir");

        fs::write(auth_dir.join("tokens.json"), "{}\n").expect("seed tokens");
        fs::write(auth_dir.join("stale.json"), "{\"stale\": true}\n").expect("seed stale");

        let mut oauth_tokens = OAuthTokens {
            tokens: HashMap::new(),
        };
        oauth_tokens.set(
            "google".to_string(),
            ProviderToken {
                access_token: "token-1".to_string(),
                refresh_token: Some("refresh-1".to_string()),
                expires_at: Some(123),
            },
        );

        oauth_tokens
            .sync_legacy_bridge_tokens_in_dir(&auth_dir)
            .expect("sync legacy mirror");

        assert!(auth_dir.join("tokens.json").exists());
        assert!(auth_dir.join("google.json").exists());
        assert!(!auth_dir.join("stale.json").exists());
    }
}
