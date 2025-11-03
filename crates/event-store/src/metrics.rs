use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

const HOT_UTIL_ALERT_THRESHOLD: f32 = 0.9;
const HOT_UTIL_RECOVER_THRESHOLD: f32 = 0.8;
const DROP_ALERT_THRESHOLD: f64 = 0.05;
const DROP_RECOVER_THRESHOLD: f64 = 0.02;
const COLD_BACKLOG_ALERT: u64 = 1_024;
const COLD_BACKLOG_RECOVER: u64 = 256;

#[derive(Clone)]
pub struct EsMetrics {
    inner: Arc<EsMetricsInner>,
}

impl Default for EsMetrics {
    fn default() -> Self {
        Self {
            inner: Arc::new(EsMetricsInner::default()),
        }
    }
}

impl EsMetrics {
    pub fn set_alert_hook(&self, hook: AlertHook) {
        *self.inner.alert_hook.lock() = Some(hook);
    }

    pub fn record_append_ok(&self) {
        self.inner.append_ok.fetch_add(1, Ordering::Relaxed);
        self.evaluate_drop_rate();
    }

    pub fn record_append_drop(&self, reason: &str) {
        self.inner.append_drop.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.inner.drop_reasons.lock();
        *guard.entry(reason.to_string()).or_insert(0) += 1;
        drop(guard);
        self.evaluate_drop_rate();
    }

    pub fn record_cold_flush(&self, duration_ms: u64) {
        self.inner
            .last_cold_flush_ms
            .store(duration_ms, Ordering::Relaxed);
    }

    pub fn record_cold_error(&self) {
        self.inner.cold_errors.fetch_add(1, Ordering::Relaxed);
        if !self
            .inner
            .cold_error_alert_active
            .swap(true, Ordering::Relaxed)
        {
            self.emit_alert(
                EsAlertKind::ColdWriterErrors,
                "cold writer reported I/O error",
                self.inner.cold_errors.load(Ordering::Relaxed) as f64,
            );
        }
    }

    pub fn reset_cold_error_alert(&self) {
        self.inner
            .cold_error_alert_active
            .store(false, Ordering::Relaxed);
    }

    pub fn observe_hot_utilization(&self, utilization: f32) {
        let scaled = (utilization.clamp(0.0, 1.0) * 1000.0) as u32;
        self.inner.hot_utilization.store(scaled, Ordering::Relaxed);
        if utilization >= HOT_UTIL_ALERT_THRESHOLD {
            if !self.inner.hot_alert_active.swap(true, Ordering::Relaxed) {
                self.emit_alert(
                    EsAlertKind::HotUtilizationHigh,
                    "hot rings near capacity",
                    utilization as f64,
                );
            }
        } else if utilization <= HOT_UTIL_RECOVER_THRESHOLD {
            self.inner.hot_alert_active.store(false, Ordering::Relaxed);
        }
    }

    pub fn cold_queue_inc(&self) {
        let pending = self.inner.cold_pending.fetch_add(1, Ordering::Relaxed) + 1;
        if pending >= COLD_BACKLOG_ALERT
            && !self
                .inner
                .backlog_alert_active
                .swap(true, Ordering::Relaxed)
        {
            self.emit_alert(
                EsAlertKind::ColdBacklogGrowing,
                format!("cold writer backlog = {}", pending),
                pending as f64,
            );
        }
    }

    pub fn cold_queue_dec(&self) {
        let mut prev = self.inner.cold_pending.load(Ordering::Relaxed);
        loop {
            if prev == 0 {
                break;
            }
            let next = prev - 1;
            match self.inner.cold_pending.compare_exchange(
                prev,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    prev = next;
                    break;
                }
                Err(actual) => prev = actual,
            }
        }
        if prev <= COLD_BACKLOG_RECOVER {
            self.inner
                .backlog_alert_active
                .store(false, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> EsMetricSnapshot {
        let drop_reasons = self.inner.drop_reasons.lock().clone();
        EsMetricSnapshot {
            append_ok: self.inner.append_ok.load(Ordering::Relaxed),
            append_drop: self.inner.append_drop.load(Ordering::Relaxed),
            cold_errors: self.inner.cold_errors.load(Ordering::Relaxed),
            cold_pending: self.inner.cold_pending.load(Ordering::Relaxed),
            last_cold_flush_ms: self.inner.last_cold_flush_ms.load(Ordering::Relaxed),
            hot_utilization: self.inner.hot_utilization.load(Ordering::Relaxed) as f32 / 1000.0,
            drop_reasons,
        }
    }

    fn evaluate_drop_rate(&self) {
        let ok = self.inner.append_ok.load(Ordering::Relaxed);
        let drop = self.inner.append_drop.load(Ordering::Relaxed);
        let total = ok + drop;
        if total < 100 {
            return;
        }
        let ratio = drop as f64 / total as f64;
        if ratio >= DROP_ALERT_THRESHOLD {
            if !self.inner.drop_alert_active.swap(true, Ordering::Relaxed) {
                self.emit_alert(
                    EsAlertKind::DropRateHigh,
                    format!("append drop ratio {:.2}%", ratio * 100.0),
                    ratio,
                );
            }
        } else if ratio <= DROP_RECOVER_THRESHOLD {
            self.inner.drop_alert_active.store(false, Ordering::Relaxed);
        }
    }

    fn emit_alert(&self, kind: EsAlertKind, message: impl Into<String>, value: f64) {
        let alert = EsAlert {
            kind,
            message: message.into(),
            value,
        };
        if let Some(hook) = self.inner.alert_hook.lock().clone() {
            (hook)(alert);
        } else {
            eprintln!(
                "[event-store][alert] {:?}: {} ({:.4})",
                alert.kind, alert.message, alert.value
            );
        }
    }
}

#[derive(Default)]
struct EsMetricsInner {
    append_ok: AtomicU64,
    append_drop: AtomicU64,
    cold_errors: AtomicU64,
    cold_pending: AtomicU64,
    last_cold_flush_ms: AtomicU64,
    hot_utilization: AtomicU32,
    drop_reasons: Mutex<HashMap<String, u64>>,
    alert_hook: Mutex<Option<AlertHook>>,
    hot_alert_active: AtomicBool,
    drop_alert_active: AtomicBool,
    backlog_alert_active: AtomicBool,
    cold_error_alert_active: AtomicBool,
}

pub type AlertHook = Arc<dyn Fn(EsAlert) + Send + Sync + 'static>;

#[derive(Clone, Debug)]
pub struct EsMetricSnapshot {
    pub append_ok: u64,
    pub append_drop: u64,
    pub cold_errors: u64,
    pub cold_pending: u64,
    pub last_cold_flush_ms: u64,
    pub hot_utilization: f32,
    pub drop_reasons: HashMap<String, u64>,
}

#[derive(Clone, Debug)]
pub enum EsAlertKind {
    HotUtilizationHigh,
    DropRateHigh,
    ColdBacklogGrowing,
    ColdWriterErrors,
}

#[derive(Clone, Debug)]
pub struct EsAlert {
    pub kind: EsAlertKind,
    pub message: String,
    pub value: f64,
}
