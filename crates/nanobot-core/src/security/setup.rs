use crate::config::EncryptedSecrets;
use crate::security::SecretManager;
use anyhow::Result;
use std::io::{self, Write};

/// Setup wizard for secrets encryption
pub fn run_setup_wizard() -> Result<()> {
    println!();
    println!("🔐 Nanobot Secrets Encryption Setup");
    println!("{}", "=".repeat(50));
    println!("This wizard will help you secure your API keys and tokens.");
    println!();

    // Check if salt already exists
    let salt_path = SecretManager::salt_path();
    if salt_path.exists() {
        println!("⚠️  Encryption is already set up.");
        print!("Do you want to reset it? (y/N): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Setup cancelled.");
            return Ok(());
        }
    }

    // Prompt for master password
    println!();
    println!("Step 1: Create Master Password");
    println!("This password will encrypt your secrets.");
    println!("You'll need it every time you start nanobot.");
    println!();

    print!("Enter master password: ");
    io::stdout().flush()?;
    let password = rpassword::read_password()?;

    if password.len() < 8 {
        return Err(anyhow::anyhow!("Password must be at least 8 characters"));
    }

    print!("Confirm password: ");
    io::stdout().flush()?;
    let confirm = rpassword::read_password()?;

    if password != confirm {
        return Err(anyhow::anyhow!("Passwords do not match"));
    }

    // Generate salt
    let salt = SecretManager::generate_salt();
    let salt_path = SecretManager::salt_path();
    if let Some(parent) = salt_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&salt_path, salt)?;

    let manager = SecretManager::new(&password, &salt)?;

    println!();
    println!("✅ Master password set!");
    println!();

    // Migrate existing secrets
    println!("Step 2: Migrate Existing Secrets");

    let config_path = crate::config::Config::config_path();
    let migrated = migrate_secrets(&manager, &config_path)?;

    if migrated {
        println!();
        println!("✅ Secrets migration complete!");
    } else {
        println!("No existing secrets found to migrate.");
    }

    // Summary
    println!();
    println!("{}", "=".repeat(50));
    println!("Setup Complete!");
    println!("Your secrets are now encrypted.");
    println!("Set NANOBOT_MASTER_PASSWORD env var to skip password prompt.");
    println!();

    Ok(())
}

/// Configure the master password if encryption has not been initialized yet.
/// Returns true if a new master password/salt was created, false if already configured.
pub fn setup_master_password_if_missing(password: &str) -> Result<bool> {
    if password.trim().len() < 8 {
        return Err(anyhow::anyhow!(
            "Primary password must be at least 8 characters"
        ));
    }

    let salt_path = SecretManager::salt_path();
    if salt_path.exists() {
        return Ok(false);
    }

    let salt = SecretManager::generate_salt();
    if let Some(parent) = salt_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&salt_path, salt)?;

    let manager = SecretManager::new(password.trim(), &salt)?;
    let config_path = crate::config::Config::config_path();
    let _ = migrate_secrets(&manager, &config_path)?;
    Ok(true)
}

fn migrate_secrets(manager: &SecretManager, config_path: &std::path::PathBuf) -> Result<bool> {
    let mut migrated = false;

    // Migrate tokens.json
    let tokens_path = crate::config::OAuthTokens::token_path();
    if tokens_path.exists() {
        println!("  Migrating tokens.json...");
        EncryptedSecrets::migrate_from_plaintext(manager)?;
        migrated = true;
    }

    // Migrate API keys from config.toml
    if config_path.exists() {
        println!("  Migrating API keys from config.toml...");
        EncryptedSecrets::migrate_api_keys_from_config(manager, config_path)?;
        migrated = true;
    }

    Ok(migrated)
}

/// Verify master password works
pub fn verify_password() -> Result<SecretManager> {
    let salt = SecretManager::load_or_create_salt()?;

    // Try primary password sources first
    if let Some(password) = crate::security::read_primary_password() {
        match SecretManager::new(&password, &salt) {
            Ok(manager) => return Ok(manager),
            Err(_) => {
                println!("❌ Invalid configured primary password");
                return Err(anyhow::anyhow!("Invalid master password"));
            }
        }
    }

    // Interactive prompt
    print!("🔐 Enter master password: ");
    io::stdout().flush()?;

    let password = rpassword::read_password()?;
    let manager = SecretManager::new(&password, &salt)
        .map_err(|_| anyhow::anyhow!("Invalid master password"))?;

    Ok(manager)
}
