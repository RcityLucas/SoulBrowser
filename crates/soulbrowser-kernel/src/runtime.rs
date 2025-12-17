use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use url::Url;

use crate::llm::LlmCachePool;
use crate::perception_service::PerceptionService;
use crate::server::{tenant_storage_path, RateLimitConfig, RateLimiter, ServeHealth, ServeState};
use crate::task_store::{prune_execution_outputs, TaskPlanStore};
use crate::utils::ensure_real_chrome_enabled;
use crate::{Config, Kernel, ServeOptions};

#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    pub tenant: String,
    pub websocket_url: Option<String>,
    pub llm_cache_dir: Option<PathBuf>,
    pub shared_session_override: Option<bool>,
}

impl From<&ServeOptions> for RuntimeOptions {
    fn from(value: &ServeOptions) -> Self {
        Self {
            tenant: value.tenant.clone(),
            websocket_url: value.websocket_url.clone(),
            llm_cache_dir: value.llm_cache_dir.clone(),
            shared_session_override: value.shared_session_override,
        }
    }
}

pub struct RuntimeHandle {
    pub state: ServeState,
    pub websocket_url: Option<String>,
    pub tenant_id: String,
    cleanup_task: JoinHandle<()>,
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.cleanup_task.abort();
    }
}

impl Kernel {
    pub async fn start_runtime(&self, options: RuntimeOptions) -> Result<RuntimeHandle> {
        apply_shared_session_override(options.shared_session_override);
        let websocket_url = resolve_ws_url(options.websocket_url.clone(), None);
        let llm_cache = build_llm_cache_pool(resolve_llm_cache_dir(options.llm_cache_dir))?;
        let rate_guards = build_rate_limit_guards();
        let tenant_id = self.normalize_or_log_tenant_internal(&options.tenant);
        let config = self.config();
        let tenant_storage_root = self.prepare_tenant_storage(&tenant_id, &config).await?;

        let tenant_storage = Some(tenant_storage_root.clone());
        let raw_context = self
            .build_app_context(tenant_id.clone(), tenant_storage)
            .await?;
        let perception_service = Arc::new(PerceptionService::with_app_context(&raw_context));
        let app_context = Arc::new(tokio::sync::RwLock::new(raw_context));
        let health = Arc::new(ServeHealth::new());
        let state = ServeState::new(
            websocket_url.clone(),
            Arc::clone(&config),
            perception_service,
            llm_cache,
            Arc::clone(&rate_guards.limiter),
            app_context,
            Arc::clone(&health),
            rate_guards.wait,
            Arc::clone(&rate_guards.semaphore),
            tenant_id.clone(),
            tenant_storage_root,
        );

        let cleanup_task = spawn_rate_limit_cleanup(Arc::clone(&rate_guards.limiter));
        state.mark_live();
        run_startup_readiness_checks(&state).await?;
        state.mark_ready();

        Ok(RuntimeHandle {
            state,
            websocket_url,
            tenant_id,
            cleanup_task,
        })
    }

    pub(crate) async fn prepare_tenant_storage(
        &self,
        tenant_id: &str,
        config: &Arc<Config>,
    ) -> Result<PathBuf> {
        let tenant_storage_root = tenant_storage_path(&config.output_dir, tenant_id);
        fs::create_dir_all(&tenant_storage_root).with_context(|| {
            format!(
                "failed to prepare tenant directory {}",
                tenant_storage_root.display()
            )
        })?;
        prune_plan_store(&tenant_storage_root).await;
        prune_output_dir(&config.output_dir).await;
        Ok(tenant_storage_root)
    }

    fn normalize_or_log_tenant_internal(&self, tenant: &str) -> String {
        let normalized =
            crate::kernel::normalize_tenant_id(tenant).unwrap_or_else(|| "serve-api".into());
        if normalized != tenant {
            info!(requested = %tenant, normalized = %normalized, "Serve tenant normalized");
        }
        normalized
    }
}

struct RateLimitGuards {
    limiter: Arc<RateLimiter>,
    semaphore: Arc<Semaphore>,
    wait: Option<Duration>,
}

