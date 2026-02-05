use anyhow::Result;
use std::path::{Path, PathBuf};
use std::collections::HashMap;

use super::metadata::SkillMetadata;

/// Skill loader - scans workspace for SKILL.md files
pub struct SkillLoader {
    workspace_dir: PathBuf,
    skills: HashMap<String, SkillMetadata>,
}

impl SkillLoader {
    /// Create a new skill loader for the given workspace
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            workspace_dir,
            skills: HashMap::new(),
        }
    }
    
    /// Scan the workspace/skills/ directory for SKILL.md files
    pub fn scan(&mut self) -> Result<()> {
        let skills_dir = self.workspace_dir.join("skills");
        
        if !skills_dir.exists() {
            log::debug!("Skills directory does not exist: {}", skills_dir.display());
            return Ok(());
        }
        
        // Scan for SKILL.md files
        for entry in std::fs::read_dir(&skills_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_dir() {
                let skill_file = path.join("SKILL.md");
                if skill_file.exists() {
                    match self.load_skill(&skill_file) {
                        Ok(metadata) => {
                            log::info!("📦 Loaded skill: {}", metadata.name);
                            self.skills.insert(metadata.name.clone(), metadata);
                        }
                        Err(e) => {
                            log::warn!("Failed to load skill from {}: {}", skill_file.display(), e);
                        }
                    }
                }
            }
        }
        
        log::info!("Loaded {} skills", self.skills.len());
        Ok(())
    }
    
    /// Load a single skill from a SKILL.md file
    fn load_skill(&self, skill_path: &Path) -> Result<SkillMetadata> {
        let content = std::fs::read_to_string(skill_path)?;
        SkillMetadata::from_markdown(skill_path.to_path_buf(), &content)
    }
    
    /// Get all loaded skills
    pub fn skills(&self) -> &HashMap<String, SkillMetadata> {
        &self.skills
    }
    
    /// Get a specific skill by name
    pub fn get_skill(&self, name: &str) -> Option<&SkillMetadata> {
        self.skills.get(name)
    }
    
    /// Get all enabled skills
    pub fn enabled_skills(&self) -> impl Iterator<Item = &SkillMetadata> {
        self.skills.values().filter(|s| s.enabled)
    }
    
    /// Enable a skill
    pub fn enable_skill(&mut self, name: &str) -> Result<()> {
        if let Some(skill) = self.skills.get_mut(name) {
            skill.enabled = true;
            log::info!("✓ Enabled skill: {}", name);
            Ok(())
        } else {
            anyhow::bail!("Skill not found: {}", name)
        }
    }
    
    /// Disable a skill
    pub fn disable_skill(&mut self, name: &str) -> Result<()> {
        if let Some(skill) = self.skills.get_mut(name) {
            skill.enabled = false;
            log::info!("✓ Disabled skill: {}", name);
            Ok(())
        } else {
            anyhow::bail!("Skill not found: {}", name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    
    #[test]
    fn test_skill_loader() -> Result<()> {
        // Create temp directory
        let temp_dir = tempfile::tempdir()?;
        let workspace = temp_dir.path();
        
        // Create skills directory
        let skills_dir = workspace.join("skills");
        fs::create_dir_all(&skills_dir)?;
        
        // Create a test skill
        let test_skill_dir = skills_dir.join("test");
        fs::create_dir_all(&test_skill_dir)?;
        
        let skill_content = r#"---
name: test
description: "Test skill"
---

# Test Skill

## Tools Provided

- `test_tool`: A test tool
"#;
        fs::write(test_skill_dir.join("SKILL.md"), skill_content)?;
        
        // Load skills
        let mut loader = SkillLoader::new(workspace.to_path_buf());
        loader.scan()?;
        
        assert_eq!(loader.skills().len(), 1);
        assert!(loader.get_skill("test").is_some());
        
        Ok(())
    }
}
