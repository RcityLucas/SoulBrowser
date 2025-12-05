use std::collections::HashSet;

use crate::agent::executor::{FlowExecutionReport, StepExecutionStatus};
use crate::structured_output::canonical_schema_id;
use agent_core::AgentRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JudgeVerdict {
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl JudgeVerdict {
    fn pass() -> Self {
        Self {
            passed: true,
            reason: None,
        }
    }

    fn fail(reason: String) -> Self {
        Self {
            passed: false,
            reason: Some(reason),
        }
    }
}

/// Evaluate a finished plan execution and decide whether it satisfies required artifacts.
pub fn evaluate_plan(request: &AgentRequest, report: &FlowExecutionReport) -> JudgeVerdict {
    if !report.success {
        return JudgeVerdict::pass();
    }
    let delivered = delivered_schemas(report);
    let missing: Vec<String> = request
        .intent
        .required_outputs
        .iter()
        .filter(|output| !delivered.contains(&canonical_schema_id(&output.schema)))
        .map(|output| output.schema.clone())
        .collect();
    if !missing.is_empty() {
        return JudgeVerdict::fail(format!(
            "Judge rejected execution: missing structured outputs {}",
            missing.join(", ")
        ));
    }
    JudgeVerdict::pass()
}

fn delivered_schemas(report: &FlowExecutionReport) -> HashSet<String> {
    let mut set = HashSet::new();
    for step in &report.steps {
        if matches!(step.status, StepExecutionStatus::Failed) {
            continue;
        }
        for dispatch in &step.dispatches {
            if let Some(output) = dispatch.output.as_ref() {
                if let Some(schema) = output.get("schema").and_then(Value::as_str) {
                    set.insert(canonical_schema_id(schema));
                }
            }
        }
    }
    set
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::executor::{DispatchRecord, StepExecutionReport};
    use agent_core::{AgentRequest, RequestedOutput};
    use serde_json::json;
    use soulbrowser_core_types::TaskId;
    use soulbrowser_core_types::{ExecRoute, FrameId, PageId, SessionId};

    #[test]
    fn judge_detects_missing_schema() {
        let mut request = AgentRequest::new(TaskId::new(), "demo");
        request.intent.required_outputs = vec![RequestedOutput::new("market_info_v1.json")];
        let report = FlowExecutionReport {
            success: true,
            steps: vec![StepExecutionReport {
                step_id: "deliver".into(),
                title: "deliver".into(),
                status: StepExecutionStatus::Success,
                attempts: 1,
                error: None,
                dispatches: vec![DispatchRecord {
                    label: "deliver-other".into(),
                    action_id: "id".into(),
                    route: dummy_route(),
                    wait_ms: 0,
                    run_ms: 0,
                    output: Some(json!({ "schema": "other" })),
                    artifacts: Vec::new(),
                    error: None,
                }],
            }],
        };
        assert!(!evaluate_plan(&request, &report).passed);
    }

    #[test]
    fn judge_accepts_when_all_present() {
        let mut request = AgentRequest::new(TaskId::new(), "demo");
        request.intent.required_outputs = vec![RequestedOutput::new("market_info_v1.json")];
        let report = FlowExecutionReport {
            success: true,
            steps: vec![StepExecutionReport {
                step_id: "deliver".into(),
                title: "deliver".into(),
                status: StepExecutionStatus::Success,
                attempts: 1,
                error: None,
                dispatches: vec![DispatchRecord {
                    label: "deliver-market_info_v1".into(),
                    action_id: "id".into(),
                    route: dummy_route(),
                    wait_ms: 0,
                    run_ms: 0,
                    output: Some(json!({ "schema": "market_info_v1" })),
                    artifacts: Vec::new(),
                    error: None,
                }],
            }],
        };
        assert!(evaluate_plan(&request, &report).passed);
    }

    fn dummy_route() -> ExecRoute {
        ExecRoute::new(SessionId::new(), PageId::new(), FrameId::new())
    }
}
