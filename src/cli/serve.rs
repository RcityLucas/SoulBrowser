use std::env;
use std::str::FromStr;

use anyhow::Result;
use clap::{Args, ValueEnum};
use tracing::warn;

use soulbrowser_kernel::{Config, Kernel, ServeOptions, ServeSurfacePreset};

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

    /// Limit the HTTP surface to a preset (console or gateway)
    #[arg(long = "surface", value_enum)]
    pub surface: Option<ServeSurfaceCli>,
}

pub async fn cmd_serve(args: ServeArgs, _metrics_port: u16, config: Config) -> Result<()> {
    let surface = resolve_surface(&args, &config);
    let options = ServeOptions {
        port: args.port,
        websocket_url: args.ws_url.clone(),
        tenant: args.tenant.clone(),
        llm_cache_dir: args.llm_cache_dir.clone(),
        shared_session_override: args.shared_session,
        auth_tokens: args.auth_token.clone(),
        allow_ips: args.allow_ip.clone(),
        disable_auth: args.disable_auth,
        surface,
    };

    Kernel::new(config).serve(options).await
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ServeSurfaceCli {
    Console,
    Gateway,
}

impl From<ServeSurfaceCli> for ServeSurfacePreset {
    fn from(value: ServeSurfaceCli) -> Self {
        match value {
            ServeSurfaceCli::Console => ServeSurfacePreset::Console,
            ServeSurfaceCli::Gateway => ServeSurfacePreset::Gateway,
        }
    }
}

fn resolve_surface(args: &ServeArgs, config: &Config) -> ServeSurfacePreset {
    if let Some(cli_surface) = args.surface {
        return cli_surface.into();
    }
    if let Some(config_surface) = config.serve_surface {
        return config_surface;
    }
    if let Ok(env_value) = env::var("SOUL_SERVE_SURFACE") {
        match ServeSurfacePreset::from_str(&env_value) {
            Ok(value) => return value,
            Err(err) => {
                warn!(value = env_value, %err, "invalid SOUL_SERVE_SURFACE; falling back to console")
            }
        }
    }
    ServeSurfacePreset::Console
}
