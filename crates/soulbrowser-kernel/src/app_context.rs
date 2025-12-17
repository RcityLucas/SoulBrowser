//! Application context and shared components
//!
//! Centralized management of shared application components like storage,
//! authentication, and tool managers to avoid multiple instances.

use parking_lot::RwLock as SyncRwLock;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use dashmap::DashMap;
use once_cell::sync::Lazy;

use crate::{
    auth::{BrowserAuthManager, SessionManager},
    config::BrowserConfiguration,
    errors::SoulBrowserError,
    integration::{default_provider, IntegrationProvider},
    plugin_registry::PluginRegistry,
    self_heal::SelfHealRegistry,
    storage::StorageManager,
    task_status::TaskStatusRegistry,
    tools::BrowserToolManager,
};
use cdp_adapter::CdpAdapter;
use l6_observe::{
    exporter as obs_exporter, guard::LabelMap as ObsLabelMap, metrics as obs_metrics,
    tracing as obs_tracing,
};
use memory_center::MemoryCenter;
use soulbrowser_core_types::{ExecRoute, SoulError};
use soulbrowser_event_bus::InMemoryBus;
use soulbrowser_policy_center::{
    default_snapshot, load_snapshot, InMemoryPolicyCenter, PolicyCenter, PolicyView,
};
use soulbrowser_registry::{IngestHandle, Registry, RegistryEvent, RegistryImpl};
use soulbrowser_scheduler::{
    api::SchedulerService,
    executor::{ToolDispatchResult, ToolExecutor as SchedulerToolExecutor},
    model::{DispatchRequest, SchedulerConfig},
    runtime::SchedulerRuntime,
};
use soulbrowser_state_center::{InMemoryStateCenter, StateCenter, StateCenterStats, StateEvent};
use tracing::warn;

use crate::l0_bridge::{L0Bridge, L0Handles};

/// Global application context
pub struct AppContext {
    storage: Arc<StorageManager>,
    auth_manager: Arc<BrowserAuthManager>,
    session_manager: Arc<SessionManager>,
    tool_manager: Arc<BrowserToolManager>,
    config: Arc<BrowserConfiguration>,
    _registry_bus: Arc<InMemoryBus<RegistryEvent>>,
    registry: Arc<RegistryImpl>,
    _registry_ingest: IngestHandle,
    _scheduler_runtime: Arc<SchedulerRuntime>,
    policy_center: Arc<dyn PolicyCenter + Send + Sync>,
    #[allow(dead_code)]
    policy_view: Arc<SyncRwLock<PolicyView>>,
    #[allow(dead_code)]
    registry_health_interval: Arc<AtomicU64>,
    #[allow(dead_code)]
    state_center_persist_enabled: Arc<AtomicBool>,
    #[allow(dead_code)]
    state_snapshot_path: PathBuf,
    state_center: Arc<InMemoryStateCenter>,
    scheduler_service: Arc<SchedulerService<RegistryImpl, ToolManagerExecutorAdapter>>,
    #[allow(dead_code)]
    l0_bridge: L0Bridge,
    #[allow(dead_code)]
    l0_handles: L0Handles,
    plugin_registry: Arc<PluginRegistry>,
    task_status_registry: Arc<TaskStatusRegistry>,
    memory_center: Arc<MemoryCenter>,
    self_heal_registry: Arc<SelfHealRegistry>,
    background_tasks: Vec<JoinHandle<()>>,
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct ContextCacheKey {
    tenant: String,
    storage: Option<PathBuf>,
    policy_hash: u64,
}

impl ContextCacheKey {
    fn new(tenant: &str, storage: Option<&PathBuf>, policy_paths: &[PathBuf]) -> Self {
        let mut canonical = policy_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        canonical.sort();
        let mut hasher = DefaultHasher::new();
        canonical.hash(&mut hasher);
        Self {
            tenant: tenant.to_string(),
            storage: storage.cloned(),
            policy_hash: hasher.finish(),
        }
    }
}

static CONTEXT_CACHE: Lazy<DashMap<ContextCacheKey, Weak<AppContext>>> = Lazy::new(DashMap::new);

impl AppContext {
    /// Create a new application context with the given configuration
    pub async fn new(
        tenant_id: String,
        storage_path: Option<PathBuf>,
        policy_paths: &[PathBuf],
    ) -> Result<Self, SoulBrowserError> {
        Self::new_with_provider(tenant_id, storage_path, policy_paths, default_provider()).await
    }

