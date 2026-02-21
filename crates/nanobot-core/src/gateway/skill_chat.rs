use anyhow::Result;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct SkillSetupState {
    skill_name: String,
    pending_keys: Vec<String>,
}

static SETUP_STATES: Lazy<tokio::sync::Mutex<HashMap<String, SkillSetupState>>> =
    Lazy::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
struct GitHubSkillTreeEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    download_url: Option<String>,
    url: String,
}

pub async fn handle_skill_slash_command(scope_id: &str, text: &str) -> Result<Option<String>> {
    let trimmed = text.trim();

    if !trimmed.starts_with("/skill") {
        if let Some(reply) = maybe_consume_setup_answer(scope_id, trimmed).await? {
            return Ok(Some(reply));
        }
        return Ok(None);
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() == 1 || parts.get(1) == Some(&"help") {
        return Ok(Some(skill_help_text()));
    }

    match parts[1] {
        "list" => Ok(Some(list_skills().await?)),
        "browse" => {
            let repo = parts.get(2).copied().unwrap_or("openclaw/openclaw");
            Ok(Some(browse_remote_skills(repo).await?))
        }
        "top" => {
            let repo = parts.get(2).copied().unwrap_or("openclaw/openclaw");
            Ok(Some(top_skills(repo).await?))
        }
        "search" => {
            let query = parts.get(2).copied().unwrap_or("").trim();
            if query.is_empty() {
                return Ok(Some("Usage: /skill search <query> [repo]".to_string()));
            }
            let repo = parts.get(3).copied().unwrap_or("openclaw/openclaw");
            Ok(Some(search_remote_skills(query, repo).await?))
        }
        "info" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() {
                return Ok(Some("Usage: /skill info <name> [repo]".to_string()));
            }
            let repo = parts.get(3).copied().unwrap_or("openclaw/openclaw");
            Ok(Some(skill_info(&name, repo).await?))
        }
        "install" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() {
                return Ok(Some("Usage: /skill install <name> [repo]".to_string()));
            }
            let repo = parts.get(3).copied().unwrap_or("openclaw/openclaw");
            Ok(Some(install_skill_simple(&name, repo).await?))
        }
        "enable" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() {
                return Ok(Some("Usage: /skill enable <name>".to_string()));
            }
            let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
            cfg.enable_skill(&name);
            cfg.save()?;
            Ok(Some(format!("Enabled skill '{}'.", name)))
        }
        "disable" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() {
                return Ok(Some("Usage: /skill disable <name>".to_string()));
            }
            let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
            cfg.disable_skill(&name);
            cfg.save()?;
            Ok(Some(format!("Disabled skill '{}'.", name)))
        }
        "runtime" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let runtime = parts
                .get(3)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() || runtime.is_empty() {
                return Ok(Some(
                    "Usage: /skill runtime <name> <deno|node|native|mcp>".to_string(),
                ));
            }
            if !matches!(runtime.as_str(), "deno" | "node" | "native" | "mcp") {
                return Ok(Some(
                    "Runtime must be one of: deno, node, native, mcp".to_string(),
                ));
            }
            let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
            cfg.set_runtime_override(&name, &runtime);
            cfg.save()?;
            Ok(Some(format!(
                "Set runtime for '{}' to '{}'.",
                name, runtime
            )))
        }
        "config" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() {
                return Ok(Some(
                    "Usage: /skill config <name> key=value [key=value...]".to_string(),
                ));
            }
            let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
            let mut changed = Vec::new();
            for kv in parts.iter().skip(3) {
                if let Some((k, v)) = kv.split_once('=') {
                    let key = k.trim();
                    let value = v.trim();
                    if key.is_empty() {
                        continue;
                    }
                    if key.eq_ignore_ascii_case("enabled") {
                        if matches!(
                            value.to_ascii_lowercase().as_str(),
                            "1" | "true" | "yes" | "on"
                        ) {
                            cfg.enable_skill(&name);
                            changed.push("enabled=true".to_string());
                        } else if matches!(
                            value.to_ascii_lowercase().as_str(),
                            "0" | "false" | "no" | "off"
                        ) {
                            cfg.disable_skill(&name);
                            changed.push("enabled=false".to_string());
                        }
                    } else if key.eq_ignore_ascii_case("runtime") {
                        let runtime = value.to_ascii_lowercase();
                        if matches!(runtime.as_str(), "deno" | "node" | "native" | "mcp") {
                            cfg.set_runtime_override(&name, &runtime);
                            changed.push(format!("runtime={}", runtime));
                        }
                    } else {
                        cfg.set_credential(&name, key, value.to_string());
                        changed.push(format!("{}=***", key));
                    }
                }
            }
            cfg.save()?;

            let missing = missing_credentials(&name, &cfg);
            let mut message = if changed.is_empty() {
                format!("No valid key=value pairs found for '{}'.", name)
            } else {
                format!("Updated '{}' ({})", name, changed.join(", "))
            };
            if !missing.is_empty() {
                message.push_str(&format!("\nMissing credentials: {}", missing.join(", ")));
            }
            Ok(Some(message))
        }
        "setup" => {
            let name = parts
                .get(2)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() {
                return Ok(Some("Usage: /skill setup <name> [repo]".to_string()));
            }
            let repo = parts.get(3).copied().unwrap_or("openclaw/openclaw");
            Ok(Some(start_setup_flow(scope_id, &name, repo).await?))
        }
        "answer" => {
            let value = parts.iter().skip(2).copied().collect::<Vec<_>>().join(" ");
            if value.trim().is_empty() {
                return Ok(Some("Usage: /skill answer <value>".to_string()));
            }
            Ok(Some(submit_setup_answer(scope_id, value.trim()).await?))
        }
        "cancel" => {
            let mut states = SETUP_STATES.lock().await;
            if states.remove(scope_id).is_some() {
                Ok(Some("Skill setup cancelled.".to_string()))
            } else {
                Ok(Some("No active skill setup.".to_string()))
            }
        }
        _ => Ok(Some(skill_help_text())),
    }
}

