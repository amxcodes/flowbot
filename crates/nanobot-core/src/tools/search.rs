use anyhow::Result;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobArgs {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub include: Option<String>,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct GrepMatch {
    file: String,
    line: usize,
    text: String,
}

pub async fn glob_files(args: GlobArgs) -> Result<String> {
    let root = if let Some(path) = &args.path {
        super::validate_path(path)?
    } else {
        std::env::current_dir()?
    };

    if !root.exists() {
        return Err(anyhow::anyhow!("Path does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(anyhow::anyhow!("Path is not a directory: {}", root.display()));
    }

    let pattern = glob::Pattern::new(&args.pattern)
        .map_err(|e| anyhow::anyhow!("Invalid glob pattern: {}", e))?;
    let max_results = args.max_results.unwrap_or(500).min(5000);

    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
        let path = entry.path();
        if path == root {
            continue;
        }

        let rel = path.strip_prefix(&root).unwrap_or(path);
        let rel_norm = rel.to_string_lossy().replace('\\', "/");
        if pattern.matches(&rel_norm) {
            out.push(rel_norm);
            if out.len() >= max_results {
                break;
            }
        }
    }

    out.sort();
    Ok(serde_json::to_string(&serde_json::json!({
        "count": out.len(),
        "paths": out,
    }))?)
}

pub async fn grep_files(args: GrepArgs) -> Result<String> {
    let root = if let Some(path) = &args.path {
        super::validate_path(path)?
    } else {
        std::env::current_dir()?
    };

    if !root.exists() {
        return Err(anyhow::anyhow!("Path does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(anyhow::anyhow!("Path is not a directory: {}", root.display()));
    }

    let regex = RegexBuilder::new(&args.pattern)
        .case_insensitive(!args.case_sensitive.unwrap_or(true))
        .build()
        .map_err(|e| anyhow::anyhow!("Invalid regex pattern: {}", e))?;

    let include_pattern = if let Some(include) = args.include.as_ref() {
        Some(
            glob::Pattern::new(include)
                .map_err(|e| anyhow::anyhow!("Invalid include glob: {}", e))?,
        )
    } else {
        None
    };

    let max_results = args.max_results.unwrap_or(200).min(5000);
    let mut matches = Vec::new();

    for entry in walkdir::WalkDir::new(&root).into_iter().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let rel = path.strip_prefix(&root).unwrap_or(path);
        let rel_norm = rel.to_string_lossy().replace('\\', "/");

        if let Some(pat) = include_pattern.as_ref() {
            if !pat.matches(&rel_norm) {
                continue;
            }
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (idx, line) in content.lines().enumerate() {
            if regex.is_match(line) {
                matches.push(GrepMatch {
                    file: rel_norm.clone(),
                    line: idx + 1,
                    text: line.chars().take(400).collect(),
                });
                if matches.len() >= max_results {
                    break;
                }
            }
        }

        if matches.len() >= max_results {
            break;
        }
    }

    Ok(serde_json::to_string(&serde_json::json!({
        "count": matches.len(),
        "matches": matches,
    }))?)
}
