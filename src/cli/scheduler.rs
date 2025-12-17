use std::sync::Arc;

use anyhow::{anyhow, Result};
use clap::{Args, ValueEnum};
use humantime::format_rfc3339;
use serde_json::{json, Value as JsonValue};
use soulbrowser_core_types::{ActionId, SoulError};
use soulbrowser_kernel::app_context::{get_or_create_context, AppContext};
use soulbrowser_kernel::Config;
use soulbrowser_scheduler::metrics as scheduler_metrics;
use soulbrowser_scheduler::model::Priority;
use soulbrowser_scheduler::Dispatcher;
use soulbrowser_state_center::{DispatchEvent, DispatchStatus, StateEvent};

#[derive(Clone, Debug, ValueEnum)]
pub enum SchedulerStatusFilter {
    Success,
    Failure,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum SchedulerOutputFormat {
    Text,
    Json,
}

#[derive(Args, Clone, Debug)]
pub struct SchedulerArgs {
    /// Number of recent events to display (default: 20)
    #[arg(short, long)]
    pub limit: Option<usize>,

    /// Only show events with the given status
    #[arg(long, value_enum)]
    pub status: Option<SchedulerStatusFilter>,

    /// Output format (`text` or `json`)
    #[arg(long, value_enum, default_value_t = SchedulerOutputFormat::Text)]
    pub format: SchedulerOutputFormat,

