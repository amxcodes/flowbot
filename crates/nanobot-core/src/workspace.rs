use std::path::PathBuf;

fn home_dir_fallback() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_workspace_dir() -> PathBuf {
    home_dir_fallback().join(".nanobot")
}

pub fn resolve_workspace_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("NANOBOT_WORKSPACE_DIR") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let modern_root = default_workspace_dir();
    let modern_nested = modern_root.join("workspace");

    if modern_nested.exists() {
        return modern_nested;
    }

    let modern_has_identity = modern_root.join("SOUL.md").exists()
        || modern_root.join("IDENTITY.md").exists()
        || modern_root.join("skills").exists();
    if modern_has_identity {
        return modern_root;
    }

    modern_root
}

pub fn resolve_skills_dir() -> PathBuf {
    resolve_workspace_dir().join("skills")
}
