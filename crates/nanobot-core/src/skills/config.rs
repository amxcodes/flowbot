// Skills configuration module
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

const ENCRYPTED_MARKER: &str = "__encrypted__";

/// Skill configuration with API credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    pub enabled: bool,
    #[serde(default)]
    pub credentials: HashMap<String, String>,
}

/// Global skills configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillsConfig {
    #[serde(default)]
    pub skills: HashMap<String, SkillConfig>,
    #[serde(default)]
    pub runtime_overrides: HashMap<String, String>,
}

impl SkillsConfig {
    fn secret_manager_from_env() -> Result<crate::security::SecretManager> {
        let password = if let Some(password) = crate::security::read_primary_password() {
            password
        } else {
            let session = crate::security::get_or_create_session_secrets()?;
            format!("nanobot-session-sealed:{}", session.gateway_session_secret)
        };

        let salt = crate::security::SecretManager::load_or_create_salt()?;
        crate::security::SecretManager::new(&password, &salt)
    }

    fn hydrate_credentials_from_encrypted_store(&mut self) -> Result<()> {
        let manager = Self::secret_manager_from_env()?;
        let secrets =
            crate::config::encrypted_storage::EncryptedSecrets::load(&manager).unwrap_or_default();

        for (skill, creds) in &secrets.skill_credentials {
            let skill_cfg = self
                .skills
                .entry(skill.to_string())
                .or_insert_with(|| SkillConfig {
                    enabled: false,
                    credentials: HashMap::new(),
                });
            for (key, value) in creds {
                skill_cfg.credentials.insert(key.clone(), value.clone());
            }
        }

        Ok(())
    }

    fn persist_credentials_to_encrypted_store(&self) -> Result<()> {
        let manager = Self::secret_manager_from_env()?;
        let mut secrets =
            crate::config::encrypted_storage::EncryptedSecrets::load(&manager).unwrap_or_default();

        for (skill, cfg) in &self.skills {
            for (key, value) in &cfg.credentials {
                if !value.trim().is_empty() && value != ENCRYPTED_MARKER {
                    secrets.set_skill_credential(skill, key, value.clone());
                }
            }
        }

        secrets.save(&manager)
    }

    /// Load from config file
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let mut cfg: Self = toml::from_str(&content)?;
        let had_plaintext_credentials = cfg.skills.values().any(|s| {
            s.credentials
                .values()
                .any(|v| !v.trim().is_empty() && v.as_str() != ENCRYPTED_MARKER)
        });

        let _ = cfg.hydrate_credentials_from_encrypted_store();
        if had_plaintext_credentials {
            let _ = cfg.save();
        }

        Ok(cfg)
    }

    /// Save to config file
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        // Create parent directory
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let _ = self.persist_credentials_to_encrypted_store();

        let mut persisted = self.clone();
        for cfg in persisted.skills.values_mut() {
            for value in cfg.credentials.values_mut() {
                if !value.trim().is_empty() {
                    *value = ENCRYPTED_MARKER.to_string();
                }
            }
        }

        let content = toml::to_string_pretty(&persisted)?;
        std::fs::write(&config_path, content)?;

        Ok(())
    }

    /// Get config file path
    fn config_path() -> Result<PathBuf> {
        let config_dir =
            dirs::config_dir().ok_or_else(|| anyhow::anyhow!("No config directory found"))?;
        Ok(config_dir.join("nanobot").join("skills.toml"))
    }

    /// Enable a skill
    pub fn enable_skill(&mut self, name: &str) {
        self.skills
            .entry(name.to_string())
            .or_insert_with(|| SkillConfig {
                enabled: true,
                credentials: HashMap::new(),
            })
            .enabled = true;
    }

    /// Disable a skill
    pub fn disable_skill(&mut self, name: &str) {
        if let Some(config) = self.skills.get_mut(name) {
            config.enabled = false;
        }
    }

    /// Set credential for a skill
    pub fn set_credential(&mut self, skill: &str, key: &str, value: String) {
        self.skills
            .entry(skill.to_string())
            .or_insert_with(|| SkillConfig {
                enabled: false,
                credentials: HashMap::new(),
            })
            .credentials
            .insert(key.to_string(), value);
    }

    /// Get credential for a skill
    pub fn get_credential(&self, skill: &str, key: &str) -> Option<String> {
        self.skills
            .get(skill)
            .and_then(|cfg| cfg.credentials.get(key))
            .filter(|v| !v.trim().is_empty() && v.as_str() != ENCRYPTED_MARKER)
            .cloned()
            .or_else(|| {
                let manager = Self::secret_manager_from_env().ok()?;
                let secrets =
                    crate::config::encrypted_storage::EncryptedSecrets::load(&manager).ok()?;
                secrets.get_skill_credential(skill, key).cloned()
            })
    }

    /// Check if skill is enabled
    pub fn is_enabled(&self, skill: &str) -> bool {
        self.skills.get(skill).map(|c| c.enabled).unwrap_or(false)
    }

    /// Get all enabled skills
    pub fn enabled_skills(&self) -> Vec<String> {
        self.skills
            .iter()
            .filter(|(_, config)| config.enabled)
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn set_runtime_override(&mut self, skill: &str, runtime: &str) {
        self.runtime_overrides
            .insert(skill.to_string(), runtime.trim().to_ascii_lowercase());
    }

    pub fn runtime_override(&self, skill: &str) -> Option<&str> {
        self.runtime_overrides.get(skill).map(|s| s.as_str())
    }
}

