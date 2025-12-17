use crate::{
    app_context::AppContext, build_exec_route, collect_events, ensure_real_chrome_enabled,
    wait_for_page_ready,
};
use anyhow::{anyhow, Context, Result};
use cdp_adapter::{
    adapter::CookieParam, config::CdpConfig, event_bus, ids::PageId as AdapterPageId, Cdp,
    CdpAdapter, EventBus,
};
use perceiver_hub::models::MultiModalPerception;
use perceiver_hub::{PerceptionHub, PerceptionHubImpl, PerceptionOptions};
use perceiver_semantic::SemanticPerceiverImpl;
use perceiver_structural::{
    AdapterPort as StructuralAdapterPort, StructuralPerceiver, StructuralPerceiverImpl,
};
use perceiver_visual::VisualPerceiverImpl;
use serde_json::{json, Value};
use soulbrowser_policy_center::{default_snapshot, InMemoryPolicyCenter, PolicyCenter};
use soulbrowser_state_center::{InMemoryStateCenter, StateCenter};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PerceptionJob {
    pub url: String,
    pub enable_structural: bool,
    pub enable_visual: bool,
    pub enable_semantic: bool,
    pub enable_insights: bool,
    pub capture_screenshot: bool,
    pub timeout_secs: u64,
    pub chrome_path: Option<PathBuf>,
    pub ws_url: Option<String>,
    pub headful: bool,
    pub viewport: Option<ViewportConfig>,
    pub cookies: Vec<CookieOverride>,
    pub inject_script: Option<String>,
    pub allow_pooling: bool,
}

impl PerceptionJob {
    pub fn normalize_modes(&self) -> (bool, bool, bool) {
        let any = self.enable_structural || self.enable_visual || self.enable_semantic;
        if any {
            (
                self.enable_structural,
                self.enable_visual,
                self.enable_semantic,
            )
        } else {
            (true, true, true)
        }
    }
}

#[derive(Debug, Clone)]
pub struct ViewportConfig {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub mobile: bool,
    pub emulate_touch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CookieOverride {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub expires: Option<f64>,
    pub http_only: Option<bool>,
    pub secure: Option<bool>,
    pub same_site: Option<String>,
}

#[derive(Debug)]
pub struct PerceptionOutput {
    pub perception: MultiModalPerception,
    pub screenshot: Option<Vec<u8>>,
    pub log_lines: Vec<String>,
}

pub struct PerceptionService {
    shared_session: Arc<Mutex<SharedSessionState>>,
    metrics: Arc<PerceptionMetrics>,
    pool_controller: Arc<PoolController>,
    shared_state_center: Option<Arc<InMemoryStateCenter>>,
    shared_policy_center: Option<Arc<dyn PolicyCenter + Send + Sync>>,
    #[cfg(test)]
    mock_executor: Option<Arc<MockExecutor>>,
    #[cfg(test)]
    mock_shared_ready: Arc<AtomicBool>,
}

impl Default for PerceptionService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
type MockExecutor = dyn Fn(PerceptionJob, bool) -> Result<PerceptionOutput> + Send + Sync;

impl Clone for PerceptionService {
    fn clone(&self) -> Self {
        Self {
            shared_session: Arc::clone(&self.shared_session),
            metrics: Arc::clone(&self.metrics),
            pool_controller: Arc::clone(&self.pool_controller),
            shared_state_center: self.shared_state_center.clone(),
            shared_policy_center: self.shared_policy_center.clone(),
            #[cfg(test)]
            mock_executor: self.mock_executor.clone(),
            #[cfg(test)]
            mock_shared_ready: Arc::clone(&self.mock_shared_ready),
        }
    }
}

impl fmt::Debug for PerceptionService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PerceptionService").finish()
    }
}

struct TempProfileGuard {
    path: PathBuf,
}

