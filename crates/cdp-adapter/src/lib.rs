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

    /// Configuration for launching and tuning the adapter.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct CdpConfig {
        pub executable: PathBuf,
        pub user_data_dir: PathBuf,
        pub headless: bool,
        pub default_deadline_ms: u64,
        pub retry_backoff_ms: u64,
    }

    impl Default for CdpConfig {
        fn default() -> Self {
            Self {
                executable: PathBuf::from("chromium"),
                user_data_dir: PathBuf::from("./.soulbrowser-profile"),
                headless: true,
                default_deadline_ms: 30_000,
                retry_backoff_ms: 250,
            }
        }
    }
}

pub mod adapter {
    use super::config::CdpConfig;
    use super::error::{AdapterError, AdapterErrorKind};
    use super::events::{EventFilter, RawEvent};
    use super::ids::{BrowserId, FrameId, PageId, SessionId};
    use super::metrics;
    use super::registry::Registry;
    use super::transport::{CdpTransport, CommandTarget, NoopTransport, TransportEvent};
    use async_trait::async_trait;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use dashmap::DashMap;
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::broadcast;
    use tokio::sync::Mutex;
    use tokio::task::JoinHandle;
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
            Self::with_transport(cfg, bus, Arc::new(NoopTransport::default()))
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

            self.emit_page_event(page_id, None, "opened", timestamp_now());
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
                self.registry.remove_page(&page);
                self.emit_page_event(page, None, "closed", timestamp_now());
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
                self.emit_page_event(page, None, "focus", timestamp_now());
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
                self.emit_page_event(page_id, frame_id, &phase, ts);
            }

            Ok(())
        }

        async fn on_frame_attached(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: FrameAttachedParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                let frame_id = FrameId::new();
                self.frames.insert(
                    payload.frame_id.clone(),
                    FrameEntry {
                        page,
                        frame: frame_id,
                    },
                );
                self.emit_page_event(page, Some(frame_id), "frame_attached", timestamp_now());
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

        fn emit_page_event(&self, page: PageId, frame: Option<FrameId>, phase: &str, ts: u64) {
            let _ = self.bus.send(RawEvent::PageLifecycle {
                page,
                frame,
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

        async fn wait_basic(
            &self,
            _page: PageId,
            _gate: String,
            _timeout: std::time::Duration,
        ) -> Result<(), AdapterError> {
            let _ = (_page, _gate, _timeout);
            Ok(())
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

            let metrics = crate::metrics::snapshot();
            assert_eq!(metrics.commands, 2);
            assert_eq!(metrics.network_summaries, 0);

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
