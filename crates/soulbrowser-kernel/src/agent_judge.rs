use crate::agent::executor::{FlowExecutionReport, StepExecutionStatus};
use crate::task_status::TaskStatusHandle;
use agent_core::AgentRequest;
use serde::Serialize;
use serde_json::json;

#[derive(Clone, Debug, Serialize)]
pub struct JudgeOverlay {
    pub kind: &'static str,
    pub detail: String,
    pub level: &'static str,
}

pub fn build_judge_overlays(
    _request: &AgentRequest,
    report: &FlowExecutionReport,
) -> Vec<JudgeOverlay> {
    let mut overlays = Vec::new();
    if !report.success {
        if let Some(step) = report
            .steps
            .iter()
            .find(|step| matches!(step.status, StepExecutionStatus::Failed))
        {
            overlays.push(JudgeOverlay {
                kind: "failure",
                detail: format!(
                    "步骤 {} 失败: {}",
                    step.title,
                    step.error.clone().unwrap_or_default()
                ),
                level: "error",
            });
        }
        return overlays;
    }
    if report.user_results.is_empty() {
        overlays.push(JudgeOverlay {
            kind: "missing_result",
            detail: "执行成功但没有输出任何结果".to_string(),
            level: "warn",
        });
    }
    overlays
}

pub fn emit_judge_overlays(handle: &TaskStatusHandle, overlays: Vec<JudgeOverlay>) {
    if overlays.is_empty() {
        return;
    }
    let payload = overlays
        .into_iter()
        .map(|overlay| {
            json!({
                "kind": overlay.kind,
                "detail": overlay.detail,
                "level": overlay.level,
            })
        })
        .collect::<Vec<_>>();
    handle.push_execution_overlays(json!(payload));
}
