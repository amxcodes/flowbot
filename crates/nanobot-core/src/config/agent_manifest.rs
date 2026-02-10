use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent manifest schema for declarative agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    pub agent: AgentMeta,
    pub identity: AgentIdentity,
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub script: Option<ScriptConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub name: String,
    pub role: String,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ChannelConfig {
    Telegram {
        token_env: String,
    },
    Terminal,
    Plugin {
        path: String,
        #[serde(default)]
        config: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub custom_plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptConfig {
    #[serde(default)]
    pub enabled: bool,
    pub source: String,
}

fn default_backend() -> String {
    "sqlite".to_string()
}

impl AgentManifest {
    /// Load manifest from TOML file
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .context(format!("Failed to read manifest: {}", path.display()))?;

        let manifest: AgentManifest =
            toml::from_str(&content).context("Failed to parse TOML manifest")?;

        Ok(manifest)
    }

    /// Validate manifest
    pub fn validate(&self) -> Result<()> {
        if self.agent.name.is_empty() {
            anyhow::bail!("Agent name cannot be empty");
        }

        if self.identity.system_prompt.is_empty() {
            anyhow::bail!("System prompt cannot be empty");
        }

        // Check for conflicting tool allow/deny
        for tool in &self.tools.allow {
            if self.tools.deny.contains(tool) {
                anyhow::bail!("Tool '{}' appears in both allow and deny lists", tool);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest() {
        let toml = r#"
[agent]
name = "test_bot"
version = "1.0.0"

[identity]
name = "Test Bot"
role = "Assistant"
system_prompt = "You are helpful."

[[channels]]
type = "terminal"

[tools]
allow = ["web_search"]

[memory]
backend = "sqlite"
namespace = "test"
"#;

        let manifest: AgentManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.agent.name, "test_bot");
        assert_eq!(manifest.channels.len(), 1);
        manifest.validate().unwrap();
    }
}
