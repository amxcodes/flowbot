use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSecrets {
    pub gateway_session_secret: String,
    pub web_token_secret: String,
}

fn secrets_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("HOME/USERPROFILE environment variable not set")?;
    let dir = PathBuf::from(home).join(".nanobot");
    fs::create_dir_all(&dir).context("Failed to create .nanobot directory")?;
    Ok(dir)
}

fn secrets_path() -> Result<PathBuf> {
    Ok(secrets_dir()?.join("session_secrets.json"))
}

pub fn read_session_secrets() -> Result<Option<SessionSecrets>> {
    let path = secrets_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path).context("Failed to read session secrets")?;
    let secrets = serde_json::from_str(&contents).context("Invalid session secrets JSON")?;
    Ok(Some(secrets))
}

pub fn write_session_secrets(secrets: &SessionSecrets) -> Result<()> {
    let path = secrets_path()?;
    let contents = serde_json::to_string_pretty(secrets)?;
    fs::write(&path, contents).context("Failed to write session secrets")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

pub fn get_or_create_session_secrets() -> Result<SessionSecrets> {
    if let Some(secrets) = read_session_secrets()? {
        return Ok(secrets);
    }

    let secrets = SessionSecrets {
        gateway_session_secret: uuid::Uuid::new_v4().to_string(),
        web_token_secret: uuid::Uuid::new_v4().to_string(),
    };
    write_session_secrets(&secrets)?;
    Ok(secrets)
}
