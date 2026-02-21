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

    /// OpenClaw-compatible load-time gates
    pub requires_bins: Vec<String>,
    pub requires_any_bins: Vec<String>,
    pub requires_env: Vec<String>,
    pub requires_config: Vec<String>,
    #[serde(default)]
    pub openclaw_install: Vec<OpenClawInstallHint>,
    pub allowed_os: Vec<String>,
    pub always: bool,
    pub homepage: Option<String>,

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenClawInstallHint {
    pub id: Option<String>,
    pub kind: Option<String>,
    pub label: Option<String>,
    pub bins: Vec<String>,
    pub formula: Option<String>,
    pub package: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
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
        let mut config_schema = None;

        let mut requires_bins: Vec<String> = Vec::new();
        let mut requires_any_bins: Vec<String> = Vec::new();
        let mut requires_env: Vec<String> = Vec::new();
        let mut requires_config: Vec<String> = Vec::new();
        let mut openclaw_install: Vec<OpenClawInstallHint> = Vec::new();
        let mut allowed_os: Vec<String> = Vec::new();
        let mut always = false;
        let mut homepage: Option<String> = None;

        // Parse YAML frontmatter first (OpenClaw/AgentSkills compatible).
        if let Some(frontmatter) = extract_frontmatter(content) {
            if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
                && let Some(map) = value.as_mapping()
            {
                name = yaml_string(map, "name").unwrap_or(name);
                description = yaml_string(map, "description").unwrap_or(description);
                category = yaml_string(map, "category").unwrap_or(category);
                status = yaml_string(map, "status").unwrap_or(status);
                author = yaml_string(map, "author");
                backend = yaml_string(map, "backend").unwrap_or(backend);
                mcp_server_name = yaml_string(map, "mcp_server_name");
                mcp_command = yaml_string(map, "mcp_command");
                deno_command = yaml_string(map, "deno_command");
                deno_script = yaml_string(map, "deno_script");
                deno_sandbox = yaml_string(map, "deno_sandbox");
                native_command = yaml_string(map, "native_command");
                config_schema = yaml_string(map, "config_schema");
                homepage = yaml_string(map, "homepage");

                mcp_args = yaml_list(map, "mcp_args");
                deno_args = yaml_list(map, "deno_args");
                deno_permissions = yaml_list(map, "deno_permissions");
                native_args = yaml_list(map, "native_args");

                collect_prefixed_env(map, "mcp_env.", &mut mcp_env);
                collect_prefixed_env(map, "deno_env.", &mut deno_env);
                collect_prefixed_env(map, "native_env.", &mut native_env);

                if let Some(openclaw) = map
                    .get(serde_yaml::Value::String("metadata".to_string()))
                    .and_then(metadata_openclaw)
                {
                    always = openclaw
                        .get("always")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if homepage.is_none() {
                        homepage = openclaw
                            .get("homepage")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    allowed_os = openclaw
                        .get("os")
                        .map(yaml_value_to_string_list)
                        .unwrap_or_default();

                    if let Some(requires) = openclaw.get("requires") {
                        requires_bins = requires
                            .get("bins")
                            .map(yaml_value_to_string_list)
                            .unwrap_or_default();
                        requires_any_bins = requires
                            .get("anyBins")
                            .map(yaml_value_to_string_list)
                            .unwrap_or_default();
                        requires_env = requires
                            .get("env")
                            .map(yaml_value_to_string_list)
                            .unwrap_or_default();
                        requires_config = requires
                            .get("config")
                            .map(yaml_value_to_string_list)
                            .unwrap_or_default();
                    }

                    if let Some(install) = openclaw.get("install") {
                        openclaw_install = parse_openclaw_install_hints(install);
                    }
                }
            } else {
                // Fallback parser for loosely formatted community frontmatter.
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
                            "mcp_args" => mcp_args = parse_list_value(value),
                            "deno_command" => deno_command = Some(value.to_string()),
                            "deno_script" => deno_script = Some(value.to_string()),
                            "deno_args" => deno_args = parse_list_value(value),
                            "deno_sandbox" => deno_sandbox = Some(value.to_string()),
                            "deno_permissions" => deno_permissions = parse_list_value(value),
                            "native_command" => native_command = Some(value.to_string()),
                            "native_args" => native_args = parse_list_value(value),
                            "config_schema" => config_schema = Some(value.to_string()),
                            "homepage" => homepage = Some(value.to_string()),
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
                    in_tools_section = current_text.contains("Tools Provided");
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

        // Community SKILL.md files often use heading-style tool sections:
        // ### `tool_name`
        // Description...
        if tools.is_empty() {
            tools = parse_heading_style_tools(content);
        }

        // Compatibility heuristics for community skills:
        // If backend is not explicitly set to mcp/native and a TS/JS entrypoint exists,
        // prefer deno execution with sensible defaults.
        if backend == "native"
            && deno_script.is_none()
            && let Some(skill_dir) = path.parent()
        {
            let candidates = ["main.ts", "index.ts", "skill.ts", "main.js", "index.js"];
            for c in candidates {
                let p = skill_dir.join(c);
                if p.exists() {
                    backend = "deno".to_string();
                    deno_script = Some(p.to_string_lossy().to_string());
                    if deno_args.is_empty() {
                        deno_args = vec![
                            "run".to_string(),
                            "--compat".to_string(),
                            "--unstable-node-globals".to_string(),
                            "--unstable-bare-node-builtins".to_string(),
                        ];
                    }
                    break;
                }
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
            requires_bins,
            requires_any_bins,
            requires_env,
            requires_config,
            openclaw_install,
            allowed_os,
            always,
            homepage,
            enabled: true, // Default to enabled
            config_schema,
            skill_path: path,
        })
    }

    /// Check if this skill has a specific tool
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.tools.iter().any(|t| t.name == tool_name)
    }
}

fn extract_frontmatter(content: &str) -> Option<&str> {
    if !content.starts_with("---") {
        return None;
    }

    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        return None;
    }

    let mut offset = first.len() + 1;
    for line in lines {
        if line.trim() == "---" {
            return content.get(4..offset - 1);
        }
        offset += line.len() + 1;
    }

    None
}

