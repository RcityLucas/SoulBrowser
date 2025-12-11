use std::env;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use axum::{middleware, Router};
use chrono::Duration as ChronoDuration;
use clap::Args;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{interval, timeout};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::app_context::get_or_create_context;
use crate::ensure_real_chrome_enabled;
use crate::normalize_tenant_id;
use crate::perception_service::PerceptionService;
use crate::server::{
    build_api_router_with_modules, console_shell_router, tenant_storage_path, RateLimitConfig,
    RateLimiter, ServeHealth, ServeRouterModules, ServeState,
};
use crate::task_store::{prune_execution_outputs, TaskPlanStore};
use crate::Config;
use crate::{
    build_llm_cache_pool, resolve_llm_cache_dir, GatewayPolicy, RATE_LIMIT_CHAT_ENV,
    RATE_LIMIT_TASK_ENV,
};
use crate::{gateway_auth_middleware, gateway_ip_middleware};

const SERVE_WS_URL_ENV: &str = "SOUL_SERVE_WS_URL";
const DISABLE_POOL_ENV: &str = "SOULBROWSER_DISABLE_PERCEPTION_POOL";

#[derive(Args, Clone)]
pub struct ServeArgs {
    /// Port for the testing server
    #[arg(long, default_value_t = 8787)]
    pub port: u16,

    /// Attach to an existing Chrome DevTools websocket (optional)
    #[arg(long)]
    pub ws_url: Option<String>,

    /// Logical tenant identifier for shared services and caches
    #[arg(long, default_value = "serve-api")]
    pub tenant: String,

    /// Directory for caching LLM planner outputs
    #[arg(long = "llm-cache-dir")]
    pub llm_cache_dir: Option<std::path::PathBuf>,

    /// Force enable/disable shared perception session pooling (default: config/env)
    #[arg(long = "shared-session", value_name = "true|false")]
    pub shared_session: Option<bool>,

    /// Provide an API token allowed to access the Serve endpoints (repeat for multiple tokens)
    #[arg(long = "auth-token", value_name = "TOKEN")]
    pub auth_token: Vec<String>,

    /// Allow requests from the specified IP address (repeat). Defaults to localhost only.
    #[arg(long = "allow-ip", value_name = "IP")]
    pub allow_ip: Vec<String>,

    /// Disable Serve authentication and IP checks (unsafe; local testing only)
    #[arg(long = "disable-auth")]
    pub disable_auth: bool,
}

