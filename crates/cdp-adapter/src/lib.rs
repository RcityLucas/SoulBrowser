//! SoulBrowser L0 CDP adapter scaffold.
//!
//! This crate hosts the future Chromium DevTools Protocol integration. For now it exposes the
//! data structures and traits that the higher layers will wire against while the concrete
//! implementation is filled in milestone by milestone.

use tokio::sync::broadcast;

pub mod ids {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    /// Unique identifier for the browser instance managed by the adapter.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
    pub struct BrowserId(pub Uuid);

    /// Unique identifier for a page/tab.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
    pub struct PageId(pub Uuid);

    /// Unique identifier for a CDP session.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
    pub struct SessionId(pub Uuid);

    /// Unique identifier for a frame.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
    pub struct FrameId(pub Uuid);

    impl BrowserId {
        pub fn new() -> Self {
            Self(Uuid::new_v4())
        }
    }

    impl PageId {
        pub fn new() -> Self {
            Self(Uuid::new_v4())
        }
    }

    impl SessionId {
        pub fn new() -> Self {
            Self(Uuid::new_v4())
        }
    }

    impl FrameId {
        pub fn new() -> Self {
            Self(Uuid::new_v4())
        }
    }
}

pub mod error {
    use serde::{Deserialize, Serialize};
    use thiserror::Error;

    /// High-level error categories surfaced by the adapter.
    #[derive(Clone, Debug, Error, Serialize, Deserialize)]
    pub enum AdapterErrorKind {
        #[error("navigation timed out")]
        NavTimeout,
        #[error("cdp i/o failure")]
        CdpIo,
        #[error("policy denied")]
        PolicyDenied,
        #[error("internal error")]
        Internal,
    }

    /// Enriched error metadata passed back to higher layers.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct AdapterError {
        pub kind: AdapterErrorKind,
        pub hint: Option<String>,
        pub retriable: bool,
        pub data: Option<serde_json::Value>,
    }

    impl AdapterError {
        pub fn new(kind: AdapterErrorKind) -> Self {
            Self {
                kind,
                hint: None,
                retriable: false,
                data: None,
            }
        }

        pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
            self.hint = Some(hint.into());
            self
        }

        pub fn retriable(mut self, flag: bool) -> Self {
            self.retriable = flag;
            self
        }

        pub fn with_data(mut self, data: serde_json::Value) -> Self {
            self.data = Some(data);
            self
        }
    }
}

pub mod events {
    use super::ids::{FrameId, PageId};
    use serde::{Deserialize, Serialize};

    /// Raw events emitted by the adapter before higher-level aggregation.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub enum RawEvent {
        PageLifecycle {
            page: PageId,
            frame: Option<FrameId>,
            parent: Option<FrameId>,
            phase: String,
            ts: u64,
        },
        NetworkSummary {
            page: PageId,
            req: u64,
            res2xx: u64,
            res4xx: u64,
            res5xx: u64,
            inflight: u64,
            quiet: bool,
            window_ms: u64,
            since_last_activity_ms: u64,
        },
        Error {
            page: Option<PageId>,
            message: String,
        },
    }

    /// Subscription filter placeholder; will expand with real predicates.
    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct EventFilter;
}

pub mod config {
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;
    use std::{env, path::Path};

    /// Configuration for launching and tuning the adapter.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct CdpConfig {
        pub executable: PathBuf,
        pub user_data_dir: PathBuf,
        pub headless: bool,
        pub default_deadline_ms: u64,
        pub retry_backoff_ms: u64,
        pub websocket_url: Option<String>,
    }

    impl Default for CdpConfig {
        fn default() -> Self {
            Self {
                executable: default_chrome_path(),
                user_data_dir: default_profile_dir(),
                headless: true,
                default_deadline_ms: 30_000,
                retry_backoff_ms: 250,
                websocket_url: None,
            }
        }
    }

    fn default_chrome_path() -> PathBuf {
        env::var("SOULBROWSER_CHROME")
            .map(PathBuf::from)
            .unwrap_or_default()
    }

    fn default_profile_dir() -> PathBuf {
        if let Ok(path) = env::var("SOULBROWSER_CHROME_PROFILE") {
            return PathBuf::from(path);
        }

        let default = Path::new("./.soulbrowser-profile");
        default.into()
    }
}

pub mod adapter {
    use super::commands::{
        Anchor, AxSnapshotConfig, AxSnapshotResult, DomSnapshotConfig, DomSnapshotResult,
        QueryScope, QuerySpec, SelectSpec, WaitGate,
    };
    use super::config::CdpConfig;
    use super::error::{AdapterError, AdapterErrorKind};
    use super::events::{EventFilter, RawEvent};
    use super::ids::{BrowserId, FrameId, PageId, SessionId};
    use super::metrics;
    use super::registry::Registry;
    use super::transport::{
        CdpTransport, ChromiumTransport, CommandTarget, NoopTransport, TransportEvent,
    };
    use async_trait::async_trait;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use dashmap::DashMap;
    use serde::Deserialize;
    use serde_json::{json, Number, Value};
    use std::env;
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::broadcast;
    use tokio::sync::Mutex;
    use tokio::task::JoinHandle;
    use tokio::time::sleep;
    use tokio::{select, spawn};
    use tokio_util::sync::CancellationToken;
    use tracing::{debug, info};

    /// Shared event bus type alias used by the adapter scaffold.
    pub type EventBus = broadcast::Sender<RawEvent>;

