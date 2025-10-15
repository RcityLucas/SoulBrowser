use std::sync::atomic::{AtomicU64, Ordering};

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

pub fn record_command() {
    COMMANDS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_event() {
    EVENTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_network_summary() {
    NETWORK_SUMMARIES.fetch_add(1, Ordering::Relaxed);
}

pub fn record_command_success(duration: std::time::Duration) {
    COMMAND_SUCCESS.fetch_add(1, Ordering::Relaxed);
    let micros = duration.as_micros().min(u64::MAX as u128) as u64;
    COMMAND_LATENCY_TOTAL_US.fetch_add(micros, Ordering::Relaxed);
}

pub fn record_command_failure() {
    COMMAND_FAILURES.fetch_add(1, Ordering::Relaxed);
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
        record_command();
        record_command_success(std::time::Duration::from_micros(150));
        record_command_failure();
        let snap = snapshot();
        assert_eq!(snap.commands, 1);
        assert_eq!(snap.command_success, 1);
        assert_eq!(snap.command_failures, 1);
        assert_eq!(snap.command_latency_total_us, 150);
    }
}