impl TempProfileGuard {
    async fn create() -> Result<Self> {
        let path = PathBuf::from(format!(".soulbrowser-profile-{}", Uuid::new_v4()));
        fs::create_dir_all(&path)
            .await
            .with_context(|| format!("creating profile directory {}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempProfileGuard {
    fn drop(&mut self) {
        let path = self.path.clone();
        tokio::spawn(async move {
            if let Err(err) = fs::remove_dir_all(&path).await {
                warn!(
                    path = %path.display(),
                    ?err,
                    "failed to remove temporary chrome profile directory"
                );
            }
        });
    }
}

impl PerceptionService {
    pub fn new() -> Self {
        Self::with_shared_components(None, None)
    }

    pub fn with_app_context(ctx: &AppContext) -> Self {
        let state_center = ctx.shared_state_center();
        let policy_center = ctx.policy_center();
        Self::with_shared_components(Some(state_center), Some(policy_center))
    }

    pub fn with_shared_components(
        state_center: Option<Arc<InMemoryStateCenter>>,
        policy_center: Option<Arc<dyn PolicyCenter + Send + Sync>>,
    ) -> Self {
        let pooling_enabled = env::var("SOULBROWSER_DISABLE_PERCEPTION_POOL")
            .map(|value| {
                let lower = value.to_ascii_lowercase();
                !(lower == "1" || lower == "true" || lower == "yes" || lower == "on")
            })
            .unwrap_or(true);
        Self {
            shared_session: Arc::new(Mutex::new(SharedSessionState::default())),
            metrics: Arc::new(PerceptionMetrics::default()),
            pool_controller: Arc::new(PoolController::new(pooling_enabled)),
            shared_state_center: state_center,
            shared_policy_center: policy_center,
            #[cfg(test)]
            mock_executor: None,
            #[cfg(test)]
            mock_shared_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn pooling_enabled(&self) -> bool {
        self.pool_controller.is_active()
    }

    pub fn pooling_cooldown_secs(&self) -> Option<u64> {
        self.pool_controller.cooldown_remaining_secs()
    }

    #[cfg(test)]
    pub fn with_mock_executor(mock: Arc<MockExecutor>) -> Self {
        Self {
            shared_session: Arc::new(Mutex::new(SharedSessionState::default())),
            metrics: Arc::new(PerceptionMetrics::default()),
            pool_controller: Arc::new(PoolController::from_policy(true, PoolingPolicy::default())),
            shared_state_center: None,
            shared_policy_center: None,
            mock_executor: Some(mock),
            mock_shared_ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn perceive(&self, job: PerceptionJob) -> Result<PerceptionOutput> {
        let start = Instant::now();
        let shared = job.ws_url.is_none()
            && job.allow_pooling
            && self
                .pool_controller
                .should_use_shared(&self.metrics.snapshot());
        #[cfg(test)]
        if let Some(executor) = &self.mock_executor {
            return self.run_mock_perception(job, shared, executor, start);
        }
        let result = if shared {
            self.perceive_with_shared(job).await
        } else {
            self.perceive_ephemeral(job).await
        };
        self.metrics.record(
            if shared {
                RunKind::Shared
            } else {
                RunKind::Ephemeral
            },
            start.elapsed(),
            result.is_ok(),
        );
        result
    }

    fn resolve_state_center(&self) -> Option<Arc<InMemoryStateCenter>> {
        self.shared_state_center.as_ref().map(Arc::clone)
    }

    fn resolve_policy_center(&self) -> Option<Arc<dyn PolicyCenter + Send + Sync>> {
        self.shared_policy_center.as_ref().map(Arc::clone)
    }

    fn resolve_runtime_centers(
        &self,
    ) -> (Arc<dyn StateCenter>, Arc<dyn PolicyCenter + Send + Sync>) {
        let concrete_state_center = self
            .resolve_state_center()
            .unwrap_or_else(|| Arc::new(InMemoryStateCenter::new(256)));
        let state_center_dyn: Arc<dyn StateCenter> = concrete_state_center.clone();
        let policy_center = self
            .resolve_policy_center()
            .unwrap_or_else(|| Arc::new(InMemoryPolicyCenter::new(default_snapshot())));
        (state_center_dyn, policy_center)
    }

    #[cfg(test)]
    fn run_mock_perception(
        &self,
        job: PerceptionJob,
        shared: bool,
        executor: &Arc<MockExecutor>,
        start: Instant,
    ) -> Result<PerceptionOutput> {
        if shared {
            if self.mock_shared_ready.swap(true, Ordering::SeqCst) {
                self.metrics.mark_shared_hit();
            } else {
                self.metrics.mark_shared_miss();
            }
        }
        let result = executor(job, shared);
        if shared && result.is_err() {
            self.metrics.mark_shared_failure();
            self.mock_shared_ready.store(false, Ordering::SeqCst);
        }
        self.metrics.record(
            if shared {
                RunKind::Shared
            } else {
                RunKind::Ephemeral
            },
            start.elapsed(),
            result.is_ok(),
        );
        result
    }

    async fn perceive_with_shared(&self, job: PerceptionJob) -> Result<PerceptionOutput> {
        let job_ref = &job;
        loop {
            let (view_opt, permit_arc) = {
                let state = self.shared_session.lock().await;
                (
                    state.view_if_compatible(job_ref),
                    Arc::clone(&state.semaphore),
                )
            };

            let permit = permit_arc
                .acquire_owned()
                .await
                .map_err(|_| anyhow!("shared perception semaphore closed"))?;

            if let Some(view) = view_opt {
                self.metrics.mark_shared_hit();
                return self.run_shared_perception(view, permit, job).await;
            }

            // No compatible session yet; release permit and create one.
            drop(permit);
            self.metrics.mark_shared_miss();
            ensure_real_chrome_enabled()?;
            let mut new_handle = Some(SharedSessionHandle::create(job_ref).await?);
            let to_shutdown = {
                let mut state = self.shared_session.lock().await;
                if state.is_compatible(job_ref) {
                    None
                } else {
                    state.replace_handle(new_handle.take().expect("handle present"))
                }
            };
            if let Some(handle) = to_shutdown {
                handle.shutdown().await;
            }
            if let Some(unused) = new_handle {
                unused.shutdown().await;
            }
        }
    }

    async fn run_shared_perception(
        &self,
        view: SharedHandleView,
        permit: OwnedSemaphorePermit,
        job: PerceptionJob,
    ) -> Result<PerceptionOutput> {
        let adapter = view.adapter();
        let mut rx = view.subscribe();
        let (state_center, policy_center) = self.resolve_runtime_centers();
        let result = execute_perception(
            adapter.clone(),
            &mut rx,
            &job,
            "shared session",
            state_center,
            policy_center,
        )
        .await;
        drop(permit);

        match result {
            Ok((output, page_id)) => {
                if let Err(err) = adapter
                    .navigate(page_id, "about:blank", Duration::from_secs(2))
                    .await
                {
                    warn!(?err, "failed to reset shared perception page");
                }
                Ok(output)
            }
            Err(err) => {
                self.metrics.mark_shared_failure();
                self.pool_controller
                    .penalize_failure("shared-session-error");
                self.discard_handle(view.id).await;
                Err(err)
            }
        }
    }

    async fn discard_handle(&self, id: Uuid) {
        let handle = {
            let mut state = self.shared_session.lock().await;
            if state
                .handle
                .as_ref()
                .map(|handle| handle.id == id)
                .unwrap_or(false)
            {
                state.handle.take()
            } else {
                None
            }
        };

        if let Some(handle) = handle {
            handle.shutdown().await;
        }
    }

    async fn perceive_ephemeral(&self, job: PerceptionJob) -> Result<PerceptionOutput> {
        let attach_existing = job.ws_url.is_some();
        if !attach_existing {
            ensure_real_chrome_enabled()?;
        }

        let temp_profile = if attach_existing {
            None
        } else {
            Some(TempProfileGuard::create().await?)
        };

        let (bus, mut rx) = event_bus(256);
        let mut adapter_cfg = CdpConfig::default();
        if let Some(ws) = &job.ws_url {
            adapter_cfg.websocket_url = Some(ws.clone());
        }
        if let Some(path) = &job.chrome_path {
            adapter_cfg.executable = path.clone();
        }
        if job.headful {
            adapter_cfg.headless = false;
        }
        if let Some(dir) = &temp_profile {
            adapter_cfg.user_data_dir = dir.path().to_path_buf();
        }

        let adapter = Arc::new(CdpAdapter::new(adapter_cfg, bus.clone()));
        attach_permissions_for_adapter(&adapter).await;
        Arc::clone(&adapter)
            .start()
            .await
            .map_err(|err| adapter_error("starting CDP adapter", err))?;

        let (state_center, policy_center) = self.resolve_runtime_centers();
        let result = execute_perception(
            Arc::clone(&adapter),
            &mut rx,
            &job,
            if attach_existing {
                "attached session"
            } else {
                "ephemeral session"
            },
            state_center,
            policy_center,
        )
        .await;

        adapter.shutdown().await;

        result.map(|(output, _)| output)
    }

    pub fn metrics_snapshot(&self) -> PerceptionMetricsSnapshot {
        self.metrics.snapshot()
    }
}

#[derive(Debug)]
struct PoolController {
    base_enabled: bool,
    policy: PoolingPolicy,
    enabled: AtomicBool,
    cooldown_until: AtomicU64,
}

impl PoolController {
    fn new(base_enabled: bool) -> Self {
        Self::from_policy(base_enabled, PoolingPolicy::from_env())
    }

    fn from_policy(base_enabled: bool, policy: PoolingPolicy) -> Self {
        let enabled = AtomicBool::new(base_enabled);
        Self {
            base_enabled,
            policy,
            enabled,
            cooldown_until: AtomicU64::new(0),
        }
    }

    fn should_use_shared(&self, metrics: &PerceptionMetricsSnapshot) -> bool {
        if !self.base_enabled {
            self.enabled.store(false, Ordering::SeqCst);
            return false;
        }

        let now = epoch_secs();
        if self.in_cooldown(now) {
            self.enabled.store(false, Ordering::SeqCst);
            return false;
        }

        let shared_total = metrics.shared_hits + metrics.shared_misses;
        if shared_total < self.policy.min_samples {
            self.enabled.store(true, Ordering::SeqCst);
            return true;
        }

        if let Some(max_avg) = self.policy.max_avg_ms {
            if metrics.avg_duration_ms > max_avg {
                self.trigger_cooldown(now, "avg-duration", Some(metrics.avg_duration_ms));
                return false;
            }
        }

        let hit_rate = if shared_total == 0 {
            1.0
        } else {
            metrics.shared_hits as f64 / shared_total as f64
        };

        if hit_rate < self.policy.disable_hit_rate {
            self.trigger_cooldown(now, "low-hit-rate", Some(hit_rate));
            return false;
        }

        if hit_rate >= self.policy.enable_hit_rate {
            let prev = self.enabled.swap(true, Ordering::SeqCst);
            if !prev {
                info!(
                    target = "perception_service",
                    hit_rate = hit_rate,
                    "perception shared pool re-enabled"
                );
            }
            self.cooldown_until.store(0, Ordering::SeqCst);
            return true;
        }

        self.enabled.load(Ordering::SeqCst)
    }

    fn penalize_failure(&self, reason: &str) {
        if !self.base_enabled {
            return;
        }
        let now = epoch_secs();
        self.trigger_cooldown(now, reason, None);
    }

    fn is_active(&self) -> bool {
        if !self.base_enabled {
            return false;
        }
        if self.in_cooldown(epoch_secs()) {
            return false;
        }
        self.enabled.load(Ordering::SeqCst)
    }

    fn cooldown_remaining_secs(&self) -> Option<u64> {
        if !self.base_enabled {
            return None;
        }
        let now = epoch_secs();
        let until = self.cooldown_until.load(Ordering::SeqCst);
        if until == 0 || until <= now {
            return None;
        }
        Some(until - now)
    }

    fn trigger_cooldown(&self, now: u64, reason: &str, measurement: Option<f64>) {
        if self.policy.cooldown_secs == 0 {
            self.enabled.store(false, Ordering::SeqCst);
        } else {
            self.cooldown_until
                .store(now + self.policy.cooldown_secs, Ordering::SeqCst);
            self.enabled.store(false, Ordering::SeqCst);
        }

        match measurement {
            Some(value) => info!(
                target = "perception_service",
                reason = reason,
                measurement = value,
                cooldown_secs = self.policy.cooldown_secs,
                "perception shared pool cooling down"
            ),
            None => info!(
                target = "perception_service",
                reason = reason,
                cooldown_secs = self.policy.cooldown_secs,
                "perception shared pool cooling down"
            ),
        }
    }

    fn in_cooldown(&self, now: u64) -> bool {
        let until = self.cooldown_until.load(Ordering::SeqCst);
        until != 0 && until > now
    }
}

#[derive(Clone, Copy, Debug)]
struct PoolingPolicy {
    min_samples: u64,
    disable_hit_rate: f64,
    enable_hit_rate: f64,
    cooldown_secs: u64,
    max_avg_ms: Option<f64>,
}

impl PoolingPolicy {
    fn from_env() -> Self {
        let mut policy = Self::default();
        if let Some(val) = read_env_u64("SOUL_PERCEPTION_POOL_MIN_SAMPLES") {
            policy.min_samples = val;
        }
        if let Some(val) = read_env_f64("SOUL_PERCEPTION_POOL_DISABLE_RATIO") {
            policy.disable_hit_rate = val.clamp(0.05, 0.95);
        }
        if let Some(val) = read_env_f64("SOUL_PERCEPTION_POOL_ENABLE_RATIO") {
            policy.enable_hit_rate = val.clamp(0.1, 0.99);
        }
        if let Some(val) = read_env_u64("SOUL_PERCEPTION_POOL_COOLDOWN_SECS") {
            policy.cooldown_secs = val;
        }
        if let Some(val) = read_env_f64("SOUL_PERCEPTION_POOL_MAX_AVG_MS") {
            policy.max_avg_ms = Some(val.max(1.0));
        }

        if policy.enable_hit_rate <= policy.disable_hit_rate {
            policy.enable_hit_rate = (policy.disable_hit_rate + 0.1).min(0.95);
        }
        policy
    }
}

impl Default for PoolingPolicy {
    fn default() -> Self {
        Self {
            min_samples: 6,
            disable_hit_rate: 0.35,
            enable_hit_rate: 0.65,
            cooldown_secs: 180,
            max_avg_ms: Some(20_000.0),
        }
    }
}

fn read_env_u64(name: &str) -> Option<u64> {
    match env::var(name) {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(value) => Some(value),
            Err(err) => {
                warn!(?err, env = name, value = raw, "invalid u64 env override");
                None
            }
        },
        Err(env::VarError::NotPresent) => None,
        Err(err) => {
            warn!(?err, env = name, "failed to read env override");
            None
        }
    }
}

fn read_env_f64(name: &str) -> Option<f64> {
    match env::var(name) {
        Ok(raw) => match raw.trim().parse::<f64>() {
            Ok(value) => Some(value),
            Err(err) => {
                warn!(?err, env = name, value = raw, "invalid f64 env override");
                None
            }
        },
        Err(env::VarError::NotPresent) => None,
        Err(err) => {
            warn!(?err, env = name, "failed to read env override");
            None
        }
    }
}

fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn build_perception_hub(
    _enable_structural: bool,
    enable_visual: bool,
    enable_semantic: bool,
    structural_perceiver: Arc<StructuralPerceiverImpl<StructuralAdapterPort<CdpAdapter>>>,
    adapter: Arc<CdpAdapter>,
) -> PerceptionHubImpl {
    if enable_visual && enable_semantic {
        let visual_perceiver = Arc::new(VisualPerceiverImpl::new(Arc::clone(&adapter)));
        let semantic_perceiver = Arc::new(SemanticPerceiverImpl::new(
            structural_perceiver.clone() as Arc<dyn StructuralPerceiver>
        ));
        PerceptionHubImpl::new(structural_perceiver, visual_perceiver, semantic_perceiver)
    } else if enable_visual {
        let visual_perceiver = Arc::new(VisualPerceiverImpl::new(Arc::clone(&adapter)));
        PerceptionHubImpl::structural_only(structural_perceiver).with_visual(visual_perceiver)
    } else if enable_semantic {
        let semantic_perceiver = Arc::new(SemanticPerceiverImpl::new(
            structural_perceiver.clone() as Arc<dyn StructuralPerceiver>
        ));
        PerceptionHubImpl::structural_only(structural_perceiver).with_semantic(semantic_perceiver)
    } else {
        PerceptionHubImpl::structural_only(structural_perceiver)
    }
}

struct SharedSessionState {
    handle: Option<SharedSessionHandle>,
    semaphore: Arc<Semaphore>,
}

impl SharedSessionState {
    fn view_if_compatible(&self, job: &PerceptionJob) -> Option<SharedHandleView> {
        self.handle
            .as_ref()
            .and_then(|handle| handle.view_if_compatible(job))
    }

    fn is_compatible(&self, job: &PerceptionJob) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_compatible(job))
            .unwrap_or(false)
    }

    fn replace_handle(&mut self, handle: SharedSessionHandle) -> Option<SharedSessionHandle> {
        self.handle.replace(handle)
    }
}

impl Default for SharedSessionState {
    fn default() -> Self {
        Self {
            handle: None,
            semaphore: Arc::new(Semaphore::new(1)),
        }
    }
}

#[derive(Clone)]
struct SharedHandleView {
    id: Uuid,
    adapter: Arc<CdpAdapter>,
    bus: EventBus,
}

impl SharedHandleView {
    fn adapter(&self) -> Arc<CdpAdapter> {
        Arc::clone(&self.adapter)
    }

    fn subscribe(&self) -> cdp_adapter::EventStream {
        self.bus.subscribe()
    }
}

struct SharedSessionHandle {
    id: Uuid,
    adapter: Arc<CdpAdapter>,
    bus: EventBus,
    profile_dir: Option<PathBuf>,
    chrome_path: Option<PathBuf>,
    headful: bool,
}

impl SharedSessionHandle {
    fn is_compatible(&self, job: &PerceptionJob) -> bool {
        self.headful == job.headful
            && self.chrome_path == job.chrome_path
            && job.viewport.is_none()
            && job.cookies.is_empty()
            && job.inject_script.is_none()
            && job.allow_pooling
    }

    fn view_if_compatible(&self, job: &PerceptionJob) -> Option<SharedHandleView> {
        if self.is_compatible(job) {
            Some(SharedHandleView {
                id: self.id,
                adapter: Arc::clone(&self.adapter),
                bus: self.bus.clone(),
            })
        } else {
            None
        }
    }

    async fn create(job: &PerceptionJob) -> Result<Self> {
        let profile_dir = Some(PathBuf::from(format!(
            ".soulbrowser-profile-shared-{}",
            Uuid::new_v4()
        )));
        if let Some(dir) = &profile_dir {
            fs::create_dir_all(dir)
                .await
                .with_context(|| format!("creating profile directory {}", dir.display()))?;
        }

        let (bus, _) = event_bus(256);
        let mut adapter_cfg = CdpConfig::default();
        if let Some(path) = &job.chrome_path {
            adapter_cfg.executable = path.clone();
        }
        if job.headful {
            adapter_cfg.headless = false;
        }
        if let Some(dir) = &profile_dir {
            adapter_cfg.user_data_dir = dir.clone();
        }

        let adapter = Arc::new(CdpAdapter::new(adapter_cfg, bus.clone()));
        attach_permissions_for_adapter(&adapter).await;
        Arc::clone(&adapter)
            .start()
            .await
            .map_err(|err| adapter_error("starting CDP adapter", err))?;

        Ok(Self {
            id: Uuid::new_v4(),
            adapter,
            bus,
            profile_dir,
            chrome_path: job.chrome_path.clone(),
            headful: job.headful,
        })
    }

    async fn shutdown(mut self) {
        self.adapter.shutdown().await;
        if let Some(dir) = self.profile_dir.take() {
            if let Err(err) = fs::remove_dir_all(&dir).await {
                warn!(
                    path = %dir.display(),
                    ?err,
                    "failed to remove shared chrome profile directory"
                );
            }
        }
    }
}

#[derive(Default)]
struct PerceptionMetrics {
    total_runs: AtomicU64,
    shared_hits: AtomicU64,
    shared_misses: AtomicU64,
    shared_failures: AtomicU64,
    ephemeral_runs: AtomicU64,
    total_duration_ns: AtomicU64,
    failed_runs: AtomicU64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use perceiver_hub::models::StructuralAnalysis;
    use serial_test::serial;
    use std::env;
    use std::sync::atomic::{AtomicBool as StdAtomicBool, Ordering as StdOrdering};

    fn dummy_output() -> PerceptionOutput {
        PerceptionOutput {
            perception: MultiModalPerception {
                structural: StructuralAnalysis {
                    snapshot_id: "snapshot".into(),
                    dom_node_count: 1,
                    interactive_element_count: 0,
                    has_forms: false,
                    has_navigation: false,
                },
                visual: None,
                semantic: None,
                insights: Vec::new(),
                confidence: 1.0,
            },
            screenshot: None,
            log_lines: Vec::new(),
        }
    }

    fn shared_job() -> PerceptionJob {
        PerceptionJob {
            url: "https://example.com".into(),
            enable_structural: true,
            enable_visual: false,
            enable_semantic: false,
            enable_insights: false,
            capture_screenshot: false,
            timeout_secs: 1,
            chrome_path: None,
            ws_url: None,
            headful: false,
            viewport: None,
            cookies: Vec::new(),
            inject_script: None,
            allow_pooling: true,
        }
    }

    #[test]
    #[serial]
    fn pooling_enabled_by_default() {
        env::remove_var("SOULBROWSER_DISABLE_PERCEPTION_POOL");
        let service = PerceptionService::new();
        assert!(service.pooling_enabled(), "pooling should be on by default");
    }

    #[test]
    #[serial]
    fn pooling_can_be_disabled_via_env() {
        env::set_var("SOULBROWSER_DISABLE_PERCEPTION_POOL", "1");
        let service = PerceptionService::new();
        assert!(
            !service.pooling_enabled(),
            "pooling flag should honor env override"
        );
        env::remove_var("SOULBROWSER_DISABLE_PERCEPTION_POOL");
    }

    #[tokio::test]
    async fn mock_executor_tracks_shared_hits_and_misses() {
        let executor: Arc<MockExecutor> = Arc::new(|_, _| Ok(dummy_output()));
        let service = PerceptionService::with_mock_executor(executor);

        let job = shared_job();
        service.perceive(job.clone()).await.expect("first run");
        let snapshot = service.metrics_snapshot();
        assert_eq!(snapshot.shared_misses, 1);
        assert_eq!(snapshot.shared_hits, 0);

        service.perceive(job).await.expect("second run");
        let snapshot = service.metrics_snapshot();
        assert_eq!(snapshot.shared_hits, 1);
        assert_eq!(snapshot.shared_misses, 1);
    }

    #[tokio::test]
    async fn mock_executor_records_shared_failures_and_resets() {
        let fail_next = Arc::new(StdAtomicBool::new(true));
        let executor: Arc<MockExecutor> = {
            let flag = fail_next.clone();
            Arc::new(move |_, shared| {
                if shared && flag.swap(false, StdOrdering::SeqCst) {
                    Err(anyhow!("synthetic failure"))
                } else {
                    Ok(dummy_output())
                }
            })
        };

        let service = PerceptionService::with_mock_executor(executor);
        let job = shared_job();

        assert!(service.perceive(job.clone()).await.is_err());
        let snapshot = service.metrics_snapshot();
        assert_eq!(snapshot.shared_failures, 1);
        assert_eq!(snapshot.shared_misses, 1);

        service
            .perceive(job.clone())
            .await
            .expect("recreate session");
        let snapshot = service.metrics_snapshot();
        assert_eq!(snapshot.shared_misses, 2);
        assert_eq!(snapshot.shared_hits, 0);

        service.perceive(job).await.expect("reuse session");
        let snapshot = service.metrics_snapshot();
        assert_eq!(snapshot.shared_hits, 1);
    }

    #[test]
    fn pool_controller_disables_on_low_hits() {
        let policy = PoolingPolicy {
            min_samples: 1,
            disable_hit_rate: 0.6,
            enable_hit_rate: 0.8,
            cooldown_secs: 30,
            max_avg_ms: None,
        };
        let controller = PoolController::from_policy(true, policy);
        let mut snapshot = PerceptionMetricsSnapshot {
            total_runs: 0,
            shared_hits: 0,
            shared_misses: 0,
            shared_failures: 0,
            ephemeral_runs: 0,
            failed_runs: 0,
            avg_duration_ms: 0.0,
        };
        assert!(controller.should_use_shared(&snapshot));

        snapshot.shared_misses = 10;
        assert!(!controller.should_use_shared(&snapshot));
        assert!(controller.cooldown_remaining_secs().is_some());

        controller.cooldown_until.store(0, StdOrdering::SeqCst);
        snapshot.shared_hits = 8;
        snapshot.shared_misses = 2;
        assert!(controller.should_use_shared(&snapshot));
    }

    #[test]
    fn pool_controller_respects_avg_duration_cap() {
        let policy = PoolingPolicy {
            min_samples: 1,
            disable_hit_rate: 0.1,
            enable_hit_rate: 0.2,
            cooldown_secs: 1,
            max_avg_ms: Some(50.0),
        };
        let controller = PoolController::from_policy(true, policy);
        let snapshot = PerceptionMetricsSnapshot {
            total_runs: 10,
            shared_hits: 10,
            shared_misses: 0,
            shared_failures: 0,
            ephemeral_runs: 0,
            failed_runs: 0,
            avg_duration_ms: 125.0,
        };
        assert!(!controller.should_use_shared(&snapshot));
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PerceptionMetricsSnapshot {
    pub total_runs: u64,
    pub shared_hits: u64,
    pub shared_misses: u64,
    pub shared_failures: u64,
    pub ephemeral_runs: u64,
    pub failed_runs: u64,
    pub avg_duration_ms: f64,
}

#[derive(Clone, Copy)]
enum RunKind {
    Shared,
    Ephemeral,
}

impl PerceptionMetrics {
    fn record(&self, kind: RunKind, duration: Duration, success: bool) {
        self.total_runs.fetch_add(1, Ordering::Relaxed);
        if matches!(kind, RunKind::Ephemeral) {
            self.ephemeral_runs.fetch_add(1, Ordering::Relaxed);
        }
        self.total_duration_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        if !success {
            self.failed_runs.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn mark_shared_hit(&self) {
        self.shared_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn mark_shared_miss(&self) {
        self.shared_misses.fetch_add(1, Ordering::Relaxed);
    }

    fn mark_shared_failure(&self) {
        self.shared_failures.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> PerceptionMetricsSnapshot {
        let total = self.total_runs.load(Ordering::Relaxed);
        let divisor = if total == 0 { 1 } else { total };
        let duration_ns = self.total_duration_ns.load(Ordering::Relaxed);
        PerceptionMetricsSnapshot {
            total_runs: total,
            shared_hits: self.shared_hits.load(Ordering::Relaxed),
            shared_misses: self.shared_misses.load(Ordering::Relaxed),
            shared_failures: self.shared_failures.load(Ordering::Relaxed),
            ephemeral_runs: self.ephemeral_runs.load(Ordering::Relaxed),
            failed_runs: self.failed_runs.load(Ordering::Relaxed),
            avg_duration_ms: duration_ns as f64 / divisor as f64 / 1_000_000.0,
        }
    }
}

#[cfg(test)]
mod metrics_tests {
    use super::{PerceptionMetrics, RunKind};
    use std::time::Duration;

    #[test]
    fn record_shared_and_ephemeral_runs() {
        let metrics = PerceptionMetrics::default();
        metrics.record(RunKind::Shared, Duration::from_millis(200), true);
        metrics.record(RunKind::Ephemeral, Duration::from_millis(100), false);
        metrics.mark_shared_hit();
        metrics.mark_shared_miss();
        metrics.mark_shared_failure();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_runs, 2);
        assert_eq!(snapshot.ephemeral_runs, 1);
        assert_eq!(snapshot.failed_runs, 1);
        assert_eq!(snapshot.shared_hits, 1);
        assert_eq!(snapshot.shared_misses, 1);
        assert_eq!(snapshot.shared_failures, 1);
        assert!(snapshot.avg_duration_ms >= 150.0 && snapshot.avg_duration_ms <= 200.0);
    }
}

async fn execute_perception(
    adapter: Arc<CdpAdapter>,
    rx: &mut cdp_adapter::EventStream,
    job: &PerceptionJob,
    session_label: &str,
    state_center: Arc<dyn StateCenter>,
    policy_center: Arc<dyn PolicyCenter + Send + Sync>,
) -> Result<(PerceptionOutput, AdapterPageId)> {
    let mut summary_logs = Vec::new();
    summary_logs.push(format!("{} → {}", session_label, job.url));

    let mut event_log = Vec::new();
    let page_id = wait_for_page_ready(
        Arc::clone(&adapter),
        rx,
        Duration::from_secs(job.timeout_secs),
        &mut event_log,
    )
    .await?;
    summary_logs.push("page ready".to_string());

    if let Some(viewport) = job.viewport.as_ref() {
        apply_viewport_override(&adapter, page_id, viewport, &mut summary_logs).await?;
    }

    if !job.cookies.is_empty() {
        apply_cookie_overrides(&adapter, page_id, &job.cookies, &job.url, &mut summary_logs)
            .await?;
    }

    adapter
        .navigate(page_id, &job.url, Duration::from_secs(job.timeout_secs))
        .await
        .map_err(|err| adapter_error("navigating to URL", err))?;
    summary_logs.push("navigation completed".to_string());

    adapter
        .wait_basic(
            page_id,
            "domready".to_string(),
            Duration::from_secs(job.timeout_secs),
        )
        .await
        .map_err(|err| adapter_error("waiting for DOM readiness", err))?;
    summary_logs.push("dom ready".to_string());

    if let Some(script) = job.inject_script.as_deref() {
        run_custom_script(&adapter, page_id, script, &mut summary_logs).await?;
    }

    let frame_stable_gate = json!({ "FrameStable": { "min_stable_ms": 200 } }).to_string();
    if let Err(err) = adapter
        .wait_basic(page_id, frame_stable_gate, Duration::from_secs(5))
        .await
    {
        summary_logs.push(format!("frame stability wait skipped: {}", err.kind));
    }

    sleep(Duration::from_millis(300)).await;
    collect_events(rx, Duration::from_millis(500), &mut event_log).await?;

    let exec_route = build_exec_route(&adapter, page_id)?;
    let perception_port = Arc::new(StructuralAdapterPort::new(Arc::clone(&adapter)));

    let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
    let structural_perceiver = Arc::new(
        StructuralPerceiverImpl::<StructuralAdapterPort<CdpAdapter>>::with_state_center_and_live_policy(
            Arc::clone(&perception_port),
            state_center_dyn,
            Arc::clone(&policy_center),
        )
        .await,
    );

    let (mut enable_structural, mut enable_visual, mut enable_semantic) = job.normalize_modes();
    if !enable_structural && !enable_visual && !enable_semantic {
        enable_structural = true;
        enable_visual = true;
        enable_semantic = true;
    }

    let hub = build_perception_hub(
        enable_structural,
        enable_visual,
        enable_semantic,
        structural_perceiver,
        Arc::clone(&adapter),
    );

    let perception_opts = PerceptionOptions {
        enable_structural,
        enable_visual,
        enable_semantic,
        enable_insights: job.enable_insights,
        capture_screenshot: job.capture_screenshot,
        extract_text: enable_semantic,
        timeout_secs: job.timeout_secs,
    };

    let perception = hub
        .perceive(&exec_route, perception_opts)
        .await
        .context("multi-modal perception failed")?;
    summary_logs.push("perception completed".to_string());

    let screenshot = if job.capture_screenshot {
        if let Some(visual) = hub.visual() {
            match visual
                .capture_screenshot(&exec_route, Default::default())
                .await
            {
                Ok(data) => Some(data.data),
                Err(err) => {
                    summary_logs.push(format!("screenshot failed: {}", err));
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut combined = summary_logs;
    combined.extend(event_log);

    Ok((
        PerceptionOutput {
            perception,
            screenshot,
            log_lines: combined,
        },
        page_id,
    ))
}

async fn apply_viewport_override(
    adapter: &Arc<CdpAdapter>,
    page_id: AdapterPageId,
    viewport: &ViewportConfig,
    summary_logs: &mut Vec<String>,
) -> Result<()> {
    adapter
        .set_device_metrics(
            page_id,
            viewport.width,
            viewport.height,
            viewport.device_scale_factor,
            viewport.mobile,
        )
        .await
        .map_err(|err| adapter_error("setting viewport overrides", err))?;
    adapter
        .set_touch_emulation(page_id, viewport.emulate_touch)
        .await
        .map_err(|err| adapter_error("configuring touch emulation", err))?;
    summary_logs.push(format!(
        "viewport {}x{} scale={} mobile={} touch={}",
        viewport.width,
        viewport.height,
        viewport.device_scale_factor,
        viewport.mobile,
        viewport.emulate_touch
    ));
    Ok(())
}

async fn apply_cookie_overrides(
    adapter: &Arc<CdpAdapter>,
    page_id: AdapterPageId,
    overrides: &[CookieOverride],
    fallback_url: &str,
    summary_logs: &mut Vec<String>,
) -> Result<()> {
    if overrides.is_empty() {
        return Ok(());
    }

    let params = build_cookie_params(overrides, fallback_url);
    adapter
        .set_cookies(page_id, &params)
        .await
        .map_err(|err| adapter_error("setting cookies", err))?;
    summary_logs.push(format!("applied {} cookie(s)", params.len()));
    Ok(())
}

fn build_cookie_params(overrides: &[CookieOverride], fallback_url: &str) -> Vec<CookieParam> {
    overrides
        .iter()
        .map(|cookie| {
            let domain = cookie
                .domain
                .as_ref()
                .and_then(|value| trimmed_non_empty(value));
            let path = cookie
                .path
                .as_ref()
                .and_then(|value| trimmed_non_empty(value));
            let url = cookie
                .url
                .as_ref()
                .and_then(|value| trimmed_non_empty(value))
                .or_else(|| domain.is_none().then(|| fallback_url.to_string()));
            let same_site = cookie
                .same_site
                .as_ref()
                .and_then(|value| trimmed_non_empty(value));

            CookieParam {
                name: cookie.name.clone(),
                value: cookie.value.clone(),
                domain,
                path,
                url,
                expires: cookie.expires,
                http_only: cookie.http_only,
                secure: cookie.secure,
                same_site,
            }
        })
        .collect()
}

fn trimmed_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn run_custom_script(
    adapter: &Arc<CdpAdapter>,
    page_id: AdapterPageId,
    script: &str,
    summary_logs: &mut Vec<String>,
) -> Result<()> {
    let value = adapter
        .evaluate_script(page_id, script)
        .await
        .map_err(|err| adapter_error("executing inject_script", err))?;
    summary_logs.push(format!("custom script result: {}", summarize_value(&value)));
    Ok(())
}

fn summarize_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => {
            const MAX_LEN: usize = 64;
            if text.len() > MAX_LEN {
                format!("{}…", &text[..MAX_LEN])
            } else {
                text.clone()
            }
        }
        Value::Array(_) => "array".to_string(),
        Value::Object(_) => "object".to_string(),
    }
}

async fn attach_permissions_for_adapter(_adapter: &Arc<CdpAdapter>) {
    // Permissions are managed at higher layers; keep hook for parity with future policy work.
}

fn adapter_error(context: &str, err: cdp_adapter::AdapterError) -> anyhow::Error {
    let hint = err.hint.clone().unwrap_or_default();
    let data = err
        .data
        .as_ref()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    anyhow!(
        "{}: kind={:?}, retriable={}, hint={}, data={}",
        context,
        err.kind,
        err.retriable,
        hint,
        data
    )
}
