use crate::agent::executor::{FlowExecutionReport, StepExecutionStatus};
use agent_core::{requires_user_facing_result, AgentRequest};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::agent::executor::{StepExecutionReport, UserResult, UserResultKind};

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

    fn fail(reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            reason: Some(reason.into()),
        }
    }
}

/// Evaluate a finished plan execution and decide whether it satisfies required artifacts.
pub fn evaluate_plan(request: &AgentRequest, report: &FlowExecutionReport) -> JudgeVerdict {
    if !report.success {
        let reason = failure_reason(report).unwrap_or_else(|| "Plan execution failed".to_string());
        return JudgeVerdict::fail(reason);
    }

    if report.user_results.is_empty()
        && (report.missing_user_result || requires_user_facing_result(request))
    {
        return JudgeVerdict::fail("执行成功但没有生成可交付的结果");
    }

    JudgeVerdict::pass()
}

fn failure_reason(report: &FlowExecutionReport) -> Option<String> {
    report
        .steps
        .iter()
        .find(|step| matches!(step.status, StepExecutionStatus::Failed))
        .and_then(|step| {
            step.error
                .clone()
                .or_else(|| Some(format!("步骤 {} 失败", step.title)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulbrowser_core_types::TaskId;

    #[test]
    fn judge_fails_when_execution_fails() {
        let request = AgentRequest::new(TaskId::new(), "demo");
        let report = FlowExecutionReport {
            success: false,
            steps: vec![step_report(
                StepExecutionStatus::Failed,
                Some("boom".to_string()),
            )],
            user_results: Vec::new(),
            missing_user_result: true,
            memory_log: Vec::new(),
            judge_verdict: None,
        };
        let verdict = evaluate_plan(&request, &report);
        assert!(!verdict.passed);
        assert!(verdict.reason.unwrap().contains("boom"));
    }

    #[test]
    fn judge_fails_when_no_user_result() {
        let request = AgentRequest::new(TaskId::new(), "demo");
        let report = FlowExecutionReport {
            success: true,
            steps: vec![step_report(StepExecutionStatus::Success, None)],
            user_results: Vec::new(),
            missing_user_result: true,
            memory_log: Vec::new(),
            judge_verdict: None,
        };
        let verdict = evaluate_plan(&request, &report);
        assert!(!verdict.passed);
        assert!(verdict.reason.unwrap().contains("没有生成"));
    }

    #[test]
    fn judge_passes_with_user_result() {
        let request = AgentRequest::new(TaskId::new(), "demo");
        let report = FlowExecutionReport {
            success: true,
            steps: vec![step_report(StepExecutionStatus::Success, None)],
            user_results: vec![UserResult {
                step_id: "note".to_string(),
                step_title: "note".to_string(),
                kind: UserResultKind::Note,
                schema: None,
                content: serde_json::json!({"text": "done"}),
                artifact_path: None,
            }],
            missing_user_result: false,
            memory_log: Vec::new(),
            judge_verdict: None,
        };
        let verdict = evaluate_plan(&request, &report);
        assert!(verdict.passed);
        assert!(verdict.reason.is_none());
    }

    fn step_report(status: StepExecutionStatus, error: Option<String>) -> StepExecutionReport {
        StepExecutionReport {
            step_id: "1".to_string(),
            title: "demo".to_string(),
            tool_kind: "navigate".to_string(),
            status,
            attempts: 1,
            error,
            dispatches: Vec::new(),
            total_wait_ms: 0,
            total_run_ms: 0,
            observation_summary: None,
            blocker_kind: None,
            agent_state: None,
        }
    }
}
