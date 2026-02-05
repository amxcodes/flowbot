mod db;

use anyhow::{Result, bail};
use rand::Rng;
use std::time::{SystemTime, UNIX_EPOCH};

pub use db::{init_database, PairingRequest};

/// Generate a random 6-digit pairing code
pub fn generate_code() -> String {
    let mut rng = rand::rng();
    format!("{:06}", rng.random_range(0..1000000))
}

/// Create a new pairing request for a user
/// Returns the generated code
pub async fn create_pairing_request(
    channel: &str,
    user_id: String,
    username: Option<String>,
) -> Result<String> {
    let code = generate_code();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let expires_at = now + 3600; // 1 hour from now
    
    db::insert_pairing_request(channel, &user_id, username.as_deref(), &code, now, expires_at).await?;
    Ok(code)
}

/// Check if a user is authorized
pub async fn is_authorized(channel: &str, user_id: &str) -> Result<bool> {
    db::is_user_authorized(channel, user_id).await
}

/// Get the code for a user if they have a pending request
pub async fn get_user_code(channel: &str, user_id: &str) -> Result<Option<String>> {
    db::get_user_pending_code(channel, user_id).await
}

/// Get all pending pairing requests for a channel (or all channels if "all")
pub async fn get_pending_requests(channel: &str) -> Result<Vec<PairingRequest>> {
    // Clean up expired requests first
    cleanup_expired().await?;
    
    if channel == "all" {
        db::get_all_pending_requests().await
    } else {
        db::get_pending_requests_for_channel(channel).await
    }
}

/// Approve a pairing request by code
/// Returns the user_id that was approved
pub async fn approve(channel: &str, code: &str) -> Result<String> {
    // Find the request
    let request = db::get_request_by_code(channel, code).await?;
    
    if let Some(req) = request {
        // Check if expired
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        if now > req.expires_at {
            db::delete_pairing_request(channel, code).await?;
            bail!("Pairing code {} has expired", code);
        }
        
        // Add to authorized users
        db::add_authorized_user(channel, &req.user_id, req.username.as_deref(), now).await?;
        
        // Remove from pending
        db::delete_pairing_request(channel, code).await?;
        
        Ok(req.user_id)
    } else {
        bail!("Pairing code {} not found for channel {}", code, channel);
    }
}

/// Reject a pairing request by code
pub async fn reject(channel: &str, code: &str) -> Result<()> {
    let deleted = db::delete_pairing_request(channel, code).await?;
    if deleted == 0 {
        bail!("Pairing code {} not found for channel {}", code, channel);
    }
    Ok(())
}

/// Clean up expired pairing requests
async fn cleanup_expired() -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    db::delete_expired_requests(now).await?;
    Ok(())
}

/// Auto-approve a user (for migration from TELEGRAM_ALLOWED_USERS)
pub async fn auto_approve(channel: &str, user_id: &str, username: &str) -> Result<()> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    db::add_authorized_user(channel, user_id, Some(username), now).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_code() {
        let code = generate_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_numeric()));
        
        // Generate multiple codes to check randomness
        let code2 = generate_code();
        // Should be different (very high probability)
        assert_ne!(code, code2);
    }

    #[tokio::test]
    async fn test_pairing_flow() {
        // This would need a test database instance
        // For now, just a placeholder
        // TODO: Implement with test database
    }
}
