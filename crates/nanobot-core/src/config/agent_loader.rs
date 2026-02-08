use anyhow::Result;
use crate::config::AgentManifest;
use std::path::Path;

/// Agent loader that builds runtime agents from manifests
pub struct AgentLoader;

impl AgentLoader {
    /// Load manifest from file
    pub fn load(path: &Path) -> Result<AgentManifest> {
        AgentManifest::load(path)
    }
    
    /// Validate manifest
    pub fn validate(manifest: &AgentManifest) -> Result<()> {
        manifest.validate()
    }
    
    /// Display manifest info
    pub fn info(manifest: &AgentManifest) {
        println!("📋 Agent Manifest");
        println!("   Name: {}", manifest.agent.name);
        println!("   Version: {}", manifest.agent.version);
        println!("   Identity: {} ({})", manifest.identity.name, manifest.identity.role);
        println!("   Channels: {}", manifest.channels.len());
        println!("   Tools: {} allowed, {} denied", 
            manifest.tools.allow.len(),
            manifest.tools.deny.len()
        );
        if let Some(ref ns) = manifest.memory.namespace {
            println!("   Memory: {} (namespace: {})", manifest.memory.backend, ns);
        } else {
            println!("   Memory: {}", manifest.memory.backend);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_load_manifest() {
        // Test manifest loading logic
        let toml = r#"
[agent]
name = "test"
version = "1.0.0"

[identity]
name = "Test"
role = "Bot"
system_prompt = "Test prompt"

[tools]
allow = ["test_tool"]
"#;
        
        let manifest: AgentManifest = toml::from_str(toml).unwrap();
        AgentLoader::validate(&manifest).unwrap();
    }
}
