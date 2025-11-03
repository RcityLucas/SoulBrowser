//! SoulBrowser L0 network tap (light) scaffold.
//!
//! The goal is to offer window-level network summaries and cached snapshots per page. The current
//! implementation keeps an in-memory registry that higher layers can use while the aggregation loop
//! is implemented.

pub mod config;

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::TapConfig;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
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

/// CDP-inspired events understood by the tap for aggregation.
#[derive(Clone, Debug)]
pub enum TapEvent {
    RequestWillBeSent,
    ResponseReceived { status: i64 },
    LoadingFinished,
    LoadingFailed,
}

/// Broadcast channel for network summaries.
pub type SummaryBus = broadcast::Sender<NetworkSummary>;

struct PageState {
    snapshot: RwLock<NetworkSnapshot>,
    counters: Mutex<Counters>,
}

impl PageState {
    fn new(config: &TapConfig) -> Self {
        Self {
            snapshot: RwLock::new(NetworkSnapshot::default()),
            counters: Mutex::new(Counters::new(config)),
        }
    }
}

#[derive(Debug)]
struct Counters {
    requests: u64,
    res2xx: u64,
    res4xx: u64,
    res5xx: u64,
    inflight: u64,
    last_activity: Instant,
    last_publish: Instant,
    last_quiet: bool,
}

impl Counters {
    fn new(config: &TapConfig) -> Self {
        let now = Instant::now();
        let last_publish = now
            .checked_sub(Duration::from_millis(config.min_publish_interval_ms))
            .unwrap_or(now);
        Self {
            requests: 0,
            res2xx: 0,
            res4xx: 0,
            res5xx: 0,
            inflight: 0,
            last_activity: now,
            last_publish,
            last_quiet: false,
        }
    }

    fn register(&mut self, event: &TapEvent, now: Instant) {
        match event {
            TapEvent::RequestWillBeSent => {
                self.requests += 1;
                self.inflight += 1;
                self.last_activity = now;
            }
            TapEvent::ResponseReceived { status } => {
                match *status {
                    200..=299 => self.res2xx += 1,
                    400..=499 => self.res4xx += 1,
                    500..=599 => self.res5xx += 1,
                    _ => {}
                }
                self.last_activity = now;
            }
            TapEvent::LoadingFinished | TapEvent::LoadingFailed => {
                if self.inflight > 0 {
                    self.inflight -= 1;
                }
                self.last_activity = now;
            }
        }
    }

    fn quiet(&self, now: Instant, config: &TapConfig) -> bool {
        if self.inflight != 0 {
            return false;
        }
        let since_last = now.saturating_duration_since(self.last_activity);
        since_last.as_millis() as u64 >= config.quiet_window_ms
    }

    fn evaluate_publish(&mut self, quiet: bool, now: Instant, config: &TapConfig) -> bool {
        let interval_elapsed = now.saturating_duration_since(self.last_publish).as_millis() as u64
            >= config.min_publish_interval_ms;
        let quiet_trigger = quiet && !self.last_quiet;
        self.last_quiet = quiet;
        if interval_elapsed || quiet_trigger {
            self.last_publish = now;
            true
        } else {
            false
        }
    }

    fn build_summary(&self, page: PageId, config: &TapConfig, now: Instant) -> NetworkSummary {
        let since_last = now
            .saturating_duration_since(self.last_activity)
            .as_millis() as u64;
        let quiet = self.inflight == 0 && since_last >= config.quiet_window_ms;
        NetworkSummary {
            page,
            window_ms: config.window_ms,
            req: self.requests,
            res2xx: self.res2xx,
            res4xx: self.res4xx,
            res5xx: self.res5xx,
            inflight: self.inflight,
            quiet,
            since_last_activity_ms: since_last,
        }
    }
}

/// Core tap object; real implementation will spawn per-page tasks and aggregation loops.
pub struct NetworkTapLight {
    pub bus: SummaryBus,
    states: DashMap<PageId, Arc<PageState>>,
    config: TapConfig,
}

/// Handle returned by [`NetworkTapLight::spawn_maintenance`] for lifecycle control.
pub struct MaintenanceHandle {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
}

impl MaintenanceHandle {
    /// Gracefully stop the maintenance loop and await its completion.
    pub async fn shutdown(mut self) -> Result<(), tokio::task::JoinError> {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            match task.await {
                Ok(_) => Ok(()),
                Err(err) if err.is_cancelled() => Ok(()),
                Err(err) => Err(err),
            }
        } else {
            Ok(())
        }
    }
}

impl Drop for MaintenanceHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl NetworkTapLight {
    pub fn new(buffer: usize) -> (Self, broadcast::Receiver<NetworkSummary>) {
        Self::with_config(TapConfig::default(), buffer)
    }

    pub fn with_config(
        config: TapConfig,
        buffer: usize,
    ) -> (Self, broadcast::Receiver<NetworkSummary>) {
        let (tx, rx) = broadcast::channel(buffer);
        (
            Self {
                bus: tx,
                states: DashMap::new(),
                config,
            },
            rx,
        )
    }

