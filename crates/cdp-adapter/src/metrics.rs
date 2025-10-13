use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterMetricsSnapshot {
    pub commands: u64,
    pub events: u64,
    pub network_summaries: u64,
}

static COMMANDS: AtomicU64 = AtomicU64::new(0);
static EVENTS: AtomicU64 = AtomicU64::new(0);
static NETWORK_SUMMARIES: AtomicU64 = AtomicU64::new(0);

pub fn record_command() {
    COMMANDS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_event() {
    EVENTS.fetch_add(1, Ordering::Relaxed);
}

pub fn record_network_summary() {
    NETWORK_SUMMARIES.fetch_add(1, Ordering::Relaxed);
}

pub fn snapshot() -> AdapterMetricsSnapshot {
    AdapterMetricsSnapshot {
        commands: COMMANDS.load(Ordering::Relaxed),
        events: EVENTS.load(Ordering::Relaxed),
        network_summaries: NETWORK_SUMMARIES.load(Ordering::Relaxed),
    }
}

pub fn reset() {
    COMMANDS.store(0, Ordering::Relaxed);
    EVENTS.store(0, Ordering::Relaxed);
    NETWORK_SUMMARIES.store(0, Ordering::Relaxed);
}
