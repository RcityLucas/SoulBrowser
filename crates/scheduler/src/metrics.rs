use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Counters {
    enqueued: AtomicU64,
    started: AtomicU64,
    completed: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
}

static COUNTERS: Lazy<Counters> = Lazy::new(Counters::default);

fn increment(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

pub fn record_enqueued(_tool: &str) {
    increment(&COUNTERS.enqueued);
}

pub fn record_started(_tool: &str) {
    increment(&COUNTERS.started);
}

pub fn record_completed(_tool: &str) {
    increment(&COUNTERS.completed);
}

pub fn record_failed(_tool: &str) {
    increment(&COUNTERS.failed);
}

pub fn record_cancelled(_tool: &str) {
    increment(&COUNTERS.cancelled);
}

#[derive(Clone, Debug, Default)]
pub struct SchedulerMetricsSnapshot {
    pub enqueued: u64,
    pub started: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
}

pub fn snapshot() -> SchedulerMetricsSnapshot {
    SchedulerMetricsSnapshot {
        enqueued: COUNTERS.enqueued.load(Ordering::Relaxed),
        started: COUNTERS.started.load(Ordering::Relaxed),
        completed: COUNTERS.completed.load(Ordering::Relaxed),
        failed: COUNTERS.failed.load(Ordering::Relaxed),
        cancelled: COUNTERS.cancelled.load(Ordering::Relaxed),
    }
}
