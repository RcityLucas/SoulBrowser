use std::time::{Duration, UNIX_EPOCH};

use crate::cli::context::CliContext;
use anyhow::Result;
use clap::{Args, ValueEnum};
use humantime::format_rfc3339;
use perceiver_structural::metrics::{self as structural_metrics, MetricSnapshot};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use soulbrowser_state_center::{
    PerceiverEvent, PerceiverEventKind, ScoreComponentRecord, StateEvent,
};

#[derive(Clone, Debug, ValueEnum)]
pub enum PerceiverKindFilter {
    Resolve,
    Judge,
    Snapshot,
    Diff,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum PerceiverCacheFilter {
    Hit,
    Miss,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum PerceiverOutputFormat {
    Table,
    Json,
}

#[derive(Args, Debug, Clone)]
pub struct PerceiverArgs {
    /// Number of recent events to display (default: 20)
    #[arg(short, long)]
    pub limit: Option<usize>,

    /// Only show events of the specified kind
    #[arg(long, value_enum)]
    pub kind: Option<PerceiverKindFilter>,

    /// Only show judge events with the specified check (e.g., visible, clickable)
    #[arg(long)]
    pub check: Option<String>,

    /// Filter by session id
    #[arg(long)]
    pub session: Option<String>,

    /// Filter by page id
    #[arg(long)]
    pub page: Option<String>,

    /// Filter by frame id
    #[arg(long)]
    pub frame: Option<String>,

    /// Filter by cache status (resolve/snapshot events only)
    #[arg(long, value_enum)]
    pub cache: Option<PerceiverCacheFilter>,

    /// Output format (`table` or `json`)
    #[arg(long, value_enum, default_value_t = PerceiverOutputFormat::Table)]
    pub format: PerceiverOutputFormat,
}

pub async fn cmd_perceiver(args: PerceiverArgs, ctx: &CliContext) -> Result<()> {
    let context = ctx.app_context().await?;

    let perceiver_events: Vec<PerceiverEvent> = context
        .state_center_snapshot()
        .into_iter()
        .filter_map(|event| match event {
            StateEvent::Perceiver(perceiver) => Some(perceiver),
            _ => None,
        })
        .collect();

    if perceiver_events.is_empty() {
        println!("No perceiver events recorded yet.");
        return Ok(());
    }

    let filtered = filter_perceiver_events(perceiver_events, &args);

    if filtered.is_empty() {
        println!("No perceiver events matched the provided filters.");
        return Ok(());
    }

    let summary = summarize_perceiver_events(&filtered);

    match args.format {
        PerceiverOutputFormat::Table => {
            print_perceiver_summary(&summary);
            print_perceiver_table(&filtered);
        }
        PerceiverOutputFormat::Json => {
            let events: Vec<_> = filtered.iter().map(perceiver_event_to_json).collect();
            let payload = json!({
                "summary": summary,
                "events": events,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }

    Ok(())
}

#[derive(Serialize)]
struct PerceiverSummary {
    resolve: usize,
    judge: usize,
    snapshot: usize,
    diff: usize,
    metrics: MetricSnapshot,
}

fn filter_perceiver_events(
    mut events: Vec<PerceiverEvent>,
    args: &PerceiverArgs,
) -> Vec<PerceiverEvent> {
    let limit = args.limit.unwrap_or(20);
    let check_filter = args.check.as_ref().map(|value| value.to_lowercase());
    let session_filter = args.session.as_ref();
    let page_filter = args.page.as_ref();
    let frame_filter = args.frame.as_ref();

    events = events
        .into_iter()
        .filter(|event| {
            if let Some(kind_filter) = args.kind.as_ref() {
                if !matches_perceiver_kind(kind_filter, &event.kind) {
                    return false;
                }
                if let (PerceiverKindFilter::Judge, Some(check_filter)) =
                    (kind_filter, &check_filter)
                {
                    let matches_check = match &event.kind {
                        PerceiverEventKind::Judge { check, .. } => {
                            check.to_lowercase() == *check_filter
                        }
                        _ => false,
                    };
                    if !matches_check {
                        return false;
                    }
                }
            } else if let Some(check_filter) = check_filter.as_ref() {
                let matches_check = match &event.kind {
                    PerceiverEventKind::Judge { check, .. } => {
                        check.to_lowercase() == *check_filter
                    }
                    _ => false,
                };
                if !matches_check {
                    return false;
                }
            }

            if let Some(cache_filter) = args.cache.as_ref() {
                if !matches_cache_filter(cache_filter, &event.kind) {
                    return false;
                }
            }

            if let Some(session) = session_filter {
                if event.route.session.0 != *session {
                    return false;
                }
            }
            if let Some(page) = page_filter {
                if event.route.page.0 != *page {
                    return false;
                }
            }
            if let Some(frame) = frame_filter {
                if event.route.frame.0 != *frame {
                    return false;
                }
            }

            true
        })
        .collect();

    events.sort_by(|a, b| event_timestamp_ms(b).cmp(&event_timestamp_ms(a)));

    if limit > 0 && events.len() > limit {
        events.truncate(limit);
    }

    events
}

fn summarize_perceiver_events(events: &[PerceiverEvent]) -> PerceiverSummary {
    let mut resolve = 0usize;
    let mut judge = 0usize;
    let mut snapshot = 0usize;
    let mut diff = 0usize;
    for event in events {
        match event.kind {
            PerceiverEventKind::Resolve { .. } => resolve += 1,
            PerceiverEventKind::Judge { .. } => judge += 1,
            PerceiverEventKind::Snapshot { .. } => snapshot += 1,
            PerceiverEventKind::Diff { .. } => diff += 1,
        }
    }

    PerceiverSummary {
        resolve,
        judge,
        snapshot,
        diff,
        metrics: structural_metrics::snapshot(),
    }
}

fn print_perceiver_summary(summary: &PerceiverSummary) {
    println!(
        "Perceiver summary → resolve: {} | judge: {} | snapshot: {} | diff: {}",
        summary.resolve, summary.judge, summary.snapshot, summary.diff
    );
    println!(
        "Metric summary → resolve: {} (avg {:.2}ms) | judge: {} (avg {:.2}ms) | snapshot: {} (avg {:.2}ms) | diff: {} (avg {:.2}ms)",
        summary.metrics.resolve.total,
        summary.metrics.resolve.avg_ms,
        summary.metrics.judge.total,
        summary.metrics.judge.avg_ms,
        summary.metrics.snapshot.total,
        summary.metrics.snapshot.avg_ms,
        summary.metrics.diff.total,
        summary.metrics.diff.avg_ms
    );
    println!(
        "Cache stats → resolve: {} hit / {} miss ({:.1}%) | snapshot: {} hit / {} miss ({:.1}%)",
        summary.metrics.resolve_cache.hits,
        summary.metrics.resolve_cache.misses,
        summary.metrics.resolve_cache.hit_rate,
        summary.metrics.snapshot_cache.hits,
        summary.metrics.snapshot_cache.misses,
        summary.metrics.snapshot_cache.hit_rate
    );
}

fn print_perceiver_table(events: &[PerceiverEvent]) {
    println!("Showing {} most recent perceiver event(s):", events.len());
    for event in events {
        let timestamp = format_rfc3339(event.recorded_at);
        let route = &event.route;
        match &event.kind {
            PerceiverEventKind::Resolve {
                strategy,
                score,
                candidate_count,
                cache_hit,
                breakdown,
                reason,
            } => {
                let breakdown_summary = summarize_breakdown(breakdown);
                println!(
                    "[{}] resolve session={} page={} frame={} strategy={} score={:.2} candidates={} cache={} reason=\"{}\" breakdown=[{}]",
                    timestamp,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    strategy,
                    score,
                    candidate_count,
                    if *cache_hit { "hit" } else { "miss" },
                    reason,
                    breakdown_summary,
                );
            }
            PerceiverEventKind::Judge {
                check,
                ok,
                reason,
                facts,
            } => {
                println!(
                    "[{}] judge::{} session={} page={} frame={} status={} reason=\"{}\" facts={}",
                    timestamp,
                    check,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    if *ok { "ok" } else { "fail" },
                    reason,
                    compact_json(facts, 80),
                );
            }
            PerceiverEventKind::Snapshot { cache_hit } => {
                println!(
                    "[{}] snapshot session={} page={} frame={} cache={}",
                    timestamp,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    if *cache_hit { "hit" } else { "miss" }
                );
            }
            PerceiverEventKind::Diff {
                change_count,
                changes,
            } => {
                println!(
                    "[{}] diff session={} page={} frame={} changes={} sample={}",
                    timestamp,
                    route.session.0,
                    route.page.0,
                    route.frame.0,
                    change_count,
                    changes
                        .get(0)
                        .map(|value| compact_json(value, 80))
                        .unwrap_or_else(|| "-".into()),
                );
            }
        }
    }
}

fn perceiver_event_to_json(event: &PerceiverEvent) -> JsonValue {
    let timestamp = format_rfc3339(event.recorded_at).to_string();
    let route = &event.route;
    match &event.kind {
        PerceiverEventKind::Resolve {
            strategy,
            score,
            candidate_count,
            cache_hit,
            breakdown,
            reason,
        } => json!({
            "timestamp": timestamp,
            "kind": "resolve",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "strategy": strategy,
            "score": score,
            "candidate_count": candidate_count,
            "cache_hit": cache_hit,
            "reason": reason,
            "score_breakdown": breakdown,
        }),
        PerceiverEventKind::Judge {
            check,
            ok,
            reason,
            facts,
        } => json!({
            "timestamp": timestamp,
            "kind": "judge",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "check": check,
            "ok": ok,
            "reason": reason,
            "facts": facts,
        }),
        PerceiverEventKind::Snapshot { cache_hit } => json!({
            "timestamp": timestamp,
            "kind": "snapshot",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "cache_hit": cache_hit,
        }),
        PerceiverEventKind::Diff {
            change_count,
            changes,
        } => json!({
            "timestamp": timestamp,
            "kind": "diff",
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
            "mutex_key": route.mutex_key,
            "change_count": change_count,
            "changes": changes,
        }),
    }
}

fn summarize_breakdown(breakdown: &[ScoreComponentRecord]) -> String {
    if breakdown.is_empty() {
        return String::new();
    }

    let sum_abs: f32 = breakdown
        .iter()
        .map(|component| component.contribution.abs())
        .sum();
    let mut components: Vec<&ScoreComponentRecord> = breakdown.iter().collect();
    components.sort_by(|a, b| {
        b.contribution
            .abs()
            .partial_cmp(&a.contribution.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    components
        .into_iter()
        .map(|component| {
            let magnitude = component.contribution.abs();
            let sign = if component.contribution >= 0.0 {
                '+'
            } else {
                '-'
            };
            let share = if sum_abs > f32::EPSILON {
                (magnitude / sum_abs) * 100.0
            } else {
                0.0
            };
            format!(
                "{}:{}{:.3} (w={:.3}, {:>3.0}%)",
                component.label, sign, magnitude, component.weight, share
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn compact_json(value: &serde_json::Value, limit: usize) -> String {
    let raw = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    truncate(&raw, limit)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        format!("{}…", &text[..limit])
    }
}

fn matches_perceiver_kind(filter: &PerceiverKindFilter, kind: &PerceiverEventKind) -> bool {
    match (filter, kind) {
        (PerceiverKindFilter::Resolve, PerceiverEventKind::Resolve { .. }) => true,
        (PerceiverKindFilter::Judge, PerceiverEventKind::Judge { .. }) => true,
        (PerceiverKindFilter::Snapshot, PerceiverEventKind::Snapshot { .. }) => true,
        (PerceiverKindFilter::Diff, PerceiverEventKind::Diff { .. }) => true,
        _ => false,
    }
}

fn matches_cache_filter(filter: &PerceiverCacheFilter, kind: &PerceiverEventKind) -> bool {
    match (filter, kind) {
        (PerceiverCacheFilter::Hit, PerceiverEventKind::Resolve { cache_hit, .. }) => *cache_hit,
        (PerceiverCacheFilter::Miss, PerceiverEventKind::Resolve { cache_hit, .. }) => !cache_hit,
        (PerceiverCacheFilter::Hit, PerceiverEventKind::Snapshot { cache_hit }) => *cache_hit,
        (PerceiverCacheFilter::Miss, PerceiverEventKind::Snapshot { cache_hit }) => !cache_hit,
        _ => false,
    }
}

fn event_timestamp_ms(event: &PerceiverEvent) -> u128 {
    event
        .recorded_at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};

    fn base_args() -> PerceiverArgs {
        PerceiverArgs {
            limit: None,
            kind: None,
            check: None,
            session: None,
            page: None,
            frame: None,
            cache: None,
            format: PerceiverOutputFormat::Table,
        }
    }

    fn route(session: &str, page: &str, frame: &str) -> ExecRoute {
        ExecRoute {
            session: SessionId(session.to_string()),
            page: PageId(page.to_string()),
            frame: FrameId(frame.to_string()),
            mutex_key: format!("frame:{}", frame),
        }
    }

    fn stamped(mut event: PerceiverEvent, millis: u64) -> PerceiverEvent {
        event.recorded_at = UNIX_EPOCH + Duration::from_millis(millis);
        event
    }

    fn score_components(items: Vec<(&str, f32, f32)>) -> Vec<ScoreComponentRecord> {
        items
            .into_iter()
            .map(|(label, weight, contribution)| ScoreComponentRecord {
                label: label.into(),
                weight,
                contribution,
            })
            .collect()
    }

    #[test]
    fn filters_by_kind_and_orders_descending() {
        let events = vec![
            stamped(
                PerceiverEvent::resolve(
                    route("s1", "p1", "f1"),
                    "css".into(),
                    0.7,
                    2,
                    false,
                    score_components(vec![("confidence", 1.0, 0.7)]),
                    "score=0.7".into(),
                ),
                10,
            ),
            stamped(
                PerceiverEvent::judge(
                    route("s1", "p1", "f1"),
                    "visible".into(),
                    true,
                    "geometry".into(),
                    json!({ "geometry": {"width": 120} }),
                ),
                30,
            ),
            stamped(
                PerceiverEvent::diff(route("s1", "p1", "f1"), 5, vec![json!({"kind": "text"})]),
                20,
            ),
        ];

        let mut args = base_args();
        args.kind = Some(PerceiverKindFilter::Judge);
        let filtered = filter_perceiver_events(events, &args);
        assert_eq!(filtered.len(), 1);
        match &filtered[0].kind {
            PerceiverEventKind::Judge { check, .. } => assert_eq!(check, "visible"),
            _ => panic!("expected judge event"),
        }
    }

    #[test]
    fn filters_by_cache_status() {
        let events = vec![
            stamped(
                PerceiverEvent::resolve(
                    route("s1", "p1", "f1"),
                    "css".into(),
                    0.8,
                    3,
                    true,
                    score_components(vec![("confidence", 1.0, 0.8)]),
                    "score=0.8".into(),
                ),
                40,
            ),
            stamped(PerceiverEvent::snapshot(route("s1", "p1", "f1"), false), 50),
        ];

        let mut args = base_args();
        args.cache = Some(PerceiverCacheFilter::Miss);
        let filtered = filter_perceiver_events(events, &args);
        assert_eq!(filtered.len(), 1);
        match &filtered[0].kind {
            PerceiverEventKind::Snapshot { cache_hit } => assert!(!cache_hit),
            _ => panic!("expected snapshot event"),
        }
    }

    #[test]
    fn applies_limit_and_session_filter() {
        let events = vec![
            stamped(
                PerceiverEvent::resolve(
                    route("a", "p1", "f1"),
                    "css".into(),
                    0.9,
                    4,
                    false,
                    score_components(vec![("confidence", 1.0, 0.9)]),
                    "score=0.9".into(),
                ),
                10,
            ),
            stamped(
                PerceiverEvent::resolve(
                    route("b", "p2", "f2"),
                    "css".into(),
                    0.6,
                    1,
                    true,
                    score_components(vec![("confidence", 1.0, 0.6)]),
                    "score=0.6".into(),
                ),
                30,
            ),
            stamped(
                PerceiverEvent::resolve(
                    route("a", "p3", "f3"),
                    "text".into(),
                    0.5,
                    2,
                    false,
                    score_components(vec![("confidence", 1.0, 0.5)]),
                    "score=0.5".into(),
                ),
                20,
            ),
        ];

        let mut args = base_args();
        args.session = Some("a".into());
        args.limit = Some(1);
        let filtered = filter_perceiver_events(events, &args);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].route.session.0, "a");
        assert_eq!(filtered[0].route.page.0, "p3");
    }

    #[test]
    fn summarize_breakdown_formats_components() {
        let records = vec![
            ScoreComponentRecord {
                label: "visibility".into(),
                weight: 0.5,
                contribution: 0.6,
            },
            ScoreComponentRecord {
                label: "text".into(),
                weight: 0.3,
                contribution: 0.2,
            },
        ];
        let summary = summarize_breakdown(&records);
        assert!(summary.contains("visibility:+0.600"));
        assert!(summary.contains("w=0.500"));
        assert!(summary.contains("text:+0.200"));
        assert!(summary.contains("%"));
    }
}