    /// Trait capturing the minimal CDP capability surface required by upper layers.
    #[async_trait]
    pub trait Cdp {
        async fn navigate(
            &self,
            page: PageId,
            url: &str,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError>;
        async fn query(&self, page: PageId, spec: QuerySpec) -> Result<Vec<Anchor>, AdapterError>;
        async fn click(
            &self,
            page: PageId,
            selector: &str,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError>;
        async fn type_text(
            &self,
            page: PageId,
            selector: &str,
            text: &str,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError>;
        async fn select_option(
            &self,
            page: PageId,
            spec: SelectSpec,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError>;
        async fn wait_basic(
            &self,
            page: PageId,
            gate: String,
            timeout: std::time::Duration,
        ) -> Result<(), AdapterError>;
        async fn screenshot(
            &self,
            page: PageId,
            deadline: std::time::Duration,
        ) -> Result<Vec<u8>, AdapterError>;
        async fn set_network_tap(&self, page: PageId, enabled: bool) -> Result<(), AdapterError>;

        async fn dom_snapshot(
            &self,
            page: PageId,
            config: DomSnapshotConfig,
        ) -> Result<DomSnapshotResult, AdapterError>;

        async fn ax_snapshot(
            &self,
            page: PageId,
            config: AxSnapshotConfig,
        ) -> Result<AxSnapshotResult, AdapterError>;
    }

    /// Adapter implementation with pluggable transport.
    pub struct CdpAdapter {
        pub browser_id: BrowserId,
        pub cfg: CdpConfig,
        pub bus: EventBus,
        pub registry: Arc<Registry>,
        shutdown: CancellationToken,
        tasks: Mutex<Vec<JoinHandle<()>>>,
        transport: Arc<dyn CdpTransport>,
        targets: DashMap<String, PageId>,
        sessions: DashMap<String, PageId>,
        frames: DashMap<String, FrameEntry>,
        network_stats: DashMap<PageId, NetworkStats>,
        page_activity: DashMap<PageId, Instant>,
    }

    #[derive(Clone, Copy, Debug)]
    struct FrameEntry {
        page: PageId,
        frame: FrameId,
    }

    #[derive(Clone, Debug)]
    struct NetworkStats {
        requests: u64,
        responses_2xx: u64,
        responses_4xx: u64,
        responses_5xx: u64,
        inflight: i64,
        last_activity: Instant,
    }

    impl NetworkStats {
        fn new() -> Self {
            Self {
                requests: 0,
                responses_2xx: 0,
                responses_4xx: 0,
                responses_5xx: 0,
                inflight: 0,
                last_activity: Instant::now(),
            }
        }

        fn register_request(&mut self) {
            self.requests += 1;
            self.inflight += 1;
            self.last_activity = Instant::now();
        }

        fn register_response(&mut self, status: i64) {
            match status {
                200..=299 => self.responses_2xx += 1,
                400..=499 => self.responses_4xx += 1,
                500..=599 => self.responses_5xx += 1,
                _ => {}
            }
            self.last_activity = Instant::now();
        }

        fn register_complete(&mut self) {
            if self.inflight > 0 {
                self.inflight -= 1;
            }
            self.last_activity = Instant::now();
        }

        fn snapshot(&self) -> (u64, u64, u64, u64, u64, bool, u64) {
            let since_last = self.last_activity.elapsed().as_millis() as u64;
            let quiet = self.inflight == 0 && since_last >= 1_000;
            (
                self.requests,
                self.responses_2xx,
                self.responses_4xx,
                self.responses_5xx,
                self.inflight.max(0) as u64,
                quiet,
                since_last,
            )
        }
    }

    impl CdpAdapter {
        pub fn new(cfg: CdpConfig, bus: EventBus) -> Self {
            let transport: Arc<dyn CdpTransport> = if should_use_real_chrome() {
                info!(target: "cdp-adapter", "using real Chromium transport");
                Arc::new(ChromiumTransport::new(cfg.clone()))
            } else {
                info!(target: "cdp-adapter", "using Noop transport (set SOULBROWSER_USE_REAL_CHROME=1 to enable real browser)");
                Arc::new(NoopTransport::default())
            };
            Self::with_transport(cfg, bus, transport)
        }

        pub fn with_transport(
            cfg: CdpConfig,
            bus: EventBus,
            transport: Arc<dyn CdpTransport>,
        ) -> Self {
            Self {
                browser_id: BrowserId::new(),
                cfg,
                bus,
                registry: Arc::new(Registry::new()),
                shutdown: CancellationToken::new(),
                tasks: Mutex::new(Vec::new()),
                transport,
                targets: DashMap::new(),
                sessions: DashMap::new(),
                frames: DashMap::new(),
                network_stats: DashMap::new(),
                page_activity: DashMap::new(),
            }
        }

        pub fn registry(&self) -> Arc<Registry> {
            Arc::clone(&self.registry)
        }

        pub fn cancel_token(&self) -> CancellationToken {
            self.shutdown.clone()
        }

        pub async fn start(self: Arc<Self>) -> Result<(), AdapterError> {
            self.transport.start().await?;
            let loop_task = spawn(Self::event_loop(Arc::clone(&self)));
            self.tasks.lock().await.push(loop_task);
            info!(target: "cdp-adapter", "event loop started (real CDP wiring pending)");
            if self.cfg.websocket_url.is_none() {
                self.ensure_initial_page().await?;
            }
            Ok(())
        }

        pub async fn shutdown(&self) {
            self.shutdown.cancel();
            let mut handles = self.tasks.lock().await;
            while let Some(handle) = handles.pop() {
                let _ = handle.await;
            }
        }

        pub fn register_page(
            &self,
            page: PageId,
            session: SessionId,
            target_id: Option<String>,
            cdp_session: Option<String>,
        ) {
            self.registry
                .insert_page(page, session, target_id, cdp_session);
        }

        async fn event_loop(self: Arc<Self>) {
            debug!(target: "cdp-adapter", "event loop entered");
            loop {
                select! {
                    _ = self.shutdown.cancelled() => {
                        break;
                    }
                    event = self.transport.next_event() => {
                        match event {
                            Some(ev) => self.handle_event(ev).await,
                            None => break,
                        }
                    }
                }
            }
            debug!(target: "cdp-adapter", "event loop exiting");
        }

        async fn handle_event(&self, event: TransportEvent) {
            if let Err(err) = self.process_event(event).await {
                let _ = self.bus.send(RawEvent::Error {
                    page: None,
                    message: format!("cdp event handling error: {:?}", err),
                });
            }
        }

        async fn process_event(&self, event: TransportEvent) -> Result<(), AdapterError> {
            metrics::record_event();
            match event.method.as_str() {
                "Target.targetCreated" => {
                    self.on_target_created(event.params)?;
                }
                "Target.targetDestroyed" => {
                    self.on_target_destroyed(event.params)?;
                }
                "Target.attachedToTarget" => {
                    self.on_target_attached(event.params)?;
                }
                "Target.detachedFromTarget" => {
                    self.on_target_detached(event.params)?;
                }
                "Page.lifecycleEvent" => {
                    self.on_page_lifecycle(event).await?;
                }
                "Page.frameAttached" => {
                    self.on_frame_attached(event).await?;
                }
                "Page.frameDetached" => {
                    self.on_frame_detached(event).await?;
                }
                "Network.requestWillBeSent" => {
                    self.on_network_request(event).await?;
                }
                "Network.responseReceived" => {
                    self.on_network_response(event).await?;
                }
                "Network.loadingFinished" => {
                    self.on_network_finished(event).await?;
                }
                "Network.loadingFailed" => {
                    self.on_network_failed(event).await?;
                }
                "Runtime.exceptionThrown" => {
                    self.on_exception_thrown(event).await?;
                }
                _ => {
                    debug!(target: "cdp-adapter", method = %event.method, "unhandled cdp event");
                    return Err(AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint(format!("unhandled cdp event: {}", event.method)));
                }
            }
            Ok(())
        }

        fn on_target_created(&self, params: Value) -> Result<(), AdapterError> {
            let payload: TargetCreatedParams = serde_json::from_value(params).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;

            if payload.target_info.target_type != "page" {
                return Ok(());
            }

            let target_id = payload.target_info.target_id;
            let page_id = PageId::new();
            let session = SessionId::new();

            self.targets.insert(target_id.clone(), page_id);
            self.network_stats.insert(page_id, NetworkStats::new());
            self.registry
                .insert_page(page_id, session, Some(target_id), None);

            if let Some(url) = payload.target_info.url.filter(|u| !u.is_empty()) {
                self.registry.set_recent_url(&page_id, url);
            }

            self.emit_page_event(page_id, None, None, "opened", timestamp_now());
            Ok(())
        }

        fn on_target_destroyed(&self, params: Value) -> Result<(), AdapterError> {
            let payload: TargetDestroyedParams = serde_json::from_value(params).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;

            if let Some((_, page)) = self.targets.remove(&payload.target_id) {
                self.sessions.retain(|_, v| *v != page);
                self.frames.retain(|_, entry| entry.page != page);
                self.network_stats.remove(&page);
                self.page_activity.remove(&page);
                self.registry.remove_page(&page);
                self.emit_page_event(page, None, None, "closed", timestamp_now());
            }
            Ok(())
        }

        fn on_target_attached(&self, params: Value) -> Result<(), AdapterError> {
            let payload: AttachedToTargetParams =
                serde_json::from_value(params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if payload.target_info.target_type != "page" {
                return Ok(());
            }

            if let Some(page_entry) = self.targets.get(&payload.target_info.target_id) {
                let page = *page_entry.value();
                self.sessions.insert(payload.session_id.clone(), page);
                self.registry
                    .set_cdp_session(&page, payload.session_id.clone());
                self.emit_page_event(page, None, None, "focus", timestamp_now());
            }

            Ok(())
        }

        fn on_target_detached(&self, params: Value) -> Result<(), AdapterError> {
            let payload: DetachedFromTargetParams =
                serde_json::from_value(params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;
            self.sessions.remove(&payload.session_id);
            Ok(())
        }

        async fn on_page_lifecycle(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: PageLifecycleParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            let page = self.page_from_session(event.session_id.as_ref());
            if let Some(page_id) = page {
                let frame_id = payload
                    .frame_id
                    .as_ref()
                    .and_then(|frame_key| self.frames.get(frame_key).map(|entry| entry.frame));
                let phase = payload.name.to_ascii_lowercase();
                let ts = payload
                    .timestamp
                    .map(|t| (t * 1_000.0) as u64)
                    .unwrap_or_else(timestamp_now);
                self.emit_page_event(page_id, frame_id, None, &phase, ts);
            }

            Ok(())
        }

        async fn on_frame_attached(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: FrameAttachedParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                let parent = payload
                    .parent_frame_id
                    .as_ref()
                    .and_then(|fid| self.frames.get(fid).map(|entry| entry.frame));
                let frame_id = FrameId::new();
                self.frames.insert(
                    payload.frame_id.clone(),
                    FrameEntry {
                        page,
                        frame: frame_id,
                    },
                );
                self.emit_page_event(
                    page,
                    Some(frame_id),
                    parent,
                    "frame_attached",
                    timestamp_now(),
                );
            }

            Ok(())
        }

        async fn on_frame_detached(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: FrameDetachedParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if let Some((_, entry)) = self.frames.remove(&payload.frame_id) {
                self.emit_page_event(
                    entry.page,
                    Some(entry.frame),
                    None,
                    "frame_detached",
                    timestamp_now(),
                );
            }
            Ok(())
        }

        async fn on_network_request(&self, event: TransportEvent) -> Result<(), AdapterError> {
            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                let mut entry = self
                    .network_stats
                    .entry(page)
                    .or_insert_with(NetworkStats::new);
                entry.value_mut().register_request();
                let snapshot = entry.value().clone();
                drop(entry);
                self.emit_network_summary(page, snapshot);
            }
            Ok(())
        }

        async fn on_network_response(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: NetworkResponseParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                let mut entry = self
                    .network_stats
                    .entry(page)
                    .or_insert_with(NetworkStats::new);
                entry.value_mut().register_response(payload.response.status);
                let snapshot = entry.value().clone();
                drop(entry);
                self.emit_network_summary(page, snapshot);
            }
            Ok(())
        }

        async fn on_network_finished(&self, event: TransportEvent) -> Result<(), AdapterError> {
            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                if let Some(mut entry) = self.network_stats.get_mut(&page) {
                    entry.value_mut().register_complete();
                    let snapshot = entry.value().clone();
                    drop(entry);
                    self.emit_network_summary(page, snapshot);
                }
            }
            Ok(())
        }

        async fn on_network_failed(&self, event: TransportEvent) -> Result<(), AdapterError> {
            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                if let Some(mut entry) = self.network_stats.get_mut(&page) {
                    entry.value_mut().register_complete();
                    let snapshot = entry.value().clone();
                    drop(entry);
                    self.emit_network_summary(page, snapshot);
                }
            }
            Ok(())
        }

        async fn on_exception_thrown(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: ExceptionThrownParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            let message = payload
                .exception_details
                .exception
                .and_then(|ex| ex.description)
                .or(payload.exception_details.text)
                .unwrap_or_else(|| "runtime exception".to_string());

            let page = event
                .session_id
                .as_ref()
                .and_then(|sid| self.sessions.get(sid))
                .map(|entry| *entry.value());

            let _ = self.bus.send(RawEvent::Error { page, message });
            Ok(())
        }

        fn page_from_session(&self, session: Option<&String>) -> Option<PageId> {
            session.and_then(|sid| self.sessions.get(sid).map(|entry| *entry.value()))
        }

        fn emit_page_event(
            &self,
            page: PageId,
            frame: Option<FrameId>,
            parent: Option<FrameId>,
            phase: &str,
            ts: u64,
        ) {
            self.page_activity.insert(page, Instant::now());
            let _ = self.bus.send(RawEvent::PageLifecycle {
                page,
                frame,
                parent,
                phase: phase.to_string(),
                ts,
            });
        }

        fn emit_network_summary(&self, page: PageId, stats: NetworkStats) {
            let (req, res2xx, res4xx, res5xx, inflight, quiet, since_last) = stats.snapshot();
            metrics::record_network_summary();
            let _ = self.bus.send(RawEvent::NetworkSummary {
                page,
                req,
                res2xx,
                res4xx,
                res5xx,
                inflight,
                quiet,
                window_ms: 1_000,
                since_last_activity_ms: since_last,
            });
        }

        async fn wait_for_dom_ready(
            &self,
            page: PageId,
            deadline: Instant,
        ) -> Result<(), AdapterError> {
            loop {
                if Instant::now() >= deadline {
                    return Err(AdapterError::new(AdapterErrorKind::NavTimeout)
                        .with_hint("wait_basic DomReady timed out"));
                }

                let response = self
                    .send_page_command(
                        page,
                        "Runtime.evaluate",
                        json!({
                            "expression": "document.readyState",
                            "returnByValue": true,
                        }),
                    )
                    .await?;

                let ready = response
                    .get("result")
                    .and_then(|v| v.get("value"))
                    .and_then(|v| v.as_str())
                    .map(|state| matches!(state, "interactive" | "complete"))
                    .unwrap_or(false);

                if ready {
                    return Ok(());
                }

                sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        async fn wait_for_network_quiet(
            &self,
            page: PageId,
            window_ms: u64,
            max_inflight: u32,
            deadline: Instant,
        ) -> Result<(), AdapterError> {
            loop {
                if Instant::now() >= deadline {
                    return Err(AdapterError::new(AdapterErrorKind::NavTimeout)
                        .with_hint("wait_basic NetworkQuiet timed out"));
                }

                let snapshot = {
                    let entry = self
                        .network_stats
                        .entry(page)
                        .or_insert_with(NetworkStats::new);
                    entry.value().snapshot()
                };

                let (_, _, _, _, inflight, _, since_last) = snapshot;
                if inflight <= max_inflight as u64 && since_last >= window_ms {
                    return Ok(());
                }

                sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        async fn wait_for_frame_stable(
            &self,
            page: PageId,
            min_stable_ms: u64,
            deadline: Instant,
        ) -> Result<(), AdapterError> {
            loop {
                if Instant::now() >= deadline {
                    return Err(AdapterError::new(AdapterErrorKind::NavTimeout)
                        .with_hint("wait_basic FrameStable timed out"));
                }

                let elapsed = self
                    .page_activity
                    .get(&page)
                    .map(|entry| Instant::now().saturating_duration_since(*entry.value()))
                    .unwrap_or_else(|| std::time::Duration::ZERO);

                if elapsed.as_millis() as u64 >= min_stable_ms {
                    return Ok(());
                }

                sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        #[allow(dead_code)]
        async fn send_command(&self, method: &str, params: Value) -> Result<Value, AdapterError> {
            metrics::record_command();
            self.transport
                .send_command(CommandTarget::Browser, method, params)
                .await
        }

        async fn send_page_command(
            &self,
            page: PageId,
            method: &str,
            params: Value,
        ) -> Result<Value, AdapterError> {
            if let Some(session) = self.registry.get_cdp_session(&page) {
                metrics::record_command();
                self.transport
                    .send_command(CommandTarget::Session(session), method, params)
                    .await
            } else {
                Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("missing cdp session for page {page:?}")))
            }
        }

        pub fn subscribe(&self, _filter: EventFilter) -> broadcast::Receiver<RawEvent> {
            self.bus.subscribe()
        }

        async fn ensure_initial_page(&self) -> Result<(), AdapterError> {
            if self
                .registry
                .iter()
                .iter()
                .any(|(_, ctx)| ctx.cdp_session.is_some())
            {
                return Ok(());
            }

            self.send_command("Target.createTarget", json!({ "url": "about:blank" }))
                .await?;
            Ok(())
        }

        async fn wait_for_page_ready(&self, page: PageId) -> Result<(), AdapterError> {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if self
                    .registry
                    .get(&page)
                    .map(|ctx| ctx.cdp_session.is_some())
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                sleep(Duration::from_millis(50)).await;
            }
            Err(AdapterError::new(AdapterErrorKind::Internal)
                .with_hint(format!("cdp session not ready for page {page:?}")))
        }
    }

    fn should_use_real_chrome() -> bool {
        matches!(
            env::var("SOULBROWSER_USE_REAL_CHROME")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
    }

    #[derive(Debug, Deserialize)]
    struct TargetCreatedParams {
        #[serde(rename = "targetInfo")]
        target_info: TargetInfoPayload,
    }

    #[derive(Debug, Deserialize)]
    struct TargetDestroyedParams {
        #[serde(rename = "targetId")]
        target_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct AttachedToTargetParams {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "targetInfo")]
        target_info: TargetInfoPayload,
    }

    #[derive(Debug, Deserialize)]
    struct DetachedFromTargetParams {
        #[serde(rename = "sessionId")]
        session_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct TargetInfoPayload {
        #[serde(rename = "targetId")]
        target_id: String,
        #[serde(rename = "type")]
        target_type: String,
        url: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct PageLifecycleParams {
        #[serde(rename = "name")]
        name: String,
        #[serde(rename = "frameId")]
        frame_id: Option<String>,
        timestamp: Option<f64>,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct FrameAttachedParams {
        #[serde(rename = "frameId")]
        frame_id: String,
        #[serde(rename = "parentFrameId")]
        parent_frame_id: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct FrameDetachedParams {
        #[serde(rename = "frameId")]
        frame_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct NetworkResponseParams {
        response: NetworkResponseInfo,
    }

    #[derive(Debug, Deserialize)]
    struct NetworkResponseInfo {
        status: i64,
    }

    #[derive(Debug, Deserialize)]
    struct ExceptionThrownParams {
        #[serde(rename = "exceptionDetails")]
        exception_details: ExceptionDetails,
    }

    #[derive(Debug, Deserialize)]
    struct ExceptionDetails {
        text: Option<String>,
        exception: Option<ExceptionObject>,
    }

    #[derive(Debug, Deserialize)]
    struct ExceptionObject {
        description: Option<String>,
    }

    fn timestamp_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_millis() as u64
    }

    fn parse_wait_gate(gate: &str) -> Result<WaitGate, AdapterError> {
        if gate.trim().is_empty() {
            return Ok(WaitGate::DomReady);
        }

        serde_json::from_str::<WaitGate>(gate).or_else(|_| {
            match gate.trim().to_ascii_lowercase().as_str() {
                "domready" | "dom_ready" => Ok(WaitGate::DomReady),
                "networkquiet" | "network_quiet" => Ok(WaitGate::NetworkQuiet {
                    window_ms: 1_000,
                    max_inflight: 0,
                }),
                "framestable" | "frame_stable" => Ok(WaitGate::FrameStable { min_stable_ms: 500 }),
                other => Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("unknown wait gate '{other}'"))),
            }
        })
    }

    #[async_trait]
    impl Cdp for CdpAdapter {
        async fn navigate(
            &self,
            page: PageId,
            url: &str,
            _deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self.send_page_command(page, "Page.navigate", json!({ "url": url }))
                .await?;
            self.registry.set_recent_url(&page, url.to_string());
            Ok(())
        }

        async fn query(&self, page: PageId, spec: QuerySpec) -> Result<Vec<Anchor>, AdapterError> {
            self.wait_for_page_ready(page).await?;
            let selector_literal = serde_json::to_string(&spec.selector).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;

            let scope_expression = match spec.scope {
                QueryScope::Document => "document".to_string(),
                QueryScope::Frame(frame_selector) => {
                    let frame_literal = serde_json::to_string(&frame_selector).map_err(|err| {
                        AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                    })?;
                    format!(
                        "(() => {{\n    try {{\n        const frameEl = document.querySelector({frame_literal});\n        if (!frameEl) {{ return null; }}\n        const doc = frameEl.contentDocument || (frameEl.contentWindow ? frameEl.contentWindow.document : null);\n        return doc || null;\n    }} catch (err) {{\n        return null;\n    }}\n}})()"
                    )
                }
            };

            let expression = format!(
                "(() => {{\n    const scope = {scope};\n    if (!scope) {{ return []; }}\n    let elements;\n    try {{\n        elements = scope.querySelectorAll({selector});\n    }} catch (err) {{\n        return [];\n    }}\n    return Array.from(elements, (el) => {{\n        if (!el) {{ return null; }}\n        const rect = el.getBoundingClientRect();\n        return {{\n            backendNodeId: null,\n            x: Number.isFinite(rect.left) ? rect.left + rect.width / 2 : 0,\n            y: Number.isFinite(rect.top) ? rect.top + rect.height / 2 : 0\n        }};\n    }}).filter(Boolean);\n}})()",
                scope = scope_expression,
                selector = selector_literal
            );

            let response = self
                .send_page_command(
                    page,
                    "Runtime.evaluate",
                    json!({
                        "expression": expression,
                        "returnByValue": true,
                    }),
                )
                .await?;

            let values = response
                .get("result")
                .and_then(|res| res.get("value"))
                .and_then(|val| val.as_array())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("query did not return an array value")
                })?;

            let mut anchors = Vec::with_capacity(values.len());
            for entry in values {
                let obj = entry.as_object().ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("query entry was not an object")
                })?;
                let x = obj.get("x").and_then(|v| v.as_f64()).ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("query entry missing 'x'")
                })?;
                let y = obj.get("y").and_then(|v| v.as_f64()).ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("query entry missing 'y'")
                })?;
                let backend = obj.get("backendNodeId").and_then(|v| v.as_u64());
                anchors.push(Anchor {
                    backend_node_id: backend,
                    x,
                    y,
                });
            }

