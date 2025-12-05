use async_trait::async_trait;
use serde_json::json;

use crate::errors::AgentError;
use crate::model::AgentRequest;
use crate::plan::{AgentPlan, AgentPlanMeta, AgentPlanStep, AgentTool, AgentToolKind, WaitMode};
use crate::planner::PlannerOutcome;

/// Abstraction over LLM-backed planners so multiple vendors can plug into the agent core.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Generate a fresh plan from the provided agent request.
    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError>;

    /// Generate a follow-up plan after a failure along with error context.
    async fn replan(
        &self,
        request: &AgentRequest,
        previous_plan: &AgentPlan,
        error_summary: &str,
    ) -> Result<PlannerOutcome, AgentError>;
}

/// Deterministic provider used for tests and offline development.
#[derive(Debug, Default, Clone)]
pub struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError> {
        ensure_goal(request)?;
        Ok(mock_plan("initial", request, None))
    }

    async fn replan(
        &self,
        request: &AgentRequest,
        previous_plan: &AgentPlan,
        error_summary: &str,
    ) -> Result<PlannerOutcome, AgentError> {
        ensure_goal(request)?;
        Ok(mock_plan(
            "replan",
            request,
            Some((previous_plan, error_summary)),
        ))
    }
}

fn ensure_goal(request: &AgentRequest) -> Result<(), AgentError> {
    if request.goal.trim().is_empty() {
        return Err(AgentError::invalid_request("goal cannot be empty"));
    }
    Ok(())
}

fn mock_plan(
    phase: &str,
    request: &AgentRequest,
    context: Option<(&AgentPlan, &str)>,
) -> PlannerOutcome {
    let trimmed_goal = request.goal.trim();
    let mut plan = AgentPlan::new(
        request.task_id.clone(),
        format!("LLM plan for {trimmed_goal}"),
    )
    .with_description(format!(
        "Mock {phase} plan synthesised for goal: {trimmed_goal}"
    ));

    let mut step = AgentPlanStep::new(
        "llm-step-1",
        "Summarize goal and prepare",
        AgentTool {
            kind: AgentToolKind::Custom {
                name: "mock.llm.plan".to_string(),
                payload: json!({
                    "goal": trimmed_goal,
                    "phase": phase,
                    "constraints": request.constraints,
                }),
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(5_000),
        },
    )
    .with_detail("Placeholder step emitted by MockLlmProvider");

    if let Some((previous_plan, error_summary)) = context {
        step.metadata.insert(
            "replan_reason".to_string(),
            json!({
                "error": error_summary,
                "previous_plan_title": previous_plan.title,
            }),
        );
    }

    plan.push_step(step);

    let mut explanations = vec![format!("Mock {phase} plan created for '{trimmed_goal}'")];
    if let Some((_, error)) = context {
        explanations.push(format!("Replanning due to: {error}"));
    }

    plan.meta = AgentPlanMeta {
        rationale: explanations.clone(),
        risk_assessment: vec!["Low risk mock plan".to_string()],
        vendor_context: Default::default(),
    };

    PlannerOutcome { plan, explanations }
}
