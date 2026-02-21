use anyhow::Result;
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone)]
pub struct PersonalityContext {
    pub soul: SoulData,
    pub identity: IdentityData,
    pub user: UserData,
}

#[derive(Debug, Clone)]
pub struct SoulData {
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct IdentityData {
    pub name: String,
    pub creature: String,
    pub vibe: String,
    pub emoji: String,
}

#[derive(Debug, Clone)]
pub struct UserData {
    pub name: String,
    pub call_them: String,
    pub timezone: String,
    pub context: String,
}

impl PersonalityContext {
    /// Load personality files from workspace directory
    pub async fn load(workspace_dir: &Path) -> Result<Self> {
        let soul = Self::load_soul(workspace_dir).await?;
        let identity = Self::load_identity(workspace_dir).await?;
        let user = Self::load_user(workspace_dir).await?;

        Ok(Self {
            soul,
            identity,
            user,
        })
    }

    async fn load_soul(workspace_dir: &Path) -> Result<SoulData> {
        let path = workspace_dir.join("SOUL.md");
        let content = fs::read_to_string(&path).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to load SOUL.md: {}. Run 'nanobot setup --wizard' first.",
                e
            )
        })?;
        Ok(SoulData { content })
    }

    async fn load_identity(workspace_dir: &Path) -> Result<IdentityData> {
        let path = workspace_dir.join("IDENTITY.md");
        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load IDENTITY.md: {}", e))?;

        // Parse IDENTITY.md - simple field extraction
        let name = extract_field(&content, "Name").unwrap_or("Assistant".to_string());
        let creature = extract_field(&content, "Creature").unwrap_or("AI".to_string());
        let vibe = extract_field(&content, "Vibe").unwrap_or("helpful".to_string());
        let emoji = extract_field(&content, "Emoji").unwrap_or("🤖".to_string());

        Ok(IdentityData {
            name,
            creature,
            vibe,
            emoji,
        })
    }

    async fn load_user(workspace_dir: &Path) -> Result<UserData> {
        let path = workspace_dir.join("USER.md");
        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load USER.md: {}", e))?;

        let name = extract_field(&content, "Name").unwrap_or("User".to_string());
        let call_them =
            extract_field(&content, "What to call them").unwrap_or_else(|| name.clone());
        let timezone = extract_field(&content, "Timezone").unwrap_or("UTC".to_string());
        let context = extract_section(&content, "Context").unwrap_or_default();

        Ok(UserData {
            name,
            call_them,
            timezone,
            context,
        })
    }

    /// Generate preamble text to inject into agent
    pub fn to_preamble(&self) -> String {
        format!(
            "{}\n\n# Your Identity\nYou are {} ({}) {}\nVibe: {}\n\n# About Your User\nName: {}\nTimezone: {}{}",
            self.soul.content,
            self.identity.name,
            self.identity.creature,
            self.identity.emoji,
            self.identity.vibe,
            self.user.call_them,
            self.user.timezone,
            if !self.user.context.is_empty() {
                format!(
                    "\n\nContext about {}:\n{}",
                    self.user.call_them, self.user.context
                )
            } else {
                String::new()
            }
        )
    }

    /// Get agent name for display
    pub fn agent_name(&self) -> &str {
        &self.identity.name
    }

    /// Get agent emoji for display
    pub fn agent_emoji(&self) -> &str {
        &self.identity.emoji
    }
}

/// Extract a field value from markdown like "**Field:** value"
fn extract_field(content: &str, field: &str) -> Option<String> {
    content
        .lines()
        .find(|line| line.contains(&format!("**{}:**", field)))
        .and_then(|line| {
            // Split by ** and get the part after the field
            line.split("**").nth(2).map(|s| s.trim().to_string())
        })
}

/// Extract section content after "## Section" heading
fn extract_section(content: &str, section: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("##") && l.contains(section))?;

    let mut section_content = String::new();
    for line in &lines[start + 1..] {
        if line.starts_with("##") || line.starts_with("---") {
            break;
        }
        if !line.trim().is_empty() || !section_content.is_empty() {
            section_content.push_str(line);
            section_content.push('\n');
        }
    }

    Some(section_content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_field() {
        let content = "- **Name:** Flowbot\n- **Emoji:** 🤖";
        assert_eq!(extract_field(content, "Name"), Some("Flowbot".to_string()));
        assert_eq!(extract_field(content, "Emoji"), Some("🤖".to_string()));
    }

    #[test]
    fn test_extract_section() {
        let content = "## Context\nSome context here\nMore context\n## Another Section";
        let result = extract_section(content, "Context").unwrap();
        assert!(result.contains("Some context"));
        assert!(result.contains("More context"));
    }
}