impl RateLimitGuards {
    fn new(limit: usize, wait: Option<Duration>, config: RateLimitConfig) -> Self {
        Self {
            limiter: Arc::new(RateLimiter::new(config)),
            semaphore: Arc::new(Semaphore::new(limit)),
            wait,
        }
    }
}

fn build_rate_limit_guards() -> RateLimitGuards {
    let chat_context_limit = resolve_chat_context_limit();
    let chat_context_wait = resolve_chat_context_wait_timeout();
    let rate_limit_config = RateLimitConfig::from_env(
        "SOUL_RATE_LIMIT_CHAT_PER_MIN",
        "SOUL_RATE_LIMIT_TASK_PER_MIN",
        30,
        15,
    );
    let guards = RateLimitGuards::new(chat_context_limit, chat_context_wait, rate_limit_config);
    info!(
        limit = chat_context_limit,
        wait_ms = chat_context_wait.map(|dur| dur.as_millis() as u64),
        "Chat context snapshot concurrency limit active"
    );
    guards
}

fn apply_shared_session_override(flag: Option<bool>) {
    match flag {
        Some(true) => std::env::remove_var("SOULBROWSER_DISABLE_PERCEPTION_POOL"),
        Some(false) => std::env::set_var("SOULBROWSER_DISABLE_PERCEPTION_POOL", "1"),
        None => {}
    }
}

fn resolve_chat_context_limit() -> usize {
    std::env::var("SOUL_CHAT_CONTEXT_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(2)
}

fn resolve_chat_context_wait_timeout() -> Option<Duration> {
    match std::env::var("SOUL_CHAT_CONTEXT_WAIT_MS") {
        Ok(raw) => raw
            .trim()
            .parse::<u64>()
            .ok()
            .map(Duration::from_millis)
            .filter(|dur| !dur.is_zero()),
        Err(_) => Some(Duration::from_millis(750)),
    }
}

fn resolve_llm_cache_dir(cli_override: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = cli_override {
        return Some(path);
    }

    if let Ok(env_path) = std::env::var("SOULBROWSER_LLM_CACHE_DIR") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    dirs::cache_dir()
        .or_else(|| dirs::home_dir())
        .map(|dir| dir.join("soulbrowser").join("llm-cache"))
}

fn build_llm_cache_pool(root: Option<PathBuf>) -> Result<Option<Arc<LlmCachePool>>> {
    let Some(path) = root else {
        return Ok(None);
    };

    info!(path = %path.display(), "LLM cache directory prepared");
    let pool = LlmCachePool::new(path)?;
    Ok(Some(Arc::new(pool)))
}

fn resolve_ws_url(cli_value: Option<String>, config_value: Option<&str>) -> Option<String> {
    cli_value
        .and_then(|value| normalize_ws_value(&value))
        .or_else(|| config_value.and_then(normalize_ws_value))
        .or_else(|| {
            std::env::var("SOUL_SERVE_WS_URL")
                .ok()
                .and_then(|value| normalize_ws_value(&value))
        })
}

fn normalize_ws_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn spawn_rate_limit_cleanup(rate_limiter: Arc<RateLimiter>) -> JoinHandle<()> {
    let ttl = resolve_rate_limit_bucket_ttl();
    if ttl.is_zero() {
        info!("Rate limiter bucket GC disabled (ttl=0)");
        return tokio::spawn(async {});
    }
    let gc_interval = resolve_rate_limit_gc_interval();
    info!(
        ttl_secs = ttl.as_secs(),
        interval_secs = gc_interval.as_secs(),
        "Rate limiter GC enabled"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(gc_interval);
        loop {
            ticker.tick().await;
            let removed = rate_limiter.prune_idle(ttl);
            if removed > 0 {
                debug!(removed, "Pruned stale rate limit buckets");
            }
        }
    })
}

async fn prune_plan_store(root: &PathBuf) {
    if let Some(ttl) = resolve_plan_ttl_duration() {
        let plan_store = TaskPlanStore::new(root.clone());
        match plan_store.prune_expired(ttl).await {
            Ok(removed) if removed > 0 => {
                info!(
                    removed,
                    ttl_days = ttl.num_days(),
                    "pruned expired task plans"
                );
            }
            Ok(_) => {}
            Err(err) => warn!(?err, "failed to prune expired task plans"),
        }
    }
}

