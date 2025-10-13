//! SoulBrowser L0 network tap (light) scaffold.
//!
//! The goal is to offer window-level network summaries and cached snapshots per page. The current
//! implementation keeps an in-memory registry that higher layers can use while the aggregation loop
//! is implemented.

pub mod config;

use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// Identifier representing a page for which the tap is collecting data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct PageId(pub Uuid);

impl PageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Window-level summary payload published on the event bus.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkSummary {
    pub page: PageId,
    pub window_ms: u64,
    pub req: u64,
    pub res2xx: u64,
    pub res4xx: u64,
    pub res5xx: u64,
    pub inflight: u64,
    pub quiet: bool,
    pub since_last_activity_ms: u64,
}

/// Snapshot representing cumulative counters exposed via pull-based API.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub req: u64,
    pub res2xx: u64,
    pub res4xx: u64,
    pub res5xx: u64,
    pub inflight: u64,
    pub quiet: bool,
    pub window_ms: u64,
    pub since_last_activity_ms: u64,
}

/// Errors emitted by the tap surface.
#[derive(Clone, Debug, Error)]
pub enum TapError {
    #[error("page not enabled")]
    PageNotEnabled,
    #[error("channel closed")]
    ChannelClosed,
    #[error("internal error: {0}")]
    Internal(String),
}

/// Broadcast channel for network summaries.
pub type SummaryBus = broadcast::Sender<NetworkSummary>;

struct PageState {
    snapshot: RwLock<NetworkSnapshot>,
}

impl Default for PageState {
    fn default() -> Self {
        Self {
            snapshot: RwLock::new(NetworkSnapshot::default()),
        }
    }
}

/// Core tap object; real implementation will spawn per-page tasks and aggregation loops.
pub struct NetworkTapLight {
    pub bus: SummaryBus,
    states: DashMap<PageId, Arc<PageState>>,
}

impl NetworkTapLight {
    pub fn new(buffer: usize) -> (Self, broadcast::Receiver<NetworkSummary>) {
        let (tx, rx) = broadcast::channel(buffer);
        (
            Self {
                bus: tx,
                states: DashMap::new(),
            },
            rx,
        )
    }

    pub async fn enable(&self, page: PageId) -> Result<(), TapError> {
        if self.states.contains_key(&page) {
            return Ok(());
        }
        self.states.insert(page, Arc::new(PageState::default()));
        Ok(())
    }

    pub async fn disable(&self, page: PageId) -> Result<(), TapError> {
        self.states
            .remove(&page)
            .map(|_| ())
            .ok_or(TapError::PageNotEnabled)
    }

    pub async fn update_snapshot(
        &self,
        page: PageId,
        snapshot: NetworkSnapshot,
    ) -> Result<(), TapError> {
        let state = self
            .states
            .get(&page)
            .ok_or(TapError::PageNotEnabled)?
            .clone();
        let mut guard = state.snapshot.write().await;
        *guard = snapshot;
        Ok(())
    }

    pub fn publish_summary(&self, summary: NetworkSummary) {
        let _ = self.bus.send(summary);
    }

    pub async fn current_snapshot(&self, page: PageId) -> Option<NetworkSnapshot> {
        let state = self.states.get(&page)?;
        let guard = state.snapshot.read().await;
        Some(guard.clone())
    }
}
