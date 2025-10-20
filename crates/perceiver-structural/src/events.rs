use soulbrowser_core_types::ExecRoute;
use std::time::Duration;
use tracing::debug;

use crate::metrics;

pub fn emit_resolve(
    route: &ExecRoute,
    strategy: &str,
    score: f32,
    candidate_count: usize,
    cache_hit: bool,
    duration: Duration,
) {
    metrics::record_resolve(cache_hit, duration);
    debug!(
        target: "perceiver.events",
        %route,
        strategy,
        score,
        candidate_count,
        cache_hit,
        "structural.resolve.completed"
    );
}

pub fn emit_judge(route: &ExecRoute, kind: &str, ok: bool, reason: &str, duration: Duration) {
    metrics::record_judge(duration);
    debug!(
        target: "perceiver.events",
        %route,
        kind,
        ok,
        reason,
        "structural.judge.completed"
    );
}

pub fn emit_snapshot(route: &ExecRoute, cache_hit: bool, duration: Duration) {
    metrics::record_snapshot(cache_hit, duration);
    debug!(
        target: "perceiver.events",
        %route,
        cache_hit,
        "structural.snapshot.recorded"
    );
}

pub fn emit_diff(change_count: usize, duration: Duration) {
    metrics::record_diff(duration);
    debug!(
        target: "perceiver.events",
        change_count,
        "structural.diff.generated"
    );
}
