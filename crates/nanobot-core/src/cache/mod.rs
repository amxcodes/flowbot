//! High-performance caching layer for nanobot-rs
//!
//! Provides multi-tier caching for:
//! - LLM responses (with content-based hashing)
//! - Tool execution results (deterministic operations only)
//! - Session state (hot paths)

use dashmap::DashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// A cache entry with expiration time
struct CacheEntry<V> {
    value: V,
    expires_at: Instant,
}

impl<V> CacheEntry<V> {
    fn new(value: V, ttl: Duration) -> Self {
        Self {
            value,
            expires_at: Instant::now() + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() > self.expires_at
    }
}

/// High-performance concurrent cache with TTL support
pub struct ConcurrentCache<K, V> {
    inner: DashMap<K, CacheEntry<V>>,
    default_ttl: Duration,
}

impl<K, V> ConcurrentCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new cache with default TTL
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            inner: DashMap::new(),
            default_ttl,
        }
    }

    /// Get a value from the cache
    pub async fn get(&self, key: &K) -> Option<V> {
        if let Some(entry) = self.inner.get(key) {
            if !entry.is_expired() {
                return Some(entry.value.clone());
            }
            // Expired entry - remove it
            drop(entry);
            self.inner.remove(key);
        }
        None
    }

    /// Insert a value into the cache
    pub async fn insert(&self, key: K, value: V) {
        self.inner
            .insert(key, CacheEntry::new(value, self.default_ttl));
    }
}

/// Response cache specifically for LLM completions
pub struct ResponseCache {
    /// Cache for tool results
    tool_results: ConcurrentCache<String, String>,
}

impl ResponseCache {
    pub fn new() -> Self {
        Self {
            tool_results: ConcurrentCache::new(Duration::from_secs(60)), // 1 min
        }
    }

    /// Cache tool result
    pub async fn cache_tool_result(&self, tool_key: String, result: String) {
        self.tool_results.insert(tool_key, result).await;
    }

    /// Get cached tool result
    pub async fn get_tool_result(&self, tool_key: &str) -> Option<String> {
        self.tool_results.get(&tool_key.to_string()).await
    }
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

// Global cache instance
lazy_static::lazy_static! {
    pub static ref GLOBAL_CACHE: ResponseCache = ResponseCache::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_concurrent_cache_basic() {
        let cache = ConcurrentCache::new(Duration::from_secs(60));

        // Insert and retrieve
        cache.insert("key1".to_string(), "value1".to_string()).await;
        assert_eq!(
            cache.get(&"key1".to_string()).await,
            Some("value1".to_string())
        );

        // Non-existent key
        assert_eq!(cache.get(&"key2".to_string()).await, None);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache = ConcurrentCache::new(Duration::from_millis(10));

        cache.insert("key".to_string(), "value".to_string()).await;
        assert!(cache.get(&"key".to_string()).await.is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(cache.get(&"key".to_string()).await.is_none());
    }

}
