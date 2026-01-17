///! Visual Perceiver main implementation
use crate::{
    cache::ScreenshotCache, diff::VisualDiff, errors::VisualError, metrics::VisualMetrics,
    models::*, screenshot::ScreenshotCapture,
};
use async_trait::async_trait;
use cdp_adapter::{CdpAdapter, PageId};
use soulbrowser_core_types::ExecRoute;
use std::sync::Arc;
use std::time::Duration;

/// Visual perceiver trait
#[async_trait]
pub trait VisualPerceiver: Send + Sync {
    /// Capture screenshot
    async fn capture_screenshot(
        &self,
        route: &ExecRoute,
        options: ScreenshotOptions,
    ) -> Result<Screenshot, VisualError>;

    /// Extract text via OCR (requires 'ocr' feature)
    #[cfg(feature = "ocr")]
    async fn extract_text(
        &self,
        screenshot: &Screenshot,
        options: OcrOptions,
    ) -> Result<OcrResult, VisualError>;

    /// Compute visual diff between screenshots
    async fn compute_diff(
        &self,
        before: &Screenshot,
        after: &Screenshot,
        options: DiffOptions,
    ) -> Result<VisualDiffResult, VisualError>;

    /// Analyze visual metrics
    async fn analyze_metrics(
        &self,
        screenshot: &Screenshot,
    ) -> Result<VisualMetricsResult, VisualError>;
}

/// Visual perceiver implementation
pub struct VisualPerceiverImpl {
    screenshot_capture: Arc<ScreenshotCapture>,
    cache: Arc<ScreenshotCache>,

    #[cfg(feature = "ocr")]
    ocr_engine: Arc<crate::ocr::OcrEngine>,
}

impl VisualPerceiverImpl {
    /// Create new visual perceiver
    pub fn new(cdp_adapter: Arc<CdpAdapter>) -> Self {
        Self {
            screenshot_capture: Arc::new(ScreenshotCapture::new(cdp_adapter)),
            cache: Arc::new(ScreenshotCache::new(60)),

            #[cfg(feature = "ocr")]
            ocr_engine: Arc::new(crate::ocr::OcrEngine::new()),
        }
    }

    /// Create with custom cache TTL
    pub fn with_cache_ttl(cdp_adapter: Arc<CdpAdapter>, cache_ttl_secs: u64) -> Self {
        Self {
            screenshot_capture: Arc::new(ScreenshotCapture::new(cdp_adapter)),
            cache: Arc::new(ScreenshotCache::new(cache_ttl_secs)),

            #[cfg(feature = "ocr")]
            ocr_engine: Arc::new(crate::ocr::OcrEngine::new()),
        }
    }

    /// Get screenshot cache
    pub fn get_cache(&self) -> Arc<ScreenshotCache> {
        Arc::clone(&self.cache)
    }

    /// Parse page ID from ExecRoute
    fn parse_page_id(route: &ExecRoute) -> Result<PageId, VisualError> {
        uuid::Uuid::parse_str(&route.page.0)
            .map(PageId)
            .map_err(|e| VisualError::InvalidInput(format!("Invalid page ID: {}", e)))
    }

    /// Generate cache key for screenshot
    fn cache_key(page_id: &str, options: &ScreenshotOptions) -> String {
        format!("{}:{:?}:{:?}", page_id, options.capture_mode, options.clip)
    }
}

#[async_trait]
impl VisualPerceiver for VisualPerceiverImpl {
    async fn capture_screenshot(
        &self,
        route: &ExecRoute,
        options: ScreenshotOptions,
    ) -> Result<Screenshot, VisualError> {
        let page_id = Self::parse_page_id(route)?;
        let cache_key = Self::cache_key(&route.page.0, &options);

        // Check cache if enabled
        if options.use_cache {
            if let Some(cached) = self.cache.get(&cache_key) {
                tracing::debug!("Screenshot cache hit for key: {}", cache_key);
                return Ok(cached);
            }
        }

        // Capture new screenshot
        let screenshot = self
            .screenshot_capture
            .capture(page_id, options.clone())
            .await?;

        // Store in cache
        if options.use_cache {
            self.cache.put(
                cache_key,
                screenshot.clone(),
                Some(Duration::from_secs(options.cache_ttl_secs)),
            );
        }

        Ok(screenshot)
    }

    #[cfg(feature = "ocr")]
    async fn extract_text(
        &self,
        screenshot: &Screenshot,
        options: OcrOptions,
    ) -> Result<OcrResult, VisualError> {
        self.ocr_engine.extract_text(screenshot, options).await
    }

    async fn compute_diff(
        &self,
        before: &Screenshot,
        after: &Screenshot,
        options: DiffOptions,
    ) -> Result<VisualDiffResult, VisualError> {
        // Clone for blocking task
        let before = before.clone();
        let after = after.clone();

        // Run diff computation in blocking task (CPU intensive)
        tokio::task::spawn_blocking(move || VisualDiff::compute(&before, &after, options))
            .await
            .map_err(|e| VisualError::DiffFailed(format!("Task join error: {}", e)))?
    }

    async fn analyze_metrics(
        &self,
        screenshot: &Screenshot,
    ) -> Result<VisualMetricsResult, VisualError> {
        // Clone for blocking task
        let screenshot = screenshot.clone();

        // Run metrics analysis in blocking task (CPU intensive)
        tokio::task::spawn_blocking(move || VisualMetrics::analyze(&screenshot))
            .await
            .map_err(|e| VisualError::ImageProcessing(format!("Task join error: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_generation() {
        let options = ScreenshotOptions::default();
        let key = VisualPerceiverImpl::cache_key("page-123", &options);
        assert!(key.contains("page-123"));
        assert!(key.contains("Viewport"));
    }
}