            Ok(anchors)
        }

        async fn click(
            &self,
            page: PageId,
            selector: &str,
            _deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self
                .send_page_command(
                    page,
                    "Runtime.evaluate",
                    json!({
                        "expression": format!(
                            "(() => {{ const el=document.querySelector(\"{}\"); if(el) el.click(); }})();",
                            selector,
                        ),
                        "awaitPromise": true,
                    }),
                )
                .await?;
            Ok(())
        }

        async fn type_text(
            &self,
            page: PageId,
            selector: &str,
            text: &str,
            _deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self
                .send_page_command(
                    page,
                    "Runtime.evaluate",
                    json!({
                        "expression": format!(
                            "(() => {{ const el=document.querySelector(\"{}\"); if(el) {{ el.value=\"{}\"; el.dispatchEvent(new Event('input',{{bubbles:true}})); }} }})();",
                            selector,
                            text.replace('"', "\\\""),
                        ),
                        "awaitPromise": true,
                    }),
                )
                .await?;
            Ok(())
        }

        async fn select_option(
            &self,
            page: PageId,
            spec: SelectSpec,
            _deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            let selector_literal = serde_json::to_string(&spec.selector).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;
            let value_literal = serde_json::to_string(&spec.value).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;
            let match_label_flag = if spec.match_label { "true" } else { "false" };

            let expression = format!(
                "(() => {{\n    const scope = document;\n    if (!scope) {{ return {{ status: 'no-document' }}; }}\n    let el;\n    try {{\n        el = scope.querySelector({selector});\n    }} catch (err) {{\n        return {{ status: 'invalid-selector', reason: String(err) }};\n    }}\n    if (!el) {{ return {{ status: 'not-found' }}; }}\n    const targetValue = {value};\n    const options = Array.from(el.options || []);\n    let option = options.find(opt => opt.value === targetValue);\n    if (!option && {match_label}) {{\n        option = options.find(opt => opt.text === targetValue);\n    }}\n    if (!option && typeof el.value === 'string') {{\n        // fallback: set value directly
        el.value = targetValue;\n    }} else if (option) {{\n        el.value = option.value;\n    }} else {{\n        return {{ status: 'option-missing' }};\n    }}\n    el.dispatchEvent(new Event('input', {{ bubbles: true }}));\n    el.dispatchEvent(new Event('change', {{ bubbles: true }}));\n    return {{ status: 'selected', value: el.value }};\n}})()",
                selector = selector_literal,
                value = value_literal,
                match_label = match_label_flag
            );

            let response = self
                .send_page_command(
                    page,
                    "Runtime.evaluate",
                    json!({
                        "expression": expression,
                        "returnByValue": true,
                    }),
                )
                .await?;

            let result_obj = response
                .get("result")
                .and_then(|res| res.get("value"))
                .and_then(|val| val.as_object())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("selectOption did not return an object")
                })?;

            let status = result_obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            match status {
                "selected" => Ok(()),
                "not-found" => Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint("selectOption target element not found")),
                "option-missing" => Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint("selectOption option not found")),
                other => Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("selectOption failed: {other}"))),
            }
        }

        async fn wait_basic(
            &self,
            page: PageId,
            gate: String,
            timeout: std::time::Duration,
        ) -> Result<(), AdapterError> {
            let parsed_gate = parse_wait_gate(&gate)?;
            let deadline = Instant::now() + timeout;

            match parsed_gate {
                WaitGate::DomReady => self.wait_for_dom_ready(page, deadline).await,
                WaitGate::NetworkQuiet {
                    window_ms,
                    max_inflight,
                } => {
                    self.wait_for_network_quiet(page, window_ms, max_inflight, deadline)
                        .await
                }
                WaitGate::FrameStable { min_stable_ms } => {
                    self.wait_for_frame_stable(page, min_stable_ms, deadline)
                        .await
                }
            }
        }

        async fn screenshot(
            &self,
            page: PageId,
            _deadline: std::time::Duration,
        ) -> Result<Vec<u8>, AdapterError> {
            let response = self
                .send_page_command(page, "Page.captureScreenshot", json!({ "format": "png" }))
                .await?;
            let data = response
                .get("data")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("missing screenshot data")
                })?;
            let bytes = STANDARD.decode(data).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;
            Ok(bytes)
        }

        async fn set_network_tap(&self, page: PageId, enabled: bool) -> Result<(), AdapterError> {
            if enabled {
                self.send_page_command(
                    page,
                    "Network.enable",
                    json!({
                        "maxTotalBufferSize": 1_048_576u64,
                        "maxResourceBufferSize": 524_288u64,
                        "maxPostDataSize": 1_048_576u64,
                    }),
                )
                .await?;
                if !self.network_stats.contains_key(&page) {
                    self.network_stats.insert(page, NetworkStats::new());
                }
            } else {
                self.send_page_command(page, "Network.disable", Value::Object(Default::default()))
                    .await?;
                self.network_stats.remove(&page);
            }
            Ok(())
        }

        async fn dom_snapshot(
            &self,
            page: PageId,
            config: DomSnapshotConfig,
        ) -> Result<DomSnapshotResult, AdapterError> {
            self.wait_for_page_ready(page).await?;
            let _ = self
                .send_page_command(
                    page,
                    "DOMSnapshot.enable",
                    Value::Object(Default::default()),
                )
                .await;
            let mut params = serde_json::Map::new();
            let whitelist = config
                .computed_style_whitelist
                .into_iter()
                .map(Value::String)
                .collect::<Vec<Value>>();
            params.insert("computedStyleWhitelist".into(), Value::Array(whitelist));
            if config.include_event_listeners {
                params.insert("includeEventListeners".into(), Value::Bool(true));
            }
            if config.include_paint_order {
                params.insert("includePaintOrder".into(), Value::Bool(true));
            }
            if config.include_user_agent_shadow_tree {
                params.insert("includeUserAgentShadowTree".into(), Value::Bool(true));
            }

            let response = self
                .send_page_command(page, "DOMSnapshot.captureSnapshot", Value::Object(params))
                .await?;
            let raw = response.clone();

            let documents = raw
                .get("documents")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("DOMSnapshot.captureSnapshot missing 'documents' array")
                })?
                .iter()
                .cloned()
                .collect::<Vec<Value>>();

            let strings = raw
                .get("strings")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("DOMSnapshot.captureSnapshot missing 'strings' array")
                })?
                .iter()
                .map(|val| {
                    val.as_str().map(|s| s.to_string()).ok_or_else(|| {
                        AdapterError::new(AdapterErrorKind::Internal).with_hint(
                            "DOMSnapshot.captureSnapshot returned non-string entry in 'strings'",
                        )
                    })
                })
                .collect::<Result<Vec<String>, AdapterError>>()?;

            Ok(DomSnapshotResult {
                documents,
                strings,
                raw,
            })
        }

        async fn ax_snapshot(
            &self,
            page: PageId,
            config: AxSnapshotConfig,
        ) -> Result<AxSnapshotResult, AdapterError> {
            self.wait_for_page_ready(page).await?;
            let _ = self
                .send_page_command(
                    page,
                    "Accessibility.enable",
                    Value::Object(Default::default()),
                )
                .await;
            let mut params = serde_json::Map::new();
            if let Some(frame_id) = config.frame_id {
                params.insert("frameId".into(), Value::String(frame_id));
            }
            if let Some(max_depth) = config.max_depth {
                params.insert("maxDepth".into(), Value::Number(Number::from(max_depth)));
            }
            if config.fetch_relatives {
                params.insert("fetchRelatives".into(), Value::Bool(true));
            }

            let response = self
                .send_page_command(page, "Accessibility.getFullAXTree", Value::Object(params))
                .await?;
            let raw = response.clone();

            let nodes = raw
                .get("nodes")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("Accessibility.getFullAXTree missing 'nodes' array")
                })?
                .iter()
                .cloned()
                .collect::<Vec<Value>>();

            let tree_id = raw
                .get("treeId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            Ok(AxSnapshotResult {
                nodes,
                tree_id,
                raw,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::transport::TransportEvent;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use tokio::sync::mpsc;

        struct MockTransport {
            started: AtomicBool,
            rx: Mutex<mpsc::Receiver<TransportEvent>>,
            commands: Mutex<Vec<(String, Value)>>,
            response: Mutex<Option<Value>>,
        }

        impl MockTransport {
            fn new_pair() -> (Arc<Self>, mpsc::Sender<TransportEvent>) {
                let (tx, rx) = mpsc::channel(16);
                (
                    Arc::new(Self {
                        started: AtomicBool::new(false),
                        rx: Mutex::new(rx),
                        commands: Mutex::new(Vec::new()),
                        response: Mutex::new(None),
                    }),
                    tx,
                )
            }

            fn started(&self) -> bool {
                self.started.load(Ordering::SeqCst)
            }

            async fn commands(&self) -> Vec<(String, Value)> {
                self.commands.lock().await.clone()
            }

            async fn set_response(&self, value: Value) {
                *self.response.lock().await = Some(value);
            }
        }

        #[async_trait]
        impl CdpTransport for MockTransport {
            async fn start(&self) -> Result<(), AdapterError> {
                self.started.store(true, Ordering::SeqCst);
                Ok(())
            }

            async fn next_event(&self) -> Option<TransportEvent> {
                let mut guard = self.rx.lock().await;
                guard.recv().await
            }

            async fn send_command(
                &self,
                _target: CommandTarget,
                method: &str,
                params: Value,
            ) -> Result<Value, AdapterError> {
                self.commands
                    .lock()
                    .await
                    .push((method.to_string(), params));
                Ok(self.response.lock().await.take().unwrap_or(Value::Null))
            }
        }

        #[tokio::test]
        async fn event_loop_broadcasts_transport_events() {
            let (bus, mut rx) = crate::event_bus(8);
            let (transport, tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            crate::metrics::reset();
            Arc::clone(&adapter).start().await.expect("start adapter");
            assert!(transport.started());

            tx.send(TransportEvent {
                method: "Test.Event".into(),
                params: Value::Null,
                session_id: None,
            })
            .await
            .unwrap();

            let event = rx.recv().await.expect("receive raw event");
            match event {
                RawEvent::Error { message, .. } => {
                    assert!(message.contains("Test.Event"));
                }
                other => panic!("unexpected event: {:?}", other),
            }

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn commands_route_through_transport() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            crate::metrics::reset();
            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            adapter
                .navigate(
                    page,
                    "https://example.com",
                    std::time::Duration::from_secs(5),
                )
                .await
                .expect("navigate through transport");

            transport
                .set_response(json!({"data": STANDARD.encode("img")}))
                .await;
            adapter
                .screenshot(page, std::time::Duration::from_secs(5))
                .await
                .expect("screenshot through transport");

            let commands = transport.commands().await;
            assert!(commands.iter().any(|(method, _)| method == "Page.navigate"));
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Page.captureScreenshot"));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn network_events_emit_summaries_and_metrics() {
            let (bus, mut rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            crate::metrics::reset();
            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            let target_id = "target-test".to_string();
            let cdp_session = "session-test".to_string();

            adapter.registry.insert_page(
                page,
                session,
                Some(target_id.clone()),
                Some(cdp_session.clone()),
            );
            adapter.targets.insert(target_id, page);
            adapter.sessions.insert(cdp_session.clone(), page);
            adapter.network_stats.insert(page, NetworkStats::new());

            adapter
                .handle_event(TransportEvent {
                    method: "Network.requestWillBeSent".into(),
                    params: Value::Null,
                    session_id: Some(cdp_session.clone()),
                })
                .await;

            adapter
                .handle_event(TransportEvent {
                    method: "Network.responseReceived".into(),
                    params: json!({"response": {"status": 200}}),
                    session_id: Some(cdp_session.clone()),
                })
                .await;

            let mut summaries = 0;
            while let Ok(evt) = rx.try_recv() {
                if let RawEvent::NetworkSummary { .. } = evt {
                    summaries += 1;
                }
            }

            assert!(summaries >= 2);

            let snapshot = crate::metrics::snapshot();
            assert!(snapshot.network_summaries >= 2);
            assert!(snapshot.events >= 2);

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn wait_basic_dom_ready_issues_runtime_evaluate() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            transport
                .set_response(json!({
                    "result": {
                        "value": "complete"
                    }
                }))
                .await;

            adapter
                .wait_basic(
                    page,
                    "DomReady".into(),
                    std::time::Duration::from_millis(200),
                )
                .await
                .expect("wait_basic dom ready");

            let commands = transport.commands().await;
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Runtime.evaluate"));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn wait_basic_network_quiet_resolves_on_stats() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            adapter.network_stats.insert(
                page,
                NetworkStats {
                    requests: 10,
                    responses_2xx: 10,
                    responses_4xx: 0,
                    responses_5xx: 0,
                    inflight: 0,
                    last_activity: Instant::now() - Duration::from_millis(2_000),
                },
            );

            let gate = serde_json::to_string(&WaitGate::NetworkQuiet {
                window_ms: 500,
                max_inflight: 0,
            })
            .expect("serialize gate");

            adapter
                .wait_basic(page, gate, std::time::Duration::from_secs(1))
                .await
                .expect("wait_basic network quiet");

            let commands = transport.commands().await;
            assert!(commands.is_empty());

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn dom_and_ax_snapshot_commands_capture_payloads() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            transport
                .set_response(json!({
                    "documents": [ { "nodeName": "#document" } ],
                    "strings": ["", "html"]
                }))
                .await;

            let dom_snapshot = adapter
                .dom_snapshot(page, DomSnapshotConfig::default())
                .await
                .expect("dom snapshot succeeds");
            assert_eq!(dom_snapshot.documents.len(), 1);
            assert_eq!(
                dom_snapshot.strings,
                vec!["".to_string(), "html".to_string()]
            );

            transport
                .set_response(json!({
                    "nodes": [ { "role": { "type": "document" } } ],
                    "treeId": "ax-tree"
                }))
                .await;

            let ax_snapshot = adapter
                .ax_snapshot(page, AxSnapshotConfig::default())
                .await
                .expect("ax snapshot succeeds");
            assert_eq!(ax_snapshot.nodes.len(), 1);
            assert_eq!(ax_snapshot.tree_id.as_deref(), Some("ax-tree"));

            let commands = transport.commands().await;
            let dom_command = commands
                .iter()
                .find(|(method, _)| method == "DOMSnapshot.captureSnapshot")
                .expect("dom snapshot command recorded");
            assert!(dom_command
                .1
                .get("computedStyleWhitelist")
                .and_then(|v| v.as_array())
                .map(|arr| !arr.is_empty())
                .unwrap_or(false));

            assert!(commands
                .iter()
                .any(|(method, _)| method == "Accessibility.getFullAXTree"));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn query_returns_anchor_positions() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            transport
                .set_response(json!({
                    "result": {
                        "type": "object",
                        "value": [
                            { "x": 10.0, "y": 15.5, "backendNodeId": null }
                        ]
                    }
                }))
                .await;

            let anchors = adapter
                .query(
                    page,
                    QuerySpec {
                        selector: "button.primary".into(),
                        scope: QueryScope::Document,
                    },
                )
                .await
                .expect("query returns anchors");

            assert_eq!(anchors.len(), 1);
            assert_eq!(anchors[0].x, 10.0);
            assert_eq!(anchors[0].y, 15.5);

            let commands = transport.commands().await;
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Runtime.evaluate"));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn select_option_dispatches_events() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            transport
                .set_response(json!({
                    "result": {
                        "type": "object",
                        "value": { "status": "selected", "value": "choice" }
                    }
                }))
                .await;

            adapter
                .select_option(
                    page,
                    SelectSpec {
                        selector: "select#choices".into(),
                        value: "choice".into(),
                        match_label: true,
                    },
                    std::time::Duration::from_secs(5),
                )
                .await
                .expect("select succeeds");

            let commands = transport.commands().await;
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Runtime.evaluate"));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn set_network_tap_toggles_transport_commands() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            Arc::clone(&adapter).start().await.expect("start adapter");

            let page = PageId::new();
            let session = SessionId::new();
            adapter.register_page(page, session, None, Some("mock-session".into()));

            transport.set_response(Value::Null).await;
            adapter
                .set_network_tap(page, true)
                .await
                .expect("enable tap");

            transport.set_response(Value::Null).await;
            adapter
                .set_network_tap(page, false)
                .await
                .expect("disable tap");

            let commands = transport.commands().await;
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Network.enable"));
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Network.disable"));

            adapter.shutdown().await;
        }
    }
}

pub use adapter::{Cdp, CdpAdapter, EventBus};
pub use commands::*;
pub use config::CdpConfig;
pub use error::{AdapterError, AdapterErrorKind};
pub use events::{EventFilter, RawEvent};
pub use ids::{BrowserId, FrameId, PageId, SessionId};
pub use metrics::AdapterMetricsSnapshot;
pub mod commands;
pub mod metrics;
pub mod registry;
pub mod transport;
pub mod util;
pub use transport::{CdpTransport, CommandTarget, TransportEvent};

/// Helper to create an event bus suitable for hooking into the adapter scaffold.
pub fn event_bus(buffer: usize) -> (EventBus, broadcast::Receiver<RawEvent>) {
    let bus = broadcast::channel(buffer);
    (bus.0, bus.1)
}

/// Placeholder stream type used until the event bus grows richer subscriptions.
pub type EventStream = broadcast::Receiver<RawEvent>;
