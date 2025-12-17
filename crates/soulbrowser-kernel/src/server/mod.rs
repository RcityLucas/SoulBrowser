mod rate_limit;
mod router;
mod state;

use std::path::{Path, PathBuf};

pub use rate_limit::{RateLimitConfig, RateLimiter};
pub use router::{
    build_api_router_with_modules, build_console_router, console_shell_router, ServeRouterModules,
    ServeSurfacePreset,
};
pub use state::{ServeHealth, ServeState};

pub fn tenant_storage_path(base: &Path, tenant: &str) -> PathBuf {
    base.join("tenants").join(tenant)
}