    /// Cancel a pending action by id
    #[arg(long)]
    pub cancel: Option<String>,
}

pub async fn cmd_scheduler(args: SchedulerArgs, config: &Config) -> Result<()> {
    let context = get_or_create_context(
        "cli-scheduler".to_string(),
        Some(config.output_dir.clone()),
        config.policy_paths.clone(),
    )
    .await?;

    if let Some(action_id) = args.cancel.as_ref() {
        let scheduler = context.scheduler_service();
        let cancelled: bool = scheduler
            .cancel(ActionId(action_id.to_string()))
            .await
            .map_err(|e: SoulError| anyhow!(e.to_string()))?;
        if cancelled {
            println!("Action {} cancelled", action_id);
        } else {
            println!("Action {} not found or already completed", action_id);
        }
        return Ok(());
    }

    let events = context.state_center_snapshot();
    let overview = scheduler_overview(&context);

    let limit = args.limit.unwrap_or(20);
    let status_filter = args.status;

    let filtered_iter = events.into_iter().rev().filter(|event| {
        if let Some(filter) = status_filter.as_ref() {
            match event {
                StateEvent::Dispatch(dispatch) => match (filter, &dispatch.status) {
                    (SchedulerStatusFilter::Success, DispatchStatus::Success) => true,
                    (SchedulerStatusFilter::Failure, DispatchStatus::Failure) => true,
                    _ => false,
                },
                StateEvent::Registry(_) | StateEvent::Perceiver(_) => false,
            }
        } else {
            matches!(event, StateEvent::Dispatch(_))
        }
    });

    let display_events: Vec<DispatchEvent> = if limit == 0 {
        filtered_iter
            .filter_map(|event| match event {
                StateEvent::Dispatch(dispatch) => Some(dispatch),
                _ => None,
            })
            .collect()
    } else {
        filtered_iter
            .take(limit)
            .filter_map(|event| match event {
                StateEvent::Dispatch(dispatch) => Some(dispatch),
                _ => None,
            })
            .collect()
    };

    match args.format {
        SchedulerOutputFormat::Text => {
            println!(
                "Scheduler counters → enqueued={} started={} completed={} failed={} cancelled={}",
                overview.metrics.enqueued,
                overview.metrics.started,
                overview.metrics.completed,
                overview.metrics.failed,
                overview.metrics.cancelled
            );
            println!(
                "Scheduler runtime → queue={} inflight={} slots_free={} (limit={}, per_task_limit={})",
                overview.runtime.queue_depth,
                overview.runtime.inflight,
                overview.runtime.slots_free,
                overview.runtime.global_limit,
                overview.runtime.per_task_limit
            );
            println!(
                "Queue breakdown → lightning={} quick={} standard={} deep={}",
                overview.queue_by_priority.lightning,
                overview.queue_by_priority.quick,
                overview.queue_by_priority.standard,
                overview.queue_by_priority.deep
            );
            if display_events.is_empty() {
                if status_filter.is_some() {
                    println!("No events match the selected filters.");
                } else {
                    println!("No dispatch events recorded yet.");
                }
                return Ok(());
            }
            println!(
                "Recent scheduler dispatch events (latest first, showing up to {}):",
                if limit == 0 {
                    display_events.len()
                } else {
                    limit.min(display_events.len())
                }
            );
            if let Some(filter) = status_filter.as_ref() {
                let label = match filter {
                    SchedulerStatusFilter::Success => "success",
                    SchedulerStatusFilter::Failure => "failure",
                };
                println!("  filter: {}", label);
            }
            for (idx, dispatch) in display_events.iter().enumerate() {
                let status = match dispatch.status {
                    DispatchStatus::Success => "success",
                    DispatchStatus::Failure => "failure",
                };
                let recorded_at = format_rfc3339(dispatch.recorded_at);
                println!(
                    "{:>2}. [{} @ {}] tool={} route={} attempts={} wait={}ms run={}ms pending={} slots={}",
                    idx + 1,
                    status,
                    recorded_at,
                    dispatch.tool,
                    dispatch.route,
                    dispatch.attempts,
                    dispatch.wait_ms,
                    dispatch.run_ms,
                    dispatch.pending,
                    dispatch.slots_available,
                );
                if let Some(err) = &dispatch.error {
                    println!("    error: {}", err);
                }
            }
        }
        SchedulerOutputFormat::Json => {
            let payload = scheduler_json_payload(&overview, &display_events);
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
pub struct SchedulerRuntimeSummary {
    pub queue_depth: usize,
    pub inflight: usize,
    pub slots_free: usize,
    pub global_limit: usize,
    pub per_task_limit: usize,
}

#[derive(Clone, Debug)]
pub struct QueueByPriority {
    pub lightning: usize,
    pub quick: usize,
    pub standard: usize,
    pub deep: usize,
}

#[derive(Clone, Debug)]
pub struct SchedulerOverview {
    pub metrics: scheduler_metrics::SchedulerMetricsSnapshot,
    pub runtime: SchedulerRuntimeSummary,
    pub queue_by_priority: QueueByPriority,
    pub captured_at: chrono::DateTime<chrono::Utc>,
}

pub fn scheduler_overview(context: &Arc<AppContext>) -> SchedulerOverview {
    let metrics = scheduler_metrics::snapshot();
    let captured_at = chrono::Utc::now();
    let runtime = context.scheduler_runtime();
    let queue_depth = runtime.pending();
    let slots_free = runtime.global_slots().available_permits();
    let config = runtime.config();
    let inflight = config.global_slots.saturating_sub(slots_free);
    let per_priority = runtime.depth_by_priority();

    SchedulerOverview {
        metrics,
        runtime: SchedulerRuntimeSummary {
            queue_depth,
            inflight,
            slots_free,
            global_limit: config.global_slots,
            per_task_limit: config.per_task_limit,
        },
        queue_by_priority: QueueByPriority {
            lightning: per_priority[Priority::Lightning.index()],
            quick: per_priority[Priority::Quick.index()],
            standard: per_priority[Priority::Standard.index()],
            deep: per_priority[Priority::Deep.index()],
        },
        captured_at,
    }
}

pub fn scheduler_json_payload(overview: &SchedulerOverview, events: &[DispatchEvent]) -> JsonValue {
    let json_events: Vec<_> = events.iter().map(dispatch_event_to_value).collect();
    json!({
        "captured_at": overview.captured_at.to_rfc3339(),
        "metrics": {
            "enqueued": overview.metrics.enqueued,
            "started": overview.metrics.started,
            "completed": overview.metrics.completed,
            "failed": overview.metrics.failed,
            "cancelled": overview.metrics.cancelled,
        },
        "runtime": {
            "queue_depth": overview.runtime.queue_depth,
            "inflight": overview.runtime.inflight,
            "slots_free": overview.runtime.slots_free,
            "global_limit": overview.runtime.global_limit,
            "per_task_limit": overview.runtime.per_task_limit,
        },
        "queue_by_priority": {
            "lightning": overview.queue_by_priority.lightning,
            "quick": overview.queue_by_priority.quick,
            "standard": overview.queue_by_priority.standard,
            "deep": overview.queue_by_priority.deep,
        },
        "events": json_events,
    })
}

fn dispatch_event_to_value(dispatch: &DispatchEvent) -> JsonValue {
    let recorded_at = format_rfc3339(dispatch.recorded_at).to_string();
    json!({
        "status": match dispatch.status {
            DispatchStatus::Success => "success",
            DispatchStatus::Failure => "failure",
        },
        "recorded_at": recorded_at,
        "tool": dispatch.tool.clone(),
        "route": dispatch.route.to_string(),
        "attempts": dispatch.attempts,
        "wait_ms": dispatch.wait_ms,
        "run_ms": dispatch.run_ms,
        "pending": dispatch.pending,
        "slots_available": dispatch.slots_available,
        "error": dispatch.error.as_ref().map(|e| e.to_string()),
    })
}

impl SchedulerRuntimeSummary {
    #[cfg(test)]
    fn new_for_test(
        queue_depth: usize,
        inflight: usize,
        slots_free: usize,
        global_limit: usize,
        per_task_limit: usize,
    ) -> Self {
        Self {
            queue_depth,
            inflight,
            slots_free,
            global_limit,
            per_task_limit,
        }
    }
}

impl QueueByPriority {
    #[cfg(test)]
    fn new_for_test(lightning: usize, quick: usize, standard: usize, deep: usize) -> Self {
        Self {
            lightning,
            quick,
            standard,
            deep,
        }
    }
}

impl SchedulerOverview {
    #[cfg(test)]
    fn new_for_test(
        metrics: scheduler_metrics::SchedulerMetricsSnapshot,
        runtime: SchedulerRuntimeSummary,
        queue_by_priority: QueueByPriority,
    ) -> Self {
        Self {
            metrics,
            runtime,
            queue_by_priority,
            captured_at: chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000, 0)
                .expect("valid test timestamp"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};

    fn mock_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }

    #[test]
    fn scheduler_json_payload_contains_expected_fields() {
        let metrics = scheduler_metrics::SchedulerMetricsSnapshot {
            enqueued: 10,
            started: 8,
            completed: 7,
            failed: 2,
            cancelled: 1,
            queue_length: 4,
        };
        let runtime = SchedulerRuntimeSummary::new_for_test(5, 3, 2, 6, 2);
        let queue = QueueByPriority::new_for_test(2, 1, 1, 1);
        let overview = SchedulerOverview::new_for_test(metrics, runtime, queue);

        let route = mock_route();
        let mutex_key = route.mutex_key.clone();
        let dispatch = DispatchEvent::success(
            ActionId::new(),
            Some("task-1".into()),
            route.clone(),
            "tool.click".into(),
            mutex_key,
            1,
            12,
            34,
            0,
            4,
            None,
        );
        let expected_timestamp = overview.captured_at.to_rfc3339();
        let payload = scheduler_json_payload(&overview, &[dispatch]);
        assert_eq!(payload["captured_at"], json!(expected_timestamp));
        assert_eq!(payload["metrics"]["enqueued"], json!(10));
        assert_eq!(payload["metrics"]["failed"], json!(2));
        assert_eq!(payload["runtime"]["queue_depth"], json!(5));
        assert_eq!(payload["runtime"]["global_limit"], json!(6));
        assert_eq!(payload["queue_by_priority"]["lightning"], json!(2));
        assert_eq!(payload["queue_by_priority"]["deep"], json!(1));
        assert!(payload["events"].is_array());
        assert_eq!(payload["events"].as_array().unwrap().len(), 1);
        assert_eq!(payload["events"][0]["tool"], "tool.click");
        assert_eq!(payload["events"][0]["status"], "success");
    }
}