fn skill_help_text() -> String {
    "Skill commands:\n  /skill list\n  /skill top [repo]\n  /skill browse [repo]\n  /skill search <query> [repo]\n  /skill info <name> [repo]\n  /skill install <name> [repo]\n  /skill enable <name>\n  /skill disable <name>\n  /skill runtime <name> <deno|node|native|mcp>\n  /skill config <name> key=value [key=value...]\n  /skill setup <name> [repo]  (guided credential wizard)\n  /skill answer <value>       (answer current setup question)\n  /skill cancel\nExamples:\n  /skill top\n  /skill browse\n  /skill search git\n  /skill info weather\n  /skill install weather\n  /skill setup weather\n  /skill answer YOUR_API_KEY\n  /skill runtime gog node"
        .to_string()
}

async fn top_skills(repo: &str) -> Result<String> {
    let curated = vec!["weather", "github", "gog", "calendar", "notion", "spotify"];

    let remote = fetch_remote_skill_names(repo).await.unwrap_or_default();
    let remote_set = remote.into_iter().collect::<std::collections::HashSet<_>>();

    let cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    let skills_dir = crate::workspace::resolve_skills_dir();

    let mut rows = Vec::new();
    for name in curated {
        let available = remote_set.contains(name);
        let installed = skills_dir.join(name).exists();
        let enabled = cfg.is_enabled(name);

        rows.push(format!(
            "- {} | available: {} | installed: {} | enabled: {}",
            name,
            if available { "yes" } else { "no" },
            if installed { "yes" } else { "no" },
            if enabled { "yes" } else { "no" }
        ));
    }

    Ok(format!(
        "Top beginner-friendly skills:\n{}\nQuick start: /skill setup <name>",
        rows.join("\n")
    ))
}

async fn fetch_remote_skill_names(repo: &str) -> Result<Vec<String>> {
    let api_url = format!("https://api.github.com/repos/{}/contents/skills", repo);
    let client = reqwest::Client::new();
    let resp = client
        .get(&api_url)
        .header("User-Agent", "nanobot-skill-chat")
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Failed to fetch skills index from {} (status {})",
            repo,
            resp.status()
        );
    }

    let json: serde_json::Value = resp.json().await?;
    let arr = json
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Unexpected skills index response format"))?;

    let mut names: Vec<String> = arr
        .iter()
        .filter(|entry| entry["type"].as_str() == Some("dir"))
        .filter_map(|entry| entry["name"].as_str().map(|s| s.to_string()))
        .collect();
    names.sort();
    Ok(names)
}

async fn browse_remote_skills(repo: &str) -> Result<String> {
    let names = fetch_remote_skill_names(repo).await?;
    if names.is_empty() {
        return Ok(format!("No skills found in {}.", repo));
    }

    let total = names.len();
    let preview = names.into_iter().take(30).collect::<Vec<_>>();
    let mut out = format!("ClawHub skills from {} ({} total):", repo, total);
    out.push_str(&format!("\n- {}", preview.join("\n- ")));
    if total > 30 {
        out.push_str("\n...more available. Use /skill search <query> to narrow down.");
    }
    Ok(out)
}

