use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdapterPolicyView {
    pub enabled: bool,
    pub tenants: Vec<TenantPolicy>,
    pub tls_required: bool,
    pub cors_enable: bool,
    pub privacy_profile: Option<String>,
    pub tracing_sample: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TenantPolicy {
    pub id: String,
    pub allow_tools: Vec<String>,
    pub allow_flows: Vec<String>,
    pub read_only: Vec<String>,
    pub rate_limit_rps: u32,
    pub rate_burst: u32,
    pub concurrency_max: u32,
    pub timeout_ms_tool: u64,
    pub timeout_ms_flow: u64,
    pub timeout_ms_read: u64,
    pub idempotency_window_sec: u64,
    pub allow_cold_export: bool,
    pub exports_max_lines: usize,
    pub authz_scopes: Vec<String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub hmac_secrets: Vec<String>,
}

impl AdapterPolicyView {
    pub fn tenant(&self, id: &str) -> Option<&TenantPolicy> {
        self.tenants.iter().find(|tenant| tenant.id == id)
    }
}

static GLOBAL_POLICY: OnceCell<Arc<RwLock<AdapterPolicyView>>> = OnceCell::new();

#[derive(Clone)]
pub struct AdapterPolicyHandle {
    inner: Arc<RwLock<AdapterPolicyView>>,
}

impl AdapterPolicyHandle {
    pub fn global() -> Self {
        let cell =
            GLOBAL_POLICY.get_or_init(|| Arc::new(RwLock::new(AdapterPolicyView::default())));
        Self {
            inner: Arc::clone(cell),
        }
    }

    pub fn snapshot(&self) -> AdapterPolicyView {
        self.inner.read().clone()
    }

    pub fn update(&self, view: AdapterPolicyView) {
        *self.inner.write() = view;
    }
}

pub fn set_policy(view: AdapterPolicyView) {
    AdapterPolicyHandle::global().update(view);
}

pub fn current_policy() -> AdapterPolicyView {
    AdapterPolicyHandle::global().snapshot()
}
