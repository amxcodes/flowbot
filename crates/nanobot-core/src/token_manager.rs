use crate::config::{OAuthTokens, ProviderToken};
use crate::oauth::OAuthFlow;
use anyhow::{Result, anyhow};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages OAuth tokens with automatic refresh logic
#[derive(Clone)]
pub struct TokenManager {
    provider: String,
    // We use RwLock to allow concurrent reads, but exclusive write during refresh
    current_token: Arc<RwLock<Option<ProviderToken>>>,
}

impl TokenManager {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            current_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Load token from disk initially
    pub async fn load_from_store(&self) -> Result<()> {
        let tokens = OAuthTokens::load()?; // Load from disk
        if let Some(token) = tokens.get(&self.provider) {
            let mut guard = self.current_token.write().await;
            *guard = Some(token.clone());
        }
        Ok(())
    }

    /// Get a valid access token, performing refresh if necessary
    pub async fn get_token(&self) -> Result<String> {
        // 1. Check existing token (Read Lock)
        {
            let guard = self.current_token.read().await;
            if let Some(token) = guard.as_ref()
                && !self.is_expired(token)
            {
                return Ok(token.access_token.clone());
            }
        }

        // 2. Refresh needed (Write Lock)
        // We re-check expiration after acquiring write lock to avoid race conditions
        let mut guard = self.current_token.write().await;

        // Double-check (in case another thread refreshed while we waited for lock)
        if let Some(token) = guard.as_ref()
            && !self.is_expired(token)
        {
            return Ok(token.access_token.clone());
        }

        // Get refresh token from current (or disk if current is missing/incomplete logic)
        // Ideally we should have the refresh token from the load.
        // If guard is None, we try to reload from disk first?

        let refresh_token_str = if let Some(token) = guard.as_ref() {
            token.refresh_token.clone()
        } else {
            // Try load from disk one last time
            let tokens = OAuthTokens::load()?;
            if let Some(disk_token) = tokens.get(&self.provider) {
                *guard = Some(disk_token.clone());
                if !self.is_expired(disk_token) {
                    return Ok(disk_token.access_token.clone());
                }
                disk_token.refresh_token.clone()
            } else {
                None
            }
        };

        let Some(refresh_token) = refresh_token_str else {
            return Err(anyhow!(
                "No refresh token available for {}. PLease login again.",
                self.provider
            ));
        };

        // Perform Refresh
        eprintln!(
            "DEBUG: Refreshing expired/missing token for provider: {}",
            self.provider
        );
        let flow = OAuthFlow::new(&self.provider);
        let mut new_token = flow.refresh_access_token(&refresh_token).await?;

        // Preserve refresh token if new one not provided
        if new_token.refresh_token.is_none() {
            new_token.refresh_token = Some(refresh_token);
        }

        // Save to Disk
        let mut tokens = OAuthTokens::load()?;
        tokens.set(self.provider.clone(), new_token.clone());
        tokens.save()?;

        // Update Memory
        let access = new_token.access_token.clone();
        *guard = Some(new_token);

        Ok(access)
    }

    fn is_expired(&self, token: &ProviderToken) -> bool {
        let now = chrono::Utc::now().timestamp();
        let grace_window = 300; // Refresh 5 minutes before expiration
        if let Some(expires_at) = token.expires_at {
            now + grace_window >= expires_at
        } else {
            false // Assume not expired if no expiry? Or true? safe to verify.
        }
    }
}
