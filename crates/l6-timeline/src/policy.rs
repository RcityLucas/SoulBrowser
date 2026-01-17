use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelinePolicyView {
    pub max_time_range_ms: u64,
    pub allow_cold_export: bool,
    pub max_lines: usize,
    pub records_sample_rate: f32,
    pub fold_noise: bool,
    pub max_payload_bytes: usize,
    pub pii_guard: bool,
    pub allow_origin_full: bool,
    pub text_hash_len: usize,
    pub log_enable: bool,
    pub log_path: String,
    pub log_rotate_bytes: u64,
    pub log_rotate_minutes: u32,
    pub log_keep: u32,
    pub log_compress: bool,
}

impl Default for TimelinePolicyView {
    fn default() -> Self {
        Self {
            max_time_range_ms: 3_600_000,
            allow_cold_export: false,
            max_lines: 10_000,
            records_sample_rate: 1.0,
            fold_noise: true,
            max_payload_bytes: 16 * 1024,
            pii_guard: true,
            allow_origin_full: false,
            text_hash_len: 256,
            log_enable: true,
            log_path: "./exports/timeline.jsonl".to_string(),
            log_rotate_bytes: 256 * 1024 * 1024,
            log_rotate_minutes: 30,
            log_keep: 4,
            log_compress: true,
        }
    }
}

static GLOBAL_POLICY: OnceCell<Arc<RwLock<TimelinePolicyView>>> = OnceCell::new();

fn policy_cell() -> Arc<RwLock<TimelinePolicyView>> {
    GLOBAL_POLICY
        .get_or_init(|| Arc::new(RwLock::new(TimelinePolicyView::default())))
        .clone()
}

#[derive(Clone)]
pub struct TimelinePolicyHandle {
    inner: Arc<RwLock<TimelinePolicyView>>,
}

impl TimelinePolicyHandle {
    pub fn new_with(view: TimelinePolicyView) -> Self {
        Self {
            inner: Arc::new(RwLock::new(view)),
        }
    }

    pub fn global() -> Self {
        Self {
            inner: policy_cell(),
        }
    }

    pub fn snapshot(&self) -> TimelinePolicyView {
        self.inner.read().clone()
    }

    pub fn update(&self, view: TimelinePolicyView) {
        *self.inner.write() = view;
    }
}

pub fn set_policy(view: TimelinePolicyView) {
    TimelinePolicyHandle::global().update(view);
}

pub fn current_policy() -> TimelinePolicyView {
    TimelinePolicyHandle::global().snapshot()
}

impl crate::ports::PolicyPort for TimelinePolicyHandle {
    fn view(&self) -> TimelinePolicyView {
        self.snapshot()
    }
}
