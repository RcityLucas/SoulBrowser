//! SoulBrowser L0 CDP adapter scaffold.
//!
//! This crate hosts the future Chromium DevTools Protocol integration. For now it exposes the
//! data structures and traits that the higher layers will wire against while the concrete
//! implementation is filled in milestone by milestone.

use serde::{Deserialize, Serialize};
use std::{env, path::PathBuf};
use tokio::sync::broadcast;
use which::which;

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
    use std::fmt;
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
        #[error("target element not found")]
        TargetNotFound,
        #[error("option not found")]
        OptionNotFound,
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

    impl fmt::Display for AdapterError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.kind)?;
            if let Some(hint) = &self.hint {
                write!(f, ": {}", hint)?;
            }
            Ok(())
        }
    }

    impl std::error::Error for AdapterError {}

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
            opener: Option<PageId>,
            phase: String,
            ts: u64,
        },
        PageNavigated {
            page: PageId,
            url: String,
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
        NetworkActivity {
            page: PageId,
            signal: NetworkSignal,
        },
        Error {
            page: Option<PageId>,
            message: String,
        },
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub enum NetworkSignal {
        RequestWillBeSent,
        ResponseReceived { status: i64 },
        LoadingFinished,
        LoadingFailed,
    }

    /// Subscription filter placeholder; will expand with real predicates.
    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct EventFilter;
}

pub mod config {
    use crate::detect_chrome_executable;
    use serde::{Deserialize, Serialize};
    use std::{
        env,
        path::{Path, PathBuf},
    };

    /// Configuration for launching and tuning the adapter.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct CdpConfig {
        pub executable: PathBuf,
        pub user_data_dir: PathBuf,
        pub headless: bool,
        pub default_deadline_ms: u64,
        pub retry_backoff_ms: u64,
        pub websocket_url: Option<String>,
        pub heartbeat_interval_ms: u64,
        pub remote_debugging_addr: Option<String>,
    }

    impl Default for CdpConfig {
        fn default() -> Self {
            Self {
                executable: default_chrome_path(),
                user_data_dir: default_profile_dir(),
                headless: resolve_headless_default(),
                default_deadline_ms: 30_000,
                retry_backoff_ms: 250,
                websocket_url: None,
                heartbeat_interval_ms: 15_000,
                remote_debugging_addr: resolve_debugger_bind_addr(),
            }
        }
    }

    fn resolve_headless_default() -> bool {
        // Check SOUL_HEADLESS env var: "0", "false", "no", "off" means headful
        match env::var("SOUL_HEADLESS") {
            Ok(value) => {
                let lower = value.to_ascii_lowercase();
                !matches!(lower.as_str(), "0" | "false" | "no" | "off")
            }
            Err(_) => true, // Default to headless if not specified
        }
    }

    fn resolve_debugger_bind_addr() -> Option<String> {
        match env::var("SOUL_MANUAL_TAKEOVER_BIND_ADDR") {
            Ok(value) => {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            Err(_) => None,
        }
    }

    fn default_chrome_path() -> PathBuf {
        detect_chrome_executable().unwrap_or_default()
    }

    fn default_profile_dir() -> PathBuf {
        if let Ok(path) = env::var("SOULBROWSER_CHROME_PROFILE") {
            return PathBuf::from(path);
        }

        let default = Path::new("./.soulbrowser-profile");
        default.into()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DebuggerEndpoint {
    pub ws_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspect_url: Option<String>,
}

fn detect_chrome_executable() -> Option<PathBuf> {
    if let Ok(raw) = env::var("SOULBROWSER_CHROME") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    for name in chrome_executable_names() {
        if let Ok(path) = which(name) {
            return Some(path);
        }
    }

    let skip_defaults = env::var("SOULBROWSER_SKIP_OS_PATHS")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    if !skip_defaults {
        for candidate in os_specific_chrome_paths() {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn chrome_executable_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["chrome.exe", "chromium.exe", "msedge.exe"]
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "freebsd"))]
    {
        &[
            "google-chrome-stable",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ]
    }

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        &["chrome"]
    }
}

fn os_specific_chrome_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut paths = Vec::new();
        for root in windows_search_roots() {
            paths.push(root.join("Google/Chrome/Application/chrome.exe"));
            paths.push(root.join("Chromium/Application/chrome.exe"));
            paths.push(root.join("Microsoft/Edge/Application/msedge.exe"));
        }
        paths
    }

    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ]
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        vec![
            PathBuf::from("/usr/bin/google-chrome-stable"),
            PathBuf::from("/usr/bin/google-chrome"),
            PathBuf::from("/usr/bin/chromium-browser"),
            PathBuf::from("/usr/bin/chromium"),
        ]
    }

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "windows")]
fn windows_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Ok(value) = env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                roots.push(PathBuf::from(trimmed));
            }
        }
    }
    roots
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterMode {
    Real,
    Stub,
}

impl AdapterMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdapterMode::Real => "real",
            AdapterMode::Stub => "stub",
        }
    }

    pub fn is_stub(&self) -> bool {
        matches!(self, AdapterMode::Stub)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChromeMode {
    Auto,
    ForceReal,
    ForceStub,
}

fn chrome_mode() -> ChromeMode {
    match env::var("SOULBROWSER_USE_REAL_CHROME")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => ChromeMode::ForceReal,
        "0" | "false" | "no" | "off" => ChromeMode::ForceStub,
        _ => ChromeMode::Auto,
    }
}

fn resolve_chrome_path(cfg: &CdpConfig) -> Option<PathBuf> {
    if !cfg.executable.as_os_str().is_empty() && cfg.executable.exists() {
        return Some(cfg.executable.clone());
    }
    detect_chrome_executable()
}

#[cfg(test)]
mod tests {
    use super::{chrome_executable_names, detect_chrome_executable};
    use std::{env, fs};
    use tempfile::tempdir;

    #[test]
    fn detects_from_env_var() {
        let dir = tempdir().unwrap();
        let exe_path = dir.path().join("my-chrome");
        fs::write(&exe_path, b"").unwrap();
        let original = env::var("SOULBROWSER_CHROME").ok();
        env::set_var("SOULBROWSER_CHROME", exe_path.to_string_lossy().to_string());
        let detected = detect_chrome_executable();
        if let Some(value) = original {
            env::set_var("SOULBROWSER_CHROME", value);
        } else {
            env::remove_var("SOULBROWSER_CHROME");
        }
        assert_eq!(detected, Some(exe_path));
    }

