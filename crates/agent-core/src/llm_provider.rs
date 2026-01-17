use async_trait::async_trait;
use serde_json::json;

use crate::agent_loop::{AgentHistoryEntry, AgentOutput, BrowserStateSummary};
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

    /// Decide next action(s) based on current browser state (agent loop mode).
    ///
    /// This method is called at each step of the agent loop to determine
    /// what action(s) to take based on the current browser state and history.
    ///
    /// # Arguments
    /// * `request` - The original agent request with goal and context
    /// * `state` - Current browser state summary with indexed elements
    /// * `history` - History of previous steps and their results
    ///
    /// # Returns
    /// An AgentOutput containing the LLM's thinking, evaluation, memory,
    /// next goal, and list of actions to execute.
    async fn decide(
        &self,
        request: &AgentRequest,
        state: &BrowserStateSummary,
        history: &[AgentHistoryEntry],
    ) -> Result<AgentOutput, AgentError> {
        // Default implementation returns an error indicating the provider
        // doesn't support agent loop mode.
        let _ = (request, state, history);
        Err(AgentError::invalid_request(
            "This LLM provider does not support agent loop mode (decide method not implemented)",
        ))
    }
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

    async fn decide(
        &self,
        request: &AgentRequest,
        state: &BrowserStateSummary,
        history: &[AgentHistoryEntry],
    ) -> Result<AgentOutput, AgentError> {
        ensure_goal(request)?;

        use crate::agent_loop::{AgentAction, AgentActionParams, AgentActionType};

        // Mock implementation: if we've taken 3+ steps, signal done
        if history.len() >= 3 {
            return Ok(AgentOutput {
                thinking: format!(
                    "Mock thinking: After {} steps on {}, task should be complete.",
                    history.len(),
                    state.url
                ),
                evaluation_previous_goal: Some("Previous step completed successfully".to_string()),
                memory: Some(format!("Completed {} steps", history.len())),
                next_goal: "Signal task completion".to_string(),
                actions: vec![AgentAction {
                    action_type: AgentActionType::Done,
                    element_index: None,
                    params: AgentActionParams {
                        done_success: Some(true),
                        done_text: Some(format!(
                            "Mock task completed after {} steps",
                            history.len()
                        )),
                        ..Default::default()
                    },
                }],
            });
        }

        // Otherwise, return a mock action based on element availability
        let action = if state.element_count > 0 {
            AgentAction {
                action_type: AgentActionType::Click,
                element_index: Some(0),
                params: AgentActionParams::default(),
            }
        } else {
            AgentAction {
                action_type: AgentActionType::Wait,
                element_index: None,
                params: AgentActionParams {
                    ms: Some(1000),
                    ..Default::default()
                },
            }
        };

        Ok(AgentOutput {
            thinking: format!(
                "Mock thinking: Analyzing page at {} with {} elements.",
                state.url, state.element_count
            ),
            evaluation_previous_goal: if history.is_empty() {
                None
            } else {
                Some("Previous action completed".to_string())
            },
            memory: Some(format!(
                "Step {} of task: {}",
                history.len() + 1,
                request.goal
            )),
            next_goal: format!("Continue task execution (step {})", history.len() + 1),
            actions: vec![action],
        })
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
        overlays: Vec::new(),
    };

    PlannerOutcome { plan, explanations }
}