pub fn known_required_credentials(skill: &str) -> Vec<String> {
    match skill.trim().to_ascii_lowercase().as_str() {
        "weather" => vec!["api_key".to_string()],
        "notion" => vec!["api_key".to_string()],
        "spotify" => vec!["client_id".to_string(), "client_secret".to_string()],
        _ => Vec::new(),
    }
}

pub fn required_credentials_from_schema(schema: Option<&str>) -> Vec<String> {
    let Some(raw) = schema else {
        return Vec::new();
    };

    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    if let Some(props) = parsed.get("properties") {
        if let Some(obj) = props.as_object() {
            for (k, v) in obj {
                if v.get("required").and_then(|x| x.as_bool()).unwrap_or(false) {
                    out.push(k.to_string());
                }
            }
        } else if let Some(arr) = props.as_array() {
            for item in arr {
                let required = item
                    .get("required")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                if required && let Some(key) = item.get("key").and_then(|x| x.as_str()) {
                    out.push(key.to_string());
                }
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

/// Skill setup wizard
pub struct SkillSetupWizard {
    config: SkillsConfig,
}

impl SkillSetupWizard {
    pub fn new() -> Result<Self> {
        let config = SkillsConfig::load()?;
        Ok(Self { config })
    }

    /// Interactive setup for a skill
    pub fn setup_skill(&mut self, skill_name: &str) -> Result<()> {
        use inquire::{Confirm, Text};

        println!("\n🔧 Setting up {} skill", skill_name);

        match skill_name {
            "github" => {
                println!("GitHub CLI (gh) must be installed and authenticated.");
                println!("Run: gh auth login");

                let enable = Confirm::new("Enable GitHub skill?")
                    .with_default(true)
                    .prompt()?;

                if enable {
                    self.config.enable_skill("github");
                }
            }

            "weather" => {
                println!("Get a free API key from: https://openweathermap.org/api");

                let api_key = Text::new("OpenWeather API Key:").prompt()?;

                self.config.set_credential("weather", "api_key", api_key);
                self.config.enable_skill("weather");
            }

            "notion" => {
                println!("Create an integration at: https://www.notion.so/my-integrations");

                let api_key = Text::new("Notion API Key:").prompt()?;

                self.config.set_credential("notion", "api_key", api_key);
                self.config.enable_skill("notion");
            }

            "spotify" => {
                println!("Create an app at: https://developer.spotify.com/dashboard");

                let client_id = Text::new("Spotify Client ID:").prompt()?;
                let client_secret = Text::new("Spotify Client Secret:").prompt()?;

                self.config
                    .set_credential("spotify", "client_id", client_id);
                self.config
                    .set_credential("spotify", "client_secret", client_secret);
                self.config.enable_skill("spotify");
            }

            "calendar" => {
                println!("Use OAuth to authorize Google Calendar access.");
                println!("Run: nanobot login google-calendar");

                let enable = Confirm::new("Enable Calendar skill (requires OAuth)?")
                    .with_default(true)
                    .prompt()?;

                if enable {
                    self.config.enable_skill("calendar");
                }
            }

            _ => {
                anyhow::bail!("Unknown skill: {}", skill_name);
            }
        }

        Ok(())
    }

    /// Save configuration
    pub fn save(&self) -> Result<()> {
        self.config.save()
    }

    /// Get current config
    pub fn config(&self) -> &SkillsConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skills_config() {
        let mut config = SkillsConfig::default();

        config.enable_skill("github");
        assert!(config.is_enabled("github"));

        config.set_credential("weather", "api_key", "test123".to_string());
        assert_eq!(
            config.get_credential("weather", "api_key"),
            Some("test123".to_string())
        );

        config.disable_skill("github");
        assert!(!config.is_enabled("github"));

        config.set_runtime_override("gog", "node");
        assert_eq!(config.runtime_override("gog"), Some("node"));
    }
}
