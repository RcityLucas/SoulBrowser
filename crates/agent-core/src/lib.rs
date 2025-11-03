//! L8 Agent Core primitives.
//!
//! Provides shared data models, errors, and conversion utilities for turning
//! conversational agent outputs into executable SoulBrowser flows.

pub mod convert;
pub mod errors;
pub mod model;
pub mod plan;
pub mod planner;

pub use convert::{plan_to_flow, PlanToFlowOptions, PlanToFlowResult};
pub use errors::AgentError;
pub use model::{AgentContext, AgentRequest, ConversationRole, ConversationTurn};
pub use plan::{
    AgentLocator, AgentPlan, AgentPlanMeta, AgentPlanStep, AgentScrollTarget, AgentTool,
    AgentToolKind, AgentValidation, AgentWaitCondition, WaitMode,
};
pub use planner::{AgentPlanner, PlannerConfig, PlannerOutcome, RuleBasedPlanner};
