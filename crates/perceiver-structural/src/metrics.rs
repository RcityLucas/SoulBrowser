//! Telemetry helpers for Structural Perceiver.
//!
//! Lightweight counters + latency aggregates so the CLI can surface basic metrics without
//! depending on an external metrics backend yet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;

static RESOLVE_TOTAL: AtomicU64 = AtomicU64::new(0);
static RESOLVE_CACHE_HIT: AtomicU64 = AtomicU64::new(0);
static RESOLVE_CACHE_MISS: AtomicU64 = AtomicU64::new(0);
static RESOLVE_LAT_NS: AtomicU64 = AtomicU64::new(0);
static RESOLVE_LAT_SAMPLES: AtomicU64 = AtomicU64::new(0);

static JUDGE_TOTAL: AtomicU64 = AtomicU64::new(0);
static JUDGE_LAT_NS: AtomicU64 = AtomicU64::new(0);
static JUDGE_LAT_SAMPLES: AtomicU64 = AtomicU64::new(0);

static SNAPSHOT_TOTAL: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_CACHE_HIT: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_CACHE_MISS: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_LAT_NS: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_LAT_SAMPLES: AtomicU64 = AtomicU64::new(0);

static DIFF_TOTAL: AtomicU64 = AtomicU64::new(0);
static DIFF_LAT_NS: AtomicU64 = AtomicU64::new(0);
static DIFF_LAT_SAMPLES: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MetricCounter {
    pub total: u64,
    pub avg_ms: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CacheMetric {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MetricSnapshot {
    pub resolve: MetricCounter,
    pub resolve_cache: CacheMetric,
    pub judge: MetricCounter,
    pub snapshot: MetricCounter,
    pub snapshot_cache: CacheMetric,
    pub diff: MetricCounter,
}

pub fn record_resolve(cache_hit: bool, duration: Duration) {
    RESOLVE_TOTAL.fetch_add(1, Ordering::Relaxed);
    if cache_hit {
        RESOLVE_CACHE_HIT.fetch_add(1, Ordering::Relaxed);
    } else {
        RESOLVE_CACHE_MISS.fetch_add(1, Ordering::Relaxed);
    }
    record_latency(&RESOLVE_LAT_NS, &RESOLVE_LAT_SAMPLES, duration);
}

pub fn record_judge(duration: Duration) {
    JUDGE_TOTAL.fetch_add(1, Ordering::Relaxed);
    record_latency(&JUDGE_LAT_NS, &JUDGE_LAT_SAMPLES, duration);
}

pub fn record_snapshot(cache_hit: bool, duration: Duration) {
    SNAPSHOT_TOTAL.fetch_add(1, Ordering::Relaxed);
    if cache_hit {
        SNAPSHOT_CACHE_HIT.fetch_add(1, Ordering::Relaxed);
    } else {
        SNAPSHOT_CACHE_MISS.fetch_add(1, Ordering::Relaxed);
    }
    record_latency(&SNAPSHOT_LAT_NS, &SNAPSHOT_LAT_SAMPLES, duration);
}

pub fn record_diff(duration: Duration) {
    DIFF_TOTAL.fetch_add(1, Ordering::Relaxed);
    record_latency(&DIFF_LAT_NS, &DIFF_LAT_SAMPLES, duration);
}

pub fn snapshot() -> MetricSnapshot {
    MetricSnapshot {
        resolve: make_counter(
            RESOLVE_TOTAL.load(Ordering::Relaxed),
            RESOLVE_LAT_NS.load(Ordering::Relaxed),
            RESOLVE_LAT_SAMPLES.load(Ordering::Relaxed),
        ),
        resolve_cache: make_cache_metric(
            RESOLVE_CACHE_HIT.load(Ordering::Relaxed),
            RESOLVE_CACHE_MISS.load(Ordering::Relaxed),
        ),
        judge: make_counter(
            JUDGE_TOTAL.load(Ordering::Relaxed),
            JUDGE_LAT_NS.load(Ordering::Relaxed),
            JUDGE_LAT_SAMPLES.load(Ordering::Relaxed),
        ),
        snapshot: make_counter(
            SNAPSHOT_TOTAL.load(Ordering::Relaxed),
            SNAPSHOT_LAT_NS.load(Ordering::Relaxed),
            SNAPSHOT_LAT_SAMPLES.load(Ordering::Relaxed),
        ),
        snapshot_cache: make_cache_metric(
            SNAPSHOT_CACHE_HIT.load(Ordering::Relaxed),
            SNAPSHOT_CACHE_MISS.load(Ordering::Relaxed),
        ),
        diff: make_counter(
            DIFF_TOTAL.load(Ordering::Relaxed),
            DIFF_LAT_NS.load(Ordering::Relaxed),
            DIFF_LAT_SAMPLES.load(Ordering::Relaxed),
        ),
    }
}

fn make_counter(total: u64, nanos: u64, samples: u64) -> MetricCounter {
    let avg_ms = if samples == 0 {
        0.0
    } else {
        (nanos as f64 / samples as f64) / 1_000_000.0
    };
    MetricCounter { total, avg_ms }
}

fn make_cache_metric(hits: u64, misses: u64) -> CacheMetric {
    let total = hits + misses;
    let hit_rate = if total == 0 {
        0.0
    } else {
        hits as f64 * 100.0 / total as f64
    };
    CacheMetric {
        hits,
        misses,
        hit_rate,
    }
}

fn record_latency(total_ns: &AtomicU64, samples: &AtomicU64, duration: Duration) {
    let nanos = duration_to_nanos(duration);
    total_ns.fetch_add(nanos, Ordering::Relaxed);
    samples.fetch_add(1, Ordering::Relaxed);
}

fn duration_to_nanos(duration: Duration) -> u64 {
    let nanos = duration.as_nanos();
    if nanos > u64::MAX as u128 {
        u64::MAX
    } else {
        nanos as u64
    }
}
