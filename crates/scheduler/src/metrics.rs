use std::time::Duration;

use lazy_static::lazy_static;
use once_cell::sync::Lazy;
use prometheus::{
    core::Collector, histogram_opts, opts, Histogram, IntCounterVec, IntGauge, Registry,
};
use tracing::error;

#[derive(Default)]
struct Counters {
    enqueued: std::sync::atomic::AtomicU64,
    started: std::sync::atomic::AtomicU64,
    completed: std::sync::atomic::AtomicU64,
    failed: std::sync::atomic::AtomicU64,
    cancelled: std::sync::atomic::AtomicU64,
}

static COUNTERS: Lazy<Counters> = Lazy::new(Counters::default);

lazy_static! {
    static ref SCHEDULER_QUEUE_LENGTH: IntGauge = IntGauge::new(
        "soul_scheduler_queue_length",
        "Current scheduler queue length"
    )
    .expect("queue gauge");
    static ref SCHEDULER_ENQUEUED_TOTAL: IntCounterVec = IntCounterVec::new(
        opts!("soul_scheduler_enqueued_total", "Total requests enqueued"),
        &["tool"]
    )
    .expect("enqueued counter");
    static ref SCHEDULER_STARTED_TOTAL: IntCounterVec = IntCounterVec::new(
        opts!("soul_scheduler_started_total", "Total requests started"),
        &["tool"]
    )
    .expect("started counter");
    static ref SCHEDULER_COMPLETED_TOTAL: IntCounterVec = IntCounterVec::new(
        opts!("soul_scheduler_completed_total", "Total requests completed"),
        &["tool"]
    )
    .expect("completed counter");
    static ref SCHEDULER_FAILED_TOTAL: IntCounterVec = IntCounterVec::new(
        opts!("soul_scheduler_failed_total", "Total requests failed"),
        &["tool"]
    )
    .expect("failed counter");
    static ref SCHEDULER_CANCELLED_TOTAL: IntCounterVec = IntCounterVec::new(
        opts!("soul_scheduler_cancelled_total", "Total requests cancelled"),
        &["tool"]
    )
    .expect("cancelled counter");
    static ref SCHEDULER_EXECUTION_DURATION: Histogram = Histogram::with_opts(histogram_opts!(
        "soul_scheduler_execution_duration_seconds",
        "Scheduler execution duration",
        vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
    ))
    .expect("execution histogram");
}

fn register<C>(registry: &Registry, collector: C)
where
    C: Collector + Clone + Send + Sync + 'static,
{
    if let Err(err) = registry.register(Box::new(collector.clone())) {
        if !matches!(err, prometheus::Error::AlreadyReg) {
            error!(?err, "failed to register scheduler metric");
        }
    }
}

pub fn register_metrics(registry: &Registry) {
    register(registry, SCHEDULER_QUEUE_LENGTH.clone());
    register(registry, SCHEDULER_ENQUEUED_TOTAL.clone());
    register(registry, SCHEDULER_STARTED_TOTAL.clone());
    register(registry, SCHEDULER_COMPLETED_TOTAL.clone());
    register(registry, SCHEDULER_FAILED_TOTAL.clone());
    register(registry, SCHEDULER_CANCELLED_TOTAL.clone());
    register(registry, SCHEDULER_EXECUTION_DURATION.clone());
}

fn increment(counter: &std::sync::atomic::AtomicU64) {
    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn record_enqueued(tool: &str) {
    SCHEDULER_ENQUEUED_TOTAL.with_label_values(&[tool]).inc();
    SCHEDULER_QUEUE_LENGTH.inc();
    increment(&COUNTERS.enqueued);
}

pub fn record_started(tool: &str) {
    SCHEDULER_STARTED_TOTAL.with_label_values(&[tool]).inc();
    if SCHEDULER_QUEUE_LENGTH.get() > 0 {
        SCHEDULER_QUEUE_LENGTH.dec();
    }
    increment(&COUNTERS.started);
}

pub fn record_completed(tool: &str, duration: Duration) {
    SCHEDULER_COMPLETED_TOTAL.with_label_values(&[tool]).inc();
    SCHEDULER_EXECUTION_DURATION.observe(duration.as_secs_f64());
    increment(&COUNTERS.completed);
}

pub fn record_failed(tool: &str, duration: Duration) {
    SCHEDULER_FAILED_TOTAL.with_label_values(&[tool]).inc();
    SCHEDULER_EXECUTION_DURATION.observe(duration.as_secs_f64());
    increment(&COUNTERS.failed);
}

pub fn record_cancelled(tool: &str) {
    SCHEDULER_CANCELLED_TOTAL.with_label_values(&[tool]).inc();
    if SCHEDULER_QUEUE_LENGTH.get() > 0 {
        SCHEDULER_QUEUE_LENGTH.dec();
    }
    increment(&COUNTERS.cancelled);
}

#[derive(Clone, Debug, Default)]
pub struct SchedulerMetricsSnapshot {
    pub enqueued: u64,
    pub started: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub queue_length: i64,
}

pub fn snapshot() -> SchedulerMetricsSnapshot {
    SchedulerMetricsSnapshot {
        enqueued: COUNTERS.enqueued.load(std::sync::atomic::Ordering::Relaxed),
        started: COUNTERS.started.load(std::sync::atomic::Ordering::Relaxed),
        completed: COUNTERS
            .completed
            .load(std::sync::atomic::Ordering::Relaxed),
        failed: COUNTERS.failed.load(std::sync::atomic::Ordering::Relaxed),
        cancelled: COUNTERS
            .cancelled
            .load(std::sync::atomic::Ordering::Relaxed),
        queue_length: SCHEDULER_QUEUE_LENGTH.get(),
    }
}