    pub async fn new_with_provider(
        tenant_id: String,
        storage_path: Option<PathBuf>,
        policy_paths: &[PathBuf],
        integration: Arc<dyn IntegrationProvider>,
    ) -> Result<Self, SoulBrowserError> {
        let mut background_tasks: Vec<JoinHandle<()>> = Vec::new();
        // Initialize storage
        let storage = integration
            .create_storage_manager(storage_path.clone())
            .await?;

        // Initialize auth components
        let auth_manager = integration
            .create_auth_manager(tenant_id.clone(), policy_paths)
            .await?;
        let session_manager = Arc::new(SessionManager::new());

        // Initialize tool manager
        let tool_manager = integration.create_tool_manager(tenant_id.clone()).await?;

        // Initialize configuration
        let mut config = BrowserConfiguration::new();
        config.load_defaults();

        let policy_path_opt = policy_paths
            .first()
            .map(|path| path.as_path())
            .filter(|path| path.exists())
            .or_else(|| {
                let default_path = std::path::Path::new("config/policy.yaml");
                if default_path.exists() {
                    Some(default_path)
                } else {
                    None
                }
            });
        let policy_snapshot = match load_snapshot(policy_path_opt) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                warn!("Failed to load policy snapshot: {err}");
                default_snapshot()
            }
        };
        let policy_center: Arc<dyn PolicyCenter + Send + Sync> =
            Arc::new(InMemoryPolicyCenter::new(policy_snapshot));
        let policy_snapshot_arc = policy_center.snapshot().await;
        let initial_view = PolicyView::from((*policy_snapshot_arc).clone());
        let policy_view: Arc<SyncRwLock<PolicyView>> =
            Arc::new(SyncRwLock::new(initial_view.clone()));
        let registry_health_interval = Arc::new(AtomicU64::new(
            initial_view.registry.health_probe_interval_ms,
        ));
        let state_center_persist_enabled = Arc::new(AtomicBool::new(
            initial_view.features.state_center_persistence,
        ));

        let state_center = Arc::new(InMemoryStateCenter::new(1024));
        let state_center_dyn: Arc<dyn StateCenter> = state_center.clone();
        let state_center_dyn_send: Arc<dyn StateCenter + Send + Sync> = state_center.clone();

        let snapshot_dir = storage_path
            .as_ref()
            .map(|path| path.join("state-center"))
            .unwrap_or_else(|| PathBuf::from("./soulbrowser-output/state-center"));
        if let Err(err) = std::fs::create_dir_all(&snapshot_dir) {
            warn!(
                path = %snapshot_dir.display(),
                %err,
                "failed to ensure state-center snapshot directory"
            );
        }
        let state_snapshot_path = snapshot_dir.join("snapshot.json");

        let registry_bus = InMemoryBus::new(128);
        let registry = Arc::new(RegistryImpl::with_state_center(
            Arc::clone(&state_center_dyn_send),
            Arc::clone(&policy_view),
        ));
        let registry_ingest = IngestHandle::spawn(
            registry_bus.clone(),
            registry.clone(),
            Arc::clone(&registry_health_interval),
        );

        let scheduler_runtime = Arc::new(SchedulerRuntime::new(SchedulerConfig {
            global_slots: initial_view.scheduler.limits.global_slots,
            per_task_limit: initial_view.scheduler.limits.per_task_limit,
        }));
        let executor_adapter = Arc::new(ToolManagerExecutorAdapter::new(tool_manager.clone()));
        let scheduler_service = Arc::new(SchedulerService::new(
            registry.clone(),
            scheduler_runtime.clone(),
            executor_adapter,
            Arc::clone(&state_center_dyn),
        ));
        scheduler_service.start().await;