async fn search_remote_skills(query: &str, repo: &str) -> Result<String> {
    let query_norm = query.trim().to_ascii_lowercase();
    if query_norm.is_empty() {
        return Ok("Search query cannot be empty.".to_string());
    }

    let names = fetch_remote_skill_names(repo).await?;
    let matches = names
        .into_iter()
        .filter(|name| name.to_ascii_lowercase().contains(&query_norm))
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return Ok(format!("No skills found for '{}' in {}.", query, repo));
    }

    let total = matches.len();
    let preview = matches.into_iter().take(30).collect::<Vec<_>>();
    let mut out = format!("Matches for '{}' in {} ({}):", query, repo, total);
    out.push_str(&format!("\n- {}", preview.join("\n- ")));
    if total > 30 {
        out.push_str("\n...more matches available. Refine query for a shorter list.");
    }
    Ok(out)
}

async fn skill_info(skill_name: &str, repo: &str) -> Result<String> {
    let api_url = format!(
        "https://api.github.com/repos/{}/contents/skills/{}/SKILL.md",
        repo, skill_name
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&api_url)
        .header("User-Agent", "nanobot-skill-chat")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(format!(
            "Skill '{}' not found in {}. Try: /skill search {}",
            skill_name, repo, skill_name
        ));
    }

    let file_json: serde_json::Value = resp.json().await?;
    let download_url = file_json
        .get("download_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing download URL for SKILL.md"))?;

    let content = client
        .get(download_url)
        .header("User-Agent", "nanobot-skill-chat")
        .send()
        .await?
        .text()
        .await?;

    let metadata = crate::skills::metadata::SkillMetadata::from_markdown(
        std::path::PathBuf::from(format!("/skills/{}/SKILL.md", skill_name)),
        &content,
    )?;

    let cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    let installed = crate::workspace::resolve_skills_dir()
        .join(skill_name)
        .exists();
    let enabled = cfg.is_enabled(skill_name);
    let runtime = cfg
        .runtime_override(skill_name)
        .unwrap_or(metadata.backend.as_str());

    let tool_names = metadata
        .tools
        .iter()
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();

    let missing = missing_credentials(skill_name, &cfg);

    let mut out = format!(
        "Skill: {}\nDescription: {}\nBackend: {}\nTools: {}\nInstalled: {}\nEnabled: {}\nRuntime: {}",
        metadata.name,
        if metadata.description.is_empty() {
            "(no description)"
        } else {
            metadata.description.as_str()
        },
        metadata.backend,
        if tool_names.is_empty() {
            "(none)".to_string()
        } else {
            tool_names.join(", ")
        },
        if installed { "yes" } else { "no" },
        if enabled { "yes" } else { "no" },
        runtime,
    );

    if installed && !missing.is_empty() {
        out.push_str(&format!("\nMissing credentials: {}", missing.join(", ")));
        out.push_str(&format!("\nRun: /skill setup {}", skill_name));
    }

    if !installed {
        out.push_str(&format!("\nInstall: /skill install {}", skill_name));
    }

    Ok(out)
}

async fn maybe_consume_setup_answer(scope_id: &str, text: &str) -> Result<Option<String>> {
    let has_state = {
        let states = SETUP_STATES.lock().await;
        states.contains_key(scope_id)
    };
    if !has_state {
        return Ok(None);
    }
    if text.starts_with('/') {
        return Ok(None);
    }

    Ok(Some(submit_setup_answer(scope_id, text).await?))
}

async fn start_setup_flow(scope_id: &str, skill_name: &str, repo: &str) -> Result<String> {
    let workspace = crate::workspace::resolve_workspace_dir();
    let mut loader = crate::skills::SkillLoader::new(workspace);
    let _ = loader.scan();

    let mut intro = String::new();
    if loader.get_skill(skill_name).is_none() {
        let install_msg = install_skill_simple(skill_name, repo).await?;
        intro.push_str(&format!("{}\n", install_msg));
    }

    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    cfg.enable_skill(skill_name);
    cfg.save()?;

    let missing = missing_credentials(skill_name, &cfg);
    if missing.is_empty() {
        let mut out = intro;
        out.push_str(&format!(
            "Skill '{}' is already fully configured and enabled.",
            skill_name
        ));
        return Ok(out);
    }

    {
        let mut states = SETUP_STATES.lock().await;
        states.insert(
            scope_id.to_string(),
            SkillSetupState {
                skill_name: skill_name.to_string(),
                pending_keys: missing.clone(),
            },
        );
    }

    let first = missing.first().cloned().unwrap_or_default();
    let mut out = intro;
    out.push_str(&format!(
        "Guided setup started for '{}'.\nQuestion 1/{}: Please provide value for '{}'\nReply normally or use: /skill answer <value>\nUse /skill cancel to stop.",
        skill_name,
        missing.len(),
        first
    ));
    Ok(out)
}

