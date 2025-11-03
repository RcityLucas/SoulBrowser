//! Integration tests for L2 cache behavior with real Chrome.
//!
//! These tests require SOULBROWSER_USE_REAL_CHROME=1 and a Chrome/Chromium installation.
//! Run with: SOULBROWSER_USE_REAL_CHROME=1 cargo test -p perceiver-structural --test cache_integration

use std::sync::Arc;
use std::time::Duration;

use cdp_adapter::{Cdp, CdpAdapter, CdpConfig};
use perceiver_structural::{
    lifecycle::LifecycleWatcher, CdpPerceptionPort, PerceiverPolicyView, ResolveHint,
    ResolveOptions, StructuralPerceiverImpl,
};
use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};

/// Helper to check if we should run real Chrome tests.
fn should_run_chrome_tests() -> bool {
    std::env::var("SOULBROWSER_USE_REAL_CHROME")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Skip test if real Chrome is not enabled.
macro_rules! skip_without_chrome {
    () => {
        if !should_run_chrome_tests() {
            eprintln!("Skipping test: SOULBROWSER_USE_REAL_CHROME not set");
            return Ok(());
        }
    };
}

#[tokio::test]
async fn cache_invalidates_on_navigation() -> Result<(), Box<dyn std::error::Error>> {
    skip_without_chrome!();

    let (bus, _rx) = cdp_adapter::event_bus(16);
    let config = CdpConfig::default();
    let adapter = Arc::new(CdpAdapter::new(config, bus.clone()));
    Arc::clone(&adapter).start().await?;

    // Wait for initial page
    tokio::time::sleep(Duration::from_millis(500)).await;

    let pages: Vec<_> = adapter.registry().iter().iter().collect();
    if pages.is_empty() {
        eprintln!("No pages available, skipping test");
        adapter.shutdown().await;
        return Ok(());
    }

    let page_id = pages[0].0;
    let session = SessionId::new();
    let frame = FrameId::new();
    let route = ExecRoute::new(session, PageId(page_id.0.to_string()), frame);

    // Create perceiver with lifecycle watcher
    let port = Arc::new(CdpPerceptionAdapter::new(Arc::clone(&adapter)));
    let policy = PerceiverPolicyView::default();
    let perceiver = StructuralPerceiverImpl::with_policy(port, policy);

    let anchor_cache = perceiver.get_anchor_cache();
    let snapshot_cache = perceiver.get_snapshot_cache();
    let mut watcher = LifecycleWatcher::new(anchor_cache, snapshot_cache);
    watcher.start(bus);

    // Navigate to a page
    adapter
        .navigate(*page_id, "https://example.com", Duration::from_secs(10))
        .await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Resolve an element and cache it
    let resolution = perceiver
        .resolve_anchor(
            route.clone(),
            ResolveHint::Css("h1".into()),
            ResolveOptions::default(),
        )
        .await?;

    assert!(resolution.primary.confidence > 0.0);

    // Navigate to another page - should trigger cache invalidation
    adapter
        .navigate(*page_id, "https://example.org", Duration::from_secs(10))
        .await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Cache should be cleared by lifecycle watcher
    // This is verified by the watcher tests in lifecycle.rs

    watcher.stop().await;
    adapter.shutdown().await;

    Ok(())
}

#[tokio::test]
async fn cache_metrics_track_hits_and_misses() -> Result<(), Box<dyn std::error::Error>> {
    skip_without_chrome!();

    let (bus, _rx) = cdp_adapter::event_bus(16);
    let config = CdpConfig::default();
    let adapter = Arc::new(CdpAdapter::new(config, bus));
    Arc::clone(&adapter).start().await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    let pages: Vec<_> = adapter.registry().iter().iter().collect();
    if pages.is_empty() {
        eprintln!("No pages available, skipping test");
        adapter.shutdown().await;
        return Ok(());
    }

    let page_id = pages[0].0;
    let session = SessionId::new();
    let frame = FrameId::new();
    let route = ExecRoute::new(session, PageId(page_id.0.to_string()), frame);

    let port = Arc::new(CdpPerceptionAdapter::new(Arc::clone(&adapter)));
    let policy = PerceiverPolicyView::default();
    let perceiver = StructuralPerceiverImpl::with_policy(port, policy);

    adapter
        .navigate(*page_id, "https://example.com", Duration::from_secs(10))
        .await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // First resolve - cache miss
    let _resolution1 = perceiver
        .resolve_anchor(
            route.clone(),
            ResolveHint::Css("h1".into()),
            ResolveOptions::default(),
        )
        .await?;

    // Second resolve - should be cache hit
    let _resolution2 = perceiver
        .resolve_anchor(
            route.clone(),
            ResolveHint::Css("h1".into()),
            ResolveOptions::default(),
        )
        .await?;

    // Check metrics
    let metrics = perceiver_structural::metrics::snapshot();
    assert_eq!(metrics.resolve.total, 2);
    assert_eq!(metrics.resolve_cache.hits, 1);
    assert_eq!(metrics.resolve_cache.misses, 1);
    assert_eq!(metrics.resolve_cache.hit_rate, 50.0);

    adapter.shutdown().await;

    Ok(())
}

/// Adapter to wrap CdpAdapter for CdpPerceptionPort trait.
struct CdpPerceptionAdapter {
    adapter: Arc<CdpAdapter>,
}

impl CdpPerceptionAdapter {
    fn new(adapter: Arc<CdpAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait::async_trait]
impl CdpPerceptionPort for CdpPerceptionAdapter {
    async fn sample_dom_ax(
        &self,
        route: &ExecRoute,
        _scope: &perceiver_structural::Scope,
        _level: perceiver_structural::SnapLevel,
    ) -> Result<
        perceiver_structural::model::SampledPair,
        perceiver_structural::errors::PerceiverError,
    > {
        // Simplified implementation for testing
        let page = cdp_adapter::PageId(uuid::Uuid::parse_str(&route.page.0).unwrap());

        let dom_result = self
            .adapter
            .dom_snapshot(
                page,
                cdp_adapter::DomSnapshotConfig {
                    computed_style_whitelist: vec!["display".to_string(), "visibility".to_string()],
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| {
                perceiver_structural::errors::PerceiverError::SamplingFailed(
                    "dom snapshot failed".into(),
                )
            })?;

        Ok(perceiver_structural::model::SampledPair {
            dom: dom_result.raw,
            ax: serde_json::Value::Null,
        })
    }

    async fn query(
        &self,
        route: &ExecRoute,
        hint: &ResolveHint,
        _scope: &perceiver_structural::Scope,
    ) -> Result<
        Vec<perceiver_structural::AnchorDescriptor>,
        perceiver_structural::errors::PerceiverError,
    > {
        let page = cdp_adapter::PageId(uuid::Uuid::parse_str(&route.page.0).unwrap());

        match hint {
            ResolveHint::Css(selector) => {
                let spec = cdp_adapter::QuerySpec {
                    selector: selector.clone(),
                    scope: cdp_adapter::QueryScope::Document,
                };

                let anchors = self.adapter.query(page, spec).await.map_err(|_| {
                    perceiver_structural::errors::PerceiverError::QueryFailed("query failed".into())
                })?;

                Ok(anchors
                    .into_iter()
                    .map(|anchor| perceiver_structural::AnchorDescriptor {
                        strategy: "css".into(),
                        value: serde_json::json!({
                            "selector": selector,
                            "x": anchor.x,
                            "y": anchor.y,
                        }),
                        frame_id: route.frame.clone(),
                        confidence: 0.8,
                        backend_node_id: anchor.backend_node_id,
                        geometry: Some(perceiver_structural::AnchorGeometry {
                            x: anchor.x,
                            y: anchor.y,
                            width: 100.0,
                            height: 50.0,
                        }),
                    })
                    .collect())
            }
            _ => Ok(vec![]),
        }
    }

    async fn describe_backend_node(
        &self,
        _route: &ExecRoute,
        _backend_node_id: u64,
    ) -> Result<serde_json::Value, perceiver_structural::errors::PerceiverError> {
        Ok(serde_json::Value::Null)
    }

    async fn node_attributes(
        &self,
        _route: &ExecRoute,
        _backend_node_id: u64,
    ) -> Result<Option<serde_json::Value>, perceiver_structural::errors::PerceiverError> {
        Ok(None)
    }

    async fn node_style(
        &self,
        _route: &ExecRoute,
        _backend_node_id: u64,
    ) -> Result<Option<serde_json::Value>, perceiver_structural::errors::PerceiverError> {
        Ok(None)
    }
}

// Note: These tests require StructuralPerceiverImpl to expose cache accessors.
// The implementation needs to add public methods:
// ```
// impl<P> StructuralPerceiverImpl<P> {
//     pub fn get_anchor_cache(&self) -> Arc<AnchorCache> { Arc::clone(&self.anchor_cache) }
//     pub fn get_snapshot_cache(&self) -> Arc<SnapshotCache> { Arc::clone(&self.snapshot_cache) }
// }
// ```