        obs_metrics::ensure_metrics();
        obs_exporter::ensure_prometheus();

        {
            let mut policy_rx = policy_center.subscribe();
            let scheduler_runtime = Arc::clone(&scheduler_runtime);
            let policy_view_shared = Arc::clone(&policy_view);
            let registry_health_interval = Arc::clone(&registry_health_interval);
            let state_center_flag = Arc::clone(&state_center_persist_enabled);
            let handle = tokio::spawn(async move {
                loop {
                    if policy_rx.changed().await.is_err() {
                        break;
                    }
                    let snapshot_arc = policy_rx.borrow().clone();
                    let snapshot = (*snapshot_arc).clone();
                    let view = PolicyView::from(snapshot);
                    {
                        let mut guard = policy_view_shared.write();
                        *guard = view.clone();
                    }
                    scheduler_runtime.update_config(SchedulerConfig {
                        global_slots: view.scheduler.limits.global_slots,
                        per_task_limit: view.scheduler.limits.per_task_limit,
                    });
                    registry_health_interval
                        .store(view.registry.health_probe_interval_ms, Ordering::Relaxed);
                    state_center_flag
                        .store(view.features.state_center_persistence, Ordering::Relaxed);
                }
            });
            background_tasks.push(handle);
        }

        {
            let persist_flag = Arc::clone(&state_center_persist_enabled);
            let state_center = Arc::clone(&state_center);
            let snapshot_path = state_snapshot_path.clone();
            let handle = tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(5));
                loop {
                    ticker.tick().await;
                    if !persist_flag.load(Ordering::Relaxed) {
                        continue;
                    }
                    let state_center = Arc::clone(&state_center);
                    let path = snapshot_path.clone();
                    match tokio::task::spawn_blocking(move || {
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        state_center
                            .write_snapshot(&path)
                            .map_err(|e| format!("{e}"))
                    })
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => warn!(%err, "state-center snapshot write failed"),
                        Err(err) => warn!(?err, "state-center snapshot task join failed"),
                    }
                }
            });
            background_tasks.push(handle);
        }

        // Create a default session/page for initial routing
        let default_session = registry.session_create("cli-default").await.map_err(|e| {
            SoulBrowserError::internal(&format!("Failed to create default session: {}", e))
        })?;
        let default_page = registry
            .page_open(default_session.clone())
            .await
            .map_err(|e| {
                SoulBrowserError::internal(&format!("Failed to open default page: {}", e))
            })?;
        if let Err(err) = registry.page_focus(default_page).await {
            warn!("Failed to focus default page: {}", err);
        }

        let (l0_bridge, l0_handles) = L0Bridge::new(
            registry.clone(),
            Arc::clone(&state_center_dyn_send),
            default_session.clone(),
        )
        .await;

        let plugin_registry = Arc::new(PluginRegistry::load_default());
        let task_status_registry = Arc::new(TaskStatusRegistry::new(200));
        let memory_center = Arc::new(match &storage_path {
            Some(path) => {
                let mut persist_path = path.clone();
                persist_path.push("memory-center.json");
                MemoryCenter::with_persistence(persist_path).unwrap_or_else(|_| MemoryCenter::new())
            }
            None => MemoryCenter::new(),
        });
        let self_heal_registry = Arc::new(
            SelfHealRegistry::load_from_path(Some(PathBuf::from("config/self_heal.yaml")))
                .or_else(|err| {
                    warn!(
                        ?err,
                        "failed to load configured self-heal registry; using defaults"
                    );
                    SelfHealRegistry::load_from_path(None)
                })
                .unwrap_or_else(|_| {
                    SelfHealRegistry::load_from_path(None)
                        .expect("self-heal registry default initialization")
                }),
        );

        Ok(Self {
            storage,
            auth_manager,
            session_manager,
            tool_manager,
            config: Arc::new(config),
            _registry_bus: registry_bus,
            registry,
            _registry_ingest: registry_ingest,
            _scheduler_runtime: scheduler_runtime,
            policy_center,
            policy_view,
            registry_health_interval,
            state_center_persist_enabled,
            state_snapshot_path,
            state_center,
            scheduler_service,
            l0_bridge,
            l0_handles,
            plugin_registry,
            task_status_registry,
            memory_center,
            self_heal_registry,
            background_tasks,
        })
    }

    /// Get the storage manager
    pub fn storage(&self) -> Arc<StorageManager> {
        self.storage.clone()
    }

    #[allow(dead_code)]
    pub fn auth_manager(&self) -> Arc<BrowserAuthManager> {
        self.auth_manager.clone()
    }

    #[allow(dead_code)]
    pub fn session_manager(&self) -> Arc<SessionManager> {
        self.session_manager.clone()
    }

    #[allow(dead_code)]
    pub fn tool_manager(&self) -> Arc<BrowserToolManager> {
        self.tool_manager.clone()
    }

    #[allow(dead_code)]
    pub fn config(&self) -> Arc<BrowserConfiguration> {
        self.config.clone()
    }

    pub fn registry(&self) -> Arc<RegistryImpl> {
        self.registry.clone()
    }

    pub fn scheduler_service(
        &self,
    ) -> Arc<SchedulerService<RegistryImpl, ToolManagerExecutorAdapter>> {
        self.scheduler_service.clone()
    }

    pub fn plugin_registry(&self) -> Arc<PluginRegistry> {
        self.plugin_registry.clone()
    }

    pub fn task_status_registry(&self) -> Arc<TaskStatusRegistry> {
        self.task_status_registry.clone()
    }

    pub fn memory_center(&self) -> Arc<MemoryCenter> {
        self.memory_center.clone()
    }

    pub fn self_heal_registry(&self) -> Arc<SelfHealRegistry> {
        self.self_heal_registry.clone()
    }

    pub fn scheduler_runtime(&self) -> Arc<SchedulerRuntime> {
        Arc::clone(&self._scheduler_runtime)
    }

    pub fn policy_center(&self) -> Arc<dyn PolicyCenter + Send + Sync> {
        self.policy_center.clone()
    }

    #[allow(dead_code)]
    pub fn policy_view(&self) -> Arc<SyncRwLock<PolicyView>> {
        Arc::clone(&self.policy_view)
    }

    #[allow(dead_code)]
    pub fn state_center_persistence_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.state_center_persist_enabled)
    }

    pub fn shared_state_center(&self) -> Arc<InMemoryStateCenter> {
        self.state_center.clone()
    }

    #[allow(dead_code)]
    pub fn registry_health_interval(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.registry_health_interval)
    }

    pub fn state_center_snapshot(&self) -> Vec<StateEvent> {
        self.state_center.snapshot()
    }

    pub fn state_center_stats(&self) -> StateCenterStats {
        self.state_center.stats()
    }

    pub fn state_center(&self) -> Arc<InMemoryStateCenter> {
        self.state_center.clone()
    }

    #[allow(dead_code)]
    pub fn l0_handles(&self) -> &L0Handles {
        &self.l0_handles
    }

    /// Attach a CDP adapter so that L0 permissions broker can drive Browser.setPermission.
    pub async fn attach_cdp_adapter(&self, adapter: &Arc<CdpAdapter>) {
        self.l0_handles.attach_cdp_adapter(adapter).await;
    }
}