async fn submit_setup_answer(scope_id: &str, value: &str) -> Result<String> {
    let (skill_name, current_key, remaining_after_current) = {
        let mut states = SETUP_STATES.lock().await;
        let Some(state) = states.get_mut(scope_id) else {
            return Ok("No active skill setup. Start with: /skill setup <name>".to_string());
        };

        if state.pending_keys.is_empty() {
            states.remove(scope_id);
            return Ok("No remaining setup questions. Use /skill setup <name> again.".to_string());
        }

        let current_key = state.pending_keys.remove(0);
        let remaining = state.pending_keys.clone();
        (state.skill_name.clone(), current_key, remaining)
    };

    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    cfg.enable_skill(&skill_name);
    cfg.set_credential(&skill_name, &current_key, value.to_string());
    cfg.save()?;

    if remaining_after_current.is_empty() {
        let mut states = SETUP_STATES.lock().await;
        states.remove(scope_id);
        let missing = missing_credentials(&skill_name, &cfg);
        if missing.is_empty() {
            return Ok(format!(
                "Saved '{}' for '{}'.\nSetup complete. Skill is enabled and ready.",
                current_key, skill_name
            ));
        }
        return Ok(format!(
            "Saved '{}' for '{}'.\nSome credentials are still missing: {}\nRun /skill setup {} again.",
            current_key,
            skill_name,
            missing.join(", "),
            skill_name
        ));
    }

    let remaining_count = remaining_after_current.len();
    let next_key = remaining_after_current
        .first()
        .cloned()
        .unwrap_or_else(|| "next".to_string());
    Ok(format!(
        "Saved '{}' for '{}'.\nNext question ({} remaining): value for '{}'\nReply normally or: /skill answer <value>",
        current_key, skill_name, remaining_count, next_key
    ))
}

