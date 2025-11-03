use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use lazy_static::lazy_static;
use prometheus::{
    core::Collector, histogram_opts, HistogramVec, IntCounter, IntCounterVec, Registry,
};
use tracing::error;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterMetricsSnapshot {
    pub commands: u64,
    pub events: u64,
    pub network_summaries: u64,
    pub command_success: u64,
    pub command_failures: u64,
    pub command_latency_total_us: u64,
}

static COMMANDS: AtomicU64 = AtomicU64::new(0);
static EVENTS: AtomicU64 = AtomicU64::new(0);
static NETWORK_SUMMARIES: AtomicU64 = AtomicU64::new(0);
static COMMAND_SUCCESS: AtomicU64 = AtomicU64::new(0);
static COMMAND_FAILURES: AtomicU64 = AtomicU64::new(0);
static COMMAND_LATENCY_TOTAL_US: AtomicU64 = AtomicU64::new(0);

lazy_static! {
    static ref CDP_COMMANDS_TOTAL: IntCounterVec = IntCounterVec::new(
        prometheus::Opts::new("soul_cdp_commands_total", "Total CDP commands executed"),
        &["method"]
    )
    .unwrap();
    static ref CDP_COMMAND_FAILURES_TOTAL: IntCounterVec = IntCounterVec::new(
        prometheus::Opts::new(
            "soul_cdp_command_failures_total",
            "Total CDP command failures"
        ),
        &["method"]
    )
    .unwrap();
    static ref CDP_COMMAND_DURATION: HistogramVec = HistogramVec::new(
        histogram_opts!(
            "soul_cdp_command_duration_seconds",
            "CDP command latency",
            vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0]
        ),
        &["method"]
    )
    .unwrap();
    static ref CDP_EVENTS_TOTAL: IntCounter =
        IntCounter::new("soul_cdp_events_total", "Total adapter events emitted").unwrap();
    static ref CDP_NETWORK_SUMMARIES_TOTAL: IntCounter = IntCounter::new(
        "soul_cdp_network_summaries_total",
        "Total network summaries emitted",
    )
    .unwrap();
}

fn register<C>(registry: &Registry, collector: C)
where
    C: Collector + Clone + Send + Sync + 'static,
{
    if let Err(err) = registry.register(Box::new(collector.clone())) {
        if !matches!(err, prometheus::Error::AlreadyReg) {
            error!(?err, "failed to register cdp metric");
        }
    }
}

pub fn register_metrics(registry: &Registry) {
    register(registry, CDP_COMMANDS_TOTAL.clone());
    register(registry, CDP_COMMAND_FAILURES_TOTAL.clone());
    register(registry, CDP_COMMAND_DURATION.clone());
    register(registry, CDP_EVENTS_TOTAL.clone());
    register(registry, CDP_NETWORK_SUMMARIES_TOTAL.clone());
}

pub fn record_command(method: &str) {
    COMMANDS.fetch_add(1, Ordering::Relaxed);
    CDP_COMMANDS_TOTAL.with_label_values(&[method]).inc();
}

pub fn record_event() {
    EVENTS.fetch_add(1, Ordering::Relaxed);
    CDP_EVENTS_TOTAL.inc();
}

pub fn record_network_summary() {
    NETWORK_SUMMARIES.fetch_add(1, Ordering::Relaxed);
    CDP_NETWORK_SUMMARIES_TOTAL.inc();
}

pub fn record_command_success(method: &str, duration: Duration) {
    COMMAND_SUCCESS.fetch_add(1, Ordering::Relaxed);
    let micros = duration.as_micros().min(u64::MAX as u128) as u64;
    COMMAND_LATENCY_TOTAL_US.fetch_add(micros, Ordering::Relaxed);
    CDP_COMMAND_DURATION
        .with_label_values(&[method])
        .observe(duration.as_secs_f64());
}

pub fn record_command_failure(method: &str) {
    COMMAND_FAILURES.fetch_add(1, Ordering::Relaxed);
    CDP_COMMAND_FAILURES_TOTAL
        .with_label_values(&[method])
        .inc();
}

pub fn snapshot() -> AdapterMetricsSnapshot {
    AdapterMetricsSnapshot {
        commands: COMMANDS.load(Ordering::Relaxed),
        events: EVENTS.load(Ordering::Relaxed),
        network_summaries: NETWORK_SUMMARIES.load(Ordering::Relaxed),
        command_success: COMMAND_SUCCESS.load(Ordering::Relaxed),
        command_failures: COMMAND_FAILURES.load(Ordering::Relaxed),
        command_latency_total_us: COMMAND_LATENCY_TOTAL_US.load(Ordering::Relaxed),
    }
}

pub fn reset() {
    COMMANDS.store(0, Ordering::Relaxed);
    EVENTS.store(0, Ordering::Relaxed);
    NETWORK_SUMMARIES.store(0, Ordering::Relaxed);
    COMMAND_SUCCESS.store(0, Ordering::Relaxed);
    COMMAND_FAILURES.store(0, Ordering::Relaxed);
    COMMAND_LATENCY_TOTAL_US.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_success_and_failure_metrics() {
        reset();
        record_command("Page.navigate");
        record_command_success("Page.navigate", Duration::from_micros(150));
        record_command_failure("Page.navigate");
        let snap = snapshot();
        assert_eq!(snap.commands, 1);
        assert_eq!(snap.command_success, 1);
        assert_eq!(snap.command_failures, 1);
        assert_eq!(snap.command_latency_total_us, 150);
    }
}
