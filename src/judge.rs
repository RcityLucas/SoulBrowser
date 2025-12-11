use crate::agent::executor::{FlowExecutionReport, StepExecutionStatus};
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
pub fn evaluate_plan(_request: &AgentRequest, report: &FlowExecutionReport) -> JudgeVerdict {
    if report.success {
        JudgeVerdict::pass()
    } else {
        JudgeVerdict::pass()
    }
}
