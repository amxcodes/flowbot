//! High-performance HTTP client with connection pooling
//!
//! Provides optimized HTTP clients with:
//! - Connection pooling and reuse
//! - Configurable timeouts
//! - Request/response reuse

use reqwest::{Client, ClientBuilder};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// Configuration for HTTP connection pooling
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// Maximum idle connections per host
    pub pool_max_idle: usize,
    /// Timeout for idle connections
    pub pool_idle_timeout: Duration,
    /// Request timeout
    pub timeout: Duration,
    /// Connect timeout
    pub connect_timeout: Duration,
    /// Maximum concurrent requests
    pub max_concurrent: usize,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            pool_max_idle: 10,
            pool_idle_timeout: Duration::from_secs(90),
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            max_concurrent: 100,
        }
    }
}

/// High-performance HTTP client wrapper
pub struct PooledHttpClient {
    inner: Client,
    semaphore: Arc<Semaphore>,
}

impl PooledHttpClient {
    /// Create a new pooled HTTP client with default configuration
    pub fn new() -> anyhow::Result<Self> {
        Self::with_config(HttpClientConfig::default())
    }

    /// Create a new pooled HTTP client with custom configuration
    pub fn with_config(config: HttpClientConfig) -> anyhow::Result<Self> {
        let client = ClientBuilder::new()
            .pool_max_idle_per_host(config.pool_max_idle)
            .pool_idle_timeout(config.pool_idle_timeout)
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .http2_prior_knowledge()
            .build()?;

        Ok(Self {
            inner: client,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent)),
        })
    }

    /// Get a reference to the inner client
    pub fn inner(&self) -> &Client {
        &self.inner
    }

    /// Acquire a concurrency permit before issuing a request.
    pub async fn acquire_permit(&self) -> anyhow::Result<tokio::sync::OwnedSemaphorePermit> {
        Ok(Arc::clone(&self.semaphore).acquire_owned().await?)
    }
}

impl Clone for PooledHttpClient {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            semaphore: Arc::clone(&self.semaphore),
        }
    }
}

/// Global HTTP client instance (lazy initialization)
use std::sync::OnceLock;

static GLOBAL_HTTP_CLIENT: OnceLock<PooledHttpClient> = OnceLock::new();

/// Get or initialize the global HTTP client
pub fn global_http_client() -> &'static PooledHttpClient {
    GLOBAL_HTTP_CLIENT
        .get_or_init(|| PooledHttpClient::new().expect("Failed to initialize global HTTP client"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_client_config_default() {
        let config = HttpClientConfig::default();
        assert_eq!(config.pool_max_idle, 10);
        assert_eq!(config.timeout, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_pooled_client_creation() {
        let client = PooledHttpClient::new();
        assert!(client.is_ok());
    }
}
