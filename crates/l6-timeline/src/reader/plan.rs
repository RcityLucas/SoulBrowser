use crate::errors::TlError;
use crate::model::{By, ExportReq, View};
use crate::policy::TimelinePolicyView;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub enum PlanSource {
    Action {
        action_id: String,
    },
    Flow {
        flow_id: String,
    },
    Task {
        task_id: String,
    },
    Range {
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct FetchPlan {
    pub view: View,
    pub source: PlanSource,
    pub allow_state_tail: bool,
    pub records_sample_rate: f32,
    pub expect_cold_data: bool,
}

impl FetchPlan {
    pub fn source_name(&self) -> &'static str {
        match &self.source {
            PlanSource::Action { .. } => "action",
            PlanSource::Flow { .. } => "flow",
            PlanSource::Task { .. } => "task",
            PlanSource::Range { .. } => "range",
        }
    }
}

pub fn build_plan(
    req: &ExportReq,
    policy: &TimelinePolicyView,
    hot_hint: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<FetchPlan, TlError> {
    let allow_state_tail = matches!(req.by, By::Range { .. } | By::Flow { .. } | By::Task { .. });
    let plan_source = match &req.by {
        By::Action { action_id } => {
            if action_id.is_empty() {
                return Err(TlError::InvalidArg("action_id must not be empty".into()));
            }
            PlanSource::Action {
                action_id: action_id.clone(),
            }
        }
        By::Flow { flow_id } => {
            if flow_id.is_empty() {
                return Err(TlError::InvalidArg("flow_id must not be empty".into()));
            }
            PlanSource::Flow {
                flow_id: flow_id.clone(),
            }
        }
        By::Task { task_id } => {
            if task_id.is_empty() {
                return Err(TlError::InvalidArg("task_id must not be empty".into()));
            }
            PlanSource::Task {
                task_id: task_id.clone(),
            }
        }
        By::Range { since, until } => {
            if until <= since {
                return Err(TlError::InvalidArg(
                    "range selector requires until > since".into(),
                ));
            }
            let delta_ms = until.signed_duration_since(*since).num_milliseconds() as u64;
            if delta_ms > policy.max_time_range_ms {
                return Err(TlError::RangeTooLarge);
            }

            if let Some((hot_since, hot_until)) = hot_hint {
                if *since < hot_since || *until > hot_until {
                    if !policy.allow_cold_export {
                        return Err(TlError::PolicyDenied);
                    }
                }
            }

            PlanSource::Range {
                since: *since,
                until: *until,
            }
        }
    };

    Ok(FetchPlan {
        view: req.view.clone(),
        source: plan_source,
        allow_state_tail,
        records_sample_rate: policy.records_sample_rate.clamp(0.0, 1.0),
        expect_cold_data: matches!(req.by, By::Range { .. }) && policy.allow_cold_export,
    })
}
