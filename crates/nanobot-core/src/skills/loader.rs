use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
        self.skills.clear();

        // Load from lowest to highest precedence:
        // bundled < managed < workspace
        let sources = skill_source_dirs(&self.workspace_dir);
        for source in sources {
            if !source.exists() {
                log::debug!("Skills source missing: {}", source.display());
                continue;
            }

            for entry in std::fs::read_dir(&source)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    let skill_file = path.join("SKILL.md");
                    if !skill_file.exists() {
                        continue;
                    }

                    match self.load_skill(&skill_file) {
                        Ok(metadata) => {
                            if !is_skill_eligible(&metadata) {
                                log::info!(
                                    "⏭️ Skipped skill '{}' (requirements not met)",
                                    metadata.name
                                );
                                continue;
                            }
                            let skill_name = metadata.name.clone();
                            if self.skills.contains_key(&skill_name) {
                                log::info!(
                                    "↺ Skill '{}' overridden by higher-precedence source: {}",
                                    skill_name,
                                    source.display()
                                );
                            } else {
                                log::info!("📦 Loaded skill: {}", skill_name);
                            }
                            self.skills.insert(skill_name, metadata);
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

    /// Get workspace directory backing this loader
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
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

fn skill_source_dirs(workspace_dir: &Path) -> Vec<PathBuf> {
    let bundled = std::env::var("NANOBOT_BUNDLED_SKILLS_DIR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("skills"));

    let managed = std::env::var("NANOBOT_MANAGED_SKILLS_DIR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".nanobot").join("skills")))
        .unwrap_or_else(|| PathBuf::from(".nanobot").join("skills"));

    let workspace = workspace_dir.join("skills");

    let mut out = Vec::new();
    for candidate in [bundled, managed, workspace] {
        let normalized = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        if !out.iter().any(|p: &PathBuf| p == &normalized) {
            out.push(normalized);
        }
    }
    out
}

fn command_exists_quick(cmd: &str) -> bool {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return false;
    }

    if std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .is_ok()
    {
        return true;
    }

    if cfg!(windows)
        && let Ok(out) = std::process::Command::new("where").arg(cmd).output()
    {
        return out.status.success() && !out.stdout.is_empty();
    }

    false
}

fn config_path_truthy(path_expr: &str) -> bool {
    let Ok(raw) = std::fs::read_to_string("config.toml") else {
        return false;
    };
    let Ok(value) = raw.parse::<toml::Value>() else {
        return false;
    };

    let mut cursor = &value;
    for segment in path_expr.split('.') {
        let key = segment.trim();
        if key.is_empty() {
            return false;
        }
        let Some(next) = cursor.get(key) else {
            return false;
        };
        cursor = next;
    }

    if let Some(b) = cursor.as_bool() {
        return b;
    }
    if let Some(s) = cursor.as_str() {
        return !s.trim().is_empty();
    }
    if let Some(i) = cursor.as_integer() {
        return i != 0;
    }
    if let Some(f) = cursor.as_float() {
        return f != 0.0;
    }
    if let Some(arr) = cursor.as_array() {
        return !arr.is_empty();
    }
    if let Some(tbl) = cursor.as_table() {
        return !tbl.is_empty();
    }

    false
}

fn is_skill_eligible(metadata: &SkillMetadata) -> bool {
    if metadata.always {
        return true;
    }

    if !metadata.allowed_os.is_empty() {
        let current = std::env::consts::OS;
        if !metadata
            .allowed_os
            .iter()
            .any(|v| v.eq_ignore_ascii_case(current))
        {
            return false;
        }
    }

    if metadata
        .requires_bins
        .iter()
        .any(|cmd| !command_exists_quick(cmd))
    {
        return false;
    }

    if !metadata.requires_any_bins.is_empty()
        && !metadata
            .requires_any_bins
            .iter()
            .any(|cmd| command_exists_quick(cmd))
    {
        return false;
    }

    if metadata.requires_env.iter().any(|key| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
    }) {
        return false;
    }

    if metadata
        .requires_config
        .iter()
        .any(|path_expr| !config_path_truthy(path_expr))
    {
        return false;
    }

    true
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

        // We should find the test skill (may find more from other sources like ~/.nanobot/skills)
        assert!(
            loader.get_skill("test").is_some(),
            "Test skill should be loaded"
        );

        Ok(())
    }

    #[test]
    fn test_precedence_workspace_overrides_managed_and_bundled() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let workspace = temp_dir.path().join("workspace");
        let bundled = temp_dir.path().join("bundled");
        let managed = temp_dir.path().join("managed");

        fs::create_dir_all(workspace.join("skills").join("github"))?;
        fs::create_dir_all(bundled.join("github"))?;
        fs::create_dir_all(managed.join("github"))?;

        fs::write(
            bundled.join("github").join("SKILL.md"),
            "---\nname: github\ndescription: bundled\n---\n\n## Tools Provided\n- `x`: x\n",
        )?;
        fs::write(
            managed.join("github").join("SKILL.md"),
            "---\nname: github\ndescription: managed\n---\n\n## Tools Provided\n- `x`: x\n",
        )?;
        fs::write(
            workspace.join("skills").join("github").join("SKILL.md"),
            "---\nname: github\ndescription: workspace\n---\n\n## Tools Provided\n- `x`: x\n",
        )?;

        unsafe {
            std::env::set_var(
                "NANOBOT_BUNDLED_SKILLS_DIR",
                bundled.to_string_lossy().to_string(),
            );
            std::env::set_var(
                "NANOBOT_MANAGED_SKILLS_DIR",
                managed.to_string_lossy().to_string(),
            );
        }

        let mut loader = SkillLoader::new(workspace);
        loader.scan()?;
        let skill = loader.get_skill("github").expect("github loaded");
        assert_eq!(skill.description, "workspace");

        unsafe {
            std::env::remove_var("NANOBOT_BUNDLED_SKILLS_DIR");
            std::env::remove_var("NANOBOT_MANAGED_SKILLS_DIR");
        }
        Ok(())
    }
}
