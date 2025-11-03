pub mod executor;

use agent_core::{
    plan_to_flow, AgentContext, AgentLocator, AgentPlan, AgentPlanStep, AgentPlanner, AgentRequest,
    AgentScrollTarget, AgentToolKind, AgentWaitCondition, ConversationRole, ConversationTurn,
    PlanToFlowOptions, PlanToFlowResult, PlannerConfig, RuleBasedPlanner, WaitMode,
};
use anyhow::{anyhow, Result};
use soulbrowser_core_types::TaskId;
use std::fmt;

pub use executor::{execute_plan, FlowExecutionOptions, FlowExecutionReport, StepExecutionStatus};

/// Runner that bridges CLI prompts to the L8 rule-based planner.
#[derive(Debug, Clone)]
pub struct ChatRunner {
    planner: RuleBasedPlanner,
    flow_options: PlanToFlowOptions,
}

impl Default for ChatRunner {
    fn default() -> Self {
        Self::with_config(PlannerConfig::default(), PlanToFlowOptions::default())
    }
}

impl ChatRunner {
    pub fn with_config(config: PlannerConfig, flow_options: PlanToFlowOptions) -> Self {
        Self {
            planner: RuleBasedPlanner::new(config),
            flow_options,
        }
    }

    /// Build an `AgentRequest` from a plain prompt, optional context, and constraints.
    pub fn request_from_prompt(
        &self,
        prompt: String,
        context: Option<AgentContext>,
        constraints: Vec<String>,
    ) -> AgentRequest {
        let mut request = AgentRequest::new(TaskId::new(), prompt.clone());
        request.push_turn(ConversationTurn::new(ConversationRole::User, prompt));
        request.constraints = constraints;
        if let Some(ctx) = context {
            request = request.with_context(ctx);
        }
        request
    }

    /// Generate a plan and flow given the prepared request envelope.
    pub fn plan(&self, mut request: AgentRequest) -> Result<ChatSessionOutput> {
        if request.goal.trim().is_empty() {
            return Err(anyhow!("Prompt cannot be empty"));
        }

        if request.conversation.is_empty() {
            request.push_turn(ConversationTurn::new(
                ConversationRole::User,
                request.goal.clone(),
            ));
        }

        let outcome = self
            .planner
            .draft_plan(&request)
            .map_err(|err| anyhow!("planner failed: {}", err))?;
        let flow = plan_to_flow(&outcome.plan, self.flow_options.clone())
            .map_err(|err| anyhow!("plan conversion failed: {}", err))?;

        Ok(ChatSessionOutput {
            plan: outcome.plan,
            explanations: outcome.explanations,
            flow,
        })
    }
}

/// Composite result returned to the CLI command.
#[derive(Debug)]
pub struct ChatSessionOutput {
    pub plan: AgentPlan,
    pub explanations: Vec<String>,
    pub flow: PlanToFlowResult,
}

impl ChatSessionOutput {
    pub fn summarize_steps(&self) -> Vec<String> {
        self.plan
            .steps
            .iter()
            .enumerate()
            .map(|(idx, step)| format!("{}. {}", idx + 1, StepSummary(step)))
            .collect()
    }
}

struct StepSummary<'a>(&'a AgentPlanStep);

impl<'a> fmt::Display for StepSummary<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let step = self.0;
        let action = match &step.tool.kind {
            AgentToolKind::Navigate { url } => format!("Navigate to {}", url),
            AgentToolKind::Click { locator } => format!("Click {}", describe_locator(locator)),
            AgentToolKind::TypeText {
                locator,
                text,
                submit,
            } => {
                let submit_note = if *submit { " and submit" } else { "" };
                format!(
                    "Type '{}' into {}{}",
                    text,
                    describe_locator(locator),
                    submit_note
                )
            }
            AgentToolKind::Select {
                locator,
                value,
                method,
            } => {
                let method_note = method.as_deref().unwrap_or("value");
                format!(
                    "Select '{}' by {} via {}",
                    value,
                    method_note,
                    describe_locator(locator)
                )
            }
            AgentToolKind::Scroll { target } => {
                format!("Scroll {}", describe_scroll_target(target))
            }
            AgentToolKind::Wait { condition } => {
                format!("Wait until {}", describe_wait_condition(condition))
            }
            AgentToolKind::Custom { name, .. } => format!("Invoke custom tool '{}'", name),
        };

        let wait_note = match step.tool.wait {
            WaitMode::None => String::new(),
            WaitMode::DomReady => String::new(),
            WaitMode::Idle => " (wait for page idle)".to_string(),
        };

        if step.detail.is_empty() {
            write!(f, "{}{}", action, wait_note)
        } else {
            write!(f, "{}{} – {}", action, wait_note, step.detail)
        }
    }
}

fn describe_locator(locator: &AgentLocator) -> String {
    match locator {
        AgentLocator::Css(selector) => format!("CSS selector '{}'", selector),
        AgentLocator::Aria { role, name } => format!("ARIA role '{}' with name '{}'", role, name),
        AgentLocator::Text { content, exact } => {
            if *exact {
                format!("text exactly '{}'", content)
            } else {
                format!("text containing '{}'", content)
            }
        }
    }
}

fn describe_scroll_target(target: &AgentScrollTarget) -> String {
    match target {
        AgentScrollTarget::Top => "to top".to_string(),
        AgentScrollTarget::Bottom => "to bottom".to_string(),
        AgentScrollTarget::Selector(locator) => {
            format!("to {}", describe_locator(locator))
        }
        AgentScrollTarget::Pixels(delta) => {
            if *delta >= 0 {
                format!("by {} pixels down", delta)
            } else {
                format!("by {} pixels up", delta.abs())
            }
        }
    }
}

fn describe_wait_condition(condition: &AgentWaitCondition) -> String {
    match condition {
        AgentWaitCondition::ElementVisible(locator) => {
            format!("{} is visible", describe_locator(locator))
        }
        AgentWaitCondition::ElementHidden(locator) => {
            format!("{} is hidden", describe_locator(locator))
        }
        AgentWaitCondition::UrlMatches(pattern) => {
            format!("URL matches '{}'", pattern)
        }
        AgentWaitCondition::TitleMatches(pattern) => {
            format!("title matches '{}'", pattern)
        }
        AgentWaitCondition::NetworkIdle(ms) => {
            format!("network idle for {} ms", ms)
        }
        AgentWaitCondition::Duration(ms) => {
            format!("{} ms elapsed", ms)
        }
    }
}