pub async fn cmd_serve(args: ServeArgs, _metrics_port: u16, config: Config) -> Result<()> {
    let shared_session_override = args.shared_session;
    apply_shared_session_override(shared_session_override);
    let ws_url = resolve_ws_url(args.ws_url.clone(), None);
    let llm_cache_dir = args.llm_cache_dir.clone();
    let llm_cache = build_llm_cache_pool(resolve_llm_cache_dir(llm_cache_dir))?;
    let rate_limit_config =
        RateLimitConfig::from_env(RATE_LIMIT_CHAT_ENV, RATE_LIMIT_TASK_ENV, 30, 15);
    let rate_limiter = Arc::new(RateLimiter::new(rate_limit_config));
    spawn_rate_limit_cleanup(Arc::clone(&rate_limiter));
    let config = Arc::new(config);
    let auth_policy = build_serve_auth_policy(&args)?;
    let chat_context_limit = resolve_chat_context_limit();
    let chat_context_wait = resolve_chat_context_wait_timeout();
    let chat_context_semaphore = Arc::new(tokio::sync::Semaphore::new(chat_context_limit));
    info!(
        limit = chat_context_limit,
        wait_ms = chat_context_wait.map(|dur| dur.as_millis() as u64),
        "Chat context snapshot concurrency limit active"
    );

    let tenant_id = normalize_tenant_id(&args.tenant).unwrap_or_else(|| "serve-api".to_string());
    if tenant_id != args.tenant {
        info!(requested = %args.tenant, normalized = %tenant_id, "Serve tenant normalized");
    }
    let tenant_storage_root = tenant_storage_path(&config.output_dir, &tenant_id);
    std::fs::create_dir_all(&tenant_storage_root).with_context(|| {
        format!(
            "failed to prepare tenant directory {}",
            tenant_storage_root.display()
        )
    })?;

    if let Some(ttl) = resolve_plan_ttl_duration() {
        let plan_store = TaskPlanStore::new(tenant_storage_root.clone());
        match plan_store.prune_expired(ttl).await {
            Ok(removed) => {
                if removed > 0 {
                    info!(
                        removed,
                        ttl_days = ttl.num_days(),
                        "pruned expired task plans"
                    );
                }
            }
            Err(err) => warn!(?err, "failed to prune expired task plans"),
        }
    }

    if let Some(ttl) = resolve_output_ttl_duration() {
        match prune_execution_outputs(&config.output_dir, ttl).await {
            Ok(removed) => {
                if removed > 0 {
                    info!(
                        removed,
                        ttl_days = ttl.num_days(),
                        root = %config.output_dir.display(),
                        "pruned expired execution bundles"
                    );
                }
            }
            Err(err) => warn!(?err, "failed to prune expired execution bundles"),
        }
    }

    let app_context = get_or_create_context(
        tenant_id.clone(),
        Some(tenant_storage_root.clone()),
        config.policy_paths.clone(),
    )
    .await?;
    let app_context = Arc::new(tokio::sync::RwLock::new(app_context));
    let perception_service = Arc::new(PerceptionService::new());
    let health = Arc::new(ServeHealth::new());
    let state = ServeState::new(
        ws_url.clone(),
        Arc::clone(&config),
        perception_service,
        llm_cache,
        rate_limiter,
        app_context,
        Arc::clone(&health),
        chat_context_limit,
        chat_context_wait,
        Arc::clone(&chat_context_semaphore),
        tenant_id,
        tenant_storage_root,
    );

    if auth_policy.is_some() && env::var("SOUL_STRICT_AUTHZ").is_err() {
        env::set_var("SOUL_STRICT_AUTHZ", "true");
        info!("Serve strict authorization enforced (SOUL_STRICT_AUTHZ=true)");
    }
    if let Some(policy) = auth_policy.as_ref() {
        info!(
            tokens = policy.allowed_tokens.len(),
            ips = policy.ip_whitelist.len(),
            "Serve auth guard enabled"
        );
    } else {
        warn!("Serve auth disabled; do not expose this port publicly");
    }

    state.mark_live();
    match run_startup_readiness_checks(&state).await {
        Ok(()) => {
            state.mark_ready();
            info!("Serve readiness checks passed");
        }
        Err(err) => {
            state.mark_unready(err.to_string());
            error!(?err, "Serve readiness checks failed");
        }
    }

    let shell_router = console_shell_router().with_state(state.clone());
    let api_router = build_api_router_with_modules(ServeRouterModules::all()).with_state(state);

    let mut router = if let Some(policy) = auth_policy.clone() {
        let shell = shell_router.layer(middleware::from_fn_with_state(
            Arc::clone(&policy),
            gateway_ip_middleware,
        ));
        let api = api_router.layer(middleware::from_fn_with_state(
            policy,
            gateway_auth_middleware,
        ));
        Router::new().merge(shell).merge(api)
    } else {
        Router::new().merge(shell_router).merge(api_router)
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind testing server on {}", addr))?;
    info!(
        "Testing console available at http://127.0.0.1:{}",
        args.port
    );
    info!("Access from Windows: http://localhost:{}", args.port);
    if let Some(ws) = ws_url.as_deref() {
        info!("Using external DevTools endpoint: {}", ws);
    } else {
        info!("Using local Chrome detection (SOULBROWSER_CHROME / auto-detect)");
    }

    info!("Server starting, waiting for requests...");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("testing server exited unexpectedly")?;
    Ok(())
}

fn resolve_chat_context_limit() -> usize {
    env::var("SOUL_CHAT_CONTEXT_LIMIT")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(2)
}

fn resolve_chat_context_wait_timeout() -> Option<std::time::Duration> {
    match env::var("SOUL_CHAT_CONTEXT_WAIT_MS") {
        Ok(raw) => raw
            .trim()
            .parse::<u64>()
            .ok()
            .map(std::time::Duration::from_millis)
            .filter(|dur| !dur.is_zero()),
        Err(_) => Some(std::time::Duration::from_millis(750)),
    }
}

fn resolve_ws_url(cli_value: Option<String>, config_value: Option<&str>) -> Option<String> {
    cli_value
        .and_then(|value| normalize_ws_value(&value))
        .or_else(|| config_value.and_then(normalize_ws_value))
        .or_else(|| {
            env::var(SERVE_WS_URL_ENV)
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

fn apply_shared_session_override(flag: Option<bool>) {
    match flag {
        Some(true) => env::remove_var(DISABLE_POOL_ENV),
        Some(false) => env::set_var(DISABLE_POOL_ENV, "1"),
        None => {}
    }
}

fn resolve_plan_ttl_duration() -> Option<ChronoDuration> {
    match env::var("SOUL_PLAN_TTL_DAYS") {
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(days) if days > 0 => Some(ChronoDuration::days(days)),
            Ok(_) => None,
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_PLAN_TTL_DAYS value");
                None
            }
        },
        Err(env::VarError::NotPresent) => Some(ChronoDuration::days(30)),
        Err(err) => {
            warn!(?err, "failed to read SOUL_PLAN_TTL_DAYS env");
            None
        }
    }
}

fn resolve_output_ttl_duration() -> Option<ChronoDuration> {
    match env::var("SOUL_OUTPUT_TTL_DAYS") {
        Ok(raw) => match raw.trim().parse::<i64>() {
            Ok(days) if days > 0 => Some(ChronoDuration::days(days)),
            Ok(_) => None,
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_OUTPUT_TTL_DAYS value");
                None
            }
        },
        Err(env::VarError::NotPresent) => Some(ChronoDuration::days(30)),
        Err(err) => {
            warn!(?err, "failed to read SOUL_OUTPUT_TTL_DAYS env");
            None
        }
    }
}

fn spawn_rate_limit_cleanup(rate_limiter: Arc<RateLimiter>) {
    let ttl = resolve_rate_limit_bucket_ttl();
    if ttl.is_zero() {
        info!("Rate limiter bucket GC disabled (ttl=0)");
        return;
    }
    let gc_interval = resolve_rate_limit_gc_interval();
    info!(
        ttl_secs = ttl.as_secs(),
        interval_secs = gc_interval.as_secs(),
        "Rate limiter GC enabled"
    );
    tokio::spawn(async move {
        let mut ticker = interval(gc_interval);
        loop {
            ticker.tick().await;
            let removed = rate_limiter.prune_idle(ttl);
            if removed > 0 {
                debug!(removed, "Pruned stale rate limit buckets");
            }
        }
    });
}

fn resolve_rate_limit_bucket_ttl() -> Duration {
    match env::var("SOUL_RATE_LIMIT_BUCKET_TTL_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => Duration::from_secs(0),
            Ok(secs) => Duration::from_secs(secs),
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_RATE_LIMIT_BUCKET_TTL_SECS");
                Duration::from_secs(600)
            }
        },
        Err(env::VarError::NotPresent) => Duration::from_secs(600),
        Err(err) => {
            warn!(?err, "failed to read SOUL_RATE_LIMIT_BUCKET_TTL_SECS");
            Duration::from_secs(600)
        }
    }
}