async fn prune_output_dir(output_dir: &PathBuf) {
    if let Some(ttl) = resolve_output_ttl_duration() {
        match prune_execution_outputs(output_dir, ttl).await {
            Ok(removed) if removed > 0 => {
                info!(
                    removed,
                    ttl_days = ttl.num_days(),
                    root = %output_dir.display(),
                    "pruned expired execution bundles"
                );
            }
            Ok(_) => {}
            Err(err) => warn!(?err, "failed to prune expired execution bundles"),
        }
    }
}

fn resolve_plan_ttl_duration() -> Option<chrono::Duration> {
    match std::env::var("SOUL_PLAN_TTL_DAYS") {
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(days) if days > 0 => Some(chrono::Duration::days(days)),
            Ok(_) => None,
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_PLAN_TTL_DAYS value");
                None
            }
        },
        Err(std::env::VarError::NotPresent) => Some(chrono::Duration::days(30)),
        Err(err) => {
            warn!(?err, "failed to read SOUL_PLAN_TTL_DAYS env");
            None
        }
    }
}

fn resolve_output_ttl_duration() -> Option<chrono::Duration> {
    match std::env::var("SOUL_OUTPUT_TTL_DAYS") {
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(days) if days > 0 => Some(chrono::Duration::days(days)),
            Ok(_) => None,
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_OUTPUT_TTL_DAYS value");
                None
            }
        },
        Err(std::env::VarError::NotPresent) => Some(chrono::Duration::days(30)),
        Err(err) => {
            warn!(?err, "failed to read SOUL_OUTPUT_TTL_DAYS env");
            None
        }
    }
}

fn resolve_rate_limit_bucket_ttl() -> Duration {
    match std::env::var("SOUL_RATE_LIMIT_BUCKET_TTL_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => Duration::from_secs(0),
            Ok(secs) => Duration::from_secs(secs),
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_RATE_LIMIT_BUCKET_TTL_SECS");
                Duration::from_secs(600)
            }
        },
        Err(std::env::VarError::NotPresent) => Duration::from_secs(600),
        Err(err) => {
            warn!(?err, "failed to read SOUL_RATE_LIMIT_BUCKET_TTL_SECS");
            Duration::from_secs(600)
        }
    }
}

fn resolve_rate_limit_gc_interval() -> Duration {
    match std::env::var("SOUL_RATE_LIMIT_GC_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => Duration::from_secs(30),
            Ok(secs) => Duration::from_secs(secs.max(5)),
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_RATE_LIMIT_GC_SECS");
                Duration::from_secs(60)
            }
        },
        Err(std::env::VarError::NotPresent) => Duration::from_secs(60),
        Err(err) => {
            warn!(?err, "failed to read SOUL_RATE_LIMIT_GC_SECS");
            Duration::from_secs(60)
        }
    }
}

pub async fn run_startup_readiness_checks(state: &ServeState) -> Result<()> {
    if let Some(ws_url) = state.websocket_url() {
        probe_devtools_socket(ws_url).await
    } else {
        ensure_real_chrome_enabled()
    }
}

async fn probe_devtools_socket(ws_url: &str) -> Result<()> {
    let url = Url::parse(ws_url).context("parsing DevTools websocket URL")?;
    match url.scheme() {
        "ws" | "wss" => {}
        scheme => {
            bail!("DevTools websocket URL must start with ws:// or wss:// (got {scheme})");
        }
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("DevTools websocket URL missing host: {ws_url}"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("DevTools websocket URL missing port: {ws_url}"))?;
    let addr = format!("{host}:{port}");
    let connect = TcpStream::connect(&addr);
    match timeout(Duration::from_secs(5), connect).await {
        Ok(Ok(_stream)) => Ok(()),
        Ok(Err(err)) => Err(anyhow!(
            "failed to connect to DevTools websocket {}: {}",
            addr,
            err
        )),
        Err(_) => Err(anyhow!("timeout while probing DevTools websocket {addr}")),
    }
}