impl Drop for AppContext {
    fn drop(&mut self) {
        for handle in self.background_tasks.drain(..) {
            handle.abort();
        }
    }
}

/// Retrieve a cached application context for repeated CLI invocations.
pub async fn get_or_create_context(
    tenant_id: String,
    storage_path: Option<PathBuf>,
    policy_paths: Vec<PathBuf>,
) -> Result<Arc<AppContext>, SoulBrowserError> {
    get_or_create_context_with_provider(tenant_id, storage_path, policy_paths, default_provider())
        .await
}

pub async fn get_or_create_context_with_provider(
    tenant_id: String,
    storage_path: Option<PathBuf>,
    policy_paths: Vec<PathBuf>,
    integration: Arc<dyn IntegrationProvider>,
) -> Result<Arc<AppContext>, SoulBrowserError> {
    let key = ContextCacheKey::new(&tenant_id, storage_path.as_ref(), &policy_paths);
    if let Some(entry) = CONTEXT_CACHE.get(&key) {
        if let Some(ctx) = entry.value().upgrade() {
            return Ok(ctx);
        }
    }

    let context = Arc::new(
        AppContext::new_with_provider(tenant_id, storage_path, &policy_paths, integration).await?,
    );
    CONTEXT_CACHE.insert(key, Arc::downgrade(&context));
    Ok(context)
}

