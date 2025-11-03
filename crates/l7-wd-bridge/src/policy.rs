use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TenantPolicy {
    pub id: String,
    pub enable: bool,
    pub allow_endpoints: Vec<String>,
    pub allow_tools: Vec<String>,
    pub origins_allow: Vec<String>,
    pub concurrency_max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDriverBridgePolicy {
    pub enabled: bool,
    pub tenants: Vec<TenantPolicy>,
    pub allow_xpath: bool,
    pub screenshot_inline_allowed: bool,
    pub screenshot_bytes_max: usize,
    pub privacy_profile: Option<String>,
}

impl Default for WebDriverBridgePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            tenants: Vec::new(),
            allow_xpath: false,
            screenshot_inline_allowed: false,
            screenshot_bytes_max: 64 * 1024,
            privacy_profile: None,
        }
    }
}

static POLICY: OnceCell<Arc<RwLock<WebDriverBridgePolicy>>> = OnceCell::new();

#[derive(Clone)]
pub struct WebDriverBridgePolicyHandle {
    inner: Arc<RwLock<WebDriverBridgePolicy>>,
}

impl WebDriverBridgePolicyHandle {
    pub fn global() -> Self {
        let cell = POLICY.get_or_init(|| Arc::new(RwLock::new(WebDriverBridgePolicy::default())));
        Self {
            inner: Arc::clone(cell),
        }
    }

    pub fn snapshot(&self) -> WebDriverBridgePolicy {
        self.inner.read().clone()
    }

    pub fn update(&self, policy: WebDriverBridgePolicy) {
        *self.inner.write() = policy;
    }

    pub fn tenant(&self, tenant_id: &str) -> Option<TenantPolicy> {
        self.inner
            .read()
            .tenants
            .iter()
            .find(|tenant| tenant.id == tenant_id)
            .cloned()
    }
}

pub fn set_policy(policy: WebDriverBridgePolicy) {
    WebDriverBridgePolicyHandle::global().update(policy);
}

pub fn current_policy() -> WebDriverBridgePolicy {
    WebDriverBridgePolicyHandle::global().snapshot()
}
