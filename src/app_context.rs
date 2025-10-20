//! Application context and shared components
//!
//! Centralized management of shared application components like storage,
//! authentication, and tool managers to avoid multiple instances.

use once_cell::sync::Lazy;
use parking_lot::RwLock as SyncRwLock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Duration;

use crate::{
    auth::{BrowserAuthManager, SessionManager},
    config::BrowserConfiguration,
    errors::SoulBrowserError,
    storage::StorageManager,
    tools::BrowserToolManager,
};
use soulbrowser_core_types::{ExecRoute, SoulError};
use soulbrowser_event_bus::InMemoryBus;
use soulbrowser_policy_center::{
    default_snapshot, load_snapshot, InMemoryPolicyCenter, PolicyCenter, PolicyView,
};
use soulbrowser_registry::{IngestHandle, Registry, RegistryEvent, RegistryImpl};
use soulbrowser_scheduler::model::DispatchRequest;
use soulbrowser_scheduler::{
    api::SchedulerService, executor::ToolExecutor as SchedulerToolExecutor, model::SchedulerConfig,
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
}

impl AppContext {
    /// Create a new application context with the given configuration
    pub async fn new(
        tenant_id: String,
        storage_path: Option<PathBuf>,
        policy_paths: &[PathBuf],
    ) -> Result<Self, SoulBrowserError> {
        // Initialize storage
        let storage = Arc::new(if let Some(path) = storage_path {
            StorageManager::file_based(path)
        } else {
            StorageManager::in_memory()
        });

        // Initialize auth components
        let auth_manager = if policy_paths.is_empty() {
            BrowserAuthManager::new(tenant_id.clone()).await
        } else {
            BrowserAuthManager::with_policy_paths(tenant_id.clone(), policy_paths).await
        }
        .map_err(|e| {
            SoulBrowserError::internal(&format!("Failed to create auth manager: {}", e))
        })?;
        let auth_manager = Arc::new(auth_manager);
        let session_manager = Arc::new(SessionManager::new());

        // Initialize tool manager
        let tool_manager = Arc::new(BrowserToolManager::new(tenant_id.clone()));
        tool_manager
            .register_default_tools()
            .await
            .map_err(|e| SoulBrowserError::internal(&format!("Failed to register tools: {}", e)))?;

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

        let snapshot_dir = PathBuf::from("./soulbrowser-output");
        if let Err(err) = std::fs::create_dir_all(&snapshot_dir) {
            warn!(
                path = %snapshot_dir.display(),
                %err,
                "failed to ensure state-center snapshot directory"
            );
        }
        let state_snapshot_path = snapshot_dir.join("state-center-snapshot.json");

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

        {
            let mut policy_rx = policy_center.subscribe();
            let scheduler_runtime = Arc::clone(&scheduler_runtime);
            let policy_view_shared = Arc::clone(&policy_view);
            let registry_health_interval = Arc::clone(&registry_health_interval);
            let state_center_flag = Arc::clone(&state_center_persist_enabled);
            tokio::spawn(async move {
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
        }

        {
            let persist_flag = Arc::clone(&state_center_persist_enabled);
            let state_center = Arc::clone(&state_center);
            let snapshot_path = state_snapshot_path.clone();
            tokio::spawn(async move {
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
            Arc::clone(&state_center_dyn),
            default_session.clone(),
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

    #[allow(dead_code)]
    pub fn l0_handles(&self) -> &L0Handles {
        &self.l0_handles
    }
}

/// Global singleton instance of AppContext
static GLOBAL_CONTEXT: Lazy<RwLock<Option<Arc<AppContext>>>> = Lazy::new(|| RwLock::new(None));

/// Get or create the global application context
pub async fn get_or_create_context(
    tenant_id: String,
    storage_path: Option<PathBuf>,
    policy_paths: Vec<PathBuf>,
) -> Result<Arc<AppContext>, SoulBrowserError> {
    let mut context = GLOBAL_CONTEXT.write().await;

    if context.is_none() {
        *context = Some(Arc::new(
            AppContext::new(tenant_id, storage_path, &policy_paths).await?,
        ));
    }

    Ok(context.as_ref().unwrap().clone())
}

/// Reset the global context (useful for testing)
#[allow(dead_code)]
pub async fn reset_context() {
    let mut context = GLOBAL_CONTEXT.write().await;
    *context = None;
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

#[async_trait::async_trait]
impl SchedulerToolExecutor for ToolManagerExecutorAdapter {
    async fn execute(&self, request: DispatchRequest, route: ExecRoute) -> Result<(), SoulError> {
        let subject = route.frame.0.clone();
        self.tools
            .execute(
                &request.tool_call.tool,
                &subject,
                request.tool_call.payload.clone(),
            )
            .await
            .map(|_| ())
            .map_err(|err| SoulError::new(err.to_string()))
    }
}
