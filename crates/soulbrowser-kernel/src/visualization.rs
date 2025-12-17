use crate::agent::executor::{FlowExecutionReport, RunArtifact, StepExecutionReport};
use agent_core::AgentPlan;
use chrono::Utc;
use serde_json::{json, Value};
use soulbrowser_core_types::ExecRoute;

pub fn build_plan_overlays(plan: &AgentPlan) -> Value {
    let overlays: Vec<Value> = plan
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
