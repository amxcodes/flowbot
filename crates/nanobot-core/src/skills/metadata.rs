use serde::{Deserialize, Serialize};
// use std::collections::HashMap;

/// Metadata for a skill, parsed from SKILL.md
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Skill name (e.g., "github", "notion")
    pub name: String,
    
    /// Category: automation, integration, productivity, custom
    pub category: String,
    
    /// Status: active, experimental, deprecated
    pub status: String,
    
    /// Author (optional)
    pub author: Option<String>,
    
    /// Brief description
    pub description: String,
    
    /// List of tools provided by this skill
    pub tools: Vec<SkillTool>,
    
    /// Dependencies (crates, binaries, etc.)
    pub dependencies: Vec<String>,
    
    /// Whether this skill is enabled
    pub enabled: bool,
    
    /// Configuration schema (TOML format)
    pub config_schema: Option<String>,
    
    /// Path to SKILL.md file
    pub skill_path: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTool {
    /// Tool name
    pub name: String,
    
    /// Tool description
    pub description: String,
    
    /// Command template (for external binary tools)
    pub command: Option<String>,
    
    /// Schema/parameters (optional JSON schema)
    pub schema: Option<String>,
}

impl SkillMetadata {
    /// Parse a SKILL.md file and extract metadata
    pub fn from_markdown(path: std::path::PathBuf, content: &str) -> anyhow::Result<Self> {
        use pulldown_cmark::{Parser, Event, Tag, TagEnd};
        
        let mut name = path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        let mut description = String::new();
        let mut category = String::from("custom");
        let mut status = String::from("active");
        let mut author = None;
        let mut tools = Vec::new();
        let dependencies = Vec::new();
        
        // Simple frontmatter parsing (YAML-like)
        if content.starts_with("---") {
            if let Some(end_idx) = content[3..].find("---") {
                let frontmatter = &content[3..end_idx + 3];
                
                // Parse key-value pairs
                for line in frontmatter.lines() {
                    if let Some((key, value)) = line.split_once(':') {
                        let key = key.trim();
                        let value = value.trim().trim_matches('"');
                        
                        match key {
                            "name" => name = value.to_string(),
                            "description" => description = value.to_string(),
                            "category" => category = value.to_string(),
                            "status" => status = value.to_string(),
                            "author" => author = Some(value.to_string()),
                            _ => {}
                        }
                    }
                }
            }
        }
        
        // Parse markdown for tool definitions
        let parser = Parser::new(content);
        let mut in_tools_section = false;
        let mut current_text = String::new();
        
        for event in parser {
            match event {
                Event::Start(Tag::Heading { level: _, .. }) => {
                    current_text.clear();
                }
                Event::End(TagEnd::Heading(_)) => {
                    if current_text.contains("Tools Provided") {
                        in_tools_section = true;
                    } else {
                        in_tools_section = false;
                    }
                    current_text.clear();
                }
                Event::Start(Tag::Item) if in_tools_section => {
                    current_text.clear();
                }
                Event::End(TagEnd::Item) if in_tools_section => {
                    // Parse tool item: `tool_name`: description
                    if let Some((tool_name, tool_desc)) = current_text.split_once(':') {
                        let tool_name = tool_name.trim().trim_matches('`').to_string();
                        let tool_desc = tool_desc.trim().to_string();
                        
                        tools.push(SkillTool {
                            name: tool_name,
                            description: tool_desc,
                            command: None,
                            schema: None,
                        });
                    }
                    current_text.clear();
                }
                Event::Text(text) => {
                    current_text.push_str(&text);
                }
                Event::Code(code) => {
                    current_text.push('`');
                    current_text.push_str(&code);
                    current_text.push('`');
                }
                _ => {}
            }
        }
        
        Ok(SkillMetadata {
            name,
            category,
            status,
            author,
            description,
            tools,
            dependencies,
            enabled: true, // Default to enabled
            config_schema: None,
            skill_path: path,
        })
    }
    
    /// Check if this skill has a specific tool
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.iter().any(|t| t.name == tool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_simple_skill() {
        let content = r#"---
name: test_skill
description: "A test skill"
category: custom
status: active
---

# Test Skill

This is a test.

## Tools Provided

- `test_tool`: Does testing things
- `another_tool`: Does more testing
"#;
        
        let metadata = SkillMetadata::from_markdown(
            std::path::PathBuf::from("/skills/test/SKILL.md"),
            content
        ).unwrap();
        
        assert_eq!(metadata.name, "test_skill");
        assert_eq!(metadata.category, "custom");
        assert_eq!(metadata.tools.len(), 2);
        assert_eq!(metadata.tools[0].name, "test_tool");
    }
}