async fn list_skills() -> Result<String> {
    let workspace = crate::workspace::resolve_workspace_dir();
    let mut loader = crate::skills::SkillLoader::new(workspace);
    loader.scan()?;
    let cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();

    let mut rows = loader
        .skills()
        .values()
        .map(|s| {
            let enabled = cfg.is_enabled(&s.name);
            let runtime = cfg.runtime_override(&s.name).unwrap_or(&s.backend);
            format!(
                "- {} [{}] ({})",
                s.name,
                if enabled { "enabled" } else { "disabled" },
                runtime
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    if rows.is_empty() {
        Ok("No skills installed yet. Use /skill install <name>".to_string())
    } else {
        Ok(format!("Installed skills:\n{}", rows.join("\n")))
    }
}

async fn install_skill_simple(skill_name: &str, repo: &str) -> Result<String> {
    let skills_root = crate::workspace::resolve_skills_dir();
    tokio::fs::create_dir_all(&skills_root).await?;
    let skill_dir = skills_root.join(skill_name);
    if skill_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&skill_dir).await;
    }
    tokio::fs::create_dir_all(&skill_dir).await?;

    let client = reqwest::Client::new();
    download_skill_tree(&client, repo, skill_name, &skill_dir).await?;

    let skill_md = tokio::fs::read_to_string(skill_dir.join("SKILL.md")).await?;
    let metadata = crate::skills::metadata::SkillMetadata::from_markdown(
        std::path::PathBuf::from(format!("/skills/{}/SKILL.md", skill_name)),
        &skill_md,
    )?;

    let mut notes = bootstrap_skill_dependencies_if_present(&skill_dir);

    let mut cfg = crate::skills::config::SkillsConfig::load().unwrap_or_default();
    cfg.enable_skill(skill_name);
    auto_fill_credentials_from_env(skill_name, &mut cfg, &mut notes);
    cfg.save()?;

    let missing = missing_credentials(skill_name, &cfg);
    let mut out = format!(
        "Installed '{}' from {}\nBackend: {}\nAuto-enabled: yes",
        skill_name, repo, metadata.backend
    );
    if !notes.is_empty() {
        out.push_str(&format!("\nBootstrap: {}", notes.join("; ")));
    }
    if !missing.is_empty() {
        out.push_str(&format!("\nMissing credentials: {}", missing.join(", ")));
        out.push_str("\nUse: /skill config <name> key=value");
    }
    Ok(out)
}

async fn download_skill_tree(
    client: &reqwest::Client,
    repo: &str,
    skill_name: &str,
    destination: &std::path::Path,
) -> Result<()> {
    let root = format!("skills/{}", skill_name);
    let mut stack = vec![format!(
        "https://api.github.com/repos/{}/contents/{}",
        repo, root
    )];

    while let Some(api_url) = stack.pop() {
        let resp = client
            .get(&api_url)
            .header("User-Agent", "nanobot-skill-chat")
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Failed to fetch skill tree from {} (status {})",
                api_url,
                resp.status()
            );
        }

        let entries: Vec<GitHubSkillTreeEntry> = resp.json().await?;
        for entry in entries {
            match entry.entry_type.as_str() {
                "dir" => stack.push(entry.url),
                "file" => {
                    let download_url = entry.download_url.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("Missing download_url for {}", entry.path)
                    })?;

                    let prefix = format!("skills/{}/", skill_name);
                    let relative = entry.path.strip_prefix(&prefix).unwrap_or(&entry.name);
                    let output = destination.join(relative);
                    if let Some(parent) = output.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }

                    let bytes = client
                        .get(download_url)
                        .header("User-Agent", "nanobot-skill-chat")
                        .send()
                        .await?
                        .bytes()
                        .await?;
                    tokio::fs::write(output, &bytes).await?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn bootstrap_skill_dependencies_if_present(skill_dir: &std::path::Path) -> Vec<String> {
    let mut notes = Vec::new();

    if skill_dir.join("package.json").exists() {
        if std::process::Command::new("npm")
            .arg("--version")
            .output()
            .is_err()
        {
            notes.push("package.json found but npm is not installed".to_string());
        } else {
            let status = std::process::Command::new("npm")
                .arg("install")
                .arg("--omit=dev")
                .current_dir(skill_dir)
                .status();

            match status {
                Ok(s) if s.success() => notes.push("npm dependencies installed".to_string()),
                Ok(s) => notes.push(format!("npm install exited with status {}", s)),
                Err(e) => notes.push(format!("failed to run npm install: {}", e)),
            }
        }
    }

    notes
}

fn auto_fill_credentials_from_env(
    skill_name: &str,
    cfg: &mut crate::skills::config::SkillsConfig,
    notes: &mut Vec<String>,
) {
    let set_if_present = |cfg: &mut crate::skills::config::SkillsConfig,
                          skill: &str,
                          key: &str,
                          env_key: &str,
                          notes: &mut Vec<String>| {
        if let Ok(value) = std::env::var(env_key)
            && !value.trim().is_empty()
        {
            cfg.set_credential(skill, key, value);
            notes.push(format!("auto-configured '{}' from {}", key, env_key));
        }
    };

    match skill_name {
        "weather" => set_if_present(cfg, skill_name, "api_key", "OPENWEATHER_API_KEY", notes),
        "notion" => set_if_present(cfg, skill_name, "api_key", "NOTION_API_KEY", notes),
        "spotify" => {
            set_if_present(cfg, skill_name, "client_id", "SPOTIFY_CLIENT_ID", notes);
            set_if_present(
                cfg,
                skill_name,
                "client_secret",
                "SPOTIFY_CLIENT_SECRET",
                notes,
            );
        }
        _ => {}
    }
}

fn missing_credentials(skill_name: &str, cfg: &crate::skills::config::SkillsConfig) -> Vec<String> {
    let workspace = crate::workspace::resolve_workspace_dir();
    let mut loader = crate::skills::SkillLoader::new(workspace);
    let _ = loader.scan();
    let schema_required = loader
        .get_skill(skill_name)
        .map(|s| {
            crate::skills::config::required_credentials_from_schema(s.config_schema.as_deref())
        })
        .unwrap_or_default();
    let mut required = crate::skills::config::known_required_credentials(skill_name);
    required.extend(schema_required);
    required.sort();
    required.dedup();
    required
        .into_iter()
        .filter(|k| cfg.get_credential(skill_name, k).is_none())
        .collect::<Vec<_>>()
}
