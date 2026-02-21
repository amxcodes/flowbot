use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

const PASSWORD_FILE: &str = "web_password";
const ENC_PREFIX: &str = "enc:v1:";

fn password_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .context("HOME/USERPROFILE environment variable not set")?;
    let dir = PathBuf::from(home).join(".nanobot");
    fs::create_dir_all(&dir).context("Failed to create .nanobot directory")?;
    Ok(dir)
}

fn password_path() -> Result<PathBuf> {
    Ok(password_dir()?.join(PASSWORD_FILE))
}

pub fn read_web_password() -> Result<Option<String>> {
    let path = password_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let password = fs::read_to_string(&path).context("Failed to read web password")?;
    let password = password.trim().to_string();
    if password.is_empty() {
        return Ok(None);
    }

    if let Some(cipher) = password.strip_prefix(ENC_PREFIX) {
        let session = crate::security::get_or_create_session_secrets()?;
        let salt = crate::security::SecretManager::load_or_create_salt()?;
        let manager = crate::security::SecretManager::new(&session.gateway_session_secret, &salt)?;
        let plain = manager
            .decrypt(cipher)
            .context("Failed to decrypt stored web password")?;
        if plain.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(plain))
        }
    } else {
        // Legacy plaintext password format; migrate in place.
        write_web_password(&password)?;
        Ok(Some(password))
    }
}

pub fn write_web_password(password: &str) -> Result<()> {
    let path = password_path()?;
    let session = crate::security::get_or_create_session_secrets()?;
    let salt = crate::security::SecretManager::load_or_create_salt()?;
    let manager = crate::security::SecretManager::new(&session.gateway_session_secret, &salt)?;
    let encrypted = manager
        .encrypt(password)
        .context("Failed to encrypt web password")?;
    let encoded = format!("{}{}", ENC_PREFIX, encrypted);

    fs::write(&path, encoded).context("Failed to write web password")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

pub fn clear_web_password() -> Result<()> {
    let path = password_path()?;
    if path.exists() {
        fs::remove_file(&path).context("Failed to remove web password")?;
    }
    Ok(())
}
