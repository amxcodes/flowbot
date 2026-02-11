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

    /// Execution backend: native | mcp | deno
    pub backend: String,

    /// Optional MCP server name to register/call
    pub mcp_server_name: Option<String>,

    /// Optional MCP server command for sidecar startup
    pub mcp_command: Option<String>,

    /// Optional MCP server args for sidecar startup
    pub mcp_args: Vec<String>,

    /// Optional MCP env vars for sidecar startup
    pub mcp_env: std::collections::HashMap<String, String>,

    /// Optional Deno command (defaults to "deno")
    pub deno_command: Option<String>,

    /// Optional Deno script path for execution
    pub deno_script: Option<String>,

    /// Optional Deno args for startup
    pub deno_args: Vec<String>,

    /// Optional Deno sandbox profile: strict | balanced | permissive
    pub deno_sandbox: Option<String>,

    /// Optional extra Deno permission flags (for example: --allow-net)
    pub deno_permissions: Vec<String>,

    /// Optional Deno env vars
    pub deno_env: std::collections::HashMap<String, String>,

    /// Optional native command for backend=native
    pub native_command: Option<String>,

    /// Optional native command args
    pub native_args: Vec<String>,

    /// Optional native env vars
    pub native_env: std::collections::HashMap<String, String>,

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
        use pulldown_cmark::{Event, Parser, Tag, TagEnd};

        let mut name = path
            .parent()
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
        let mut backend = String::from("native");
        let mut mcp_server_name = None;
        let mut mcp_command = None;
        let mut mcp_args: Vec<String> = Vec::new();
        let mut mcp_env: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut deno_command = None;
        let mut deno_script = None;
        let mut deno_args: Vec<String> = Vec::new();
        let mut deno_sandbox = None;
        let mut deno_permissions: Vec<String> = Vec::new();
        let mut deno_env: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut native_command = None;
        let mut native_args: Vec<String> = Vec::new();
        let mut native_env: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

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
                            "backend" => backend = value.to_string(),
                            "mcp_server_name" => mcp_server_name = Some(value.to_string()),
                            "mcp_command" => mcp_command = Some(value.to_string()),
                            "mcp_args" => {
                                mcp_args = parse_list_value(value);
                            }
                            "deno_command" => deno_command = Some(value.to_string()),
                            "deno_script" => deno_script = Some(value.to_string()),
                            "deno_args" => {
                                deno_args = parse_list_value(value);
                            }
                            "deno_sandbox" => deno_sandbox = Some(value.to_string()),
                            "deno_permissions" => {
                                deno_permissions = parse_list_value(value);
                            }
                            "native_command" => native_command = Some(value.to_string()),
                            "native_args" => {
                                native_args = parse_list_value(value);
                            }
                            _ => {}
                        }

                        if key.starts_with("mcp_env.") {
                            let env_key = key.trim_start_matches("mcp_env.").trim();
                            if !env_key.is_empty() {
                                mcp_env.insert(env_key.to_string(), value.to_string());
                            }
                        }
                        if key.starts_with("deno_env.") {
                            let env_key = key.trim_start_matches("deno_env.").trim();
                            if !env_key.is_empty() {
                                deno_env.insert(env_key.to_string(), value.to_string());
                            }
                        }
                        if key.starts_with("native_env.") {
                            let env_key = key.trim_start_matches("native_env.").trim();
                            if !env_key.is_empty() {
                                native_env.insert(env_key.to_string(), value.to_string());
                            }
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
            backend,
            mcp_server_name,
            mcp_command,
            mcp_args,
            mcp_env,
            deno_command,
            deno_script,
            deno_args,
            deno_sandbox,
            deno_permissions,
            deno_env,
            native_command,
            native_args,
            native_env,
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

fn parse_list_value(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        return inner
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\''))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
    }

    trimmed
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
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
backend: mcp
mcp_server_name: test-srv
mcp_command: npx
mcp_args: ["-y", "@modelcontextprotocol/server-filesystem", "."]
mcp_env.API_KEY: "${API_KEY}"
deno_command: deno
deno_script: skills/test/main.ts
deno_args: ["run", "--allow-read"]
deno_sandbox: balanced
deno_permissions: ["--allow-net"]
deno_env.MODE: test
native_command: python
native_args: ["script.py"]
native_env.LEVEL: debug
---

# Test Skill

This is a test.

## Tools Provided

- `test_tool`: Does testing things
- `another_tool`: Does more testing
"#;

        let metadata = SkillMetadata::from_markdown(
            std::path::PathBuf::from("/skills/test/SKILL.md"),
            content,
        )
        .unwrap();

        assert_eq!(metadata.name, "test_skill");
        assert_eq!(metadata.category, "custom");
        assert_eq!(metadata.tools.len(), 2);
        assert_eq!(metadata.tools[0].name, "test_tool");
        assert_eq!(metadata.backend, "mcp");
        assert_eq!(metadata.mcp_server_name.as_deref(), Some("test-srv"));
        assert_eq!(metadata.mcp_command.as_deref(), Some("npx"));
        assert_eq!(metadata.mcp_args.len(), 3);
        assert_eq!(metadata.mcp_args[0], "-y");
        assert_eq!(
            metadata.mcp_env.get("API_KEY").map(|s| s.as_str()),
            Some("${API_KEY}")
        );
        assert_eq!(metadata.deno_command.as_deref(), Some("deno"));
        assert_eq!(metadata.deno_script.as_deref(), Some("skills/test/main.ts"));
        assert_eq!(metadata.deno_args.len(), 2);
        assert_eq!(metadata.deno_sandbox.as_deref(), Some("balanced"));
        assert_eq!(metadata.deno_permissions, vec!["--allow-net"]);
        assert_eq!(
            metadata.deno_env.get("MODE").map(|s| s.as_str()),
            Some("test")
        );
        assert_eq!(metadata.native_command.as_deref(), Some("python"));
        assert_eq!(metadata.native_args.len(), 1);
        assert_eq!(metadata.native_args[0], "script.py");
        assert_eq!(
            metadata.native_env.get("LEVEL").map(|s| s.as_str()),
            Some("debug")
        );
    }
}
