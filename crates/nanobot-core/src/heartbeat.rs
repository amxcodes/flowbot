use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct HeartbeatSpec {
    pub schedule: String,
    pub timezone: Option<String>,
    pub tasks: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HeartbeatFrontmatter {
    schedule: String,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

pub fn load_first(paths: &[PathBuf]) -> Result<Option<(PathBuf, HeartbeatSpec)>> {
    for path in paths {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            let parsed = parse_heartbeat_markdown(&content)?;
            return Ok(Some((path.clone(), parsed)));
        }
    }
    Ok(None)
}

pub fn parse_heartbeat_markdown(content: &str) -> Result<HeartbeatSpec> {
    let content = content.trim_start_matches('\u{feff}');
    let (frontmatter_text, body) = split_frontmatter(content)?;

    let frontmatter: HeartbeatFrontmatter = serde_yaml::from_str(frontmatter_text)
        .map_err(|e| anyhow!("Invalid HEARTBEAT.md frontmatter: {}", e))?;

    if !frontmatter.enabled {
        return Err(anyhow!("HEARTBEAT.md is disabled (enabled: false)"));
    }

    let tasks = parse_markdown_tasks(body);
    if tasks.is_empty() {
        return Err(anyhow!(
            "HEARTBEAT.md must include at least one markdown list task"
        ));
    }

    Ok(HeartbeatSpec {
        schedule: frontmatter.schedule.trim().to_string(),
        timezone: frontmatter.timezone.map(|s| s.trim().to_string()),
        tasks,
    })
}

impl HeartbeatSpec {
    pub fn system_prompt(&self) -> String {
        let mut prompt = String::from("It is time to perform your heartbeat tasks:\n");
        for task in &self.tasks {
            prompt.push_str("- ");
            prompt.push_str(task);
            prompt.push('\n');
        }
        prompt.trim_end().to_string()
    }
}

fn split_frontmatter(content: &str) -> Result<(&str, &str)> {
    if content.is_empty() {
        return Err(anyhow!("HEARTBEAT.md is empty"));
    }

    let (after_open_idx, remaining) = if let Some(rest) = content.strip_prefix("---\r\n") {
        (5usize, rest)
    } else if let Some(rest) = content.strip_prefix("---\n") {
        (4usize, rest)
    } else {
        return Err(anyhow!(
            "HEARTBEAT.md must start with YAML frontmatter delimited by ---"
        ));
    };

    let delimiters = ["\n---\r\n", "\n---\n", "\n---"];
    let mut closing: Option<(usize, usize)> = None;
    for delimiter in delimiters {
        if let Some(pos) = remaining.find(delimiter) {
            match closing {
                Some((best_pos, _)) if best_pos <= pos => {}
                _ => closing = Some((pos, delimiter.len())),
            }
        }
    }

    let Some((close_pos, delimiter_len)) = closing else {
        return Err(anyhow!("HEARTBEAT.md frontmatter closing --- not found"));
    };

    let frontmatter_body = &remaining[..close_pos];
    let body_start = after_open_idx + close_pos + delimiter_len;
    let body = &content[body_start..];
    Ok((frontmatter_body, body))
}

fn parse_markdown_tasks(body: &str) -> Vec<String> {
    let mut tasks = Vec::new();

    for raw in body.lines() {
        let line = raw.trim();

        if let Some(task) = line.strip_prefix("- ") {
            let t = task.trim();
            if !t.is_empty() {
                tasks.push(t.to_string());
            }
            continue;
        }

        if let Some(task) = line.strip_prefix("* ") {
            let t = task.trim();
            if !t.is_empty() {
                tasks.push(t.to_string());
            }
            continue;
        }

        if let Some(idx) = line.find('.')
            && idx > 0
            && line[..idx].chars().all(|c| c.is_ascii_digit())
        {
            let task = line[idx + 1..].trim();
            if !task.is_empty() {
                tasks.push(task.to_string());
            }
        }
    }

    tasks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_tasks() {
        let content = r#"---
schedule: "0 * * * *"
timezone: "UTC"
---
- Check backlog
- Sync priorities
"#;

        let parsed = parse_heartbeat_markdown(content).unwrap();
        assert_eq!(parsed.schedule, "0 * * * *");
        assert_eq!(parsed.timezone.as_deref(), Some("UTC"));
        assert_eq!(parsed.tasks.len(), 2);
    }
}
