//! CDP lifecycle event integration for automatic cache invalidation.
//!
//! This module subscribes to CDP Page lifecycle events (navigate, load, DOMContentLoaded)
//! and automatically invalidates perception caches when the page state changes.

use std::sync::Arc;

use cdp_adapter::{EventBus, PageId, RawEvent};
use tokio::select;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::cache::{AnchorCache, SnapshotCache};

/// Lifecycle event watcher that invalidates caches on page state changes.
pub struct LifecycleWatcher {
    anchor_cache: Arc<AnchorCache>,
    snapshot_cache: Arc<SnapshotCache>,
    task: Option<JoinHandle<()>>,
    shutdown: CancellationToken,
}

impl LifecycleWatcher {
    /// Create a new lifecycle watcher.
    pub fn new(anchor_cache: Arc<AnchorCache>, snapshot_cache: Arc<SnapshotCache>) -> Self {
        Self {
            anchor_cache,
            snapshot_cache,
            task: None,
            shutdown: CancellationToken::new(),
        }
    }

    /// Start watching CDP lifecycle events from the given event bus.
    ///
    /// Cache invalidation policy:
    /// - `navigate`, `load`: Invalidate all caches for the page
    /// - `domcontentloaded`: Invalidate snapshot cache only
    /// - `networkidle`: No invalidation (network settling doesn't change DOM)
    /// - `frameattached`, `framedetached`: Invalidate snapshots (frames changed)
    pub fn start(&mut self, event_bus: EventBus) {
        if let Some(handle) = self.task.take() {
            handle.abort();
        }

        let anchor_cache = Arc::clone(&self.anchor_cache);
        let snapshot_cache = Arc::clone(&self.snapshot_cache);
        let shutdown = self.shutdown.clone();
        let mut rx = event_bus.subscribe();

        self.task = Some(tokio::spawn(async move {
            debug!(target: "perceiver-lifecycle", "lifecycle watcher started");
            loop {
                select! {
                    _ = shutdown.cancelled() => {
                        debug!(target: "perceiver-lifecycle", "lifecycle watcher shutting down");
                        break;
                    }
                    event = rx.recv() => {
                        match event {
                            Ok(raw_event) => {
                                Self::handle_event(
                                    &raw_event,
                                    &anchor_cache,
                                    &snapshot_cache,
                                );
                            }
                            Err(err) => {
                                warn!(?err, "lifecycle watcher event channel closed");
                                break;
                            }
                        }
                    }
                }
            }
            debug!(target: "perceiver-lifecycle", "lifecycle watcher exited");
        }));
    }

    /// Stop the lifecycle watcher.
    pub async fn stop(&mut self) {
        self.shutdown.cancel();
        if let Some(handle) = self.task.take() {
            let _ = handle.await;
        }
    }

    fn handle_event(
        event: &RawEvent,
        anchor_cache: &Arc<AnchorCache>,
        snapshot_cache: &Arc<SnapshotCache>,
    ) {
        match event {
            RawEvent::PageLifecycle { page, phase, .. } => {
                Self::handle_lifecycle_phase(*page, phase, anchor_cache, snapshot_cache);
            }
            RawEvent::PageNavigated { page, .. } => {
                Self::handle_lifecycle_phase(*page, "navigate", anchor_cache, snapshot_cache);
            }
            RawEvent::NetworkActivity { .. } => {
                // Network activity does not affect cached DOM structures
            }
            RawEvent::NetworkSummary { .. } => {
                // Network events don't change DOM structure, no invalidation needed
            }
            RawEvent::Error { .. } => {
                // Errors don't change page state, no invalidation needed
            }
        }
    }

    fn handle_lifecycle_phase(
        page: PageId,
        phase: &str,
        anchor_cache: &Arc<AnchorCache>,
        snapshot_cache: &Arc<SnapshotCache>,
    ) {
        let phase_lower = phase.to_ascii_lowercase();
        match phase_lower.as_str() {
            // Full page navigation - invalidate everything
            "navigate" | "load" | "commit" => {
                debug!(
                    target: "perceiver-lifecycle",
                    ?page,
                    phase = %phase,
                    "full page lifecycle event, invalidating all caches"
                );
                Self::invalidate_page(page, anchor_cache, snapshot_cache);
            }
            // DOM changed - invalidate snapshots, anchors might still be valid
            "domcontentloaded" => {
                debug!(
                    target: "perceiver-lifecycle",
                    ?page,
                    "domcontentloaded event, invalidating snapshot cache"
                );
                Self::invalidate_snapshots(page, snapshot_cache);
            }
            // Frame structure changed - invalidate snapshots
            "frame_attached" | "frame_detached" | "frameattached" | "framedetached" => {
                debug!(
                    target: "perceiver-lifecycle",
                    ?page,
                    phase = %phase,
                    "frame structure changed, invalidating snapshot cache"
                );
                Self::invalidate_snapshots(page, snapshot_cache);
            }
            // Network idle, page opened/closed/focus - no DOM changes
            "networkidle" | "opened" | "closed" | "focus" => {
                // No invalidation needed
            }
            other => {
                debug!(
                    target: "perceiver-lifecycle",
                    ?page,
                    phase = %other,
                    "unrecognized lifecycle phase, no cache invalidation"
                );
            }
        }
    }

