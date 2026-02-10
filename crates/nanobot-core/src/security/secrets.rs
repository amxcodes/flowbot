use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use sha2::Sha256;
use std::path::PathBuf;

const PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;

/// Manages encryption/decryption of secrets using AES-256-GCM
pub struct SecretManager {
    cipher: Aes256Gcm,
}

impl SecretManager {
    /// Create a new SecretManager from a password
    /// Derives encryption key using PBKDF2 with a stored salt
    pub fn new(password: &str, salt: &[u8; SALT_SIZE]) -> Result<Self> {
        if password.is_empty() {
            return Err(anyhow!("Password cannot be empty"));
        }

        // Derive 256-bit key from password
        let mut key = [0u8; 32];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| anyhow!("Failed to create cipher: {}", e))?;

        Ok(Self { cipher })
    }

    /// Encrypt plaintext and return base64-encoded ciphertext with nonce
    /// Format: base64(nonce || ciphertext)
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        // Generate random nonce
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;

        // Combine nonce + ciphertext
        let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        // Base64 encode
        Ok(general_purpose::STANDARD.encode(combined))
    }

    /// Decrypt base64-encoded ciphertext
    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        // Base64 decode
        let combined = general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| anyhow!("Invalid base64: {}", e))?;

        if combined.len() < NONCE_SIZE {
            return Err(anyhow!("Ciphertext too short"));
        }

        // Split nonce and ciphertext
        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        // Decrypt
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("Decryption failed (wrong password?): {}", e))?;

        String::from_utf8(plaintext).map_err(|e| anyhow!("Invalid UTF-8: {}", e))
    }

    pub fn generate_salt() -> [u8; SALT_SIZE] {
        let mut salt = [0u8; SALT_SIZE];
        rand::rng().fill_bytes(&mut salt);
        salt
    }

    /// Get salt storage path
    pub fn salt_path() -> PathBuf {
        PathBuf::from(".").join(".nanobot").join(".salt")
    }

    /// Load or create salt file
    pub fn load_or_create_salt() -> Result<[u8; SALT_SIZE]> {
        let salt_path = Self::salt_path();

        if salt_path.exists() {
            // Load existing salt
            let bytes = std::fs::read(&salt_path)?;
            if bytes.len() != SALT_SIZE {
                return Err(anyhow!("Invalid salt file"));
            }
            let mut salt = [0u8; SALT_SIZE];
            salt.copy_from_slice(&bytes);
            Ok(salt)
        } else {
            // Create new salt
            let salt = Self::generate_salt();

            // Ensure parent directory exists
            if let Some(parent) = salt_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(&salt_path, &salt)?;
            Ok(salt)
        }
    }

    /// Create SecretManager from environment variable or prompt
    pub fn from_env_or_prompt() -> Result<Self> {
        // Try environment variable first
        if let Ok(password) = std::env::var("NANOBOT_MASTER_PASSWORD") {
            let salt = Self::load_or_create_salt()?;
            return Self::new(&password, &salt);
        }

        // Interactive prompt (CLI only)
        #[cfg(not(test))]
        {
            use std::io::{self, Write};
            print!("Enter master password: ");
            io::stdout().flush()?;

            let password = rpassword::read_password()
                .map_err(|e| anyhow!("Failed to read password: {}", e))?;

            let salt = Self::load_or_create_salt()?;
            Self::new(&password, &salt)
        }

        #[cfg(test)]
        Err(anyhow!("Password required (set NANOBOT_MASTER_PASSWORD)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let salt = SecretManager::generate_salt();
        let manager = SecretManager::new("test-password-123", &salt).unwrap();

        let plaintext = "my-secret-api-key";
        let encrypted = manager.encrypt(plaintext).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_wrong_password() {
        let salt = SecretManager::generate_salt();
        let manager1 = SecretManager::new("password1", &salt).unwrap();
        let manager2 = SecretManager::new("password2", &salt).unwrap();

        let encrypted = manager1.encrypt("secret").unwrap();
        let result = manager2.decrypt(&encrypted);

        assert!(result.is_err());
    }

    #[test]
    fn test_empty_password() {
        let salt = SecretManager::generate_salt();
        let result = SecretManager::new("", &salt);
        assert!(result.is_err());
    }
}
