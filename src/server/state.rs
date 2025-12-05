use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::perception_service::PerceptionService;

use super::rate_limit::RateLimiter;

#[derive(Clone)]
pub(crate) struct ServeState {
    pub(crate) ws_url: Option<String>,
    pub(crate) perception_service: Arc<PerceptionService>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    pub(crate) health: Arc<ServeHealth>,
}

impl ServeState {
    pub(crate) fn with_health(
        ws_url: Option<String>,
        perception_service: Arc<PerceptionService>,
        rate_limiter: Arc<RateLimiter>,
        health: Arc<ServeHealth>,
    ) -> Self {
        Self {
            ws_url,
            perception_service,
            rate_limiter,
            health,
        }
    }

    pub(crate) fn health_snapshot(&self) -> HealthSnapshot {
        let inner = self.health.snapshot();
        HealthSnapshot {
            ready: inner.ready,
            live: inner.live,
            last_ready_check: inner.last_ready_check,
            last_error: inner.last_error,
        }
    }

    pub(crate) fn mark_live(&self) {
        self.health.mark_live();
    }

    pub(crate) fn mark_ready(&self) {
        self.health.mark_ready();
    }

    pub(crate) fn mark_unready(&self, error: impl Into<String>) {
        self.health.mark_unready(error);
    }
}

pub(crate) struct HealthSnapshot {
    pub(crate) ready: bool,
    pub(crate) live: bool,
    pub(crate) last_ready_check: Option<u64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct ServeHealth {
    live: AtomicBool,
    ready: AtomicBool,
    last_ready_check: AtomicU64,
    last_error: Mutex<Option<String>>,
}

impl ServeHealth {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn mark_live(&self) {
        self.live.store(true, Ordering::SeqCst);
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
        self.update_last_check();
        let mut guard = self.last_error.lock().expect("health lock poisoned");
        *guard = None;
    }

    pub(crate) fn mark_unready(&self, error: impl Into<String>) {
        self.ready.store(false, Ordering::SeqCst);
        self.update_last_check();
        let mut guard = self.last_error.lock().expect("health lock poisoned");
        *guard = Some(error.into());
    }

    pub(crate) fn snapshot(&self) -> ServeHealthSnapshot {
        ServeHealthSnapshot {
            ready: self.ready.load(Ordering::SeqCst),
            live: self.live.load(Ordering::SeqCst),
            last_ready_check: self.last_ready_check(),
            last_error: self
                .last_error
                .lock()
                .expect("health lock poisoned")
                .clone(),
        }
    }

    fn update_last_check(&self) {
        if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
            self.last_ready_check
                .store(duration.as_secs(), Ordering::SeqCst);
        }
    }

    fn last_ready_check(&self) -> Option<u64> {
        match self.last_ready_check.load(Ordering::SeqCst) {
            0 => None,
            value => Some(value),
        }
    }
}

pub(crate) struct ServeHealthSnapshot {
    pub(crate) ready: bool,
    pub(crate) live: bool,
    pub(crate) last_ready_check: Option<u64>,
    pub(crate) last_error: Option<String>,
}
