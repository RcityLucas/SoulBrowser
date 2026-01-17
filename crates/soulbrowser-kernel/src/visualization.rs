use crate::agent::executor::{FlowExecutionReport, RunArtifact, StepExecutionReport};
use agent_core::AgentPlan;
use chrono::Utc;
use serde_json::{json, Value};
use soulbrowser_core_types::ExecRoute;

pub fn build_plan_overlays(plan: &AgentPlan) -> Value {
    let mut overlays: Vec<Value> = plan
        .steps
        .iter()
        .filter_map(|step| {
            let source = step.metadata.get("source");
            let locator = source
                .and_then(|value| value.get("locator"))
                .cloned()
                .filter(|v| !v.is_null());
            locator.map(|loc| {
                let recorded_at = Utc::now().to_rfc3339();
                json!({
                    "step_id": step.id,
                    "title": step.title,
                    "action": source
                        .and_then(|value| value.get("action"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "locator": loc,
                    "recorded_at": recorded_at,
                })
            })
        })
        .collect();

    if !plan.meta.overlays.is_empty() {
        overlays.extend(plan.meta.overlays.iter().cloned());
    }

    if let Some(timeline) = plan.meta.vendor_context.get("stage_timeline") {
        if let Some(stages) = timeline.get("stages").cloned() {
            let recorded_at = Utc::now().to_rfc3339();
            overlays.push(json!({
                "kind": "stage_timeline",
                "deterministic": timeline
                    .get("deterministic")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                "stages": stages,
                "recorded_at": recorded_at,
            }));
        }
    }

    Value::Array(overlays)
}

pub fn build_execution_overlays(steps: &[StepExecutionReport]) -> Value {
    let overlays: Vec<Value> = steps
        .iter()
        .flat_map(|step| {
            step.dispatches
                .iter()
                .enumerate()
                .flat_map(move |(dispatch_index, dispatch)| {
                    dispatch.artifacts.iter().filter_map(move |artifact| {
                        if !artifact.content_type.starts_with("image/") {
                            return None;
                        }
                        let mut value = json!({
                            "step_id": step.step_id,
                            "dispatch_label": dispatch.label,
                            "dispatch_index": dispatch_index,
                            "action_id": dispatch.action_id,
                            "route": {
                                "session": dispatch.route.session.0,
                                "page": dispatch.route.page.0,
                                "frame": dispatch.route.frame.0,
                            },
                            "label": artifact.label,
                            "content_type": artifact.content_type,
                            "byte_len": artifact.byte_len,
                            "filename": artifact.filename,
                            "data_base64": artifact.data_base64,
                        });
                        value
                            .as_object_mut()
                            .expect("overlay payload is object")
                            .insert(
                                "recorded_at".to_string(),
                                Value::String(Utc::now().to_rfc3339()),
                            );
                        Some(value)
                    })
                })
        })
        .collect();

    Value::Array(overlays)
}

pub fn execution_artifacts_from_report(report: &FlowExecutionReport) -> Vec<Value> {
    report
        .steps
        .iter()
        .flat_map(|step| {
            step.dispatches
                .iter()
                .enumerate()
                .flat_map(move |(dispatch_index, dispatch)| {
                    dispatch.artifacts.iter().map(move |artifact| {
                        artifact_event_value(
                            &step.step_id,
                            &dispatch.label,
                            dispatch_index,
                            &dispatch.route,
                            &dispatch.action_id,
                            artifact,
                        )
                    })
                })
        })
        .collect()
}

pub fn artifact_event_value(
    step_id: &str,
    dispatch_label: &str,
    dispatch_index: usize,
    route: &ExecRoute,
    action_id: &str,
    artifact: &RunArtifact,
) -> Value {
    let mut value = json!({
        "step_id": step_id,
        "dispatch_label": dispatch_label,
        "dispatch_index": dispatch_index,
        "action_id": action_id,
        "route": {
            "session": route.session.0,
            "page": route.page.0,
            "frame": route.frame.0,
        },
        "label": artifact.label,
        "content_type": artifact.content_type,
        "byte_len": artifact.byte_len,
        "filename": artifact.filename,
        "data_base64": artifact.data_base64,
    });

    value
        .as_object_mut()
        .expect("artifact payload is object")
        .insert(
            "recorded_at".to_string(),
            Value::String(Utc::now().to_rfc3339()),
        );

    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::plan::{AgentPlanStep, AgentTool, AgentToolKind};
    use agent_core::WaitMode;
    use soulbrowser_core_types::TaskId;

    #[test]
    fn plan_overlays_include_stage_timeline() {
        let mut plan = AgentPlan::new(TaskId::new(), "plan");
        plan.push_step(AgentPlanStep::new(
            "navigate-1",
            "导航",
            AgentTool {
                kind: AgentToolKind::Navigate {
                    url: "https://example.com".to_string(),
                },
                wait: WaitMode::DomReady,
                timeout_ms: Some(1_000),
            },
        ));
        plan.meta.vendor_context.insert(
            "stage_timeline".to_string(),
            json!({
                "deterministic": true,
                "stages": [
                    {"stage": "navigate", "label": "导航", "status": "existing", "strategy": "plan", "detail": "原计划"}
                ]
            }),
        );

        let overlays = build_plan_overlays(&plan);
        let array = overlays.as_array().expect("overlays array");
        assert!(array
            .iter()
            .any(|entry| entry.get("kind") == Some(&Value::String("stage_timeline".to_string()))));
    }
}
