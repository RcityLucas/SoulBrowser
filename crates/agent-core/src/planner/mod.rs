mod quote_sources;
mod rule_based;
mod stage_graph;
mod stages;

use crate::{errors::AgentError, model::AgentRequest, plan::AgentPlan};

pub use quote_sources::mark_source_unhealthy;
pub(crate) use quote_sources::{resolve_quote_plan, QuoteQuery};
pub use rule_based::RuleBasedPlanner;
pub use stage_graph::{IntentStagePlan, PlanStageGraph, StageStrategyChain};
pub use stages::{classify_step, plan_contains_stage, stage_index, PlanStageKind};

/// Planner configuration controlling heuristic behaviour.
#[derive(Debug, Clone)]
pub struct PlannerConfig {
    /// Maximum number of steps the planner may emit.
    pub max_steps: usize,
    /// Whether to automatically prepend navigation when URL detected.
    pub auto_navigate: bool,
    /// Whether to enforce strict plan validation without auto-repair.
    pub strict_plan_validation: bool,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            max_steps: 12,
            auto_navigate: true,
            strict_plan_validation: false,
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
