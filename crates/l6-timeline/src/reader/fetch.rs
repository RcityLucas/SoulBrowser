use super::plan::{FetchPlan, PlanSource};
use crate::errors::TlResult;
use crate::model::EventEnvelope;
use crate::ports::{EventStorePort, StateCenterPort};
use chrono::{DateTime, Utc};
use std::borrow::Cow;

const STATE_CENTER_TAIL_LIMIT: usize = 256;

#[derive(Debug, Default)]
pub struct FetchOutcome {
    pub primary: Vec<EventEnvelope>,
    pub state_tail: Vec<EventEnvelope>,
}

pub async fn run_fetch(
    event_store: &dyn EventStorePort,
    state_center: Option<&dyn StateCenterPort>,
    plan: &FetchPlan,
) -> TlResult<FetchOutcome> {
    let mut outcome = FetchOutcome::default();

    outcome.primary = match &plan.source {
        PlanSource::Action { action_id } => event_store.by_action(action_id).await?,
        PlanSource::Flow { flow_id } => event_store.by_flow_window(flow_id).await?,
        PlanSource::Task { task_id } => event_store.by_task_window(task_id).await?,
        PlanSource::Range { since, until } => event_store.export_range(*since, *until).await?,
    };

    if plan.allow_state_tail {
        if let Some(sc) = state_center {
            outcome.state_tail = sc.tail(STATE_CENTER_TAIL_LIMIT).await.unwrap_or_default();
        }
    }

    Ok(outcome)
}

pub fn merge_outcome(outcome: FetchOutcome) -> Vec<EventEnvelope> {
    let mut merged = outcome.primary;
    if !outcome.state_tail.is_empty() {
        merged.extend(outcome.state_tail);
    }
    merged
}

pub fn clamp_range(
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    hint: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    if let Some((hot_since, hot_until)) = hint {
        (since.max(hot_since), until.min(hot_until))
    } else {
        (since, until)
    }
}

pub fn describe_source(plan: &FetchPlan) -> Cow<'static, str> {
    match &plan.source {
        PlanSource::Action { .. } => Cow::Borrowed("event_store:action"),
        PlanSource::Flow { .. } => Cow::Borrowed("event_store:flow"),
        PlanSource::Task { .. } => Cow::Borrowed("event_store:task"),
        PlanSource::Range { .. } => Cow::Borrowed("event_store:range"),
    }
}
