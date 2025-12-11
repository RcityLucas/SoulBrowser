use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use tokio::sync::{RwLock, Semaphore};

use crate::app_context::{get_or_create_context, reset_context, AppContext};
use crate::llm::LlmCachePool;
use crate::perception_service::PerceptionService;
use crate::task_status::TaskStatusRegistry;
use crate::Config;

use super::rate_limit::RateLimiter;

#[derive(Clone)]
pub(crate) struct ServeState {
    pub(crate) ws_url: Option<String>,
    pub(crate) config: Arc<Config>,
    pub(crate) perception_service: Arc<PerceptionService>,
    pub(crate) llm_cache: Option<Arc<LlmCachePool>>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    app_context: Arc<RwLock<Arc<AppContext>>>,
    pub(crate) health: Arc<ServeHealth>,
    pub(crate) chat_context_limit: usize,
    pub(crate) chat_context_wait: Option<std::time::Duration>,
    pub(crate) chat_context_semaphore: Arc<Semaphore>,
    tenant_id: String,
    tenant_storage_root: PathBuf,
}

impl ServeState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        ws_url: Option<String>,
        config: Arc<Config>,
        perception_service: Arc<PerceptionService>,
        llm_cache: Option<Arc<LlmCachePool>>,
        rate_limiter: Arc<RateLimiter>,
        app_context: Arc<RwLock<Arc<AppContext>>>,
        health: Arc<ServeHealth>,
        chat_context_limit: usize,
        chat_context_wait: Option<std::time::Duration>,
        chat_context_semaphore: Arc<Semaphore>,
        tenant_id: String,
        tenant_storage_root: PathBuf,
    ) -> Self {
        Self {
            ws_url,
            config,
            perception_service,
            llm_cache,
            rate_limiter,
            app_context,
            health,
            chat_context_limit,
            chat_context_wait,
            chat_context_semaphore,
            tenant_id,
            tenant_storage_root,
        }
    }

    pub(crate) fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub(crate) fn default_storage_root(&self) -> PathBuf {
        self.tenant_storage_root.clone()
    }

    pub(crate) async fn app_context(&self) -> Arc<AppContext> {
        self.app_context.read().await.clone()
    }

    pub(crate) async fn task_status_registry(&self) -> Arc<TaskStatusRegistry> {
        self.app_context().await.task_status_registry()
    }

    pub(crate) async fn refresh_app_context(&self) -> Result<()> {
        reset_context().await;
        let context = self.build_context().await?;
        let mut guard = self.app_context.write().await;
        *guard = context;
        Ok(())
    }

    async fn build_context(&self) -> Result<Arc<AppContext>> {
        get_or_create_context(
            self.tenant_id.clone(),
            Some(self.tenant_storage_root.clone()),
            self.config.policy_paths.clone(),
        )
        .await
        .map_err(|err| anyhow!(err.to_string()))
    }

    pub(crate) fn health_snapshot(&self) -> HealthSnapshot {
        let inner = self.health.snapshot();
        HealthSnapshot {
            pooling_enabled: self.perception_service.pooling_enabled(),
            pooling_cooldown_secs: self.perception_service.pooling_cooldown_secs(),
            llm_cache_enabled: self.llm_cache.is_some(),
            ready: inner.ready,
            live: inner.live,
            last_ready_check: inner.last_ready_check,
            last_error: inner.last_error,
        }
    }

    pub(crate) fn mark_live(&self) {
        self.health.mark_live();
    }

    pub(crate) fn mark_ready(&self) {
        self.health.mark_ready();
    }

    pub(crate) fn mark_unready(&self, error: impl Into<String>) {
        self.health.mark_unready(error);
    }
}

pub(crate) struct HealthSnapshot {
    pub(crate) pooling_enabled: bool,
    pub(crate) pooling_cooldown_secs: Option<u64>,
    pub(crate) llm_cache_enabled: bool,
    pub(crate) ready: bool,
    pub(crate) live: bool,
    pub(crate) last_ready_check: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct ServeHealth {
    live: AtomicBool,
    ready: AtomicBool,
    last_ready_check: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl ServeHealth {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn mark_live(&self) {
        self.live.store(true, Ordering::SeqCst);
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
        self.update_last_check();
        let mut guard = self.last_error.lock().expect("health lock poisoned");
        *guard = None;
    }

    pub(crate) fn mark_unready(&self, error: impl Into<String>) {
        self.ready.store(false, Ordering::SeqCst);
        self.update_last_check();
        let mut guard = self.last_error.lock().expect("health lock poisoned");
        *guard = Some(error.into());
    }

    pub(crate) fn snapshot(&self) -> ServeHealthSnapshot {
        ServeHealthSnapshot {
            ready: self.ready.load(Ordering::SeqCst),
            live: self.live.load(Ordering::SeqCst),
            last_ready_check: self.last_ready_check(),
            last_error: self
                .last_error
                .lock()
                .expect("health lock poisoned")
                .clone(),
        }
    }

    fn update_last_check(&self) {
        if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
            self.last_ready_check
                .store(duration.as_secs(), Ordering::SeqCst);
        }
    }

    fn last_ready_check(&self) -> Option<u64> {
        match self.last_ready_check.load(Ordering::SeqCst) {
            0 => None,
            value => Some(value),
        }
    }
}

pub(crate) struct ServeHealthSnapshot {
    pub(crate) ready: bool,
    pub(crate) live: bool,
    pub(crate) last_ready_check: Option<u64>,
    pub(crate) last_error: Option<String>,
}