fn resolve_rate_limit_gc_interval() -> Duration {
    match env::var("SOUL_RATE_LIMIT_GC_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => Duration::from_secs(30),
            Ok(secs) => Duration::from_secs(secs.max(5)),
            Err(err) => {
                warn!(?err, value = raw, "invalid SOUL_RATE_LIMIT_GC_SECS");
                Duration::from_secs(60)
            }
        },
        Err(env::VarError::NotPresent) => Duration::from_secs(60),
        Err(err) => {
            warn!(?err, "failed to read SOUL_RATE_LIMIT_GC_SECS");
            Duration::from_secs(60)
        }
    }
}

fn build_serve_auth_policy(args: &ServeArgs) -> Result<Option<Arc<GatewayPolicy>>> {
    if args.disable_auth {
        warn!("Serve auth disabled via --disable-auth");
        return Ok(None);
    }

    let mut tokens: Vec<String> = args
        .auth_token
        .iter()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .collect();
    for env_var in ["SOUL_CONSOLE_TOKEN", "SOUL_SERVE_TOKEN"] {
        if let Ok(value) = env::var(env_var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                tokens.push(trimmed.to_string());
            }
        }
    }
    if tokens.is_empty() {
        warn!("Serve auth disabled; no auth tokens provided");
        return Ok(None);
    }

    let ip_strings = if args.allow_ip.is_empty() {
        default_allow_ips()
    } else {
        args.allow_ip.clone()
    };
    let mut ip_allowlist = Vec::new();
    for entry in ip_strings {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let ip: IpAddr = trimmed
            .parse()
            .with_context(|| format!("invalid --allow-ip value '{trimmed}'"))?;
        ip_allowlist.push(ip);
    }
    Ok(Some(Arc::new(GatewayPolicy::from_tokens_and_ips(
        tokens,
        ip_allowlist,
    ))))
}

fn default_allow_ips() -> Vec<String> {
    let mut ips = vec!["127.0.0.1".to_string(), "::1".to_string()];
    if let Some(host_ip) = detect_windows_host_ip() {
        ips.push(host_ip.to_string());
    }
    ips
}

fn detect_windows_host_ip() -> Option<IpAddr> {
    if std::env::var("WSL_DISTRO_NAME").is_err() && std::env::var("WSL_INTEROP").is_err() {
        return None;
    }

    let contents = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("nameserver") {
            let candidate = rest.trim();
            if candidate.is_empty() {
                continue;
            }
            if let Ok(ip) = candidate.parse::<IpAddr>() {
                if !ip.is_loopback() {
                    info!(%ip, "Detected Windows host IP for Serve allowlist");
                    return Some(ip);
                }
            }
        }
    }
    None
}

async fn run_startup_readiness_checks(state: &ServeState) -> Result<()> {
    if let Some(ws_url) = &state.ws_url {
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
