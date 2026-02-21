use crate::security::secrets::SecretManager;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Encrypted secrets storage
/// Replaces plain text tokens.json and sensitive config fields
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EncryptedSecrets {
    /// OAuth tokens (encrypted)
    pub tokens: HashMap<String, ProviderToken>,
    /// API keys (encrypted)
    pub api_keys: HashMap<String, String>,
    /// Skill credentials keyed by skill -> key -> value
    #[serde(default)]
    pub skill_credentials: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

impl EncryptedSecrets {
    fn legacy_secrets_path() -> PathBuf {
        PathBuf::from(".").join(".nanobot").join("secrets.enc")
    }

    /// Load encrypted secrets (requires master password)
    pub fn load(manager: &SecretManager) -> Result<Self> {
        let path = Self::secrets_path();

        let read_path = if path.exists() {
            path
        } else {
            let legacy = Self::legacy_secrets_path();
            if legacy.exists() {
                legacy
            } else {
                return Ok(Self::default());
            }
        };

        let encrypted_data = fs::read_to_string(&read_path)?;
        let decrypted_json = manager.decrypt(&encrypted_data)?;
        let secrets: EncryptedSecrets = serde_json::from_str(&decrypted_json)?;

        Ok(secrets)
    }

    /// Save encrypted secrets
    pub fn save(&self, manager: &SecretManager) -> Result<()> {
        let path = Self::secrets_path();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;
        let encrypted = manager.encrypt(&json)?;
        fs::write(&path, encrypted)?;

        Ok(())
    }

    /// Get secrets file path
    pub fn secrets_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
            .join("secrets.enc")
    }

    /// Get API key for a provider
    pub fn get_api_key(&self, provider: &str) -> Option<&String> {
        self.api_keys.get(provider)
    }

    /// Set API key for a provider
    pub fn set_api_key(&mut self, provider: String, api_key: String) {
        self.api_keys.insert(provider, api_key);
    }

    /// Get OAuth token for a provider
    pub fn get_token(&self, provider: &str) -> Option<&ProviderToken> {
        self.tokens.get(provider)
    }

    /// Set OAuth token for a provider
    pub fn set_token(&mut self, provider: String, token: ProviderToken) {
        self.tokens.insert(provider, token);
    }

    pub fn get_skill_credential(&self, skill: &str, key: &str) -> Option<&String> {
        self.skill_credentials.get(skill)?.get(key)
    }

    pub fn set_skill_credential(&mut self, skill: &str, key: &str, value: String) {
        self.skill_credentials
            .entry(skill.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Migrate from plain text tokens.json
    pub fn migrate_from_plaintext(manager: &SecretManager) -> Result<()> {
        let old_tokens_path = PathBuf::from(".").join(".nanobot").join("tokens.json");

        if !old_tokens_path.exists() {
            return Ok(()); // Nothing to migrate
        }

        // Load old tokens
        let old_data = fs::read_to_string(&old_tokens_path)?;
        let old_tokens: HashMap<String, ProviderToken> = serde_json::from_str(&old_data)?;

        // Create new encrypted secrets
        let secrets = EncryptedSecrets {
            tokens: old_tokens,
            ..EncryptedSecrets::default()
        };

        // Save encrypted
        secrets.save(manager)?;

        // Rename old file (keep as backup)
        let backup_path = old_tokens_path.with_extension("json.bak");
        fs::rename(&old_tokens_path, &backup_path)?;

        tracing::info!("Migrated tokens.json to encrypted storage");
        tracing::info!("Backup saved to: {:?}", backup_path);

        Ok(())
    }

    /// Migrate API keys from config.toml
    pub fn migrate_api_keys_from_config(
        manager: &SecretManager,
        config_path: &PathBuf,
    ) -> Result<()> {
        if !config_path.exists() {
            return Ok(());
        }

        let config_content = fs::read_to_string(config_path)?;
        let config: toml::Value = toml::from_str(&config_content)?;

        let mut secrets = Self::load(manager).unwrap_or_default();
        let mut modified = false;

        // Extract API keys from providers section
        if let Some(providers) = config.get("providers").and_then(|v| v.as_table()) {
            for (provider_name, provider_config) in providers {
                if let Some(api_key) = provider_config.get("api_key").and_then(|v| v.as_str())
                    && !api_key.is_empty()
                {
                    secrets.set_api_key(provider_name.clone(), api_key.to_string());
                    modified = true;
                }
            }
        }

        if modified {
            secrets.save(manager)?;
            tracing::info!("Migrated API keys from config.toml to encrypted storage");
            tracing::info!("You should now remove api_key fields from config.toml");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypted_secrets_roundtrip() {
        let salt = SecretManager::generate_salt();
        let manager = SecretManager::new("test-password", &salt).unwrap();

        let mut secrets = EncryptedSecrets::default();
        secrets.set_api_key("openai".to_string(), "sk-test-123".to_string());
        secrets.set_token(
            "google".to_string(),
            ProviderToken {
                access_token: "ya29.test".to_string(),
                refresh_token: Some("1//refresh".to_string()),
                expires_at: Some(1234567890),
            },
        );

        let json = serde_json::to_string(&secrets).unwrap();
        let encrypted = manager.encrypt(&json).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        let restored: EncryptedSecrets = serde_json::from_str(&decrypted).unwrap();

        assert_eq!(
            restored.get_api_key("openai"),
            Some(&"sk-test-123".to_string())
        );
        assert_eq!(
            restored.get_token("google").unwrap().access_token,
            "ya29.test"
        );
    }
}