    #[test]
    fn detects_from_path_entries() {
        let dir = tempdir().unwrap();
        let name = chrome_executable_names()
            .get(0)
            .expect("chrome executable names must not be empty");
        let exe_path = dir.path().join(name);
        fs::write(&exe_path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o755);
            fs::set_permissions(&exe_path, perms).unwrap();
        }
        let original_path = env::var("PATH").ok();
        let original_env = env::var("SOULBROWSER_CHROME").ok();
        let skip_flag = env::var("SOULBROWSER_SKIP_OS_PATHS").ok();
        env::set_var("SOULBROWSER_CHROME", "");
        env::set_var("SOULBROWSER_SKIP_OS_PATHS", "1");
        env::set_var("PATH", dir.path());
        let detected = detect_chrome_executable();
        if let Some(value) = original_path {
            env::set_var("PATH", value);
        }
        if let Some(value) = original_env {
            env::set_var("SOULBROWSER_CHROME", value);
        } else {
            env::remove_var("SOULBROWSER_CHROME");
        }
        if let Some(value) = skip_flag {
            env::set_var("SOULBROWSER_SKIP_OS_PATHS", value);
        } else {
            env::remove_var("SOULBROWSER_SKIP_OS_PATHS");
        }
        assert_eq!(detected, Some(exe_path));
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
    use super::{chrome_mode, resolve_chrome_path, AdapterMode, ChromeMode, DebuggerEndpoint};
    use async_trait::async_trait;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use dashmap::DashMap;
    use network_tap_light::{
        config::TapConfig as NetworkTapConfig, MaintenanceHandle as NetworkTapHandle,
        NetworkSnapshot as TapSnapshot, NetworkTapLight, PageId as TapPageId,
        TapError as NetworkTapError, TapEvent as NetworkTapEvent,
    };
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Number, Value};
    use soulbrowser_core_types::ExecRoute;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::broadcast;
    use tokio::sync::Mutex;
    use tokio::task::JoinHandle;
    use tokio::time::sleep;
    use tokio::{select, spawn};
    use tokio_util::sync::CancellationToken;
    use tracing::{debug, info, warn};

    /// Parameters accepted by `Network.setCookies`.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct CookieParam {
        pub name: String,
        pub value: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub domain: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub expires: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub http_only: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub secure: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub same_site: Option<String>,
    }

    /// Shared event bus type alias used by the adapter scaffold.
    pub type EventBus = broadcast::Sender<RawEvent>;

    #[derive(Clone, Debug)]
    pub struct ResolvedExecutionContext {
        pub page: PageId,
        pub frame_selector: Option<String>,
        pub execution_context_id: Option<String>,
    }

    impl ResolvedExecutionContext {
        pub fn for_page(page: PageId) -> Self {
            Self {
                page,
                frame_selector: None,
                execution_context_id: None,
            }
        }

        pub fn with_frame(page: PageId, frame_selector: Option<String>) -> Self {
            Self {
                page,
                frame_selector,
                execution_context_id: None,
            }
        }

        pub fn query_scope(&self) -> QueryScope {
            match &self.frame_selector {
                Some(selector) if !selector.is_empty() => QueryScope::Frame(selector.clone()),
                _ => QueryScope::Document,
            }
        }
    }

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
        async fn click_in_context(
            &self,
            ctx: &ResolvedExecutionContext,
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
        async fn type_text_in_context(
            &self,
            ctx: &ResolvedExecutionContext,
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
        async fn evaluate_script(
            &self,
            page: PageId,
            expression: &str,
        ) -> Result<Value, AdapterError>;
        async fn evaluate_script_in_context(
            &self,
            ctx: &ResolvedExecutionContext,
            expression: &str,
        ) -> Result<Value, AdapterError>;
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
        async fn grant_permissions(
            &self,
            origin: &str,
            permissions: &[String],
        ) -> Result<(), AdapterError>;
        async fn reset_permissions(
            &self,
            origin: &str,
            permissions: &[String],
        ) -> Result<(), AdapterError>;
        async fn set_cookies(
            &self,
            page: PageId,
            cookies: &[CookieParam],
        ) -> Result<(), AdapterError>;
        async fn set_user_agent(
            &self,
            page: PageId,
            user_agent: &str,
            accept_language: Option<&str>,
            platform: Option<&str>,
            locale: Option<&str>,
        ) -> Result<(), AdapterError>;
        async fn set_timezone(&self, page: PageId, timezone: &str) -> Result<(), AdapterError>;
        async fn set_device_metrics(
            &self,
            page: PageId,
            width: u32,
            height: u32,
            device_scale_factor: f64,
            mobile: bool,
        ) -> Result<(), AdapterError>;
        async fn set_touch_emulation(
            &self,
            page: PageId,
            enabled: bool,
        ) -> Result<(), AdapterError>;

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
        mode: AdapterMode,
        shutdown: CancellationToken,
        tasks: Mutex<Vec<JoinHandle<()>>>,
        transport: Arc<dyn CdpTransport>,
        targets: DashMap<String, PageId>,
        sessions: DashMap<String, PageId>,
        frames: DashMap<String, FrameEntry>,
        route_pages: DashMap<String, PageId>,
        page_routes: DashMap<PageId, String>,
        pending_routes: DashMap<String, Instant>,
        page_activity: DashMap<PageId, Instant>,
        network_tap: Arc<NetworkTapLight>,
        tap_maintenance: Mutex<Option<NetworkTapHandle>>,
    }

    #[derive(Clone, Copy, Debug)]
    struct FrameEntry {
        page: PageId,
        frame: FrameId,
    }

    impl CdpAdapter {
        pub fn new(mut cfg: CdpConfig, bus: EventBus) -> Self {
            let mode = chrome_mode();
            let detected = resolve_chrome_path(&cfg);
            let wants_stub = matches!(mode, ChromeMode::ForceStub);
            let mut use_real = cfg.websocket_url.is_some() || matches!(mode, ChromeMode::ForceReal);
            let mut stub_reason: Option<&'static str> = wants_stub.then_some("forced_stub_mode");
            if !use_real && !wants_stub {
                use_real = detected.is_some();
            }

            if use_real && cfg.websocket_url.is_none() {
                if let Some(path) = detected.clone() {
                    cfg.executable = path;
                } else {
                    if matches!(mode, ChromeMode::ForceReal) {
                        panic!(
                            "Chrome/Chromium executable not found while SOULBROWSER_USE_REAL_CHROME=1"
                        );
                    }
                    warn!(
                        target: "cdp-adapter",
                        "Chrome executable not found; falling back to stub transport"
                    );
                    use_real = false;
                    stub_reason = Some("chrome_not_found");
                }
            }

            let transport: Arc<dyn CdpTransport> = if use_real {
                info!(target: "cdp-adapter", "using real Chromium transport");
                Arc::new(ChromiumTransport::new(cfg.clone()))
            } else {
                info!(
                    target: "cdp-adapter",
                    "using Noop transport (set SOULBROWSER_USE_REAL_CHROME=1 to force real browser)"
                );
                let reason = stub_reason.unwrap_or("unknown");
                warn!(
                    target: "cdp-adapter",
                    event = "cdp_adapter.stub_mode",
                    mode = %AdapterMode::Stub.as_str(),
                    reason,
                    remediation = "Install Chrome/Chromium and set SOULBROWSER_USE_REAL_CHROME=1 with SOULBROWSER_CHROME=/path/to/chrome or pass --chrome-path/--ws-url",
                    "CDP adapter initialized without a real browser; DOM automation will be disabled"
                );
                Arc::new(NoopTransport::default())
            };
            Self::with_transport(cfg, bus, transport)
        }

        pub fn with_transport(
            cfg: CdpConfig,
            bus: EventBus,
            transport: Arc<dyn CdpTransport>,
        ) -> Self {
            let (network_tap, _) = NetworkTapLight::with_config(NetworkTapConfig::default(), 512);
            let network_tap = Arc::new(network_tap);
            let mode = transport.adapter_mode();
            Self {
                browser_id: BrowserId::new(),
                cfg,
                bus,
                registry: Arc::new(Registry::new()),
                mode,
                shutdown: CancellationToken::new(),
                tasks: Mutex::new(Vec::new()),
                transport,
                targets: DashMap::new(),
                sessions: DashMap::new(),
                frames: DashMap::new(),
                route_pages: DashMap::new(),
                page_routes: DashMap::new(),
                pending_routes: DashMap::new(),
                page_activity: DashMap::new(),
                network_tap,
                tap_maintenance: Mutex::new(None),
            }
        }

        pub fn mode(&self) -> AdapterMode {
            self.mode
        }

        pub fn registry(&self) -> Arc<Registry> {
            Arc::clone(&self.registry)
        }

        pub async fn debugger_endpoint(&self, page: PageId) -> Option<DebuggerEndpoint> {
            let context = self.registry.get(&page)?;
            let target_id = context.target_id.as_ref()?;
            let base = self.transport.debugger_base().await?;
            Some(base.endpoint_for_target(target_id))
        }

        pub fn cancel_token(&self) -> CancellationToken {
            self.shutdown.clone()
        }

        pub async fn resolve_execution_context(
            &self,
            route: &ExecRoute,
        ) -> Result<ResolvedExecutionContext, AdapterError> {
            let page = self.resolve_page_for_route(route).await?;
            let frame_selector = Self::frame_selector_from_route(route);
            Ok(ResolvedExecutionContext::with_frame(page, frame_selector))
        }

        /// Extract a CSS frame selector from an execution route when available.
        ///
        /// Today ExecRoute::frame is primarily a logical identifier used by the
        /// scheduler. Only values prefixed with `css=` are treated as real
        /// selectors; everything else falls back to the document scope to avoid
        /// generating invalid DOM queries (which previously caused timeouts for
        /// every DOM action).
        fn frame_selector_from_route(route: &ExecRoute) -> Option<String> {
            let raw = route.frame.0.trim();
            if raw.is_empty() {
                return None;
            }

            let selector = raw.strip_prefix("css=")?.trim();
            if selector.is_empty() {
                None
            } else {
                Some(selector.to_string())
            }
        }

        async fn resolve_page_for_route(&self, route: &ExecRoute) -> Result<PageId, AdapterError> {
            let route_key = route.page.0.clone();
            if let Some(existing) = self.route_pages.get(&route_key) {
                return Ok(*existing.value());
            }

            let deadline = Instant::now() + Duration::from_secs(5);

            loop {
                if Instant::now() >= deadline {
                    return Err(AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("No available CDP pages for execution route"));
                }

                self.cleanup_route_mappings();

                if let Some(existing) = self.route_pages.get(&route_key) {
                    return Ok(*existing.value());
                }

                let claimed: HashSet<PageId> =
                    self.page_routes.iter().map(|entry| *entry.key()).collect();

                let candidate = self
                    .registry
                    .iter()
                    .into_iter()
                    .filter(|(_, ctx)| ctx.cdp_session.is_some())
                    .map(|(page, _)| page)
                    .find(|page| !claimed.contains(page));

                if let Some(page) = candidate {
                    self.route_pages.insert(route_key.clone(), page);
                    self.page_routes.insert(page, route_key.clone());
                    return Ok(page);
                }

                if self.pending_routes.get(&route_key).is_some() {
                    sleep(Duration::from_millis(50)).await;
                    continue;
                }

                self.pending_routes
                    .insert(route_key.clone(), Instant::now());
                let create_result = self.create_page("about:blank").await;
                self.pending_routes.remove(&route_key);

                match create_result {
                    Ok(_) => {
                        sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    Err(err) => {
                        if self.registry.iter().is_empty() {
                            let synthetic = Self::synthetic_page_id(route);
                            self.route_pages.insert(route_key.clone(), synthetic);
                            return Ok(synthetic);
                        }
                        return Err(err);
                    }
                }
            }
        }

        fn cleanup_route_mappings(&self) {
            let active_pages: HashSet<PageId> = self
                .registry
                .iter()
                .into_iter()
                .map(|(page, _)| page)
                .collect();

            if active_pages.is_empty() {
                return;
            }

            self.route_pages
                .retain(|_, page| active_pages.contains(page));
            self.page_routes
                .retain(|page, _| active_pages.contains(page));
        }

        fn synthetic_page_id(route: &ExecRoute) -> PageId {
            match uuid::Uuid::parse_str(&route.page.0) {
                Ok(id) => PageId(id),
                Err(_) => PageId(uuid::Uuid::new_v4()),
            }
        }

        fn scope_expression(scope: &QueryScope) -> Result<String, AdapterError> {
            match scope {
                QueryScope::Document => Ok("document".to_string()),
                QueryScope::Frame(frame_selector) => {
                    let frame_literal = serde_json::to_string(frame_selector).map_err(|err| {
                        AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                    })?;
                    Ok(format!(
                        "(() => {{\n    try {{\n        const frameEl = document.querySelector({frame});\n        if (!frameEl) {{ return null; }}\n        const doc = frameEl.contentDocument || (frameEl.contentWindow ? frameEl.contentWindow.document : null);\n        return doc || null;\n    }} catch (err) {{\n        return null;\n    }}\n}})()",
                        frame = frame_literal
                    ))
                }
            }
        }

        pub async fn start(self: Arc<Self>) -> Result<(), AdapterError> {
            // Check if already started (idempotent)
            {
                let guard = self.tasks.lock().await;
                if !guard.is_empty() {
                    // Already started, nothing to do
                    return Ok(());
                }
            }

            {
                let mut maintenance = self.tap_maintenance.lock().await;
                if maintenance.is_none() {
                    let handle = self.network_tap.spawn_maintenance();
                    *maintenance = Some(handle);
                }
            }
            self.transport.start().await?;
            let loop_task = spawn(Self::event_loop(Arc::clone(&self)));
            let forward_task = self.spawn_tap_forwarder();
            let mut guard = self.tasks.lock().await;
            guard.push(loop_task);
            guard.push(forward_task);
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
            if let Some(handle) = self.tap_maintenance.lock().await.take() {
                let _ = handle.shutdown().await;
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
            self.schedule_tap_enable(page);
        }

        pub async fn create_page(&self, url: &str) -> Result<PageId, AdapterError> {
            let response = self
                .send_command("Target.createTarget", json!({ "url": url }))
                .await?;
            let target_id = response
                .get("targetId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("createTarget missing targetId")
                })?
                .to_string();

            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(entry) = self.targets.get(&target_id) {
                    let page = *entry.value();
                    if self
                        .registry
                        .get(&page)
                        .map(|ctx| ctx.cdp_session.is_some())
                        .unwrap_or(false)
                    {
                        return Ok(page);
                    }
                }

                if Instant::now() >= deadline {
                    return Err(AdapterError::new(AdapterErrorKind::Internal)
                        .with_hint("Timed out waiting for target attach"));
                }

                sleep(Duration::from_millis(50)).await;
            }
        }

        fn tap_page_id(page: PageId) -> TapPageId {
            TapPageId(page.0)
        }

        fn schedule_tap_enable(&self, page: PageId) {
            let tap = Arc::clone(&self.network_tap);
            let tap_page = Self::tap_page_id(page);
            spawn(async move {
                if let Err(err) = tap.enable(tap_page).await {
                    warn!(target: "cdp-adapter", ?err, "network tap enable failed");
                }
            });
        }

        fn schedule_tap_disable(&self, page: PageId) {
            let tap = Arc::clone(&self.network_tap);
            let tap_page = Self::tap_page_id(page);
            spawn(async move {
                if let Err(err) = tap.disable(tap_page).await {
                    if !matches!(err, NetworkTapError::PageNotEnabled) {
                        warn!(target: "cdp-adapter", ?err, "network tap disable failed");
                    }
                }
            });
        }

        async fn tap_ingest(&self, page: PageId, event: NetworkTapEvent) {
            let tap_page = Self::tap_page_id(page);
            if let Err(err) = self.network_tap.ingest(tap_page, event).await {
                if matches!(err, NetworkTapError::PageNotEnabled) {
                    self.schedule_tap_enable(page);
                } else {
                    warn!(target: "cdp-adapter", ?err, "network tap ingest failed");
                }
            }
        }

        fn is_snapshot_quiet(snapshot: &TapSnapshot, window_ms: u64, max_inflight: u32) -> bool {
            snapshot.inflight <= max_inflight as u64
                && snapshot.since_last_activity_ms >= window_ms
                && snapshot.quiet
        }

        fn spawn_tap_forwarder(self: &Arc<Self>) -> JoinHandle<()> {
            let adapter = Arc::clone(self);
            spawn(async move {
                let mut rx = adapter.network_tap.bus.subscribe();
                loop {
                    tokio::select! {
                        _ = adapter.shutdown.cancelled() => {
                            break;
                        }
                        summary = rx.recv() => {
                            match summary {
                                Ok(summary) => adapter.emit_tap_summary(&summary),
                                Err(broadcast::error::RecvError::Lagged(_)) => {
                                    continue;
                                }
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    }
                }
            })
        }

        async fn event_loop(self: Arc<Self>) {
            debug!(target: "cdp-adapter", "event loop entered");
            const MIN_BACKOFF: Duration = Duration::from_millis(100);
            const MAX_BACKOFF: Duration = Duration::from_secs(5);
            let mut backoff = MIN_BACKOFF;

            loop {
                select! {
                    _ = self.shutdown.cancelled() => {
                        break;
                    }
                    event = self.transport.next_event() => {
                        match event {
                            Some(ev) => {
                                backoff = MIN_BACKOFF;
                                self.handle_event(ev).await;
                            }
                            None => {
                                if self.shutdown.is_cancelled() {
                                    break;
                                }
                                self.handle_transport_disconnect();
                                warn!(target = "cdp-adapter", "transport stream ended; attempting restart");
                                if let Err(err) = self.transport.start().await {
                                    warn!(target = "cdp-adapter", ?err, "transport restart failed");
                                }
                                if self.shutdown.is_cancelled() {
                                    break;
                                }
                                sleep(backoff).await;
                                if backoff < MAX_BACKOFF {
                                    backoff += MIN_BACKOFF;
                                    if backoff > MAX_BACKOFF {
                                        backoff = MAX_BACKOFF;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            debug!(target: "cdp-adapter", "event loop exiting");
        }

        fn handle_transport_disconnect(&self) {
            let existing_pages: Vec<PageId> = self
                .registry
                .iter()
                .into_iter()
                .map(|(page, _)| page)
                .collect();
            let had_pages = !existing_pages.is_empty();

            for page in existing_pages {
                self.emit_page_event(page, None, None, None, "closed", timestamp_now());
                self.schedule_tap_disable(page);
                self.registry.remove_page(&page);
            }

            self.targets.clear();
            self.sessions.clear();
            self.frames.clear();
            self.page_activity.clear();
            self.route_pages.clear();
            self.page_routes.clear();
            self.pending_routes.clear();

            let message = if had_pages {
                "cdp transport restarted; active pages were reset"
            } else {
                "cdp transport restarted"
            };

            let _ = self.bus.send(RawEvent::Error {
                page: None,
                message: message.to_string(),
            });
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
                "Target.targetInfoChanged" => {
                    self.on_target_info_changed(event).await?;
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
                    return Ok(());
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
            self.registry
                .insert_page(page_id, session, Some(target_id), None);
            self.schedule_tap_enable(page_id);

            if let Some(url) = payload.target_info.url.filter(|u| !u.is_empty()) {
                self.registry.set_recent_url(&page_id, url);
            }

            let opener = payload
                .target_info
                .opener_id
                .and_then(|opener_id| self.targets.get(&opener_id).map(|entry| *entry.value()));
            self.emit_page_event(page_id, None, None, opener, "opened", timestamp_now());
            Ok(())
        }

        fn on_target_destroyed(&self, params: Value) -> Result<(), AdapterError> {
            let payload: TargetDestroyedParams = serde_json::from_value(params).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;

            if let Some((_, page)) = self.targets.remove(&payload.target_id) {
                self.sessions.retain(|_, v| *v != page);
                self.frames.retain(|_, entry| entry.page != page);
                self.page_activity.remove(&page);
                self.registry.remove_page(&page);
                self.schedule_tap_disable(page);
                self.emit_page_event(page, None, None, None, "closed", timestamp_now());
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
                self.emit_page_event(page, None, None, None, "focus", timestamp_now());
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
                self.emit_page_event(page_id, frame_id, None, None, &phase, ts);
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
                    None,
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
                    None,
                    "frame_detached",
                    timestamp_now(),
                );
            }
            Ok(())
        }

        async fn on_target_info_changed(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: TargetInfoChangedParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if payload.target_info.target_type != "page" {
                return Ok(());
            }

            if let Some(page_entry) = self.targets.get(&payload.target_info.target_id) {
                let page = *page_entry.value();
                if let Some(url) = payload.target_info.url.as_ref().filter(|u| !u.is_empty()) {
                    self.registry.set_recent_url(&page, url.clone());
                    self.emit_navigation_event(page, url.clone(), timestamp_now());
                }
            }

            Ok(())
        }

        async fn on_network_request(&self, event: TransportEvent) -> Result<(), AdapterError> {
            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                self.tap_ingest(page, NetworkTapEvent::RequestWillBeSent)
                    .await;
            }
            Ok(())
        }

        async fn on_network_response(&self, event: TransportEvent) -> Result<(), AdapterError> {
            let payload: NetworkResponseParams =
                serde_json::from_value(event.params).map_err(|err| {
                    AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
                })?;

            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                self.tap_ingest(
                    page,
                    NetworkTapEvent::ResponseReceived {
                        status: payload.response.status,
                    },
                )
                .await;
            }
            Ok(())
        }

        async fn on_network_finished(&self, event: TransportEvent) -> Result<(), AdapterError> {
            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                self.tap_ingest(page, NetworkTapEvent::LoadingFinished)
                    .await;
            }
            Ok(())
        }

        async fn on_network_failed(&self, event: TransportEvent) -> Result<(), AdapterError> {
            if let Some(page) = self.page_from_session(event.session_id.as_ref()) {
                self.tap_ingest(page, NetworkTapEvent::LoadingFailed).await;
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
            opener: Option<PageId>,
            phase: &str,
            ts: u64,
        ) {
            self.page_activity.insert(page, Instant::now());
            let _ = self.bus.send(RawEvent::PageLifecycle {
                page,
                frame,
                parent,
                opener,
                phase: phase.to_string(),
                ts,
            });
        }

        fn emit_navigation_event(&self, page: PageId, url: String, ts: u64) {
            self.page_activity.insert(page, Instant::now());
            let _ = self.bus.send(RawEvent::PageNavigated { page, url, ts });
        }

        fn emit_tap_summary(&self, summary: &network_tap_light::NetworkSummary) {
            metrics::record_network_summary();
            let page = PageId(summary.page.0);
            let _ = self.bus.send(RawEvent::NetworkSummary {
                page,
                req: summary.req,
                res2xx: summary.res2xx,
                res4xx: summary.res4xx,
                res5xx: summary.res5xx,
                inflight: summary.inflight,
                quiet: summary.quiet,
                window_ms: summary.window_ms,
                since_last_activity_ms: summary.since_last_activity_ms,
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

                let tap_page = Self::tap_page_id(page);
                if let Some(snapshot) = self.network_tap.current_snapshot(tap_page).await {
                    if Self::is_snapshot_quiet(&snapshot, window_ms, max_inflight) {
                        return Ok(());
                    }
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
            let start = Instant::now();
            metrics::record_command(method);
            match self
                .transport
                .send_command(CommandTarget::Browser, method, params)
                .await
            {
                Ok(value) => {
                    metrics::record_command_success(method, start.elapsed());
                    Ok(value)
                }
                Err(err) => {
                    metrics::record_command_failure(method);
                    Err(err)
                }
            }
        }

        async fn send_page_command(
            &self,
            page: PageId,
            method: &str,
            params: Value,
        ) -> Result<Value, AdapterError> {
            if let Some(session) = self.registry.get_cdp_session(&page) {
                let start = Instant::now();
                metrics::record_command(method);
                match self
                    .transport
                    .send_command(CommandTarget::Session(session), method, params)
                    .await
                {
                    Ok(value) => {
                        metrics::record_command_success(method, start.elapsed());
                        Ok(value)
                    }
                    Err(err) => {
                        metrics::record_command_failure(method);
                        Err(err)
                    }
                }
            } else {
                Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("missing cdp session for page {page:?}")))
            }
        }

        pub fn subscribe(&self, _filter: EventFilter) -> broadcast::Receiver<RawEvent> {
            self.bus.subscribe()
        }

        pub async fn dispatch_mouse_event(
            &self,
            page: PageId,
            payload: Value,
        ) -> Result<(), AdapterError> {
            self.send_page_command(page, "Input.dispatchMouseEvent", payload)
                .await
                .map(|_| ())
        }

        pub async fn insert_text_event(
            &self,
            page: PageId,
            text: &str,
        ) -> Result<(), AdapterError> {
            self.send_page_command(page, "Input.insertText", json!({ "text": text }))
                .await
                .map(|_| ())
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
    struct TargetInfoChangedParams {
        #[serde(rename = "targetInfo")]
        target_info: TargetInfoPayload,
    }

    #[derive(Debug, Deserialize)]
    struct TargetInfoPayload {
        #[serde(rename = "targetId")]
        target_id: String,
        #[serde(rename = "type")]
        target_type: String,
        url: Option<String>,
        #[serde(rename = "openerId")]
        opener_id: Option<String>,
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
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self.send_page_command(page, "Page.navigate", json!({ "url": url }))
                .await?;
            self.registry.set_recent_url(&page, url.to_string());
            let start = Instant::now();
            let deadline_at = start
                .checked_add(deadline)
                .unwrap_or_else(|| start + Duration::from_secs(30));
            self.wait_for_dom_ready(page, deadline_at).await?;
            Ok(())
        }

        async fn query(&self, page: PageId, spec: QuerySpec) -> Result<Vec<Anchor>, AdapterError> {
            self.wait_for_page_ready(page).await?;
            let selector_literal = serde_json::to_string(&spec.selector).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;

            let scope_expression = Self::scope_expression(&spec.scope)?;

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
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            let ctx = ResolvedExecutionContext::for_page(page);
            self.click_in_context(&ctx, selector, deadline).await
        }

        async fn click_in_context(
            &self,
            ctx: &ResolvedExecutionContext,
            selector: &str,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self.wait_for_page_ready(ctx.page).await?;
            let poll_interval = Duration::from_millis(100);
            let deadline_instant = Instant::now() + deadline;
            let anchor = loop {
                let anchors = self
                    .query(
                        ctx.page,
                        QuerySpec {
                            selector: selector.to_string(),
                            scope: ctx.query_scope(),
                        },
                    )
                    .await?;

                if let Some(anchor) = anchors.first() {
                    break anchor.clone();
                }

                if Instant::now() >= deadline_instant {
                    return Err(AdapterError::new(AdapterErrorKind::TargetNotFound)
                        .with_hint(format!("click target not found for selector '{selector}'")));
                }

                sleep(poll_interval).await;
            };

            let press_payload = json!({
                "type": "mousePressed",
                "x": anchor.x,
                "y": anchor.y,
                "button": "left",
                "buttons": 1,
                "clickCount": 1,
                "pointerType": "mouse",
            });
            self.send_page_command(ctx.page, "Input.dispatchMouseEvent", press_payload)
                .await?;

            let release_payload = json!({
                "type": "mouseReleased",
                "x": anchor.x,
                "y": anchor.y,
                "button": "left",
                "buttons": 1,
                "clickCount": 1,
                "pointerType": "mouse",
            });
            self.send_page_command(ctx.page, "Input.dispatchMouseEvent", release_payload)
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
            let ctx = ResolvedExecutionContext::for_page(page);
            self.type_text_in_context(&ctx, selector, text, _deadline)
                .await
        }

        async fn type_text_in_context(
            &self,
            ctx: &ResolvedExecutionContext,
            selector: &str,
            text: &str,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self.wait_for_page_ready(ctx.page).await?;
            let selector_literal = serde_json::to_string(&selector).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;

            let scope_expression = Self::scope_expression(&ctx.query_scope())?;
            let focus_expression = format!(
                "(() => {{\n    const scope = {scope};\n    if (!scope) {{ return {{ status: 'not-found' }}; }}\n    const el = scope.querySelector({selector});\n    if (!el) {{ return {{ status: 'not-found' }}; }}\n    if (typeof el.focus === 'function') {{ el.focus(); }}\n    return {{ status: 'focused' }};\n}})()",
                selector = selector_literal,
                scope = scope_expression,
            );

            let focus_retry_interval = Duration::from_millis(100);
            let focus_deadline = Instant::now() + deadline;

            loop {
                let focus_response = self
                    .send_page_command(
                        ctx.page,
                        "Runtime.evaluate",
                        json!({
                            "expression": focus_expression,
                            "returnByValue": true,
                        }),
                    )
                    .await?;

                let status = focus_response
                    .get("result")
                    .and_then(|res| res.get("value"))
                    .and_then(|val| val.get("status"))
                    .and_then(|val| val.as_str())
                    .unwrap_or("unknown");

                match status {
                    "focused" => break,
                    "not-found" => {
                        if Instant::now() >= focus_deadline {
                            return Err(AdapterError::new(AdapterErrorKind::TargetNotFound)
                                .with_hint(format!(
                                    "selector '{}' not found before deadline",
                                    selector
                                )));
                        }
                        sleep(focus_retry_interval).await;
                    }
                    other => {
                        return Err(AdapterError::new(AdapterErrorKind::Internal).with_hint(
                            format!(
                                "failed to focus element for selector '{}' (status: {})",
                                selector, other
                            ),
                        ));
                    }
                }
            }

            self.send_page_command(ctx.page, "Input.insertText", json!({ "text": text }))
                .await?;
            Ok(())
        }

        async fn select_option(
            &self,
            page: PageId,
            spec: SelectSpec,
            deadline: std::time::Duration,
        ) -> Result<(), AdapterError> {
            self.wait_for_page_ready(page).await?;

            let SelectSpec {
                selector,
                value,
                match_label,
            } = spec;

            let selector_literal = serde_json::to_string(&selector).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string())
            })?;
            let selector_expression = format!(
                "document.querySelector({selector})",
                selector = selector_literal
            );

            let poll_interval = Duration::from_millis(100);
            let deadline_instant = Instant::now() + deadline;

            let object_id = loop {
                let evaluate_response = self
                    .send_page_command(
                        page,
                        "Runtime.evaluate",
                        json!({
                        "expression": selector_expression.clone(),
                            "objectGroup": "soulbrowser-select",
                            "returnByValue": false,
                        }),
                    )
                    .await?;

                if let Some(object_id) = evaluate_response
                    .get("result")
                    .and_then(|res| res.get("objectId"))
                    .and_then(|val| val.as_str())
                {
                    break object_id.to_string();
                }

                if Instant::now() >= deadline_instant {
                    return Err(AdapterError::new(AdapterErrorKind::TargetNotFound)
                        .with_hint("selectOption target element not found"));
                }

                sleep(poll_interval).await;
            };

            const SELECT_FN: &str = r#"
function(targetValue, matchLabel) {
    if (!this) { return { status: 'not-found' }; }
    const options = Array.from(this.options || []);
    let option = options.find(opt => opt.value === targetValue);
    if (!option && matchLabel) {
        option = options.find(opt => opt.text === targetValue);
    }
    if (!option && typeof this.value === 'string') {
        this.value = targetValue;
    } else if (option) {
        this.value = option.value;
    } else {
        return { status: 'option-missing' };
    }
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return { status: 'selected', value: this.value };
}
"#;

            let call_response = self
                .send_page_command(
                    page,
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": object_id.clone(),
                        "functionDeclaration": SELECT_FN.trim(),
                        "arguments": [
                            { "value": value },
                            { "value": match_label },
                        ],
                        "awaitPromise": true,
                        "returnByValue": true,
                    }),
                )
                .await?;

            let result_obj = call_response
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

            let _ = self
                .send_page_command(
                    page,
                    "Runtime.releaseObject",
                    json!({ "objectId": object_id }),
                )
                .await;

            match status {
                "selected" => Ok(()),
                "not-found" => Err(AdapterError::new(AdapterErrorKind::TargetNotFound)
                    .with_hint("selectOption target element not found")),
                "option-missing" => Err(AdapterError::new(AdapterErrorKind::OptionNotFound)
                    .with_hint("selectOption option not found")),
                other => Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("selectOption failed: {other}"))),
            }
        }

        async fn evaluate_script(
            &self,
            page: PageId,
            expression: &str,
        ) -> Result<Value, AdapterError> {
            let ctx = ResolvedExecutionContext::for_page(page);
            self.evaluate_script_in_context(&ctx, expression).await
        }

        async fn evaluate_script_in_context(
            &self,
            ctx: &ResolvedExecutionContext,
            expression: &str,
        ) -> Result<Value, AdapterError> {
            self.wait_for_page_ready(ctx.page).await?;
            let response = self
                .send_page_command(
                    ctx.page,
                    "Runtime.evaluate",
                    json!({
                        "expression": expression,
                        "awaitPromise": true,
                        "returnByValue": true,
                        "userGesture": true,
                    }),
                )
                .await?;

            if let Some(details) = response.get("exceptionDetails") {
                return Err(AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint("evaluate_script raised exception")
                    .with_data(details.clone()));
            }

            let value = response
                .get("result")
                .and_then(|res| res.get("value"))
                .cloned()
                .unwrap_or(Value::Null);

            Ok(value)
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
                self.schedule_tap_enable(page);
            } else {
                self.send_page_command(page, "Network.disable", Value::Object(Default::default()))
                    .await?;
                self.schedule_tap_disable(page);
            }
            Ok(())
        }

        async fn grant_permissions(
            &self,
            origin: &str,
            permissions: &[String],
        ) -> Result<(), AdapterError> {
            if permissions.is_empty() {
                return Ok(());
            }
            self.send_command(
                "Browser.grantPermissions",
                json!({
                    "origin": origin,
                    "permissions": permissions,
                }),
            )
            .await?;
            Ok(())
        }

        async fn reset_permissions(
            &self,
            origin: &str,
            permissions: &[String],
        ) -> Result<(), AdapterError> {
            let mut params = serde_json::Map::new();
            params.insert("origin".into(), Value::String(origin.to_string()));
            if !permissions.is_empty() {
                let list = permissions
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect::<Vec<Value>>();
                params.insert("permissions".into(), Value::Array(list));
            }
            self.send_command("Browser.resetPermissions", Value::Object(params))
                .await?;
            Ok(())
        }

        async fn set_cookies(
            &self,
            page: PageId,
            cookies: &[CookieParam],
        ) -> Result<(), AdapterError> {
            if cookies.is_empty() {
                return Ok(());
            }

            let payload: Vec<Value> = cookies
                .iter()
                .map(|cookie| {
                    let mut map = serde_json::Map::new();
                    map.insert("name".into(), Value::String(cookie.name.clone()));
                    map.insert("value".into(), Value::String(cookie.value.clone()));
                    if let Some(domain) = cookie.domain.as_ref() {
                        map.insert("domain".into(), Value::String(domain.clone()));
                    }
                    if let Some(path) = cookie.path.as_ref() {
                        map.insert("path".into(), Value::String(path.clone()));
                    }
                    if let Some(url) = cookie.url.as_ref() {
                        map.insert("url".into(), Value::String(url.clone()));
                    }
                    if let Some(expires) = cookie.expires {
                        if let Some(number) = Number::from_f64(expires) {
                            map.insert("expires".into(), Value::Number(number));
                        }
                    }
                    if let Some(flag) = cookie.http_only {
                        map.insert("httpOnly".into(), Value::Bool(flag));
                    }
                    if let Some(flag) = cookie.secure {
                        map.insert("secure".into(), Value::Bool(flag));
                    }
                    if let Some(site) = cookie.same_site.as_ref() {
                        map.insert("sameSite".into(), Value::String(site.clone()));
                    }
                    Value::Object(map)
                })
                .collect();

            self.send_page_command(page, "Network.setCookies", json!({ "cookies": payload }))
                .await?;
            Ok(())
        }

        async fn set_user_agent(
            &self,
            page: PageId,
            user_agent: &str,
            accept_language: Option<&str>,
            platform: Option<&str>,
            locale: Option<&str>,
        ) -> Result<(), AdapterError> {
            let mut params = serde_json::Map::new();
            params.insert("userAgent".into(), Value::String(user_agent.to_string()));
            if let Some(lang) = accept_language {
                params.insert("acceptLanguage".into(), Value::String(lang.to_string()));
            }
            if let Some(platform) = platform {
                params.insert("platform".into(), Value::String(platform.to_string()));
            }
            self.send_page_command(
                page,
                "Emulation.setUserAgentOverride",
                Value::Object(params),
            )
            .await?;

            if let Some(locale) = locale {
                self.send_page_command(
                    page,
                    "Emulation.setLocaleOverride",
                    json!({ "locale": locale }),
                )
                .await?;
            }

            Ok(())
        }

        async fn set_timezone(&self, page: PageId, timezone: &str) -> Result<(), AdapterError> {
            self.send_page_command(
                page,
                "Emulation.setTimezoneOverride",
                json!({ "timezoneId": timezone }),
            )
            .await?;
            Ok(())
        }

        async fn set_device_metrics(
            &self,
            page: PageId,
            width: u32,
            height: u32,
            device_scale_factor: f64,
            mobile: bool,
        ) -> Result<(), AdapterError> {
            self.send_page_command(
                page,
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": width,
                    "height": height,
                    "deviceScaleFactor": device_scale_factor,
                    "mobile": mobile,
                }),
            )
            .await?;
            Ok(())
        }

        async fn set_touch_emulation(
            &self,
            page: PageId,
            enabled: bool,
        ) -> Result<(), AdapterError> {
            self.send_page_command(
                page,
                "Emulation.setTouchEmulationEnabled",
                json!({ "enabled": enabled }),
            )
            .await?;
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
            // Note: The CDP parameter is "computedStyles", not "computedStyleWhitelist"
            let computed_styles = config
                .computed_style_whitelist
                .into_iter()
                .map(Value::String)
                .collect::<Vec<Value>>();
            params.insert("computedStyles".into(), Value::Array(computed_styles));
            // Note: includePaintOrder and includeDOMRects are valid CDP parameters
            // includeEventListeners and includeUserAgentShadowTree are not valid for captureSnapshot
            if config.include_paint_order {
                params.insert("includePaintOrder".into(), Value::Bool(true));
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
        use serde_json::Value;
        use std::collections::VecDeque;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Instant;
        use tokio::sync::mpsc;

        struct MockTransport {
            started: AtomicBool,
            rx: Mutex<mpsc::Receiver<TransportEvent>>,
            commands: Mutex<Vec<(String, Value)>>,
            responses: Mutex<VecDeque<Value>>,
        }

        impl MockTransport {
            fn new_pair() -> (Arc<Self>, mpsc::Sender<TransportEvent>) {
                let (tx, rx) = mpsc::channel(16);
                (
                    Arc::new(Self {
                        started: AtomicBool::new(false),
                        rx: Mutex::new(rx),
                        commands: Mutex::new(Vec::new()),
                        responses: Mutex::new(VecDeque::new()),
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
                self.responses.lock().await.push_back(value);
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
                Ok(self
                    .responses
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or(Value::Null))
            }
        }

        struct DisconnectingTransport {
            start_calls: AtomicUsize,
            next_calls: AtomicUsize,
            rx: Mutex<mpsc::Receiver<TransportEvent>>,
        }

        impl DisconnectingTransport {
            fn new_pair() -> (Arc<Self>, mpsc::Sender<TransportEvent>) {
                let (tx, rx) = mpsc::channel(16);
                (
                    Arc::new(Self {
                        start_calls: AtomicUsize::new(0),
                        next_calls: AtomicUsize::new(0),
                        rx: Mutex::new(rx),
                    }),
                    tx,
                )
            }

            fn start_calls(&self) -> usize {
                self.start_calls.load(Ordering::SeqCst)
            }
        }

        #[async_trait]
        impl CdpTransport for DisconnectingTransport {
            async fn start(&self) -> Result<(), AdapterError> {
                self.start_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            async fn next_event(&self) -> Option<TransportEvent> {
                let call = self.next_calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    return None;
                }
                let mut guard = self.rx.lock().await;
                guard.recv().await
            }

            async fn send_command(
                &self,
                _target: CommandTarget,
                _method: &str,
                _params: Value,
            ) -> Result<Value, AdapterError> {
                Ok(Value::Null)
            }
        }

        #[tokio::test]
        async fn ignores_unknown_events() {
            use tokio::time::{timeout, Duration as TokioDuration};

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

            let result = timeout(TokioDuration::from_millis(100), rx.recv()).await;
            assert!(
                result.is_err(),
                "unexpected raw event broadcast: {result:?}"
            );

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

            transport.set_response(Value::Null).await;
            transport
                .set_response(json!({
                    "result": {
                        "value": "complete"
                    }
                }))
                .await;

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
        async fn event_loop_recovers_after_transport_disconnect() {
            use tokio::time::{sleep, timeout, Duration as TokioDuration};

            let (bus, mut rx) = crate::event_bus(8);
            let (transport, tx) = DisconnectingTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            let stale_page = PageId::new();
            let stale_session = SessionId::new();
            let stale_target = "stale-target".to_string();
            let stale_cdp_session = "stale-session".to_string();

            adapter.registry.insert_page(
                stale_page,
                stale_session,
                Some(stale_target.clone()),
                Some(stale_cdp_session.clone()),
            );
            adapter.targets.insert(stale_target.clone(), stale_page);
            adapter
                .sessions
                .insert(stale_cdp_session.clone(), stale_page);
            adapter.page_activity.insert(stale_page, Instant::now());

            crate::metrics::reset();
            Arc::clone(&adapter).start().await.expect("start adapter");
            assert_eq!(transport.start_calls(), 1);

            timeout(TokioDuration::from_millis(200), async {
                while transport.start_calls() < 2 {
                    sleep(TokioDuration::from_millis(10)).await;
                }
            })
            .await
            .expect("transport restart");

            tx.send(TransportEvent {
                method: "Target.targetCreated".into(),
                params: json!({
                    "targetInfo": {
                        "targetId": "page-1",
                        "type": "page",
                        "url": "https://example.com"
                    }
                }),
                session_id: None,
            })
            .await
            .unwrap();

            let mut saw_closed = false;
            let mut saw_opened = false;
            let mut saw_error = false;

            for _ in 0..6 {
                let evt = timeout(TokioDuration::from_millis(200), rx.recv())
                    .await
                    .expect("receive raw event")
                    .expect("raw event payload");
                match evt {
                    RawEvent::PageLifecycle { page, phase, .. } => {
                        if phase == "closed" && page == stale_page {
                            saw_closed = true;
                        } else if phase == "opened" {
                            saw_opened = true;
                        }
                    }
                    RawEvent::PageNavigated { page, .. } => {
                        if page == stale_page {
                            saw_opened = true;
                        }
                    }
                    RawEvent::Error { .. } => saw_error = true,
                    _ => {}
                }
                if saw_closed && saw_opened && saw_error {
                    break;
                }
            }

            assert!(saw_closed, "expected closed lifecycle for stale page");
            assert!(saw_opened, "expected opened lifecycle after restart");
            assert!(saw_error, "expected transport restart error notification");
            assert!(adapter.targets.get(&stale_target).is_none());
            assert!(adapter.registry.get(&stale_page).is_none());
            assert!(adapter.page_activity.get(&stale_page).is_none());

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn network_events_emit_summaries_and_metrics() {
            use tokio::time::Duration as TokioDuration;
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

            tokio::time::timeout(TokioDuration::from_millis(500), async {
                loop {
                    if let Ok(evt) = rx.recv().await {
                        if let RawEvent::NetworkSummary { .. } = evt {
                            break;
                        }
                    }
                }
            })
            .await
            .expect("network summary event");

            let snapshot = crate::metrics::snapshot();
            assert!(snapshot.network_summaries >= 1);
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
            use network_tap_light::NetworkSnapshot;
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
                .network_tap
                .enable(CdpAdapter::tap_page_id(page))
                .await
                .expect("enable tap page");

            adapter
                .network_tap
                .update_snapshot(
                    CdpAdapter::tap_page_id(page),
                    NetworkSnapshot {
                        req: 10,
                        res2xx: 10,
                        res4xx: 0,
                        res5xx: 0,
                        inflight: 0,
                        quiet: true,
                        window_ms: 500,
                        since_last_activity_ms: 2_000,
                    },
                )
                .await
                .expect("update snapshot");

            let gate = serde_json::to_string(&WaitGate::NetworkQuiet {
                window_ms: 500,
                max_inflight: 0,
            })
            .expect("serialize gate");

            let baseline = transport.commands().await.len();

            adapter
                .wait_basic(page, gate, std::time::Duration::from_secs(1))
                .await
                .expect("wait_basic network quiet");

            let after = transport.commands().await.len();
            assert_eq!(after, baseline);

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

            transport.set_response(Value::Null).await;
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

            transport.set_response(Value::Null).await;
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
        async fn grant_permissions_dispatches_browser_command() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            crate::metrics::reset();
            Arc::clone(&adapter).start().await.expect("start adapter");

            adapter
                .grant_permissions("https://example.com", &vec!["clipboardRead".into()])
                .await
                .expect("grant permissions");

            let commands = transport.commands().await;
            let entry = commands
                .iter()
                .find(|(method, _)| method == "Browser.grantPermissions")
                .expect("grant command");
            assert_eq!(
                "https://example.com",
                entry.1.get("origin").and_then(|v| v.as_str()).unwrap()
            );
            let perms = entry
                .1
                .get("permissions")
                .and_then(|v| v.as_array())
                .expect("permissions array");
            assert_eq!(perms, &vec![Value::String("clipboardRead".into())]);

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn reset_permissions_dispatches_browser_command() {
            let (bus, _rx) = crate::event_bus(8);
            let (transport, _tx) = MockTransport::new_pair();
            let adapter = Arc::new(CdpAdapter::with_transport(
                CdpConfig::default(),
                bus,
                transport.clone() as Arc<dyn CdpTransport>,
            ));

            crate::metrics::reset();
            Arc::clone(&adapter).start().await.expect("start adapter");

            adapter
                .reset_permissions("https://example.com", &vec!["camera".into()])
                .await
                .expect("reset permissions");

            let commands = transport.commands().await;
            let entry = commands
                .iter()
                .find(|(method, _)| method == "Browser.resetPermissions")
                .expect("reset command");
            assert_eq!(
                "https://example.com",
                entry.1.get("origin").and_then(|v| v.as_str()).unwrap()
            );
            let perms = entry
                .1
                .get("permissions")
                .and_then(|v| v.as_array())
                .expect("permissions array");
            assert_eq!(perms, &vec![Value::String("camera".into())]);

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn set_user_agent_dispatches_emulation_override() {
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
            adapter.register_page(page, session, None, Some("session-ua".into()));

            adapter
                .set_user_agent(
                    page,
                    "Mozilla/5.0",
                    Some("en-US"),
                    Some("Win32"),
                    Some("en-US"),
                )
                .await
                .expect("set user agent");

            let commands = transport.commands().await;
            let ua_cmd = commands
                .iter()
                .find(|(method, _)| method == "Emulation.setUserAgentOverride")
                .expect("user agent command");
            assert_eq!(
                ua_cmd.1.get("userAgent").and_then(|v| v.as_str()),
                Some("Mozilla/5.0")
            );
            assert_eq!(
                ua_cmd.1.get("acceptLanguage").and_then(|v| v.as_str()),
                Some("en-US")
            );
            assert_eq!(
                ua_cmd.1.get("platform").and_then(|v| v.as_str()),
                Some("Win32")
            );

            let locale_cmd = commands
                .iter()
                .find(|(method, _)| method == "Emulation.setLocaleOverride")
                .expect("locale command");
            assert_eq!(
                locale_cmd.1.get("locale").and_then(|v| v.as_str()),
                Some("en-US")
            );

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn set_timezone_dispatches_emulation_override() {
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
            adapter.register_page(page, session, None, Some("session-tz".into()));

            adapter
                .set_timezone(page, "America/Los_Angeles")
                .await
                .expect("set timezone");

            let commands = transport.commands().await;
            let entry = commands
                .iter()
                .find(|(method, _)| method == "Emulation.setTimezoneOverride")
                .expect("timezone command");
            assert_eq!(
                entry.1.get("timezoneId").and_then(|v| v.as_str()),
                Some("America/Los_Angeles")
            );

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn set_device_metrics_dispatches_emulation_override() {
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
            adapter.register_page(page, session, None, Some("session-metrics".into()));

            adapter
                .set_device_metrics(page, 1280, 720, 1.5, false)
                .await
                .expect("set device metrics");

            let commands = transport.commands().await;
            let entry = commands
                .iter()
                .find(|(method, _)| method == "Emulation.setDeviceMetricsOverride")
                .expect("metrics command");
            assert_eq!(entry.1.get("width").and_then(|v| v.as_u64()), Some(1280));
            assert_eq!(entry.1.get("height").and_then(|v| v.as_u64()), Some(720));
            assert_eq!(
                entry.1.get("deviceScaleFactor").and_then(|v| v.as_f64()),
                Some(1.5)
            );
            assert_eq!(entry.1.get("mobile").and_then(|v| v.as_bool()), Some(false));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn set_touch_emulation_dispatches_command() {
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
            adapter.register_page(page, session, None, Some("session-touch".into()));

            adapter
                .set_touch_emulation(page, true)
                .await
                .expect("set touch emulation");

            let commands = transport.commands().await;
            let entry = commands
                .iter()
                .find(|(method, _)| method == "Emulation.setTouchEmulationEnabled")
                .expect("touch command");
            assert_eq!(entry.1.get("enabled").and_then(|v| v.as_bool()), Some(true));

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
                        "objectId": "remote-1"
                    }
                }))
                .await;

            transport
                .set_response(json!({
                    "result": {
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
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Runtime.callFunctionOn"));
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Runtime.releaseObject"));

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn click_dispatches_mouse_events() {
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
                        "value": [
                            { "x": 42.0, "y": 24.0, "backendNodeId": null }
                        ]
                    }
                }))
                .await;

            adapter
                .click(page, "button.primary", std::time::Duration::from_secs(2))
                .await
                .expect("click dispatch succeeds");

            let commands = transport.commands().await;
            let mut mouse_events: Vec<&Value> = commands
                .iter()
                .filter(|(method, _)| method == "Input.dispatchMouseEvent")
                .map(|(_, params)| params)
                .collect();
            assert_eq!(mouse_events.len(), 2);
            mouse_events.sort_by_key(|params| {
                params
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            });

            let pressed = mouse_events
                .iter()
                .find(|params| params.get("type").and_then(|v| v.as_str()) == Some("mousePressed"))
                .expect("mousePressed event present");
            assert_eq!(pressed.get("x").and_then(|v| v.as_f64()), Some(42.0));
            assert_eq!(pressed.get("y").and_then(|v| v.as_f64()), Some(24.0));

            assert!(
                mouse_events
                    .iter()
                    .any(|params| params.get("type").and_then(|v| v.as_str())
                        == Some("mouseReleased"))
            );

            adapter.shutdown().await;
        }

        #[tokio::test]
        async fn type_text_dispatches_key_events() {
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
                        "value": { "status": "focused" }
                    }
                }))
                .await;

            adapter
                .type_text(page, "#search", "ok", std::time::Duration::from_secs(2))
                .await
                .expect("type_text dispatch succeeds");

            let commands = transport.commands().await;
            let key_events: Vec<&Value> = commands
                .iter()
                .filter(|(method, _)| method == "Input.dispatchKeyEvent")
                .map(|(_, params)| params)
                .collect();
            assert_eq!(key_events.len(), 2);
            assert!(commands
                .iter()
                .any(|(method, _)| method == "Input.insertText"));

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

pub use adapter::{Cdp, CdpAdapter, EventBus, ResolvedExecutionContext};
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