    /// Spawn a background task that periodically calls [`evaluate_timeouts`] based on
    /// [`TapConfig::maintenance_interval_ms`].
    pub fn spawn_maintenance(self: &Arc<Self>) -> MaintenanceHandle {
        let tap = Arc::clone(self);
        let cancel = CancellationToken::new();
        let loop_token = cancel.clone();
        let tick_interval = Duration::from_millis(self.config.maintenance_interval_ms.max(1));
        let task = tokio::spawn(async move {
            let mut ticker = interval(tick_interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = loop_token.cancelled() => {
                        break;
                    }
                    _ = ticker.tick() => {
                        tap.evaluate_timeouts().await;
                    }
                }
            }
        });
        MaintenanceHandle {
            cancel,
            task: Some(task),
        }
    }

    pub async fn enable(&self, page: PageId) -> Result<(), TapError> {
        if self.states.contains_key(&page) {
            return Ok(());
        }
        self.states
            .insert(page, Arc::new(PageState::new(&self.config)));
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

    pub async fn ingest(&self, page: PageId, event: TapEvent) -> Result<(), TapError> {
        let state = self
            .states
            .get(&page)
            .ok_or(TapError::PageNotEnabled)?
            .clone();
        let now = Instant::now();

        let mut counters = state.counters.lock().await;
        counters.register(&event, now);
        let summary = counters.build_summary(page, &self.config, now);
        let should_publish = counters.evaluate_publish(summary.quiet, now, &self.config);
        drop(counters);

        {
            let mut snapshot = state.snapshot.write().await;
            *snapshot = snapshot_from_summary(&summary);
        }

        if should_publish {
            self.publish_summary(summary);
        }

        Ok(())
    }

    pub async fn evaluate_timeouts(&self) {
        let now = Instant::now();
        for entry in self.states.iter() {
            let page = *entry.key();
            let state = entry.value().clone();
            let mut counters = state.counters.lock().await;
            let quiet = counters.quiet(now, &self.config);
            let should_publish = counters.evaluate_publish(quiet, now, &self.config);
            let summary = counters.build_summary(page, &self.config, now);
            drop(counters);

            if should_publish {
                {
                    let mut snapshot = state.snapshot.write().await;
                    *snapshot = snapshot_from_summary(&summary);
                }
                self.publish_summary(summary);
            }
        }
    }
}

fn snapshot_from_summary(summary: &NetworkSummary) -> NetworkSnapshot {
    NetworkSnapshot {
        req: summary.req,
        res2xx: summary.res2xx,
        res4xx: summary.res4xx,
        res5xx: summary.res5xx,
        inflight: summary.inflight,
        quiet: summary.quiet,
        window_ms: summary.window_ms,
        since_last_activity_ms: summary.since_last_activity_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::{sleep, timeout, Duration as TokioDuration};

    #[tokio::test]
    async fn ingest_updates_and_publishes_summary() {
        let (tap, mut rx) = NetworkTapLight::new(8);
        let page = PageId::new();
        tap.enable(page).await.expect("enable page");

        tap.ingest(page, TapEvent::RequestWillBeSent)
            .await
            .expect("record request");

        let summary = rx.recv().await.expect("receive summary");
        assert_eq!(summary.page, page);
        assert_eq!(summary.req, 1);
        assert_eq!(summary.inflight, 1);
        assert!(!summary.quiet);

        let snapshot = tap.current_snapshot(page).await.expect("snapshot");
        assert_eq!(snapshot.req, 1);
        assert_eq!(snapshot.inflight, 1);
    }

    #[tokio::test]
    async fn quiet_detection_emits_summary_after_timeout() {
        let config = TapConfig {
            window_ms: 100,
            quiet_window_ms: 50,
            min_publish_interval_ms: 1,
            maintenance_interval_ms: 10,
        };
        let (tap, mut rx) = NetworkTapLight::with_config(config, 8);
        let page = PageId::new();
        tap.enable(page).await.expect("enable page");

        tap.ingest(page, TapEvent::RequestWillBeSent)
            .await
            .expect("request event");
        // Drain initial publish
        let _ = rx.recv().await;

        tap.ingest(page, TapEvent::LoadingFinished)
            .await
            .expect("finish event");

        tokio::time::sleep(TokioDuration::from_millis(60)).await;
        tap.evaluate_timeouts().await;

        let summary = rx.recv().await.expect("quiet summary");
        assert_eq!(summary.page, page);
        assert!(summary.quiet);
        assert_eq!(summary.inflight, 0);
    }

    #[tokio::test]
    async fn maintenance_loop_emits_quiet_summary() {
        let config = TapConfig {
            window_ms: 100,
            quiet_window_ms: 40,
            min_publish_interval_ms: 1,
            maintenance_interval_ms: 10,
        };
        let (tap_raw, mut rx) = NetworkTapLight::with_config(config, 8);
        let tap = Arc::new(tap_raw);
        let maint = tap.spawn_maintenance();

        let page = PageId::new();
        tap.enable(page).await.expect("enable page");

        tap.ingest(page, TapEvent::RequestWillBeSent)
            .await
            .expect("request event");
        let _ = rx.recv().await; // drain first publish

        tap.ingest(page, TapEvent::LoadingFinished)
            .await
            .expect("finish event");

        sleep(TokioDuration::from_millis(60)).await;

        let summary = timeout(TokioDuration::from_millis(200), async move {
            let mut inner_rx = rx;
            loop {
                if let Ok(summary) = inner_rx.recv().await {
                    if summary.quiet {
                        break summary;
                    }
                }
            }
        })
        .await
        .expect("quiet summary timeout");

        assert!(summary.quiet);
        assert_eq!(summary.inflight, 0);

        maint.shutdown().await.expect("shutdown maintenance");
    }
}