    fn invalidate_page(
        page: PageId,
        anchor_cache: &Arc<AnchorCache>,
        snapshot_cache: &Arc<SnapshotCache>,
    ) {
        let anchor_prefix = format!("{:?}::", page);
        anchor_cache.invalidate_prefix(&anchor_prefix);
        let snapshot_prefix = format!("snapshot:{}:", page.0);
        snapshot_cache.invalidate_prefix(&snapshot_prefix);
    }

    fn invalidate_snapshots(page: PageId, snapshot_cache: &Arc<SnapshotCache>) {
        let snapshot_prefix = format!("snapshot:{}:", page.0);
        snapshot_cache.invalidate_prefix(&snapshot_prefix);
    }
}

impl Drop for LifecycleWatcher {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn lifecycle_watcher_invalidates_on_navigate() {
        let (bus, _rx) = cdp_adapter::event_bus(8);
        let anchor_cache = Arc::new(AnchorCache::new(Duration::from_secs(60)));
        let snapshot_cache = Arc::new(SnapshotCache::new(Duration::from_secs(60)));

        let page = cdp_adapter::PageId::new();
        let cache_key = format!("{:?}::test-selector", page);
        anchor_cache.put(
            cache_key.clone(),
            crate::model::AnchorResolution {
                primary: crate::model::AnchorDescriptor {
                    strategy: "test".into(),
                    value: serde_json::Value::Null,
                    frame_id: soulbrowser_core_types::FrameId::new(),
                    confidence: 1.0,
                    backend_node_id: None,
                    geometry: None,
                },
                candidates: vec![],
                reason: "test".into(),
                score: crate::model::ScoreBreakdown {
                    total: 1.0,
                    components: vec![],
                },
            },
        );

        let mut watcher =
            LifecycleWatcher::new(Arc::clone(&anchor_cache), Arc::clone(&snapshot_cache));
        watcher.start(bus.clone());

        // Emit navigate event
        let _ = bus.send(RawEvent::PageLifecycle {
            page,
            frame: None,
            parent: None,
            opener: None,
            phase: "navigate".into(),
            ts: 0,
        });

        // Give watcher time to process
        sleep(Duration::from_millis(50)).await;

        // Cache should be invalidated
        assert!(anchor_cache.get(&cache_key, None).is_none());

        watcher.stop().await;
    }

    #[tokio::test]
    async fn lifecycle_watcher_preserves_anchors_on_domcontentloaded() {
        let (bus, _rx) = cdp_adapter::event_bus(8);
        let anchor_cache = Arc::new(AnchorCache::new(Duration::from_secs(60)));
        let snapshot_cache = Arc::new(SnapshotCache::new(Duration::from_secs(60)));

        let page = cdp_adapter::PageId::new();
        let cache_key = format!("{:?}::test-selector", page);
        anchor_cache.put(
            cache_key.clone(),
            crate::model::AnchorResolution {
                primary: crate::model::AnchorDescriptor {
                    strategy: "test".into(),
                    value: serde_json::Value::Null,
                    frame_id: soulbrowser_core_types::FrameId::new(),
                    confidence: 1.0,
                    backend_node_id: None,
                    geometry: None,
                },
                candidates: vec![],
                reason: "test".into(),
                score: crate::model::ScoreBreakdown {
                    total: 1.0,
                    components: vec![],
                },
            },
        );

        let mut watcher =
            LifecycleWatcher::new(Arc::clone(&anchor_cache), Arc::clone(&snapshot_cache));
        watcher.start(bus.clone());

        // Emit domcontentloaded event
        let _ = bus.send(RawEvent::PageLifecycle {
            page,
            frame: None,
            parent: None,
            opener: None,
            phase: "domcontentloaded".into(),
            ts: 0,
        });

        // Give watcher time to process
        sleep(Duration::from_millis(50)).await;

        // Anchor cache should still be valid (only snapshots invalidated)
        assert!(anchor_cache.get(&cache_key, None).is_some());

        watcher.stop().await;
    }

    #[tokio::test]
    async fn lifecycle_watcher_stops_cleanly() {
        let (bus, _rx) = cdp_adapter::event_bus(8);
        let anchor_cache = Arc::new(AnchorCache::new(Duration::from_secs(60)));
        let snapshot_cache = Arc::new(SnapshotCache::new(Duration::from_secs(60)));

        let mut watcher = LifecycleWatcher::new(anchor_cache, snapshot_cache);
        watcher.start(bus);

        watcher.stop().await;

        // Should be stopped without panic
    }
}
