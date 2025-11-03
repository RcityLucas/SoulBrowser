use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Trust {
    Trusted,
    Internal,
    ThirdPartyLow,
}

impl Default for Trust {
    fn default() -> Self {
        Trust::ThirdPartyLow
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookAllow {
    pub plugin: String,
    pub hook: String,
    pub for_tools: Option<Vec<String>>,
    pub views: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TenantPolicy {
    pub id: String,
    pub enable: bool,
    pub allow_plugins: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPolicyView {
    pub enable: bool,
    pub tenants: Vec<TenantPolicy>,
    pub default_trust: Trust,
    pub allowed_extpoints: Vec<String>,
    pub kill_switch: Vec<String>,
    pub require_signature: bool,
    pub allowed_cas: Vec<String>,
    pub allow_runtime: Vec<String>,
    pub cpu_ms: u32,
    pub wall_ms: u32,
    pub mem_mb: u32,
    pub ipc_bytes: u32,
    pub kv_space_mb: u32,
    pub concurrency: u32,
    pub net_enable: bool,
    pub net_whitelist: Vec<String>,
    pub fs_enable: bool,
    pub hook_allow: Vec<HookAllow>,
}

impl Default for PluginPolicyView {
    fn default() -> Self {
        Self {
            enable: false,
            tenants: Vec::new(),
            default_trust: Trust::default(),
            allowed_extpoints: Vec::new(),
            kill_switch: Vec::new(),
            require_signature: true,
            allowed_cas: Vec::new(),
            allow_runtime: vec!["wasm32-wasi".into()],
            cpu_ms: 50,
            wall_ms: 100,
            mem_mb: 64,
            ipc_bytes: 32 * 1024,
            kv_space_mb: 4,
            concurrency: 1,
            net_enable: false,
            net_whitelist: Vec::new(),
            fs_enable: false,
            hook_allow: Vec::new(),
        }
    }
}

static GLOBAL_POLICY: OnceCell<Arc<RwLock<PluginPolicyView>>> = OnceCell::new();

#[derive(Clone)]
pub struct PluginPolicyHandle {
    inner: Arc<RwLock<PluginPolicyView>>,
}

impl PluginPolicyHandle {
    pub fn global() -> Self {
        let cell = GLOBAL_POLICY.get_or_init(|| Arc::new(RwLock::new(PluginPolicyView::default())));
        Self {
            inner: Arc::clone(cell),
        }
    }

    pub fn snapshot(&self) -> PluginPolicyView {
        self.inner.read().clone()
    }

    pub fn update(&self, view: PluginPolicyView) {
        *self.inner.write() = view;
    }
}

pub fn set_policy(view: PluginPolicyView) {
    PluginPolicyHandle::global().update(view);
}

pub fn current_policy() -> PluginPolicyView {
    PluginPolicyHandle::global().snapshot()
}
