use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsPolicyView {
    pub enable_metrics: bool,
    pub enable_tracing: bool,
    pub tracing_sample_default: f32,
    pub tracing_sample_on_fail: f32,
    pub tracing_sample_on_slow: f32,
    pub slow_threshold_ms: u64,
    pub prom_enable: bool,
    pub prom_bind: String,
    pub otlp_enable: bool,
    pub otlp_endpoint: String,
    pub otlp_timeout_ms: u64,
    pub log_enable: bool,
    pub log_path: String,
    pub log_rotate_bytes: u64,
    pub log_rotate_minutes: u32,
    pub log_keep: u32,
    pub log_compress: bool,
    pub cpu_budget_pct: u8,
    pub series_limit: usize,
    pub span_attr_limit: usize,
    pub payload_bytes_limit: usize,
    pub pii_guard: bool,
    pub allow_origin_full: bool,
}

impl Default for ObsPolicyView {
    fn default() -> Self {
        Self {
            enable_metrics: true,
            enable_tracing: true,
            tracing_sample_default: 0.1,
            tracing_sample_on_fail: 1.0,
            tracing_sample_on_slow: 1.0,
            slow_threshold_ms: 1500,
            prom_enable: true,
            prom_bind: "0.0.0.0:9090".into(),
            otlp_enable: false,
            otlp_endpoint: "http://127.0.0.1:4317".into(),
            otlp_timeout_ms: 3_000,
            log_enable: true,
            log_path: "./logs/observe.jsonl".into(),
            log_rotate_bytes: 256 << 20,
            log_rotate_minutes: 30,
            log_keep: 4,
            log_compress: true,
            cpu_budget_pct: 2,
            series_limit: 2_048,
            span_attr_limit: 32,
            payload_bytes_limit: 32 << 10,
            pii_guard: true,
            allow_origin_full: false,
        }
    }
}

static GLOBAL_POLICY: OnceCell<Arc<RwLock<ObsPolicyView>>> = OnceCell::new();

#[derive(Clone)]
pub struct PolicyHandle {
    inner: Arc<RwLock<ObsPolicyView>>,
}

impl PolicyHandle {
    pub fn get() -> Self {
        let cell = GLOBAL_POLICY.get_or_init(|| Arc::new(RwLock::new(ObsPolicyView::default())));
        Self {
            inner: Arc::clone(cell),
        }
    }

    pub fn snapshot(&self) -> ObsPolicyView {
        self.inner.read().clone()
    }

    pub fn update(&self, view: ObsPolicyView) {
        *self.inner.write() = view;
    }
}

pub fn set_policy(view: ObsPolicyView) {
    PolicyHandle::get().update(view);
}

pub fn current_policy() -> ObsPolicyView {
    PolicyHandle::get().snapshot()
}
