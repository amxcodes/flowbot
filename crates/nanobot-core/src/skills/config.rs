// Skills configuration module
use serde::{Deserialize, Serialize};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

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
}

impl SkillsConfig {
    /// Load from config file
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        
        if !config_path.exists() {
            return Ok(Self::default());
        }
        
        let content = std::fs::read_to_string(&config_path)?;
        Ok(toml::from_str(&content)?)
    }
    
    /// Save to config file
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;
        
        // Create parent directory
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        
        Ok(())
    }
    
    /// Get config file path
    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("No config directory found"))?;
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
    pub fn get_credential(&self, skill: &str, key: &str) -> Option<&String> {
        self.skills.get(skill)?.credentials.get(key)
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
        use inquire::{Text, Confirm};
        
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
                
                let api_key = Text::new("OpenWeather API Key:")
                    .prompt()?;
                
                self.config.set_credential("weather", "api_key", api_key);
                self.config.enable_skill("weather");
            }
            
            "notion" => {
                println!("Create an integration at: https://www.notion.so/my-integrations");
                
                let api_key = Text::new("Notion API Key:")
                    .prompt()?;
                
                self.config.set_credential("notion", "api_key", api_key);
                self.config.enable_skill("notion");
            }
            
            "spotify" => {
                println!("Create an app at: https://developer.spotify.com/dashboard");
                
                let client_id = Text::new("Spotify Client ID:")
                    .prompt()?;
                let client_secret = Text::new("Spotify Client Secret:")
                    .prompt()?;
                
                self.config.set_credential("spotify", "client_id", client_id);
                self.config.set_credential("spotify", "client_secret", client_secret);
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
        assert_eq!(config.get_credential("weather", "api_key"), Some(&"test123".to_string()));
        
        config.disable_skill("github");
        assert!(!config.is_enabled("github"));
    }
}
