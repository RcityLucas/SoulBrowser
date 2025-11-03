mod rule_based;

use crate::{errors::AgentError, model::AgentRequest, plan::AgentPlan};

pub use rule_based::RuleBasedPlanner;

/// Planner configuration controlling heuristic behaviour.
#[derive(Debug, Clone)]
pub struct PlannerConfig {
    /// Maximum number of steps the planner may emit.
    pub max_steps: usize,
    /// Whether to automatically prepend navigation when URL detected.
    pub auto_navigate: bool,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            max_steps: 12,
            auto_navigate: true,
        }
    }
}

/// Result from the planner including additional explanations.
#[derive(Debug, Clone)]
pub struct PlannerOutcome {
    pub plan: AgentPlan,
    /// Bullet-style explanations summarising reasoning.
    pub explanations: Vec<String>,
}

/// Trait implemented by agent planners that can transform user
/// requests into executable plans.
pub trait AgentPlanner {
    fn draft_plan(&self, request: &AgentRequest) -> Result<PlannerOutcome, AgentError>;
}
