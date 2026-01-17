///! Screenshot caching with TTL support
use crate::models::*;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Cache for screenshots
pub struct ScreenshotCache {
    cache: Arc<DashMap<String, CachedScreenshot>>,
    default_ttl: Duration,
}

struct CachedScreenshot {
    screenshot: Screenshot,
    expires_at: SystemTime,
}

impl ScreenshotCache {
    pub fn new(default_ttl_secs: u64) -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            default_ttl: Duration::from_secs(default_ttl_secs),
        }
    }

    /// Get screenshot from cache if valid
    pub fn get(&self, key: &str) -> Option<Screenshot> {
        if let Some(entry) = self.cache.get(key) {
            if entry.expires_at > SystemTime::now() {
                return Some(entry.screenshot.clone());
            }
            // Expired, remove it
            drop(entry);
            self.cache.remove(key);
        }
        None
    }

    /// Put screenshot into cache
    pub fn put(&self, key: String, screenshot: Screenshot, ttl: Option<Duration>) {
        let expires_at = SystemTime::now() + ttl.unwrap_or(self.default_ttl);
        self.cache.insert(
            key,
            CachedScreenshot {
                screenshot,
                expires_at,
            },
        );
    }

    /// Invalidate cache for a page
    pub fn invalidate_page(&self, page_id: &str) {
        self.cache.retain(|k, _| !k.starts_with(page_id));
    }

    /// Clear all cached screenshots
    pub fn clear(&self) {
        self.cache.clear();
    }

    /// Get cache size
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_operations() {
        let cache = ScreenshotCache::new(60);
        assert!(cache.is_empty());

        let screenshot = Screenshot {
            id: "test-1".to_string(),
            data: vec![1, 2, 3],
            format: ImageFormat::Png,
            width: 100,
            height: 100,
            timestamp: SystemTime::now(),
            page_id: "page-1".to_string(),
            capture_mode: CaptureMode::Viewport,
            clip: None,
        };

        cache.put("key-1".to_string(), screenshot.clone(), None);
        assert_eq!(cache.len(), 1);

        let retrieved = cache.get("key-1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test-1");
    }

    #[test]
    fn test_cache_invalidation() {
        let cache = ScreenshotCache::new(60);

        let screenshot = Screenshot {
            id: "test-1".to_string(),
            data: vec![1, 2, 3],
            format: ImageFormat::Png,
            width: 100,
            height: 100,
            timestamp: SystemTime::now(),
            page_id: "page-1".to_string(),
            capture_mode: CaptureMode::Viewport,
            clip: None,
        };

        cache.put("page-1:viewport".to_string(), screenshot, None);
        assert_eq!(cache.len(), 1);

        cache.invalidate_page("page-1");
        assert_eq!(cache.len(), 0);
    }
}