fn yaml_string(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    map.get(serde_yaml::Value::String(key.to_string()))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn yaml_list(map: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    map.get(serde_yaml::Value::String(key.to_string()))
        .map(yaml_value_to_string_list)
        .unwrap_or_default()
}

fn yaml_value_to_string_list(v: &serde_yaml::Value) -> Vec<String> {
    if let Some(arr) = v.as_sequence() {
        return arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(s) = v.as_str() {
        return parse_list_value(s);
    }
    Vec::new()
}

fn collect_prefixed_env(
    map: &serde_yaml::Mapping,
    prefix: &str,
    output: &mut std::collections::HashMap<String, String>,
) {
    for (k, v) in map {
        let Some(key) = k.as_str() else {
            continue;
        };
        if !key.starts_with(prefix) {
            continue;
        }
        let env_key = key.trim_start_matches(prefix).trim();
        let Some(env_val) = v.as_str() else {
            continue;
        };
        if !env_key.is_empty() {
            output.insert(env_key.to_string(), env_val.to_string());
        }
    }
}

fn metadata_openclaw(metadata: &serde_yaml::Value) -> Option<serde_yaml::Mapping> {
    if let Some(map) = metadata.as_mapping() {
        return map
            .get(serde_yaml::Value::String("openclaw".to_string()))
            .and_then(|v| v.as_mapping().cloned());
    }

    let raw = metadata.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    serde_yaml::from_str::<serde_yaml::Value>(raw)
        .ok()?
        .as_mapping()?
        .get(serde_yaml::Value::String("openclaw".to_string()))
        .and_then(|v| v.as_mapping().cloned())
}

fn parse_heading_style_tools(content: &str) -> Vec<SkillTool> {
    let mut tools = Vec::new();
    let mut in_tools_section = false;
    let mut lines = content.lines().peekable();

    while let Some(raw) = lines.next() {
        let line = raw.trim();

        if line.starts_with("## ") {
            in_tools_section = line.to_ascii_lowercase().contains("tools provided");
            continue;
        }

        if !in_tools_section {
            continue;
        }

        if let Some(rest) = line.strip_prefix("### ") {
            let heading = rest.trim();
            let tool_name = heading
                .trim_matches('`')
                .split_whitespace()
                .next()
                .unwrap_or(heading)
                .trim_matches('`')
                .to_string();

            if tool_name.is_empty() {
                continue;
            }

            let mut description = String::new();
            while let Some(next) = lines.peek() {
                let t = next.trim();
                if t.starts_with("### ") || t.starts_with("## ") {
                    break;
                }
                let consumed = lines.next().unwrap_or_default();
                let consumed_trimmed = consumed.trim();
                if consumed_trimmed.is_empty() {
                    continue;
                }
                if consumed_trimmed.starts_with('-') {
                    continue;
                }
                description = consumed_trimmed.to_string();
                break;
            }

            tools.push(SkillTool {
                name: tool_name,
                description,
                command: None,
                schema: None,
            });
        }
    }

    tools
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

fn parse_openclaw_install_hints(v: &serde_yaml::Value) -> Vec<OpenClawInstallHint> {
    let Some(items) = v.as_sequence() else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|entry| {
            let map = entry.as_mapping()?;
            Some(OpenClawInstallHint {
                id: map
                    .get(serde_yaml::Value::String("id".to_string()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                kind: map
                    .get(serde_yaml::Value::String("kind".to_string()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                label: map
                    .get(serde_yaml::Value::String("label".to_string()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                bins: map
                    .get(serde_yaml::Value::String("bins".to_string()))
                    .map(yaml_value_to_string_list)
                    .unwrap_or_default(),
                formula: map
                    .get(serde_yaml::Value::String("formula".to_string()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                package: map
                    .get(serde_yaml::Value::String("package".to_string()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                command: map
                    .get(serde_yaml::Value::String("command".to_string()))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                args: map
                    .get(serde_yaml::Value::String("args".to_string()))
                    .map(yaml_value_to_string_list)
                    .unwrap_or_default(),
            })
        })
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

    #[test]
    fn parses_heading_style_tools() {
        let content = r#"---
name: github
description: "GitHub ops"
---

## Tools Provided

### `gh_issue_list`
List issues in a repository.
- **Args**: repo, state, limit

### `gh_pr_list`
List pull requests.
"#;

        let metadata = SkillMetadata::from_markdown(
            std::path::PathBuf::from("/skills/github/SKILL.md"),
            content,
        )
        .unwrap();

        assert_eq!(metadata.tools.len(), 2);
        assert_eq!(metadata.tools[0].name, "gh_issue_list");
        assert!(metadata.tools[0].description.contains("List issues"));
        assert_eq!(metadata.tools[1].name, "gh_pr_list");
    }

    #[test]
    fn parses_gog_style_frontmatter_metadata_block() {
        let content = r#"---
name: gog
description: Google Workspace CLI
metadata:
  {
    "openclaw":
      {
        "requires": { "bins": ["gog"] },
      },
  }
---

# gog
Use gog CLI.
"#;

        let metadata =
            SkillMetadata::from_markdown(std::path::PathBuf::from("/skills/gog/SKILL.md"), content)
                .unwrap();

        assert_eq!(metadata.name, "gog");
        assert_eq!(metadata.backend, "native");
    }

    #[test]
    fn parses_openclaw_install_hints() {
        let content = r#"---
name: github
description: GitHub CLI
metadata:
  openclaw:
    install:
      - id: brew
        kind: brew
        formula: gh
        bins: [gh]
        label: Install GitHub CLI (brew)
      - id: apt
        kind: apt
        package: gh
        bins: [gh]
---

# github
"#;

        let metadata = SkillMetadata::from_markdown(
            std::path::PathBuf::from("/skills/github/SKILL.md"),
            content,
        )
        .unwrap();

        assert_eq!(metadata.openclaw_install.len(), 2);
        assert_eq!(metadata.openclaw_install[0].id.as_deref(), Some("brew"));
        assert_eq!(metadata.openclaw_install[0].formula.as_deref(), Some("gh"));
        assert_eq!(metadata.openclaw_install[1].package.as_deref(), Some("gh"));
    }
}
