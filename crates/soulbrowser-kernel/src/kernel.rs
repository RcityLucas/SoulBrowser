use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::{middleware, Router};
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::app_context::{create_context_with_provider, AppContext};
use crate::gateway::GatewayOptions;
use crate::gateway_policy::{gateway_auth_middleware, gateway_ip_middleware, GatewayPolicy};
use crate::integration::{default_provider, IntegrationProvider};
use crate::runtime::{RuntimeHandle, RuntimeOptions};
use crate::server::{build_api_router_with_modules, console_shell_router, ServeSurfacePreset};
use crate::Config;

/// Reusable kernel facade that owns shared configuration/state wiring.
pub struct Kernel {
    config: Arc<Config>,
    integration: Arc<dyn IntegrationProvider>,
}

impl Kernel {
    /// Construct a kernel instance from an owned configuration snapshot.
    pub fn new(config: Config) -> Self {
        Self::with_integration(config, default_provider())
    }

    /// Construct a kernel with an explicit integration provider.
    pub fn with_integration(config: Config, integration: Arc<dyn IntegrationProvider>) -> Self {
        Self {
            config: Arc::new(config),
            integration,
        }
    }

    /// Construct a kernel instance from a shared configuration reference.
    pub fn from_shared(config: Arc<Config>) -> Self {
        Self::from_shared_with_integration(config, default_provider())
    }

    /// Construct from shared configuration and custom integration provider.
    pub fn from_shared_with_integration(
        config: Arc<Config>,
        integration: Arc<dyn IntegrationProvider>,
    ) -> Self {
        Self {
            config,
            integration,
        }
    }

    /// Expose the underlying configuration for read-only access.
    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config)
    }

    /// Serve entry point used by the CLI, web-console launcher, and tests.
    pub async fn serve(&self, options: ServeOptions) -> Result<()> {
        let runtime = self.start_runtime(RuntimeOptions::from(&options)).await?;
        self.launch_runtime(runtime, &options).await
    }

    async fn launch_runtime(&self, runtime: RuntimeHandle, options: &ServeOptions) -> Result<()> {
        let auth_policy = build_serve_auth_policy(options)?;
        Self::log_auth_policy(&auth_policy);

        let modules = options.surface.modules();
        let api_router = build_api_router_with_modules(modules).with_state(runtime.state.clone());
        let shell_router = if options.surface.includes_console_shell() {
            Some(console_shell_router().with_state(runtime.state.clone()))
        } else {
            None
        };
        let router = Self::compose_router(shell_router, api_router, auth_policy);

        self.start_http(
            router,
            runtime.websocket_url.clone(),
            options.host,
            options.port,
        )
        .await
    }

    async fn start_http(
        &self,
        router: Router,
        websocket_url: Option<String>,
        host: IpAddr,
        port: u16,
    ) -> Result<()> {
        let addr = SocketAddr::new(host, port);
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("failed to bind testing server on {}", addr))?;
        info!("Testing console available at http://{}:{}", addr.ip(), port);
        if host.is_loopback() {
            info!("Access from Windows: http://localhost:{}", port);
        } else if host.is_unspecified() {
            info!(
                "Listening on all interfaces; try http://127.0.0.1:{} locally",
                port
            );
        }
        if let Some(ws) = websocket_url.as_deref() {
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
        .context("testing server exited unexpectedly")
    }

    /// Gateway entry point wiring the L7 adapter stack.
    pub async fn gateway(&self, options: GatewayOptions) -> Result<()> {
        crate::gateway::run_gateway(self, options).await
    }

    fn compose_router(
        shell_router: Option<Router>,
        api_router: Router,
        auth_policy: Option<Arc<GatewayPolicy>>,
    ) -> Router {
        if let Some(policy) = auth_policy {
            let mut combined = Router::new();
            if let Some(shell) = shell_router {
                let shell = shell.layer(middleware::from_fn_with_state(
                    Arc::clone(&policy),
                    gateway_ip_middleware,
                ));
                combined = combined.merge(shell);
            }
            let api = api_router.layer(middleware::from_fn_with_state(
                policy,
                gateway_auth_middleware,
            ));
            combined.merge(api)
        } else {
            let mut combined = Router::new().merge(api_router);
            if let Some(shell) = shell_router {
                combined = combined.merge(shell);
            }
            combined
        }
    }

    fn log_auth_policy(auth_policy: &Option<Arc<GatewayPolicy>>) {
        if auth_policy.is_some() && env::var("SOUL_STRICT_AUTHZ").is_err() {
            env::set_var("SOUL_STRICT_AUTHZ", "true");
            info!("Serve strict authorization enforced (SOUL_STRICT_AUTHZ=true)");
        }
        if let Some(policy) = auth_policy {
            info!(
                tokens = policy.allowed_tokens.len(),
                ips = policy.ip_whitelist.len(),
                "Serve auth guard enabled"
            );
        } else {
            warn!("Serve auth disabled; do not expose this port publicly");
        }
    }
}

/// Options accepted by the kernel Serve entrypoint.
#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub host: IpAddr,
    pub port: u16,
    pub websocket_url: Option<String>,
    pub tenant: String,
    pub llm_cache_dir: Option<PathBuf>,
    pub shared_session_override: Option<bool>,
    pub auth_tokens: Vec<String>,
    pub allow_ips: Vec<String>,
    pub disable_auth: bool,
    pub surface: ServeSurfacePreset,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            host: IpAddr::from([127, 0, 0, 1]),
            port: 0,
            websocket_url: None,
            tenant: String::new(),
            llm_cache_dir: None,
            shared_session_override: None,
            auth_tokens: Vec::new(),
            allow_ips: Vec::new(),
            disable_auth: false,
            surface: ServeSurfacePreset::Console,
        }
    }
}

fn build_serve_auth_policy(options: &ServeOptions) -> Result<Option<Arc<GatewayPolicy>>> {
    if options.disable_auth {
        warn!("Serve auth disabled via --disable-auth");
        return Ok(None);
    }

    let mut tokens: Vec<String> = options
        .auth_tokens
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

    let ip_strings = if options.allow_ips.is_empty() {
        default_allow_ips()
    } else {
        options.allow_ips.clone()
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
    if env::var("WSL_DISTRO_NAME").is_err() && env::var("WSL_INTEROP").is_err() {
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

pub(crate) fn normalize_tenant_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut previous_dash = false;
    for ch in trimmed.chars() {
        let mapped = match ch {
            'a'..='z' | '0'..='9' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            '-' | '_' => '-',
            _ => '-',
        };

        let lower = mapped.to_ascii_lowercase();
        if lower == '-' {
            if previous_dash {
                continue;
            }
            previous_dash = true;
            normalized.push('-');
        } else {
            previous_dash = false;
            normalized.push(lower);
        }
    }

    let normalized = normalized.trim_matches('-').to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

impl Kernel {
    /// Construct (or reuse) an AppContext for a given tenant/storage root using
    /// the kernel's configuration and policy paths.
    pub async fn build_app_context(
        &self,
        tenant_id: String,
        storage_root: Option<PathBuf>,
    ) -> Result<Arc<AppContext>> {
        let config = self.config();
        let context = create_context_with_provider(
            tenant_id,
            storage_root,
            config.policy_paths.clone(),
            Arc::clone(&self.integration),
        )
        .await
        .map_err(|err| anyhow!(err.to_string()))?;
        Ok(context)
    }
}
