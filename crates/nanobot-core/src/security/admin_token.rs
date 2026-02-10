use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

const TOKEN_FILE: &str = "admin_token";

fn token_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("HOME/USERPROFILE environment variable not set")?;
    let dir = PathBuf::from(home).join(".nanobot");
    fs::create_dir_all(&dir).context("Failed to create .nanobot directory")?;
    Ok(dir)
}

fn token_path() -> Result<PathBuf> {
    Ok(token_dir()?.join(TOKEN_FILE))
}

pub fn read_admin_token() -> Result<Option<String>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let token = fs::read_to_string(&path).context("Failed to read admin token")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(token))
    }
}

pub fn write_admin_token(token: &str) -> Result<()> {
    let path = token_path()?;
    fs::write(&path, token).context("Failed to write admin token")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

pub fn clear_admin_token() -> Result<()> {
    let path = token_path()?;
    if path.exists() {
        fs::remove_file(&path).context("Failed to remove admin token")?;
    }
    Ok(())
}