/// Create a brand new context bypassing the cache.
pub async fn create_context(
    tenant_id: String,
    storage_path: Option<PathBuf>,
    policy_paths: Vec<PathBuf>,
) -> Result<Arc<AppContext>, SoulBrowserError> {
    create_context_with_provider(tenant_id, storage_path, policy_paths, default_provider()).await
}

pub async fn create_context_with_provider(
    tenant_id: String,
    storage_path: Option<PathBuf>,
    policy_paths: Vec<PathBuf>,
    integration: Arc<dyn IntegrationProvider>,
) -> Result<Arc<AppContext>, SoulBrowserError> {
    Ok(Arc::new(
        AppContext::new_with_provider(tenant_id, storage_path, &policy_paths, integration).await?,
    ))
}

#[derive(Clone)]
pub struct ToolManagerExecutorAdapter {
    tools: Arc<BrowserToolManager>,
}

impl ToolManagerExecutorAdapter {
    fn new(tools: Arc<BrowserToolManager>) -> Self {
        Self { tools }
    }
}

const METRIC_SCHED_DISPATCHES: &str = "soul.l1.scheduler.dispatches";
const METRIC_SCHED_LATENCY: &str = "soul.l1.scheduler.dispatch_latency_ms";

#[async_trait::async_trait]
impl SchedulerToolExecutor for ToolManagerExecutorAdapter {
    async fn execute(
        &self,
        request: DispatchRequest,
        route: ExecRoute,
    ) -> Result<ToolDispatchResult, SoulError> {
        let start = Instant::now();
        let span = obs_tracing::tool_span(&request.tool_call.tool);
        let _guard = span.enter();

        let subject = route.frame.0.clone();
        let timeout_ms = request.options.timeout.as_millis().min(u64::MAX as u128) as u64;
        let result = self
            .tools
            .execute_with_route(
                &request.tool_call.tool,
                &subject,
                request.tool_call.payload.clone(),
                Some(route.clone()),
                Some(timeout_ms),
            )
            .await;

        let duration_ms = start.elapsed().as_millis() as u64;
        obs_tracing::observe_latency(&span, duration_ms);

        let mut labels: ObsLabelMap = ObsLabelMap::new();
        labels.insert("tool".into(), request.tool_call.tool.clone());

        match result {
            Ok(output) => {
                labels.insert("success".into(), "true".into());
                obs_metrics::inc(METRIC_SCHED_DISPATCHES, labels.clone());
                obs_metrics::observe(METRIC_SCHED_LATENCY, duration_ms, labels);
                Ok(ToolDispatchResult {
                    output: Some(output),
                })
            }
            Err(err) => {
                labels.insert("success".into(), "false".into());
                obs_metrics::inc(METRIC_SCHED_DISPATCHES, labels.clone());
                obs_metrics::observe(METRIC_SCHED_LATENCY, duration_ms, labels);
                Err(SoulError::new(err.to_string()))
            }
        }
    }
}
